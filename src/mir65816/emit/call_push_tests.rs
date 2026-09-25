use super::*;

#[test]
fn pushes_place_each_payload_and_padding_byte_once_at_dynamic_stack_offsets() {
    // Enumerate independent layouts/values, then decode the emitted LDA/PHA
    // stream into a tiny stack oracle. No selector choices drive the oracle.
    for pattern in 0..1024u32 {
        let mut arguments = vec![];
        let mut padding = vec![];
        let mut expected = vec![];
        for i in 0..5 {
            let bytes = 1 + ((pattern >> (i * 2)) & 3) as u8;
            if bytes != 1 && expected.len() % 2 != 0 {
                padding.push(expected.len() as u8 + 1);
                expected.push(0);
            }
            arguments.push(Argument {
                source: if i % 2 == 0 {
                    Source::Immediate(0x89ab8172 + i)
                } else {
                    Source::Home(Memory::Stack(32 + i * 4))
                },
                displacement: expected.len() as u8 + 1,
                bytes,
                copy: ArgumentCopy::Bytes,
            });
            expected.extend_from_slice(&(0x89ab8172u32 + i as u32).to_le_bytes()[..bytes as usize]);
        }
        if expected.len() % 2 == 0 {
            padding.push(expected.len() as u8 + 1);
            expected.push(0);
        }
        select_widths(&mut arguments, (!padding.is_empty()).then_some(false), true);
        let plan = Plan::new(&arguments, &[], &padding, expected.len() as u16).unwrap();
        let p = super::super::tests::program();
        let mut b = super::super::tests::builder(&p.routines[1]);
        // Emission begins without width permission, just as after a guard join.
        b.code.barrier();
        let label = b.code.label();
        b.code.mark(label);
        let start = b.code.position();
        plan.emit(&mut b, &[]).unwrap();
        let code = &b.code.code().bytes[start..];
        let (mut at, mut word, mut a, mut s) = (0, false, 0u16, 200usize);
        let mut stack = [0xa5u8; 512];
        for i in 0..5 {
            stack[200 + 32 + i * 4..200 + 36 + i * 4]
                .copy_from_slice(&(0x89ab8172u32 + i as u32).to_le_bytes());
        }
        let mut writes = vec![];
        while at < code.len() {
            match code[at] {
                0xc2 | 0xe2 => {
                    assert_eq!(code[at + 1], 0x20);
                    word = code[at] == 0xc2;
                    at += 2;
                }
                0xa9 => {
                    a = u16::from(code[at + 1]);
                    if word {
                        a |= u16::from(code[at + 2]) << 8;
                    }
                    at += if word { 3 } else { 2 };
                }
                0xa3 => {
                    let pos = s + code[at + 1] as usize;
                    a = u16::from(stack[pos]);
                    if word {
                        a |= u16::from(stack[pos + 1]) << 8;
                    }
                    at += 2;
                }
                0x48 => {
                    if word {
                        stack[s] = (a >> 8) as u8;
                        writes.push(s);
                        s -= 1;
                    }
                    stack[s] = a as u8;
                    writes.push(s);
                    s -= 1;
                    at += 1;
                }
                op => panic!("unexpected {op:02x}"),
            }
        }
        assert!(word);
        assert_eq!(&stack[s + 1..201], expected);
        assert_eq!(writes, (s + 1..201).rev().collect::<Vec<_>>());
        assert_eq!(b.code.delta(), expected.len() as u32);
        assert!(code.len() < store_cost(&arguments, &padding));
    }
}

#[test]
fn unsupported_push_plans_keep_complete_store_construction() {
    for bytes in 1..=4 {
        let arg = Argument {
            source: Source::Bytes,
            bytes,
            displacement: 1,
            copy: ArgumentCopy::Bytes,
        };
        assert!(Plan::new(&[arg], &[], &[], 5).is_none());
    }
    let p = super::super::tests::program();
    let mut b = super::super::tests::builder(&p.routines[1]);
    b.code.a8();
    b.code.op(Implied::Pha);
    let before = format!("{:?}", b.code);
    assert!(b.code.instruction(Instruction::ArgumentPush).is_err());
    assert_eq!(format!("{:?}", b.code), before);
}

#[test]
fn symbolic_pushes_keep_individual_byte_fixup_identity_and_order() {
    let values = [
        Mir65816Value::GlobalAddress(crate::nir::SymbolId(7), ByteSize::new(3)),
        Mir65816Value::StaticAddress(crate::nir::SymbolId(9), ByteSize::new(3)),
        Mir65816Value::RoutineAddress(0, ByteSize::new(3)),
    ];
    let arguments: Vec<_> = (0..3)
        .map(|i| Argument {
            source: Source::Bytes,
            bytes: 3,
            displacement: 1 + 3 * i,
            copy: ArgumentCopy::Bytes,
        })
        .collect();
    let plan = Plan::new(&arguments, &values, &[], 9).unwrap();
    let p = super::super::tests::program();
    let mut b = super::super::tests::builder(&p.routines[1]);
    b.code.barrier();
    let label = b.code.label();
    b.code.mark(label);
    plan.emit(&mut b, &values).unwrap();
    let fixups = &b.code.code().fixups;
    assert_eq!(fixups.len(), 9);
    for (i, f) in fixups.iter().enumerate() {
        let target = match i / 3 {
            0 => Target::Routine(RoutineId(0)),
            1 => Target::Data(Mir65816DataId::Static(crate::nir::SymbolId(9))),
            _ => Target::Data(Mir65816DataId::Global(crate::nir::SymbolId(7))),
        };
        assert_eq!(
            (f.target, f.addend, f.byte),
            (target, 0, Some((2 - i % 3) as u8))
        );
        assert_eq!(b.code.code().bytes[f.offset - 1], 0xa9);
        assert_eq!(b.code.code().bytes[f.offset + 1], 0x48);
    }
    assert_eq!(b.code.delta(), 9);
}
