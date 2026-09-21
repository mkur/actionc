mod support;
use actionc::{
    compiler::native65816,
    mir65816::{
        self,
        emit::{Label, Target},
        image::Image,
        *,
    },
    nir::{BlockId, NirBinaryOp, TempId},
    target::{AddressValue, ByteOffset, ByteSize},
};
use actionc_vm::native65816::{Inputs, Machine, Registers};
use std::collections::{BTreeMap, BTreeSet};
use support::*;

fn program(optimize: bool, ordinary: bool) -> native65816::Prepared {
    let mut p = prepare(
        "BYTE choose=$7100 CARD out=$7200 CARD FUNC Work(CARD n) IF n=0 THEN RETURN($A55A) FI RETURN($5AA5) PROC Main() out=Work(CARD(choose)) RETURN",
        optimize,
    );
    let r = p
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Work")
        .unwrap();
    if ordinary {
        let ty = r
            .temps
            .iter()
            .find(|(_, ty)| ty.width == Some(ByteSize::new(2)))
            .unwrap()
            .1
            .clone();
        let dest = TempId(r.temps.iter().map(|(id, _)| id.0).max().unwrap() + 1);
        r.temps.push((dest, ty));
        // Keep the Boolean live across an operation, forcing ordinary Branch.
        r.blocks[0].ops.push(Mir65816Op::Binary {
            dest,
            width: ByteSize::new(2),
            signed: false,
            operation: NirBinaryOp::Add,
            left: Mir65816Value::U16(0x8000),
            right: Mir65816Value::U16(1),
        });
    }
    let first = r.blocks[0].id;
    let next = r.blocks.iter().map(|b| b.id.0).max().unwrap() + 1;
    r.blocks[0].id = BlockId(next + 1);
    r.blocks.insert(
        0,
        Mir65816Block {
            id: BlockId(next),
            params: vec![],
            ops: vec![],
            terminator: Mir65816Terminator::Fallthrough,
        },
    );
    r.blocks.insert(
        0,
        Mir65816Block {
            id: first,
            params: vec![],
            ops: vec![Mir65816Op::Store {
                address: Mir65816Address {
                    base: Mir65816AddressBase::External(Mir65816ExternalAddress::Absolute(
                        AddressValue::data(0x7210),
                    )),
                    displacement: ByteOffset::ZERO,
                    index: None,
                    mode: Mir65816AddressMode::External,
                },
                value: Mir65816Value::U8(0xcc),
                width: ByteSize::ONE,
                volatile: true,
            }],
            terminator: Mir65816Terminator::Goto(Mir65816Edge {
                target: BlockId(next),
                args: vec![],
            }),
        },
    );
    mir65816::verify_program(&p.mir).unwrap();
    p
}

