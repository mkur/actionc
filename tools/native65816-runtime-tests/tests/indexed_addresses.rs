mod support;
use actionc::mir65816::{
    Mir65816AbiHome, Mir65816AddressBase, Mir65816Op, Mir65816Value, emit::proof, o65 as format,
};
use actionc::target::{ByteOffset, ByteSize};
use actionc_vm::native65816::{Inputs, Machine};
use support::*;

const CONSTANT: &str = "BYTE POINTER input=$7100\nADDRESS output=$7200\nPROC Touch() RETURN\nADDRESS FUNC Work(BYTE POINTER p) ADDRESS value value=ADDRESS(@p(0)) Touch() RETURN(value)\nPROC Main() output=Work(input) RETURN";

fn shaped(
    source: &str,
    optimize: bool,
    index: u32,
    stride: u32,
    displacement: u32,
) -> actionc::compiler::native65816::Prepared {
    shape(prepare(source, optimize), Some(index), stride, displacement)
}

fn shape(
    mut p: actionc::compiler::native65816::Prepared,
    index: Option<u32>,
    stride: u32,
    displacement: u32,
) -> actionc::compiler::native65816::Prepared {
    let mut changed = 0;
    for r in &mut p.mir.routines {
        if !r.name.ends_with("Work") && !r.name.contains("_WORK_") {
            continue;
        }
        for op in r.blocks.iter_mut().flat_map(|b| &mut b.ops) {
            if let Mir65816Op::AddressOf { address, .. } = op {
                if let Some(i) = &mut address.index {
                    if let Some(index) = index {
                        i.value = Mir65816Value::U32(index);
                    }
                    i.stride = ByteSize::new(stride);
                    address.displacement = ByteOffset::new(displacement);
                    changed += 1;
                }
            }
        }
    }
    assert_eq!(changed, 1);
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
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

// Independent source identity: a retained allocation or an omitted immutable
// parameter/local capture. Do not use emitter forwarding witnesses as an oracle.
fn pointer_home(
    r: &actionc::mir65816::Mir65816Routine,
    m: &actionc::mir65816::emit::MachineRoutine,
    value: &Mir65816Value,
) -> u16 {
    let Mir65816Value::Temp(id, width) = value else {
        panic!("captured pointer");
    };
    assert_eq!(width.get(), 3);
    if let Some(home) = m.frame.temps.get(id) {
        let home = home.stack().unwrap();
        assert_eq!(home.width, 3);
        return home.offset;
    }
    let (b, i, address) = r
        .blocks
        .iter()
        .find_map(|b| {
            b.ops.iter().enumerate().find_map(|(i, op)| match op {
                Mir65816Op::Load {
                    dest,
                    width,
                    address,
                    volatile: false,
                } if dest == id && width.get() == 3 => Some((b.id, i, address)),
                _ => None,
            })
        })
        .unwrap();
    assert!(m.code.mir_spans[&(b, i)].is_empty());
    assert!(address.index.is_none());
    assert_eq!(address.displacement.get(), 0);
    match address.base {
        Mir65816AddressBase::Parameter(id) => {
            let p = r.frame.parameters.iter().find(|p| p.param == id).unwrap();
            assert!(p.frame_object.is_none());
            let Mir65816AbiHome::StackArgument { offset, size, .. } = p.incoming else {
                panic!();
            };
            assert_eq!(size.get(), 3);
            u16::try_from(u32::from(m.frame.extent) + 4 + offset.get()).unwrap()
        }
        Mir65816AddressBase::AutomaticFrame(id) => {
            let o = r.frame.objects.iter().find(|o| o.id == id).unwrap();
            assert_eq!(o.size.get(), 3);
            assert!(!o.addressable);
            u16::try_from(o.stack_offset.get()).unwrap()
        }
        _ => panic!("private source"),
    }
}

#[test]
fn constant_indexed_addresses_match_ca65_registers_and_exact_private_traffic() {
    for (index, stride, displacement) in [
        (0, 1, 0),
        (0, 128, 0),
        (3, 3, 17),
        (255, 257, 0),
        (1, 1, 65534),
    ] {
        let offset = index * stride + displacement;
        for optimize in [false, true] {
            let p = shaped(CONSTANT, optimize, index, stride, displacement);
            assert_eq!(
                p.compile(&layout()).unwrap().image.to_json().unwrap(),
                shaped(
                    &CONSTANT.replace('\n', "\r\n"),
                    optimize,
                    index,
                    stride,
                    displacement
                )
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
                let linked = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
                let (block, i, dest, base) = r
                    .blocks
                    .iter()
                    .find_map(|b| {
                        b.ops.iter().enumerate().find_map(|(i, op)| match op {
                            Mir65816Op::AddressOf { dest, address, .. }
                                if address.index.is_some() =>
                            {
                                let Mir65816AddressBase::Indirect(base) = &address.base else {
                                    panic!();
                                };
                                Some((b.id, i, *dest, base))
                            }
                            _ => None,
                        })
                    })
                    .unwrap();
                let src = pointer_home(r, m, base);
                let dst = m.frame.temps[&dest].stack().unwrap().offset;
                assert!(src.abs_diff(dst) >= 3);
                let span = m.code.mir_spans[&(block, i)].clone();
                let start = linked.address + span.start as u32;
                let end = linked.address + span.end as u32;
                let (replayed, _) = proof::replay_code(&m.code, false).unwrap();
                proof::compare_replay_output(&m.code, &replayed).unwrap();
                let mut assembled = None;
                for base in [0u32, 1, 0xffff, 0x12fffe, 0xfffffe, 0xffffff] {
                    let expected = (base + offset) & 0xffffff;
                    let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                    h.bus.ram[0x71ff..0x7204].fill(0xa5);
                    reach(&mut h, start);
                    let mut before = h.cpu.registers();
                    before.a = 0xd5ab;
                    h.cpu = Machine::start_at(before);
                    let expected_code = assembled.get_or_insert_with(|| {
                        let mut asm = if before.p & 0x20 != 0 { "rep #$20\n".to_string() } else { String::new() };
                        if offset == 0 { asm += &format!("lda {src},s\nsta {dst},s\nlda {},s\nsta {},s\n", src+1, dst+1); }
                        else { asm += &format!("lda {src},s\nclc\nadc #{offset}\nsta {dst},s\nsep #$20\n.a8\nlda {},s\nadc #0\nsta {},s\n", src+2, dst+2); }
                        assemble(&asm, start)
                    });
                    assert_eq!(&m.code.bytes[span.clone()], expected_code);
                    let mut independent = Machine::start_at(before);
                    let mut memory = h.bus.clone();
                    memory.ram[start as usize..end as usize].copy_from_slice(expected_code);
                    let reads = h.bus.reads.len();
                    let writes = h.bus.writes.len();
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
                    let read_bytes: Vec<u32> = if offset == 0 {
                        vec![0, 1, 1, 2]
                    } else {
                        vec![0, 1, 2]
                    };
                    let private: Vec<_> = h.bus.reads[reads..]
                        .iter()
                        .copied()
                        .filter(|a| (0x4000..0x6000).contains(a))
                        .collect();
                    assert_eq!(
                        private,
                        read_bytes
                            .iter()
                            .map(|i| u32::from(before.s) + u32::from(src) + i)
                            .collect::<Vec<_>>()
                    );
                    assert_eq!(
                        h.bus.writes[writes..],
                        read_bytes
                            .iter()
                            .map(|i| (
                                u32::from(before.s) + u32::from(dst) + i,
                                (expected >> (i * 8)) as u8
                            ))
                            .collect::<Vec<_>>()
                    );
                    h.run();
                    h.guards(0);
                    assert_eq!(h.bus.value(0x7200, 3), expected);
                    assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7203]), (0xa5, 0xa5));
                }
            }
        }
    }
}

