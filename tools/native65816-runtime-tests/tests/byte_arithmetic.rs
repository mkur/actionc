mod support;
use actionc::mir65816::{Mir65816Op, Mir65816Value, emit::proof, o65 as format};
use actionc_vm::native65816::{Inputs, Machine};
use support::*;

const OPERATIONS: [(&str, &str, &str); 5] = [
    ("+", "adc", "clc\n"),
    ("-", "sbc", "sec\n"),
    ("&", "and", ""),
    ("%", "ora", ""),
    ("XOR", "eor", ""),
];

fn source(operation: &str, right: &str) -> String {
    format!(
        "VOLATILE BYTE input=$7100,second=$7101,output=$7200\nPROC Touch() RETURN\nPROC Work(BYTE x,y) Touch() output=x {operation} {right} RETURN\nPROC Main() Work(input,second) RETURN"
    )
}

fn result(operation: &str, a: u8, b: u8) -> u8 {
    match operation {
        "+" => a.wrapping_add(b),
        "-" => a.wrapping_sub(b),
        "&" => a & b,
        "%" => a | b,
        "XOR" => a ^ b,
        _ => unreachable!(),
    }
}

fn flags(operation: &str, a: u8, b: u8, before: u8) -> u8 {
    let value = result(operation, a, b);
    let mut p = (before & !0x82) | (value & 0x80) | if value == 0 { 2 } else { 0 };
    if matches!(operation, "+" | "-") {
        let (carry, overflow) = if operation == "+" {
            (
                u16::from(a) + u16::from(b) > 255,
                (!(a ^ b) & (a ^ value) & 128) != 0,
            )
        } else {
            (a >= b, ((a ^ b) & (a ^ value) & 128) != 0)
        };
        p = (p & !0x41) | u8::from(carry) | if overflow { 0x40 } else { 0 };
    }
    p | 0x20 // The selected window exits in A8.
}

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

fn immediate(source: &str, optimize: bool, rhs: u8) -> actionc::compiler::native65816::Prepared {
    // Retain identity/mask boundary operations that source optimization can
    // otherwise fold away, then verify the exact MIR fed to this selector.
    let mut p = prepare(source, optimize);
    let r = p
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Work")
        .unwrap();
    let mut changed = 0;
    for op in r.blocks.iter_mut().flat_map(|b| &mut b.ops) {
        if let Mir65816Op::Binary { right, width, .. } = op {
            assert_eq!(width.get(), 1);
            assert_eq!(*right, Mir65816Value::U8(17));
            *right = Mir65816Value::U8(rhs);
            changed += 1;
        }
    }
    assert_eq!(changed, 1);
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
}

