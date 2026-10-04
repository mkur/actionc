mod support;
use actionc::mir65816::{Mir65816Op, emit::proof};
use actionc_vm::native65816::{Inputs, Machine};
use support::*;

fn reach(h: &mut Harness, pc: u32) {
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                100_000,
                |_| Inputs::default(),
                |c| c.is_instruction_boundary() && c.pc() == pc
            )
            .unwrap()
    );
}

#[test]
fn adjacent_long_reads_match_independent_arithmetic_and_exact_private_traffic() {
    for local in [false, true] {
        for (operator, mnemonic, carry) in [
            ("+", "adc", "clc"),
            ("-", "sbc", "sec"),
            ("&", "and", ""),
            ("%", "ora", ""),
            ("XOR", "eor", ""),
        ] {
            let source = format!(
                "LONGINT input=$7100,result=$7200\nPROC Touch() RETURN\nLONGINT FUNC Work(LONGINT x,y) {} RETURN(x {operator} {})\nPROC Main() result=Work(input,LONGINT($00010001)) RETURN",
                if local {
                    "LONGINT saved saved=y Touch()"
                } else {
                    ""
                },
                if local { "saved" } else { "y" }
            );
            for optimize in [false, true] {
                let p = prepare(&source, optimize);
                let (reference, rt) = proof::materialize_reference(&p.mir, true).unwrap();
                let (replayed, pt) = proof::materialize_replayed(&p.mir, true).unwrap();
                for ((a, b), (x, y)) in reference
                    .routines
                    .iter()
                    .zip(&replayed.routines)
                    .zip(rt.iter().zip(&pt))
                {
                    proof::compare_replay_output(&a.code, &b.code).unwrap();
                    assert_eq!(x.snapshots, y.snapshots);
                }
                for guards in [false, true] {
                    let mut options = layout();
                    options.stack_checks = guards;
                    let c = p.compile(&options).unwrap();
                    let r = p.mir.routines.iter().find(|r| r.name == "Work").unwrap();
                    let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                    let linked = c.image.routines.iter().find(|m| m.id == r.id.0).unwrap();
                    let reads: Vec<_> = proof::scalar_reads(r, &m.frame)
                        .unwrap()
                        .into_iter()
                        .filter(|b| {
                            matches!(
                                r.blocks
                                    .iter()
                                    .find(|block| block.id == b.block)
                                    .unwrap()
                                    .ops[b.consumer],
                                Mir65816Op::Binary { .. }
                            )
                        })
                        .collect();
                    assert_eq!(reads.len(), 1);
                    let binding = &reads[0];
                    assert_eq!(
                        binding.offset,
                        scalar_read_home(r, m, binding.block, binding.consumer, binding.temp)
                    );
                    assert_eq!(
                        matches!(binding.source, proof::HomeOwner::FrameObject(_)),
                        local
                    );
                    assert!(m.code.mir_spans[&(binding.block, binding.producer)].is_empty());
                    let block = r.blocks.iter().find(|b| b.id == binding.block).unwrap();
                    let Mir65816Op::Binary {
                        dest,
                        left: actionc::mir65816::Mir65816Value::Temp(left, _),
                        ..
                    } = block.ops[binding.consumer]
                    else {
                        panic!()
                    };
                    let dest = m.frame.temps[&dest].stack().unwrap().offset;
                    let left = m.frame.temps[&left].stack().unwrap().offset;
                    let span = &m.code.mir_spans[&(binding.block, binding.consumer)];
                    let (start, end) = (
                        linked.address + span.start as u32,
                        linked.address + span.end as u32,
                    );
                    let asm = format!(
                        "lda {left},s\n{carry}\n{mnemonic} {},s\nsta {dest},s\nlda {},s\n{mnemonic} {},s\nsta {},s\n",
                        binding.offset,
                        left + 2,
                        binding.offset + 2,
                        dest + 2
                    );
                    let expected_code = assemble(&asm, start);
                    assert_eq!(&m.code.bytes[span.clone()], expected_code);
                    let (replayed, _) = proof::replay_code(&m.code, false).unwrap();
                    proof::compare_replay_output(&m.code, &replayed).unwrap();
                    for value in [0u32, 1, 0xffff, 0x10000, 0x7fffffff, 0x80000000, u32::MAX] {
                        let expected = match operator {
                            "+" => value.wrapping_add(0x10001),
                            "-" => value.wrapping_sub(0x10001),
                            "&" => value & 0x10001,
                            "%" => value | 0x10001,
                            _ => value ^ 0x10001,
                        };
                        let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                        h.bus.ram[0x7100..0x7104].copy_from_slice(&value.to_le_bytes());
                        h.bus.ram[0x71ff..0x7205].fill(0xa5);
                        reach(&mut h, start);
                        let before = h.cpu.registers();
                        let mut independent = Machine::start_at(before);
                        let mut memory = h.bus.clone();
                        memory.ram[start as usize..end as usize].copy_from_slice(&expected_code);
                        let read_at = h.bus.reads.len();
                        let write_at = h.bus.writes.len();
                        reach(&mut h, end);
                        assert!(
                            independent
                                .run_until(
                                    &mut memory,
                                    1000,
                                    |_| Inputs::default(),
                                    |c| c.is_instruction_boundary() && c.pc() == end
                                )
                                .unwrap()
                        );
                        assert_eq!(h.cpu.registers(), independent.registers());
                        let base = u32::from(before.s);
                        assert_eq!(
                            h.bus.reads[read_at..]
                                .iter()
                                .copied()
                                .filter(|a| (0x4000..0x6000).contains(a))
                                .collect::<Vec<_>>(),
                            [0u32, 2]
                                .into_iter()
                                .flat_map(|i| [
                                    base + u32::from(left) + i,
                                    base + u32::from(left) + i + 1,
                                    base + u32::from(binding.offset) + i,
                                    base + u32::from(binding.offset) + i + 1
                                ])
                                .collect::<Vec<_>>()
                        );
                        assert_eq!(
                            h.bus.writes[write_at..],
                            expected
                                .to_le_bytes()
                                .into_iter()
                                .enumerate()
                                .map(|(i, v)| (base + u32::from(dest) + i as u32, v))
                                .collect::<Vec<_>>()
                        );
                        h.run();
                        h.guards(0);
                        assert_eq!(h.bus.value(0x7200, 4), expected);
                        assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7204]), (0xa5, 0xa5));
                    }
                }
            }
        }
    }
}