#[test]
fn constant_addresses_keep_fallbacks_relocation_and_no_pointee_access() {
    for (index, stride, displacement) in [
        (0, 1, 0),
        (3, 3, 17),
        (255, 257, 0),
        (256, 256, 0),
        (1, 1, 65535),
    ] {
        for optimize in [false, true] {
            let p = shaped(CONSTANT, optimize, index, stride, displacement);
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
                let base = 0xfffffeu32;
                let expected = (base + index * stride + displacement) & 0xffffff;
                h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                h.bus.watched.extend([base, expected]);
                h.run();
                h.guards(4);
                assert_eq!(h.bus.value(0x7200, 3), expected);
                assert!(h.bus.trace.is_empty());
            }
        }
    }
}

#[test]
fn constant_indexed_address_windows_restore_irq_nmi_state_after_nested_calls() {
    let source = "MODULE TEST VOLATILE BYTE irqAck=$7800 ADDRESS scratch PROC Touch() RETURN ADDRESS FUNC Work(BYTE POINTER p) ADDRESS value Touch() value=ADDRESS(@p(3)) Touch() RETURN(value) CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(BYTE POINTER($FFFFFE)) irqAck=1 RETURN(saved) PROC Task(ADDRESS POINTER argument) argument^=Work(BYTE POINTER(argument^)) RETURN PROC Main() RETURN ENDMODULE";
    windows::check_work_interrupts_with_calls(source);
}

const DYNAMIC: &str = "BYTE POINTER input=$7100\nBYTE index=$7110\nADDRESS output=$7200\nPROC Touch() RETURN\nADDRESS FUNC Work(BYTE POINTER p BYTE i) ADDRESS value value=ADDRESS(@p(i)) Touch() RETURN(value)\nPROC Main() output=Work(input,index) RETURN";

