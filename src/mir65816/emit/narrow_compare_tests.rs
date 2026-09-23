use super::*;

fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let frame = AllocatedFrame::new(routine).unwrap();
    let mut code = TrackedEmitter65816::for_test(&frame);
    let blocks = routine
        .blocks
        .iter()
        .map(|b| (b.id, code.label()))
        .collect();
    Builder {
        routine,
        frame,
        code,
        blocks,
        next_block: None,
        loop_x: None,
    }
}

fn program(source: &str) -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    super::super::super::lower_program(&nir).unwrap()
}

fn operands(r: &Mir65816Routine) -> (TempId, Mir65816Value, Mir65816Value) {
    r.blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| match op {
            Mir65816Op::Compare {
                dest, left, right, ..
            } => Some((*dest, left.clone(), right.clone())),
            _ => None,
        })
        .unwrap()
}

#[test]
fn byte_predicates_use_exact_a8_operands_and_two_boolean_arms() {
    let p = program("BYTE FUNC Work(BYTE a,b) a==+1 RETURN(a<b)");
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    assert!(r.frame.parameters[0].frame_object.is_some());
    for (op, predicate, swap) in [
        (NirCompareOp::Eq, 0xf0, false),
        (NirCompareOp::Ne, 0xd0, false),
        (NirCompareOp::Lt, 0x90, false),
        (NirCompareOp::Ge, 0xb0, false),
        (NirCompareOp::Gt, 0x90, true),
        (NirCompareOp::Le, 0xb0, true),
    ] {
        for (left, right) in [
            (left.clone(), right.clone()),
            (left.clone(), Mir65816Value::U8(0x80)),
            (Mir65816Value::U8(0xff), right.clone()),
            (
                Mir65816Value::Param(r.frame.parameters[0].param),
                Mir65816Value::Param(r.frame.parameters[1].param),
            ),
        ] {
            let mut b = builder(r);
            b.code.a16();
            let start = b.code.position();
            let a = b.byte_operand(&left).unwrap().unwrap();
            let c = b.byte_operand(&right).unwrap().unwrap();
            let (a, c) = if swap { (c, a) } else { (a, c) };
            let encode = |v, load| match v {
                ByteOperand::Immediate(n) => vec![if load { 0xa9 } else { 0xc9 }, n],
                ByteOperand::Stack(n) => vec![if load { 0xa3 } else { 0xc3 }, n],
            };
            let mut expected = vec![0xe2, 0x20];
            expected.extend(encode(a, true));
            expected.extend(encode(c, false));
            expected.extend([
                predicate ^ 0x20,
                4,
                0x5c,
                0,
                0,
                0,
                0xa9,
                0,
                0x5c,
                0,
                0,
                0,
                0xe2,
                0x20,
                0xa9,
                1,
                0xe2,
                0x20,
                0x83,
                b.temp(dest).unwrap().slot().offset as u8,
            ]);
            assert!(b.native_compare(dest, 1, false, op, &left, &right).unwrap());
            assert_eq!(&b.code.code().bytes[start..], expected);
        }
    }
}

#[test]
fn byte_preflight_is_atomic_for_unsupported_and_malformed_operands() {
    let p = program("BYTE FUNC Work(BYTE a,b) RETURN(a<b)");
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    for problem in 0..11 {
        let mut b = builder(r);
        let mut a = left.clone();
        let mut c = right.clone();
        let mut signed = false;
        match problem {
            0 => a = Mir65816Value::U16(1),
            1 => signed = true,
            2 => a = Mir65816Value::RoutineAddress(0, ByteSize::new(3)),
            3 => {
                b.frame.temps.insert(
                    dest,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 1,
                    }),
                );
            }
            4 => {
                b.frame.temps.remove(&dest);
            }
            5 => {
                b.frame.temps.insert(
                    dest,
                    Location::Stack(Slot {
                        offset: 1,
                        width: 2,
                    }),
                );
            }
            6 => {
                b.frame.temps.insert(
                    dest,
                    Location::Stack(Slot {
                        offset: 255,
                        width: 1,
                    }),
                );
                b.code.test_delta(1);
            }
            7 => a = Mir65816Value::Temp(TempId(9999), ByteSize::ONE),
            8 => {
                a = Mir65816Value::U16(0);
                c = Mir65816Value::Temp(TempId(9999), ByteSize::ONE);
            }
            9 | 10 => {
                let Mir65816Value::Temp(id, ..) = a else {
                    panic!()
                };
                b.frame.temps.insert(
                    id,
                    Location::Stack(Slot {
                        offset: if problem == 9 { 255 } else { 2 },
                        width: if problem == 10 { 2 } else { 1 },
                    }),
                );
                if problem == 9 {
                    b.code.test_delta(1);
                }
            }
            _ => unreachable!(),
        }
        let before = format!("{:?}", b.code);
        let result = b.native_compare(dest, 1, signed, NirCompareOp::Lt, &a, &c);
        if problem < 4 {
            assert_eq!(result, Ok(false));
        } else {
            assert!(result.is_err(), "{problem}");
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
    for signed in [false, true] {
        for op in [NirCompareOp::Eq, NirCompareOp::Ne] {
            assert!(
                builder(r)
                    .native_compare(dest, 1, signed, op, &left, &right)
                    .unwrap()
            );
        }
    }
}

