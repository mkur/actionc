use super::*;

fn routine() -> Mir65816Routine {
    let source = "PROC Sink(BYTE x) RETURN BYTE FUNC Work(CARD a,b) Sink(0) RETURN(a<b)";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    super::super::super::lower_program(&nir)
        .unwrap()
        .routines
        .remove(1)
}
fn value() -> Mir65816Value {
    Mir65816Value::Temp(TempId(999), ByteSize::ONE)
}
fn branch() -> Mir65816Terminator {
    let edge = Mir65816Edge {
        target: BlockId(1),
        args: vec![],
    };
    Mir65816Terminator::Branch {
        condition: value(),
        then_edge: edge.clone(),
        else_edge: edge,
    }
}
fn base() -> Mir65816Routine {
    let mut r = routine();
    r.blocks = vec![
        Mir65816Block {
            id: BlockId(0),
            params: vec![],
            ops: vec![],
            terminator: branch(),
        },
        Mir65816Block {
            id: BlockId(1),
            params: vec![],
            ops: vec![],
            terminator: Mir65816Terminator::Exit,
        },
    ];
    r
}
#[test]
fn sole_condition_proof_distinguishes_every_terminator_input_from_definitions() {
    let expected = BTreeSet::from([TempId(999)]);
    assert_eq!(sole_branch_conditions(&base()), expected);
    for side in 0..3 {
        let mut r = base();
        let Mir65816Terminator::Branch {
            then_edge,
            else_edge,
            ..
        } = &mut r.blocks[0].terminator
        else {
            unreachable!()
        };
        if side != 1 {
            then_edge.args.push(value());
        }
        if side != 0 {
            else_edge.args.push(value());
        }
        assert!(sole_branch_conditions(&r).is_empty());
    }
    let mut ret = routine().blocks.last().unwrap().terminator.clone();
    let Mir65816Terminator::Return {
        value: returned, ..
    } = &mut ret
    else {
        panic!()
    };
    *returned = Some(value());
    for term in [
        branch(),
        ret,
        Mir65816Terminator::Goto(Mir65816Edge {
            target: BlockId(0),
            args: vec![value()],
        }),
    ] {
        let mut r = base();
        r.blocks[1].terminator = term;
        assert!(sole_branch_conditions(&r).is_empty());
    }
    // Even an unreachable block disqualifies an otherwise unique condition.
    let mut r = base();
    r.blocks.push(Mir65816Block {
        id: BlockId(2),
        params: vec![],
        ops: vec![],
        terminator: branch(),
    });
    assert!(sole_branch_conditions(&r).is_empty());
    let mut r = base();
    r.blocks[1].params.push((TempId(999), ByteSize::ONE));
    assert_eq!(sole_branch_conditions(&r), expected); // definitions are not uses
}
#[test]
fn noncondition_proof_reuses_operation_address_and_call_inputs() {
    let r = routine();
    let address = Mir65816Address {
        base: Mir65816AddressBase::Indirect(value()),
        displacement: ByteOffset::ZERO,
        index: None,
        mode: Mir65816AddressMode::LongIndirect,
    };
    let mut indexed = address.clone();
    indexed.base = Mir65816AddressBase::Parameter(r.frame.parameters[0].param);
    indexed.index = Some(Mir65816Index {
        value: value(),
        stride: ByteSize::ONE,
    });
    let dest = TempId(1000);
    let mut operations = vec![
        Mir65816Op::Compare {
            dest,
            width: ByteSize::ONE,
            signed: false,
            operation: NirCompareOp::Eq,
            left: value(),
            right: value(),
        },
        Mir65816Op::Store {
            address: indexed.clone(),
            value: Mir65816Value::U8(0),
            width: ByteSize::ONE,
            volatile: true,
        },
        Mir65816Op::Store {
            address: Mir65816Address {
                index: None,
                ..indexed.clone()
            },
            value: value(),
            width: ByteSize::ONE,
            volatile: false,
        },
        Mir65816Op::Load {
            dest,
            address: address.clone(),
            width: ByteSize::ONE,
            volatile: false,
        },
        Mir65816Op::AddressOf {
            dest,
            address: indexed.clone(),
            width: ByteSize::new(3),
        },
        Mir65816Op::Copy {
            source: address.clone(),
            destination: indexed,
            bytes: ByteSize::ONE,
            overlap_safe: true,
            source_volatile: false,
            destination_volatile: false,
        },
        Mir65816Op::PointerOffset {
            dest,
            width: ByteSize::new(3),
            base: value(),
            offset: value(),
            subtract: false,
            offset_signed: false,
        },
    ];
    let call = r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap();
    for indirect in [false, true] {
        let mut call = call.clone();
        let Mir65816Op::Call { args, target, .. } = &mut call else {
            unreachable!()
        };
        if indirect {
            *target = Mir65816CallTarget::Indirect(value(), ByteSize::new(3));
        } else {
            *args = vec![value(), value()];
        }
        operations.push(call);
    }
    for op in operations {
        let mut r = base();
        r.blocks[1].ops.push(op.clone());
        assert!(sole_branch_conditions(&r).is_empty(), "{op:?}");
    }
}

#[test]
fn measured_corpus_has_one_proven_adjacent_condition_in_each_expected_kernel() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools/native65816-runtime-tests/tests/fixtures/code_quality");
    for optimize in [false, true] {
        for name in [
            "maximum",
            "loop_rotation",
            "sum_loop",
            "recursive_sum",
            "byte_sum",
            "forward_copy",
        ] {
            let prepared = crate::compiler::native65816::prepare_file(
                &directory.join(format!("{name}.act")),
                optimize,
                &Default::default(),
            )
            .unwrap();
            let count = prepared.mir.routines.iter().map(|r| {
                let sole = sole_branch_conditions(r);
                r.blocks.iter().filter(|b| matches!((b.ops.last(), &b.terminator),
                    (Some(Mir65816Op::Compare { dest, width, signed: false, .. }),
                     Mir65816Terminator::Branch { condition: Mir65816Value::Temp(id, bytes), .. })
                     if dest == id && *width == ByteSize::new(2) && *bytes == ByteSize::ONE && sole.contains(id))).count()
            }).sum::<usize>();
            assert_eq!(count, 1, "{name}/{optimize}");
        }
    }
}