#[test]
fn immediate_byte_arithmetic_matches_ca65_flags_and_exact_private_traffic() {
    for (operation, instruction, carry) in OPERATIONS {
        for right in [0, 1, 127, 128, 255] {
            for optimize in [false, true] {
                let source = source(operation, "17");
                let p = immediate(&source, optimize, right);
                assert_eq!(
                    p.compile(&layout()).unwrap().image.to_json().unwrap(),
                    immediate(&source.replace('\n', "\r\n"), optimize, right)
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
                    let r = c
                        .machine
                        .prepared
                        .routines
                        .iter()
                        .find(|r| r.name == "Work")
                        .unwrap();
                    let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                    let linked = c.image.routines.iter().find(|m| m.id == r.id.0).unwrap();
                    let (block, index, dest, left) = r
                        .blocks
                        .iter()
                        .find_map(|b| {
                            b.ops.iter().enumerate().find_map(|(i, op)| match op {
                                Mir65816Op::Binary {
                                    dest,
                                    width,
                                    left: Mir65816Value::Temp(left, _),
                                    right: Mir65816Value::U8(rhs),
                                    ..
                                } if width.get() == 1 && *rhs == right => {
                                    Some((b.id, i, *dest, *left))
                                }
                                _ => None,
                            })
                        })
                        .unwrap();
                    let left = m.frame.temps[&left].stack().unwrap();
                    let dest = m.frame.temps[&dest].stack().unwrap();
                    assert_eq!((left.width, dest.width), (1, 1));
                    let span = m.code.mir_spans[&(block, index)].clone();
                    let start = linked.address + span.start as u32;
                    let end = linked.address + span.end as u32;
                    let (replayed, _) = proof::replay_code(&m.code, false).unwrap();
                    proof::compare_replay_output(&m.code, &replayed).unwrap();
                    let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&[7, 11]);
                    h.bus.ram[0x71ff..0x7202].fill(0xa5);
                    reach(&mut h, start);
                    let before = h.cpu.registers();
                    let mode = if before.p & 0x20 == 0 {
                        "sep #$20\n"
                    } else {
                        ""
                    };
                    let reference = assemble(
                        &format!(
                            "{mode}.a8\nlda {},s\n{carry}{instruction} #{right}\nsta {},s",
                            left.offset, dest.offset
                        ),
                        start,
                    );
                    assert_eq!(&m.code.bytes[span.clone()], reference);
                    for a in 0..=255u8 {
                        for cv in [0, 0x41] {
                            let mut entry = before;
                            entry.a = 0xd5ab;
                            entry.p = (before.p & !0x41) | cv;
                            h.cpu = Machine::start_at(entry);
                            h.bus.ram[usize::from(entry.s + left.offset)] = a;
                            let mut independent = [0, 1, 127, 128, 255]
                                .contains(&a)
                                .then(|| (Machine::start_at(entry), h.bus.clone()));
                            let reads = h.bus.reads.len();
                            let writes = h.bus.writes.len();
                            reach(&mut h, end);
                            let expected = result(operation, a, right);
                            let mut expected_registers = entry;
                            expected_registers.a = 0xd500 | u16::from(expected);
                            expected_registers.p = flags(operation, a, right, entry.p);
                            expected_registers.pc = end as u16;
                            assert_eq!(
                                h.cpu.registers(),
                                expected_registers,
                                "{operation}/{a}/{right}/{cv}"
                            );
                            if let Some((cpu, memory)) = independent.as_mut() {
                                memory.ram[start as usize..end as usize]
                                    .copy_from_slice(&reference);
                                assert!(
                                    cpu.run_until(
                                        memory,
                                        1000,
                                        |_| Inputs::default(),
                                        |c| c.is_instruction_boundary() && c.pc() == end
                                    )
                                    .unwrap()
                                );
                                assert_eq!(h.cpu.registers(), cpu.registers());
                            }
                            assert_eq!(
                                h.bus.reads[reads..]
                                    .iter()
                                    .copied()
                                    .filter(|a| (0x4000..0x6000).contains(a))
                                    .collect::<Vec<_>>(),
                                [u32::from(entry.s + left.offset)]
                            );
                            assert_eq!(
                                h.bus.writes[writes..],
                                [(u32::from(entry.s + dest.offset), expected)]
                            );
                        }
                    }
                    h.run();
                    h.guards(0);
                    assert_eq!(h.bus.ram[0x7200], result(operation, 255, right));
                    assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7201]), (0xa5, 0xa5));
                }
            }
        }
    }
}

#[test]
fn immediate_byte_arithmetic_keeps_external_captures_and_relocates() {
    for (operation, _, _) in OPERATIONS {
        let source = source(operation, "128").replace(
            "PROC Touch() RETURN",
            "PROC Touch() input=0 second=0 RETURN",
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            let c = p.compile(&layout()).unwrap();
            let object = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..3 {
                let loaded = (variant > 0).then(|| {
                    format::relocate(
                        &object,
                        &o65::placement(&object, variant - 1, vec![o65::fault(variant - 1)]),
                    )
                    .unwrap()
                });
                let mut h = if let Some(l) = &loaded {
                    Harness::new_o65(l, &caller(l.entry()), 4)
                } else {
                    Harness::new(&c.image, &caller(c.image.entry), 4)
                };
                h.bus.ram[0x7100..0x7102].copy_from_slice(&[255, 37]);
                h.run();
                h.guards(4);
                assert_eq!(h.bus.ram[0x7200], result(operation, 255, 128));
                assert_eq!(
                    h.bus
                        .reads
                        .iter()
                        .copied()
                        .filter(|a| (0x7100..0x7102).contains(a))
                        .collect::<Vec<_>>(),
                    [0x7100, 0x7101]
                );
                assert_eq!(
                    h.bus
                        .writes
                        .iter()
                        .copied()
                        .filter(|(a, _)| [0x7100, 0x7101, 0x7200].contains(a))
                        .collect::<Vec<_>>(),
                    [
                        (0x7100, 0),
                        (0x7101, 0),
                        (0x7200, result(operation, 255, 128))
                    ]
                );
            }
        }
    }
}