#[test]
fn byte_fusion_restores_a16_and_omits_boolean_home_traffic() {
    let p = program("CARD FUNC Work(BYTE a,b) IF a<b THEN RETURN(1) FI RETURN(0)");
    let r = &p.routines[0];
    let block = &r.blocks[0];
    let (dest, left, right) = operands(r);
    let mut b = builder(r);
    b.code.a16();
    let start = b.code.position();
    let ByteOperand::Stack(a) = b.byte_operand(&left).unwrap().unwrap() else {
        panic!()
    };
    let ByteOperand::Stack(c) = b.byte_operand(&right).unwrap().unwrap() else {
        panic!()
    };
    let homes = b.frame.temps.clone();
    let extent = b.frame.extent;
    assert!(
        b.compare_branch(
            block.ops.last().unwrap(),
            &block.terminator,
            &liveness::sole_branch_conditions(r)
        )
        .unwrap()
    );
    assert_eq!(
        &b.code.code().bytes[start..],
        &[
            0xe2, 0x20, 0xa3, a, 0xc3, c, 0xc2, 0x20, 0xb0, 4, 0x5c, 0, 0, 0, 0x5c, 0, 0, 0, 0xc2,
            0x20, 0x5c, 0, 0, 0
        ]
    );
    assert_eq!(b.frame.temps, homes);
    assert_eq!(b.frame.extent, extent);
    assert!(b.frame.temps.contains_key(&dest));
    let end = b.code.position();
    b.code.a16();
    assert_eq!(b.code.position(), end);
}

/// A repeatable inventory of verified MIR, not a count inferred from opcode bytes.
#[test]
#[ignore = "set A816_COMPARE_SOURCE, A816_COMPARE_MODULES and A816_COMPARE_INVENTORY"]
fn external_comparison_inventory() {
    let source = std::env::var_os("A816_COMPARE_SOURCE").unwrap();
    let mut modules = crate::includes::ModuleLoadOptions::default();
    modules.module_paths =
        std::env::split_paths(&std::env::var_os("A816_COMPARE_MODULES").unwrap()).collect();
    let mut results = Vec::new();
    for optimize in [false, true] {
        let p = crate::compiler::native65816::prepare_file(&source, optimize, &modules).unwrap();
        let mut groups = BTreeMap::<String, usize>::new();
        for r in p
            .mir
            .routines
            .iter()
            .filter(|r| !r.entry.external && !r.blocks.is_empty())
        {
            let b = builder(r);
            let sole = liveness::sole_branch_conditions(r);
            for block in &r.blocks {
                for (index, op) in block.ops.iter().enumerate() {
                    let Mir65816Op::Compare {
                        dest,
                        width,
                        signed,
                        operation,
                        left,
                        right,
                    } = op
                    else {
                        continue;
                    };
                    let adjacent = index + 1 == block.ops.len()
                        && sole.contains(dest)
                        && matches!(
                        &block.terminator,Mir65816Terminator::Branch{condition:Mir65816Value::Temp(id,w),..}
                        if id==dest && *w==ByteSize::ONE);
                    let native_word = b
                        .word_condition(*dest, width.get() as u8, *signed, *operation, left, right)
                        .unwrap()
                        .is_some();
                    let key = format!(
                        "width={}/signed={signed}/{operation:?}/{}/{}/{}",
                        width.get(),
                        if adjacent {
                            "sole-branch"
                        } else {
                            "materialized"
                        },
                        if native_word {
                            "word-selector"
                        } else {
                            "generic"
                        },
                        r.name.split('_').nth(1).unwrap_or(&r.name)
                    );
                    *groups.entry(key).or_default() += 1;
                }
            }
        }
        results.push(serde_json::json!({"optimize":optimize,"groups":groups}));
    }
    std::fs::write(
        std::env::var_os("A816_COMPARE_INVENTORY").unwrap(),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}