#[test]
fn byte_indexed_addresses_match_ca65_and_full_arithmetic_domain() {
    let scratch = actionc::mir65816::abi::generated::DP_SCRATCH_OFFSET + 20;
    for (stride, displacement, scale) in [
        (1, 65280, String::new()),
        (3, 17, format!("sta {scratch}\nasl a\nclc\nadc {scratch}\n")),
        (128, 255, "asl a\n".repeat(7)),
        (
            257,
            0,
            format!("sta {scratch}\n{}clc\nadc {scratch}\n", "asl a\n".repeat(8)),
        ),
    ] {
        for optimize in [false, true] {
            let p = shape(prepare(DYNAMIC, optimize), None, stride, displacement);
            assert_eq!(
                p.compile(&layout()).unwrap().image.to_json().unwrap(),
                shape(
                    prepare(&DYNAMIC.replace('\n', "\r\n"), optimize),
                    None,
                    stride,
                    displacement
                )
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
                let linked = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
                let (block, i, dest, base, index) = r
                    .blocks
                    .iter()
                    .find_map(|b| {
                        b.ops.iter().enumerate().find_map(|(i, op)| {
                            let Mir65816Op::AddressOf { dest, address, .. } = op else {
                                return None;
                            };
                            let index = address.index.as_ref()?;
                            let Mir65816AddressBase::Indirect(base) = &address.base else {
                                panic!();
                            };
                            let Mir65816Value::Temp(index, width) = index.value else {
                                panic!();
                            };
                            assert_eq!(width.get(), 1);
                            Some((b.id, i, *dest, base, index))
                        })
                    })
                    .unwrap();
                let src = pointer_home(r, m, base);
                let dst = m.frame.temps[&dest].stack().unwrap().offset;
                let idx = m.frame.temps[&index].stack().unwrap();
                assert_eq!(idx.width, 1);
                let idx = idx.offset;
                assert!(src.abs_diff(dst) >= 3);
                let span = m.code.mir_spans[&(block, i)].clone();
                let start = linked.address + span.start as u32;
                let end = linked.address + span.end as u32;
                let (replayed, _) = proof::replay_code(&m.code, false).unwrap();
                proof::compare_replay_output(&m.code, &replayed).unwrap();
                let mut assembled = None;
                for index in 0..=255u32 {
                    // Exercise every index with a bank-carry/wrap boundary, plus
                    // separate zero, low-bank and highest-bank bases below.
                    let base: u32 = match index % 4 {
                        0 => 0,
                        1 => 0xffff,
                        2 => 0xfffffe,
                        _ => 0x12fffe,
                    };
                    let expected = (base + index * stride + displacement) & 0xffffff;
                    let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                    h.bus.ram[0x7110] = index as u8;
                    h.bus.ram[0x71ff..0x7204].fill(0xa5);
                    reach(&mut h, start);
                    let mut before = h.cpu.registers();
                    before.a = 0xd5ab;
                    h.cpu = Machine::start_at(before);
                    let expected_code = assembled.get_or_insert_with(|| {
                        let mut asm = if before.p & 0x20 == 0 { "sep #$20\n".to_string() } else { String::new() };
                        asm += &format!(".a8\nlda {idx},s\nrep #$20\n.a16\nand #255\n{scale}");
                        if displacement != 0 { asm += &format!("clc\nadc #{displacement}\n"); }
                        asm += &format!("clc\nadc {src},s\nsta {dst},s\nsep #$20\n.a8\nlda {},s\nadc #0\nsta {},s\n", src+2, dst+2);
                        assemble(&asm, start)
                    });
                    assert_eq!(&m.code.bytes[span.clone()], expected_code);
                    let reads = h.bus.reads.len();
                    let writes = h.bus.writes.len();
                    // Independent ca65 execution covers all operand corners;
                    // arithmetic and traffic assertions cover the full domain.
                    let mut independent = [0, 1, 127, 128, 254, 255]
                        .contains(&index)
                        .then(|| (Machine::start_at(before), h.bus.clone()));
                    reach(&mut h, end);
                    if let Some((cpu, memory)) = independent.as_mut() {
                        memory.ram[start as usize..end as usize].copy_from_slice(expected_code);
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
                    let private: Vec<_> = h.bus.reads[reads..]
                        .iter()
                        .copied()
                        .filter(|a| (0x4000..0x6000).contains(a))
                        .collect();
                    assert_eq!(
                        private,
                        [idx, src, src + 1, src + 2].map(|at| u32::from(before.s) + u32::from(at))
                    );
                    let stack_writes: Vec<_> = h.bus.writes[writes..]
                        .iter()
                        .copied()
                        .filter(|(a, _)| (0x4000..0x6000).contains(a))
                        .collect();
                    assert_eq!(
                        stack_writes,
                        (0..3)
                            .map(|i| (
                                u32::from(before.s) + u32::from(dst) + i,
                                (expected >> (8 * i)) as u8
                            ))
                            .collect::<Vec<_>>()
                    );
                    let other_writes: Vec<_> = h.bus.writes[writes..]
                        .iter()
                        .copied()
                        .filter(|(a, _)| !(0x4000..0x6000).contains(a))
                        .collect();
                    if stride.is_power_of_two() {
                        assert!(other_writes.is_empty());
                    } else {
                        assert_eq!(
                            other_writes,
                            [
                                (u32::from(before.d) + scratch, index as u8),
                                (u32::from(before.d) + scratch + 1, 0)
                            ]
                        );
                    }
                    h.run();
                    h.guards(0);
                    assert_eq!(
                        h.bus.value(0x7200, 3),
                        expected,
                        "{index}/{stride}/{displacement}"
                    );
                    assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7203]), (0xa5, 0xa5));
                }
            }
        }
    }
}