#[test]
fn adjacent_signed_and_unsigned_comparisons_cover_both_roles_and_boundary_values() {
    for ty in ["LONGCARD", "LONGINT"] {
        let source = format!(
            "{ty} input=$7100\nBYTE ARRAY out=$7200\nPROC Work({ty} x) out(0)=x<{ty}($80000000) out(1)={ty}($80000000)<x out(2)=x={ty}($80000000) out(3)=x#{ty}($80000000) out(4)=x>={ty}($80000000) out(5)=x<={ty}($80000000) RETURN PROC Main() Work(input) RETURN"
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            assert_eq!(
                p.compile(&layout()).unwrap().image.to_json().unwrap(),
                prepare(&source.replace('\n', "\r\n"), optimize)
                    .compile(&layout())
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            for guards in [false, true] {
                let mut options = layout();
                options.stack_checks = guards;
                let c = p.compile(&options).unwrap();
                let r = p.mir.routines.iter().find(|r| r.name == "Work").unwrap();
                let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                assert!(!proof::scalar_reads(r, &m.frame).unwrap().is_empty());
                for value in [0u32, 1, 0x7fffffff, 0x80000000, 0x80000001, u32::MAX] {
                    let mut h = Harness::new(&c.image, &caller(c.image.entry), 4);
                    h.bus.ram[0x7100..0x7104].copy_from_slice(&value.to_le_bytes());
                    h.run();
                    h.guards(4);
                    let less = if ty == "LONGINT" {
                        (value as i32) < i32::MIN
                    } else {
                        value < 0x80000000
                    };
                    let greater = if ty == "LONGINT" {
                        (value as i32) > i32::MIN
                    } else {
                        value > 0x80000000
                    };
                    let equal = value == 0x80000000;
                    assert_eq!(
                        &h.bus.ram[0x7200..0x7206],
                        &[
                            less as u8,
                            greater as u8,
                            equal as u8,
                            !equal as u8,
                            !less as u8,
                            !greater as u8
                        ]
                    );
                }
            }
        }
    }
}

#[test]
fn borrowed_long_operands_restore_full_state_at_every_irq_nmi_boundary() {
    for local in [false, true] {
        let source = format!(
            "MODULE TEST VOLATILE BYTE irqAck=$7800 LONGINT scratch PROC Touch() RETURN LONGINT FUNC Work(LONGINT x) {} RETURN(LONGINT($10001)+{}) CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(3) irqAck=1 RETURN(saved) PROC Task(LONGINT POINTER argument) argument^=Work(argument^) RETURN PROC Main() RETURN ENDMODULE",
            if local {
                "LONGINT saved saved=x Touch()"
            } else {
                ""
            },
            if local { "saved" } else { "x" }
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            let c = p.compile(&layout()).unwrap();
            let address = context::routine(&c.image, "Work");
            let linked = c
                .image
                .routines
                .iter()
                .find(|r| r.address == address)
                .unwrap();
            let m = c
                .machine
                .routines
                .iter()
                .find(|m| m.id.0 == linked.id)
                .unwrap();
            let r = p.mir.routines.iter().find(|r| r.id == m.id).unwrap();
            assert!(
                proof::scalar_reads(r, &m.frame)
                    .unwrap()
                    .iter()
                    .any(|b| matches!(b.source, proof::HomeOwner::FrameObject(_)) == local)
            );
        }
        windows::check_work_interrupts_with_calls(&source);
    }
}

#[test]
fn terminal_arguments_and_stores_preserve_values_padding_banked_traffic_and_relocation() {
    use actionc::mir65816::{Mir65816Value, o65 as format};
    use actionc_vm::native65816::Access;
    for (ty, bytes, value) in [
        ("BYTE", 1, 0x81u32),
        ("CARD", 2, 0x80ff),
        ("LONGINT", 4, 0x89abcdef),
    ] {
        for local in [false, true] {
            let source = format!(
                "BYTE tagOut=$7208 CARD markerOut=$720A {ty} result=$7200\nPROC Touch() RETURN\n{ty} FUNC Sink(BYTE tag CARD marker {ty} v) tagOut=tag markerOut=marker RETURN(v)\n{ty} FUNC Work({ty} x) {} RETURN(Sink(7,$CAFE,{}))\nPROC Main() result=Work({ty}(${value:X})) RETURN",
                if local {
                    format!("{ty} saved saved=x Touch()")
                } else {
                    String::new()
                },
                if local { "saved" } else { "x" }
            );
            for optimize in [false, true] {
                let p = prepare(&source, optimize);
                for guards in [false, true] {
                    let mut options = layout();
                    options.stack_checks = guards;
                    let c = p.compile(&options).unwrap();
                    let r = p.mir.routines.iter().find(|r| r.name == "Work").unwrap();
                    let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                    let reads = proof::scalar_reads(r, &m.frame).unwrap();
                    let binding = reads
                        .iter()
                        .find(|b| {
                            matches!(
                                r.blocks
                                    .iter()
                                    .find(|block| block.id == b.block)
                                    .unwrap()
                                    .ops[b.consumer],
                                Mir65816Op::Call { .. }
                            )
                        })
                        .unwrap();
                    assert_eq!(u32::from(binding.bytes), bytes);
                    assert_eq!(
                        matches!(binding.source, proof::HomeOwner::FrameObject(_)),
                        local
                    );
                    assert!(m.code.mir_spans[&(binding.block, binding.producer)].is_empty());
                    scalar_read_home(r, m, binding.block, binding.consumer, binding.temp);
                    let (replayed, _) = proof::replay_code(&m.code, false).unwrap();
                    proof::compare_replay_output(&m.code, &replayed).unwrap();
                    let linked = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
                    let span = &m.code.mir_spans[&(binding.block, binding.consumer)];
                    let sink = context::routine(&c.image, "Sink");
                    let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                    h.bus.ram[0x71ff..0x7205].fill(0xa5);
                    reach(&mut h, linked.address + span.start as u32);
                    let body_s = u32::from(h.cpu.registers().s);
                    let read_at = h.bus.reads.len();
                    reach(&mut h, sink);
                    let block = r
                        .blocks
                        .iter()
                        .find(|block| block.id == binding.block)
                        .unwrap();
                    let Mir65816Op::Call { plan, args, .. } = &block.ops[binding.consumer] else {
                        panic!()
                    };
                    let outgoing_s = body_s - plan.outgoing_bytes.get();
                    assert_eq!(u32::from(h.cpu.registers().s), outgoing_s - 3);
                    let mut expected = vec![0; plan.outgoing_bytes.get() as usize];
                    for (argument, home) in args.iter().zip(&plan.arguments) {
                        let actionc::mir65816::Mir65816AbiHome::StackArgument {
                            offset, size, ..
                        } = home
                        else {
                            panic!()
                        };
                        let v = match argument {
                            Mir65816Value::U8(v) => u32::from(*v),
                            Mir65816Value::U16(v) => u32::from(*v),
                            _ => value,
                        };
                        expected[offset.get() as usize..(offset.get() + size.get()) as usize]
                            .copy_from_slice(&v.to_le_bytes()[..size.get() as usize]);
                    }
                    assert_eq!(
                        &h.bus.ram[outgoing_s as usize + 1..body_s as usize + 1],
                        expected
                    );
                    let source_start = body_s + u32::from(binding.offset);
                    let mut reads: Vec<_> = h.bus.reads[read_at..]
                        .iter()
                        .copied()
                        .filter(|a| (source_start..source_start + bytes).contains(a))
                        .collect();
                    reads.sort_unstable();
                    assert_eq!(
                        reads,
                        (source_start..source_start + bytes).collect::<Vec<_>>()
                    );
                    h.run();
                    h.guards(0);
                    assert_eq!(h.bus.value(0x7200, bytes as usize), value);
                    assert_eq!(h.bus.value(0x7208, 1), 7);
                    assert_eq!(h.bus.value(0x720a, 2), 0xcafe);
                    assert_eq!(
                        (h.bus.ram[0x71ff], h.bus.ram[0x7200 + bytes as usize]),
                        (0xa5, 0xa5)
                    );
                }
            }
        }
    }
    // Exact four-byte terminal stores through a pointer crossing a bank edge.
    for local in [false, true] {
        let source = format!(
            "PROC Touch() RETURN PROC Work(LONGCARD POINTER p LONGCARD x) {} p^={} RETURN PROC Main() Work(LONGCARD POINTER($12FFFE),$89ABCDEF) RETURN",
            if local {
                "LONGCARD saved saved=x Touch()"
            } else {
                ""
            },
            if local { "saved" } else { "x" }
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            let c = p.compile(&layout()).unwrap();
            let r = p.mir.routines.iter().find(|r| r.name == "Work").unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            assert!(proof::scalar_reads(r, &m.frame).unwrap().iter().any(|b| {
                matches!(
                    r.blocks
                        .iter()
                        .find(|block| block.id == b.block)
                        .unwrap()
                        .ops[b.consumer],
                    Mir65816Op::Store { .. }
                )
            }));
            let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..3 {
                let loaded = (variant > 0).then(|| {
                    format::relocate(
                        &bytes,
                        &o65::placement(&bytes, variant - 1, vec![o65::fault(variant - 1)]),
                    )
                    .unwrap()
                });
                let mut h = if let Some(loaded) = &loaded {
                    Harness::new_o65(loaded, &caller(loaded.entry()), 4)
                } else {
                    Harness::new(&c.image, &caller(c.image.entry), 4)
                };
                h.bus.map(0x12fffd, &[0xa5; 6], true);
                h.bus.watched.extend(0x12fffd..0x130003);
                h.run();
                h.guards(4);
                assert_eq!(h.bus.value(0x12fffe, 4), 0x89abcdef);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, a, k)| (a, k))
                        .collect::<Vec<_>>(),
                    (0x12fffe..0x130002)
                        .zip(0x89abcdefu32.to_le_bytes())
                        .map(|(a, byte)| (a, Access::Write(byte)))
                        .collect::<Vec<_>>()
                );
                assert_eq!((h.bus.ram[0x12fffd], h.bus.ram[0x130002]), (0xa5, 0xa5));
            }
        }
    }
}

#[test]
fn terminal_scalar_consumers_survive_irq_and_nmi_before_and_after_nested_calls() {
    let source = "MODULE TEST VOLATILE BYTE irqAck=$7800 LONGINT scratch PROC Touch() RETURN LONGINT FUNC Sink(BYTE tag CARD marker LONGINT x) RETURN(x) LONGINT FUNC Work(LONGINT x) LONGINT saved saved=x Touch() RETURN(Sink(7,$CAFE,saved)) CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(3) irqAck=1 RETURN(saved) PROC Task(LONGINT POINTER argument) argument^=Work(argument^) RETURN PROC Main() RETURN ENDMODULE";
    windows::check_work_interrupts_with_calls(source);
}