#[test]
fn immediate_byte_arithmetic_restores_irq_nmi_state_after_nested_calls() {
    let source = "MODULE TEST VOLATILE BYTE irqAck=$7800,output=$7200 BYTE scratch PROC Touch() RETURN BYTE FUNC Work(BYTE value) Touch() output=value-128 output=output XOR 255 RETURN(output) CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(255) irqAck=1 RETURN(saved) PROC Task(BYTE POINTER argument) argument^=Work(argument^) RETURN PROC Main() RETURN ENDMODULE";
    windows::check_work_interrupts_with_calls(source);
}

#[test]
fn private_byte_arithmetic_exhausts_pairs_and_matches_ca65_without_scratch() {
    for (operation, instruction, carry) in OPERATIONS {
        for optimize in [false, true] {
            let source = source(operation, "y");
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
                let r = c
                    .machine
                    .prepared
                    .routines
                    .iter()
                    .find(|r| r.name == "Work")
                    .unwrap();
                let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                let linked = c.image.routines.iter().find(|m| m.id == r.id.0).unwrap();
                let (block, index, dest, left, right) = r
                    .blocks
                    .iter()
                    .find_map(|b| {
                        b.ops.iter().enumerate().find_map(|(i, op)| match op {
                            Mir65816Op::Binary {
                                dest,
                                width,
                                left: Mir65816Value::Temp(left, _),
                                right: Mir65816Value::Temp(right, _),
                                ..
                            } if width.get() == 1 => Some((b.id, i, *dest, *left, *right)),
                            _ => None,
                        })
                    })
                    .unwrap();
                let homes = [left, right, dest].map(|id| {
                    let s = m.frame.temps[&id].stack().unwrap();
                    assert_eq!(s.width, 1);
                    s.offset
                });
                let [left, right, dest] = homes;
                let span = m.code.mir_spans[&(block, index)].clone();
                let start = linked.address + span.start as u32;
                let end = linked.address + span.end as u32;
                let (replayed, _) = proof::replay_code(&m.code, false).unwrap();
                proof::compare_replay_output(&m.code, &replayed).unwrap();
                let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                h.bus.ram[0x7100..0x7102].copy_from_slice(&[7, 11]);
                h.bus.ram[0x71ff..0x7202].fill(0xa5);
                reach(&mut h, start);
                let before = h.cpu.registers();
                let mode = if before.p & 0x20 == 0 {
                    "sep #$20\n"
                } else {
                    ""
                };
                let reference = assemble(
                    &format!(
                        "{mode}.a8\nlda {left},s\n{carry}{instruction} {right},s\nsta {dest},s"
                    ),
                    start,
                );
                assert_eq!(&m.code.bytes[span], reference);
                let rights: Vec<u8> = if optimize && !guards {
                    (0..=255).collect()
                } else {
                    vec![0, 1, 127, 128, 255]
                };
                for a in 0..=255u8 {
                    for &b in &rights {
                        for cv in [0, 0x41] {
                            let mut entry = before;
                            entry.a = 0xd5ab;
                            entry.p = (entry.p & !0x41) | cv;
                            h.cpu = Machine::start_at(entry);
                            h.bus.ram[usize::from(entry.s + left)] = a;
                            h.bus.ram[usize::from(entry.s + right)] = b;
                            h.bus.reads.clear();
                            h.bus.writes.clear();
                            let mut independent = ([0, 1, 127, 128, 255].contains(&a)
                                && [0, 1, 127, 128, 255].contains(&b))
                            .then(|| (Machine::start_at(entry), h.bus.clone()));
                            reach(&mut h, end);
                            let value = result(operation, a, b);
                            let mut expected = entry;
                            expected.a = 0xd500 | u16::from(value);
                            expected.pc = end as u16;
                            expected.p = flags(operation, a, b, entry.p);
                            assert_eq!(h.cpu.registers(), expected, "{operation}/{a}/{b}/{cv}");
                            if let Some((cpu, memory)) = independent.as_mut() {
                                memory.ram[start as usize..end as usize]
                                    .copy_from_slice(&reference);
                                assert!(
                                    cpu.run_until(
                                        memory,
                                        1000,
                                        |_| Inputs::default(),
                                        |c| c.is_instruction_boundary() && c.pc() == end
                                    )
                                    .unwrap()
                                );
                                assert_eq!(h.cpu.registers(), cpu.registers());
                            }
                            assert_eq!(
                                h.bus
                                    .reads
                                    .iter()
                                    .copied()
                                    .filter(|a| (0x4000..0x6000).contains(a))
                                    .collect::<Vec<_>>(),
                                [u32::from(entry.s + left), u32::from(entry.s + right)]
                            );
                            assert_eq!(h.bus.writes, [(u32::from(entry.s + dest), value)]);
                        }
                    }
                }
                h.run();
                h.guards(0);
                assert_eq!(h.bus.ram[0x7200], result(operation, 255, 255));
                assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7201]), (0xa5, 0xa5));
            }
        }
    }
}