#[test]
fn byte_indexed_address_boundaries_and_wider_fallbacks_relocate_without_dereferencing() {
    for (stride, displacement) in [(38, 17), (257, 0), (258, 0), (1, 65280), (1, 65281)] {
        for index_type in ["BYTE", "CARD", "INT"] {
            let source = DYNAMIC
                .replace("BYTE index", &format!("{index_type} index"))
                .replace("BYTE i)", &format!("{index_type} i)"));
            for optimize in [false, true] {
                let p = shape(prepare(&source, optimize), None, stride, displacement);
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
                    let entry = loaded.as_ref().map_or(c.image.entry, |l| l.entry());
                    let mut h = if let Some(l) = &loaded {
                        Harness::new_o65(l, &caller(entry), 4)
                    } else {
                        Harness::new(&c.image, &caller(entry), 4)
                    };
                    let base = 0xfffffeu32;
                    let expected = (base + 255 * stride + displacement) & 0xffffff;
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                    h.bus.ram[0x7110..0x7112].copy_from_slice(&[255, 0]);
                    h.bus.watched.extend([base, expected]);
                    h.run();
                    h.guards(4);
                    assert_eq!(h.bus.value(0x7200, 3), expected);
                    assert!(h.bus.trace.is_empty());
                }
            }
        }
    }
}

#[test]
fn byte_indexed_address_windows_restore_irq_nmi_after_nested_calls() {
    let source = "MODULE TEST VOLATILE BYTE irqAck=$7800 ADDRESS scratch PROC Touch() RETURN ADDRESS FUNC Work(BYTE POINTER p BYTE i) ADDRESS value Touch() value=ADDRESS(@p(i)) Touch() RETURN(value) CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(BYTE POINTER($FFFFFE),255) irqAck=1 RETURN(saved) PROC Task(ADDRESS POINTER argument) argument^=Work(BYTE POINTER(argument^),128) RETURN PROC Main() RETURN ENDMODULE";
    windows::check_work_interrupts_with_calls(source);
    let scaled = source.replace("MODULE TEST", "MODULE TEST TYPE Item=[BYTE a,b,c]")
        .replace("BYTE POINTER", "Item POINTER")
        .replace("ADDRESS value Touch() value=ADDRESS(@p(i))", "Item POINTER local ADDRESS value local=p IF i=0 THEN local=p FI Touch() value=ADDRESS(@local(i))");
    windows::check_work_interrupts_with_calls(&scaled);
}

#[test]
fn byte_indexed_addresses_read_borrowed_local_bases_after_calls() {
    let source = DYNAMIC.replace("ADDRESS value value=ADDRESS(@p(i))", "BYTE POINTER local ADDRESS value local=p IF i=0 THEN local=p FI Touch() value=ADDRESS(@local(i))");
    for optimize in [false, true] {
        let p = shape(prepare(&source, optimize), None, 38, 17);
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
            assert!(r.blocks.iter().any(|b| b.ops.iter().enumerate().any(|(i, op)|
                matches!(op, Mir65816Op::Load { width, address, volatile: false, .. }
                    if width.get()==3 && matches!(address.base, Mir65816AddressBase::AutomaticFrame(_)))
                    && m.code.mir_spans[&(b.id, i)].is_empty()
            )));
            for index in [0, 1, 127, 128, 255] {
                let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                let base = 0xfffffeu32;
                let expected = (base + index * 38 + 17) & 0xffffff;
                h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                h.bus.ram[0x7110] = index as u8;
                h.bus.watched.extend([base, expected]);
                h.run();
                h.guards(0);
                assert_eq!(h.bus.value(0x7200, 3), expected);
                assert!(h.bus.trace.is_empty());
            }
        }
    }
}