#[test]
fn empty_goto_fallthrough_and_both_branch_forms_execute_without_data_traffic() {
    for optimize in [false, true] {
        for ordinary in [false, true] {
            let p = program(optimize, ordinary);
            let r = p.mir.routines.iter().find(|r| r.name == "Work").unwrap();
            let compiled = p.compile(&layout()).unwrap();
            let image = Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
            let work = image.routines.iter().find(|r| r.name == "Work").unwrap();
            let machine = compiled
                .machine
                .routines
                .iter()
                .find(|m| m.id == r.id)
                .unwrap();
            let n = r.blocks.len() as u32;
            // Typed fixups identify actual MIR transfers, excluding guard and
            // conditional-dispatch JMLs. All executed bytes come from JSON.
            let mut edges = BTreeMap::new();
            for f in &machine.code.fixups {
                let Target::Label(Label(id)) = f.target else {
                    continue;
                };
                if id >= n {
                    continue;
                }
                let jump = f.offset - 1;
                assert_eq!(machine.code.bytes[jump], 0x5c);
                let prefix = jump >= 2
                    && machine
                        .code
                        .labels
                        .iter()
                        .any(|(label, &offset)| label.0 >= n && offset == jump - 2);
                let start = if prefix { jump - 2 } else { jump };
                edges.insert(
                    work.address + start as u32,
                    (
                        work.address + machine.code.labels[&Label(id)] as u32,
                        usize::from(prefix) + 1,
                    ),
                );
            }
            assert_eq!(edges.len(), 4); // Goto, Fallthrough, false and true edges.
            let mut seen = BTreeSet::new();
            let mut records = vec![];
            for value in [0, 1, 255] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), mask);
                    h.bus.ram[0x7100] = value;
                    let mut fusions = 0;
                    for _ in 0..10000 {
                        if h.cpu.is_stopped() {
                            break;
                        }
                        if h.cpu.is_instruction_boundary() {
                            if let Some(w) =
                                comparison::fused_window(&h.cpu, &h.bus, &image.routines)
                            {
                                fusions += 1;
                                assert_eq!((w.edges[0].len(), w.edges[1].len()), (1, 2));
                            }
                            if let Some(&(target, count)) = edges.get(&h.cpu.pc()) {
                                let pc = h.cpu.pc();
                                seen.insert(pc);
                                let (sites, decoded, end) = comparison::empty_edge(
                                    &h.bus,
                                    pc,
                                    work.address..work.address + work.size,
                                )
                                .unwrap();
                                assert_eq!((decoded, sites.len()), (target, count));
                                assert_eq!(end - pc, if count == 1 { 4 } else { 6 });
                                let before = h.cpu.registers();
                                let reads = h.bus.reads.len();
                                let writes = h.bus.writes.len();
                                let cycles = h.cpu.cycles();
                                assert_eq!(before.p & 0x30, 0);
                                let mut actual_sites = vec![];
                                for _ in 0..20 {
                                    if h.cpu.is_instruction_boundary() {
                                        actual_sites.push(h.cpu.pc());
                                    }
                                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                                    if h.cpu.is_instruction_boundary() && h.cpu.pc() == target {
                                        break;
                                    }
                                }
                                assert_eq!(actual_sites, sites);
                                let mut expected = before;
                                expected.pc = target as u16;
                                expected.pbr = (target >> 16) as u8;
                                assert_eq!(h.cpu.registers(), expected);
                                assert_eq!(h.cpu.cycles() - cycles, if count == 1 { 4 } else { 7 });
                                assert_eq!(h.bus.writes.len(), writes);
                                assert!(
                                    h.bus.reads[reads..]
                                        .iter()
                                        .all(|a| (work.address..work.address + work.size)
                                            .contains(a))
                                );
                                records.push(serde_json::json!({"input":value,"i":mask,"pc":pc,"target":target,"sites":sites,"cycles":h.cpu.cycles()-cycles,"data_reads":0,"writes":0}));
                                continue;
                            }
                        }
                        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    }
                    assert!(h.cpu.is_stopped());
                    h.guards(mask);
                    assert_eq!(fusions, usize::from(!ordinary));
                    assert_eq!(
                        h.bus.value(0x7200, 2),
                        if value == 0 { 0xa55a } else { 0x5aa5 }
                    );
                    assert_eq!(h.bus.value(0x7210, 1), 0xcc);
                }
            }
            assert_eq!(seen, edges.keys().copied().collect());
            if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
                let path = std::path::Path::new(&directory);
                std::fs::write(
                    path.join(format!("empty-edges-{optimize}-{ordinary}.json")),
                    image.to_json().unwrap(),
                )
                .unwrap();
                std::fs::write(
                    path.join(format!("empty-edge-traffic-{optimize}-{ordinary}.json")),
                    serde_json::to_vec_pretty(&records).unwrap(),
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn empty_edge_encodings_preserve_registers_flags_and_memory_with_either_incoming_width() {
    for restore in [false, true] {
        let source = if restore {
            "rep #$20\njml $048010"
        } else {
            "jml $048010"
        };
        let code = assemble(source, 0x040000);
        assert_eq!(
            code,
            if restore {
                vec![0xc2, 0x20, 0x5c, 0x10, 0x80, 4]
            } else {
                vec![0x5c, 0x10, 0x80, 4]
            }
        );
        for p in [0u8, 1, 2, 4, 0x40, 0x80, 0xc7, 0x20, 0xe7] {
            if !restore && p & 0x20 != 0 {
                continue;
            }
            let mut bus = Bus::new();
            bus.map(0x040000, &code, false);
            bus.map(0x048010, &[0xdb, 0xea], false);
            let before = Registers {
                a: 0xa55a,
                x: 0x8001,
                y: 0xffff,
                s: 0x5fe0,
                d: 0x2000,
                dbr: 0x12,
                pbr: 4,
                pc: 0,
                p,
                emulation_mode: false,
            };
            let mut cpu = Machine::start_at(before);
            assert!(
                cpu.run_until(
                    &mut bus,
                    20,
                    |_| Inputs::default(),
                    |cpu| cpu.is_instruction_boundary() && cpu.pc() == 0x048010
                )
                .unwrap()
            );
            let mut expected = before;
            expected.pc = 0x8010;
            expected.p &= !0x20;
            assert_eq!(cpu.registers(), expected);
            assert!(bus.writes.is_empty());
        }
    }
}

#[test]
fn empty_decoder_rejects_truncated_modes_and_targets_outside_the_routine() {
    let start = 0x040000;
    for prefix in [vec![], vec![0xc2, 0x20]] {
        let mut code = prefix.clone();
        code.extend([0x5c, 0x10, 0, 4]);
        code.resize(18, 0xea);
        let mut bus = Bus::new();
        bus.map(start, &code, false);
        assert!(comparison::empty_edge(&bus, start, start..start + 18).is_some());
        for len in 0..prefix.len() + 4 {
            assert!(comparison::empty_edge(&bus, start, start..start + len as u32).is_none());
        }
        for (offset, byte) in [(0, 0xe2), (prefix.len() + 1, 0xff), (prefix.len() + 3, 5)] {
            let mut bad = bus.clone();
            bad.ram[start as usize + offset] = byte;
            assert!(comparison::empty_edge(&bad, start, start..start + 18).is_none());
        }
        if !prefix.is_empty() {
            bus.ram[start as usize + 1] = 0x10;
            assert!(comparison::empty_edge(&bus, start, start..start + 18).is_none());
        }
    }
}
