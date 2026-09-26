mod support;
use actionc::mir65816::emit::proof::{
    self, EffectAccess as Access, EffectControl as Control, EffectMemory as Memory,
    effect_env as env,
};
use actionc_vm::native65816::{Inputs, Machine, Registers};
use std::collections::BTreeSet;
use support::*;

fn addresses(memory: Memory, r: Registers, bus: &Bus) -> Vec<u32> {
    match memory {
        Memory::Stack {
            displacement,
            bytes,
        } => (0..bytes)
            .map(|i| u32::from(r.s.wrapping_add(displacement as u16).wrapping_add(i)))
            .collect(),
        Memory::DirectPage { offset, bytes } => (0..bytes)
            .map(|i| u32::from(r.d.wrapping_add(offset).wrapping_add(i)))
            .collect(),
        Memory::Long { address, bytes } => (0..bytes)
            .map(|i| (address + u32::from(i)) & 0xffffff)
            .collect(),
        Memory::IndirectLong {
            pointer,
            indexed_y,
            bytes,
        } => {
            let pointer = (0..3).fold(0, |a, i| {
                a | (u32::from(
                    bus.ram[r.d.wrapping_add(u16::from(pointer)).wrapping_add(i) as usize],
                ) << (8 * i))
            });
            let base = pointer + if indexed_y { u32::from(r.y) } else { 0 };
            (0..bytes)
                .map(|i| (base + u32::from(i)) & 0xffffff)
                .collect()
        }
        _ => panic!("probe must have independently resolvable memory"),
    }
}

#[test]
fn declared_effects_cover_actual_bus_accesses_and_preserve_every_unwritten_bit() {
    let mut probes = vec![proof::memory_probe(false).0, proof::memory_probe(true).0];
    for value in [0, 0x7f, 0xff, 0x7fff, 0x8000, 0xffff] {
        for byte_a in [false, true] {
            for byte_x in [false, true] {
                probes.push(proof::increment_instruction_probe(value, byte_a, byte_x).0);
                probes.push(proof::index_instruction_probe(value, byte_a, byte_x).0);
                probes.push(proof::immediate_push_probe(value, byte_a, byte_x).0);
            }
        }
    }
    for code in probes {
        for irq in [0, 4] {
            for flags in [0, 1, 2, 0x40, 0x80, 0xc3] {
                let mut bus = Bus::new();
                let mut bytes = code.bytes.clone();
                bytes.extend([0xdb, 0xea]);
                bus.map(0x40000, &bytes, false);
                bus.map(0x4000, &[0; 0x2000], true);
                bus.map(0x2000, &[0; 256], true);
                bus.ram[0x2000..0x2003].copy_from_slice(&[0, 0, 0x12]);
                bus.map(0x120000, &[0x55, 0xaa, 0, 0x80, 0xff, 0x7f, 0, 0], true);
                let mut cpu = Machine::start_at(Registers {
                    a: 0xabcd,
                    x: 0x1234,
                    y: 0x5678,
                    s: 0x5fe0,
                    d: 0x2000,
                    dbr: 0,
                    pbr: 4,
                    pc: 0,
                    p: irq | flags,
                    emulation_mode: false,
                });
                for record in proof::instruction_effects(&code) {
                    assert_eq!(cpu.pc(), 0x40000 + record.start as u32);
                    let before = cpu.registers();
                    let e = &record.effects;
                    assert_eq!(e.control, Control::Next);
                    let mut reads = BTreeSet::new();
                    let mut writes = BTreeSet::new();
                    for access in &e.memory {
                        let set = if access.access == Access::Read {
                            &mut reads
                        } else {
                            &mut writes
                        };
                        set.extend(addresses(access.memory, before, &bus));
                    }
                    bus.reads.clear();
                    bus.writes.clear();
                    cpu.tick(&mut bus, Inputs::default()).unwrap();
                    for _ in 0..100 {
                        if cpu.is_instruction_boundary() {
                            break;
                        }
                        cpu.tick(&mut bus, Inputs::default()).unwrap();
                    }
                    assert!(cpu.is_instruction_boundary());
                    assert_eq!(cpu.pc(), 0x40000 + record.end as u32);
                    let after = cpu.registers();
                    for (a, b, mask) in [
                        (before.a, after.a, e.writes.a | e.clobbers.a),
                        (before.x, after.x, e.writes.x | e.clobbers.x),
                        (before.y, after.y, e.writes.y | e.clobbers.y),
                    ] {
                        assert_eq!((a ^ b) & !mask, 0, "register at {record:?}");
                    }
                    assert_eq!(
                        (before.p ^ after.p) & 0xc3 & !(e.flag_writes | e.flag_clobbers),
                        0,
                        "flags at {record:?}"
                    );
                    for (mask, part) in [
                        (0x20, env::M),
                        (0x10, env::X),
                        (8, env::DECIMAL),
                        (4, env::I),
                    ] {
                        if e.environment_writes & part == 0 {
                            assert_eq!((before.p ^ after.p) & mask, 0, "status at {record:?}");
                        }
                    }
                    if e.environment_writes & env::S == 0 {
                        assert_eq!(before.s, after.s);
                    }
                    if e.environment_writes & env::D == 0 {
                        assert_eq!(before.d, after.d);
                    }
                    if e.environment_writes & env::DBR == 0 {
                        assert_eq!(before.dbr, after.dbr);
                    }
                    if e.environment_writes & env::PBR == 0 {
                        assert_eq!(before.pbr, after.pbr);
                    }
                    assert!(!after.emulation_mode);
                    let actual_reads: BTreeSet<_> = bus
                        .reads
                        .iter()
                        .copied()
                        .filter(|a| !(0x40000..0x40000 + bytes.len() as u32).contains(a))
                        .collect();
                    let actual_writes: BTreeSet<_> = bus.writes.iter().map(|&(a, _)| a).collect();
                    assert_eq!(actual_reads, reads, "reads at {record:?}");
                    assert_eq!(actual_writes, writes, "writes at {record:?}");
                }
            }
        }
    }
}