#[test]
fn private_byte_arithmetic_preserves_mutable_parameters_local_captures_and_relocation() {
    for (operation, _, _) in OPERATIONS {
        for local in [false, true] {
            let source = source(operation, if local { "local" } else { "y" })
                .replace(
                    "Work(BYTE x,y) Touch()",
                    "Work(BYTE x,y) BYTE local local=y Touch() y==+1",
                )
                .replace(
                    "PROC Touch() RETURN",
                    "PROC Touch() input=0 second=0 RETURN",
                );
            for optimize in [false, true] {
                let p = prepare(&source, optimize);
                let c = p.compile(&layout()).unwrap();
                let object = p.compile_o65(&Default::default()).unwrap().bytes;
                for variant in 0..3 {
                    let loaded = (variant > 0).then(|| {
                        format::relocate(
                            &object,
                            &o65::placement(&object, variant - 1, vec![o65::fault(variant - 1)]),
                        )
                        .unwrap()
                    });
                    let mut h = if let Some(l) = &loaded {
                        Harness::new_o65(l, &caller(l.entry()), 4)
                    } else {
                        Harness::new(&c.image, &caller(c.image.entry), 4)
                    };
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&[255, 37]);
                    h.run();
                    h.guards(4);
                    let expected = result(operation, 255, if local { 37 } else { 38 });
                    assert_eq!(h.bus.ram[0x7200], expected);
                    assert_eq!(
                        h.bus
                            .reads
                            .iter()
                            .copied()
                            .filter(|a| (0x7100..0x7102).contains(a))
                            .collect::<Vec<_>>(),
                        [0x7100, 0x7101]
                    );
                    assert_eq!(
                        h.bus
                            .writes
                            .iter()
                            .copied()
                            .filter(|(a, _)| [0x7100, 0x7101, 0x7200].contains(a))
                            .collect::<Vec<_>>(),
                        [(0x7100, 0), (0x7101, 0), (0x7200, expected)]
                    );
                }
            }
        }
    }
}

#[test]
fn private_byte_arithmetic_restores_irq_nmi_after_calls_and_parameter_mutation() {
    let source = "MODULE TEST VOLATILE BYTE irqAck=$7800,output=$7200 BYTE scratch PROC Touch() RETURN BYTE FUNC Work(BYTE value,rhs) Touch() rhs==+1 output=value-rhs output=output XOR rhs RETURN(output) CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(255,127) irqAck=1 RETURN(saved) PROC Task(BYTE POINTER argument) argument^=Work(argument^,255) RETURN PROC Main() RETURN ENDMODULE";
    windows::check_work_interrupts_with_calls(source);
}