#[test]
fn complete_effect_coverage_survives_layout_and_does_not_change_images() {
    use actionc::mir65816::{emit, image};
    let cases = [
        "identity",
        "add",
        "subtract",
        "constant_chain",
        "maximum",
        "wide_shift",
        "loop_rotation",
        "sum_loop",
        "recursive_sum",
        "direct_calls",
        "byte_sum",
        "record_field",
        "unlink",
        "forward_copy",
    ];
    let mut summary = Vec::new();
    for case in cases {
        for optimize in [false, true] {
            let prepared = prepare(&fixture(&format!("code_quality/{case}.act")), optimize);
            let plain = emit::materialize(&prepared.mir).unwrap();
            let (traced, _) = proof::materialize_with_trace(&prepared.mir).unwrap();
            assert_eq!(
                image::link(&prepared.mir, &plain, &layout())
                    .unwrap()
                    .to_json()
                    .unwrap(),
                image::link(&prepared.mir, &traced, &layout())
                    .unwrap()
                    .to_json()
                    .unwrap()
            );
            let mut count = 0;
            for routine in &traced.routines {
                let mut cursor = 0;
                for record in proof::instruction_effects(&routine.code) {
                    assert_eq!(record.start, cursor, "{case}/{optimize}");
                    assert!(record.end > record.start && record.end <= routine.code.bytes.len());
                    cursor = record.end;
                    if let Control::Call { .. } = record.effects.control {
                        assert_eq!(
                            record.effects.reads,
                            proof::EffectRegisters::default(),
                            "production calls require their verified ABI summary"
                        );
                        assert!(record.effects.barrier);
                    }
                    if let Control::Branch { predicate, target } = record.effects.control {
                        let destination = routine.code.labels[&target];
                        let pc = record.start;
                        if record.end - pc == 2 {
                            assert_eq!(routine.code.bytes[pc], predicate);
                            assert_eq!(
                                (pc as isize + 2 + isize::from(routine.code.bytes[pc + 1] as i8))
                                    as usize,
                                destination
                            );
                        } else {
                            assert_eq!(
                                &routine.code.bytes[pc..pc + 3],
                                &[predicate ^ 0x20, 4, 0x5c]
                            );
                        }
                    }
                    count += 1;
                }
                assert_eq!(cursor, routine.code.bytes.len());
            }
            summary.push(serde_json::json!({"case":case,"optimized":optimize,"effect_records":count,"complete":true,"image_equal":true}));
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("instruction-effects-summary.json"),
            serde_json::to_string_pretty(&summary).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn verified_direct_and_indirect_calls_keep_declared_result_lanes() {
    for (ty, value, bytes) in [
        ("BYTE", "$B7", 1),
        ("CARD", "$8001", 2),
        ("ADDRESS", "$834567", 3),
        ("LONGINT", "LONGINT($89ABCDEF)", 4),
    ] {
        for optimize in [false, true] {
            let source = format!(
                "{ty} direct,indirect {ty} FUNC POINTER cb({ty} value) \
                 {ty} FUNC Echo({ty} value) RETURN(value) \
                 PROC Main() direct=Echo({value}) cb=@Echo indirect=cb({value}) Echo({value}) RETURN"
            );
            let prepared = prepare(&source, optimize);
            let (module, _) = proof::materialize_with_trace(&prepared.mir).unwrap();
            let mut calls = [0, 0];
            let mut returns = 0;
            for routine in &module.routines {
                for record in proof::instruction_effects(&routine.code) {
                    let e = &record.effects;
                    let expected = proof::EffectRegisters {
                        a: 0xffff,
                        x: if bytes >= 3 { 0xffff } else { 0 },
                        y: 0,
                    };
                    match e.control {
                        Control::Call { target } => {
                            let indirect = target.is_none();
                            calls[usize::from(indirect)] += 1;
                            assert_eq!(e.writes, expected);
                            assert_eq!(e.clobbers.a, 0);
                            assert_eq!(e.clobbers.x, !expected.x);
                            assert_eq!(e.clobbers.y, 0xffff);
                            assert_eq!(e.reads, proof::EffectRegisters::default());
                            assert_eq!(e.memory[1].access, Access::Read);
                            assert_eq!(
                                e.memory[1].memory,
                                Memory::Stack {
                                    displacement: if indirect { 7 } else { 1 },
                                    bytes,
                                }
                            );
                        }
                        Control::Return if e.reads.a != 0 => {
                            assert_eq!(e.reads, expected);
                            returns += 1;
                        }
                        _ => {}
                    }
                }
            }
            // The discarded direct result retains the full declared ABI effects.
            assert_eq!(calls, [2, 1]);
            assert_eq!(returns, 1);
        }
    }
}
