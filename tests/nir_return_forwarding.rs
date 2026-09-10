use actionc::nir::{self, *};
use actionc::semantic::{self, SemanticOptions};
use actionc::target::TargetId;

fn lower(source: &str, target: TargetId) -> NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap();
    let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program).unwrap();
    program
}

fn simple() -> NirProgram {
    lower(
        "BYTE FUNC Pick(BYTE flag,a,b) RETURN(IF flag THEN a ELSE b FI)\nPROC Main() RETURN",
        TargetId::Atari6502,
    )
}

fn join(routine: &NirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .position(|b| {
            !b.params.is_empty() && matches!(b.terminator, NirTerminator::Return(Some(_)))
        })
        .unwrap()
}

fn optimize(program: &NirProgram) -> NirProgram {
    nir::verify_program(program).unwrap();
    let result = nir::optimize_program(program).unwrap();
    nir::verify_program(&result).unwrap();
    assert_eq!(result, nir::optimize_program(&result).unwrap());
    result
}

#[test]
fn nested_return_joins_preserve_exact_integer_enum_and_address_widths() {
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
        TargetId::Motorola68000,
    ] {
        for ty in [
            "BYTE", "CHAR", "CARD", "INT", "LONGCARD", "LONGINT", "ADDRESS", "SIZE", "Choice",
        ] {
            let source = format!(
                "TYPE Choice=ENUM [DARK LIGHT]\n{ty} FUNC Pick(BYTE flag {ty} a,b)\nRETURN(IF flag THEN a ELSE CASE flag OF\nWHEN 0 THEN\nb\nELSE\na\nESAC FI)\nPROC Main() RETURN"
            );
            let raw = lower(&source, target);
            assert!(
                raw.routines[0]
                    .blocks
                    .iter()
                    .filter(|b| !b.params.is_empty())
                    .count()
                    >= 2
            );
            let optimized = optimize(&raw);
            let pick = &optimized.routines[0];
            assert!(
                pick.blocks.iter().all(|b| b.params.is_empty()),
                "{target:?}/{ty}"
            );
            assert!(
                pick.blocks
                    .iter()
                    .filter(|b| matches!(b.terminator, NirTerminator::Return(Some(_))))
                    .count()
                    >= 2
            );
            for block in &pick.blocks {
                if let NirTerminator::Return(Some(NirValue::Temp { ty: actual, .. })) =
                    &block.terminator
                {
                    assert_eq!(Some(actual), pick.signature.result.as_ref());
                }
            }
        }
    }
}

#[test]
fn return_forwarding_substitutes_the_correct_parameter_through_chained_joins() {
    let mut raw = simple();
    let routine = &mut raw.routines[0];
    let index = join(routine);
    let target = routine.blocks[index].id;
    let original = routine.blocks[index].params[0].clone();
    let extra = TempId(routine.temps.iter().map(|t| t.id.0).max().unwrap() + 1);
    routine.blocks[index].params.insert(
        0,
        NirBlockParam {
            dest: extra,
            ty: original.ty.clone(),
        },
    );
    routine.temps.push(NirTemp {
        id: extra,
        ty: original.ty.clone(),
        def: NirTempDef {
            block: target,
            op_index: None,
        },
    });
    let mut expected = Vec::new();
    for block in &mut routine.blocks {
        if let NirTerminator::Goto(edge) = &mut block.terminator
            && edge.target == target
        {
            expected.push((block.id, edge.args[0].clone()));
            edge.args.insert(0, NirValue::ConstU8(expected.len() as u8));
        }
    }
    let tail = BlockId(
        raw.routines
            .iter()
            .flat_map(|r| &r.blocks)
            .map(|b| b.id.0)
            .max()
            .unwrap()
            + 1,
    );
    let routine = &mut raw.routines[0];
    let forwarded = TempId(extra.0 + 1);
    let result = NirValue::Temp {
        id: original.dest,
        ty: original.ty.clone(),
    };
    routine.blocks[index].terminator = NirTerminator::Goto(NirEdge {
        target: tail,
        args: vec![result],
    });
    routine.blocks.push(NirBlock {
        id: tail,
        label: "return_tail".into(),
        params: vec![NirBlockParam {
            dest: forwarded,
            ty: original.ty.clone(),
        }],
        ops: Vec::new(),
        terminator: NirTerminator::Return(Some(NirValue::Temp {
            id: forwarded,
            ty: original.ty.clone(),
        })),
    });
    routine.temps.push(NirTemp {
        id: forwarded,
        ty: original.ty,
        def: NirTempDef {
            block: tail,
            op_index: None,
        },
    });
    let optimized = optimize(&raw);
    let routine = &optimized.routines[0];
    assert!(
        !routine
            .blocks
            .iter()
            .any(|b| b.id == target || b.id == tail)
    );
    for (id, value) in expected {
        let block = routine.blocks.iter().find(|b| b.id == id).unwrap();
        assert_eq!(block.terminator, NirTerminator::Return(Some(value)));
    }
}

#[test]
fn conditional_edges_keep_their_return_target_when_other_edges_are_forwarded() {
    let mut raw = simple();
    let routine = &mut raw.routines[0];
    let target = routine.blocks[join(routine)].id;
    let NirTerminator::Branch { then_edge, .. } = &mut routine.blocks[0].terminator else {
        panic!()
    };
    let removed = then_edge.target;
    *then_edge = NirEdge {
        target,
        args: vec![NirValue::ConstU8(77)],
    };
    routine.blocks.retain(|b| b.id != removed);
    routine.temps.retain(|t| t.def.block != removed);
    let optimized = optimize(&raw);
    let routine = &optimized.routines[0];
    assert!(
        matches!(&routine.blocks[0].terminator, NirTerminator::Branch { then_edge, .. } if then_edge.target == target)
    );
    assert_eq!(
        routine
            .blocks
            .iter()
            .find(|b| b.id == target)
            .unwrap()
            .terminator,
        NirTerminator::Return(Some(NirValue::ConstU8(77)))
    );
    assert!(routine.blocks.iter().any(|b| b.id != target
        && matches!(
            b.terminator,
            NirTerminator::Return(Some(NirValue::Temp { .. }))
        )));
}

#[test]
fn nonempty_return_blocks_keep_calls_volatile_stores_and_the_value_join() {
    let raw = lower(
        "VOLATILE BYTE signal=$600\nPROC Observe() signal=77 RETURN\nBYTE FUNC Pick(BYTE flag,a,b)\nLET value=IF flag THEN a ELSE b FI\nObserve() signal=9\nRETURN(value)\nPROC Main() RETURN",
        TargetId::Atari6502,
    );
    let optimized = optimize(&raw);
    let pick = optimized
        .routines
        .iter()
        .find(|r| r.name == "Pick")
        .unwrap();
    let tail = pick
        .blocks
        .iter()
        .find(|b| b.ops.iter().any(|op| matches!(op, NirOp::Call { .. })))
        .unwrap();
    assert!(
        tail.ops
            .iter()
            .any(|op| matches!(op, NirOp::VolatileStore { .. }))
    );
    assert!(matches!(tail.terminator, NirTerminator::Return(Some(_))));
    assert!(
        pick.blocks
            .iter()
            .any(|b| matches!(&b.terminator, NirTerminator::Goto(edge) if edge.target == tail.id))
    );
}

#[test]
fn forwarding_keeps_dynamic_variant_faults_terminal() {
    let raw = lower(
        "TYPE MaybeByte=VARIANT [NONE SOME [BYTE value]]\nMaybeByte current\nBYTE FUNC Pick()\nRETURN(CASE current OF\nWHEN MaybeByte.NONE THEN\n0\nWHEN MaybeByte.SOME(n) THEN\nn\nESAC)\nPROC Main() PrintBE(Pick()) RETURN",
        TargetId::Atari6502,
    );
    let optimized = optimize(&raw);
    let faults = |p: &NirProgram| {
        p.routines
            .iter()
            .flat_map(|r| &r.blocks)
            .filter(|b| {
                b.ops.iter().any(|op| {
                    matches!(
                        op,
                        NirOp::Call {
                            callee: NirCallee::Fault(_),
                            ..
                        }
                    )
                })
            })
            .map(|b| (b.id, b.terminator.clone()))
            .collect::<Vec<_>>()
    };
    let expected = faults(&raw);
    assert!(!expected.is_empty());
    assert!(
        expected
            .iter()
            .all(|(_, term)| matches!(term, NirTerminator::Exit))
    );
    assert_eq!(faults(&optimized), expected);
}

#[test]
fn aggregate_returns_keep_their_storage_boundary() {
    let mut raw = lower(
        "TYPE Pair=[BYTE left,right]\nPair data\nPair FUNC Pick() RETURN(data)\nPROC Main() RETURN",
        TargetId::Atari6502,
    );
    let tail = BlockId(
        raw.routines
            .iter()
            .flat_map(|r| &r.blocks)
            .map(|b| b.id.0)
            .max()
            .unwrap()
            + 1,
    );
    let routine = &mut raw.routines[0];
    let block = routine
        .blocks
        .iter_mut()
        .find(|b| {
            matches!(
                b.terminator,
                NirTerminator::Return(Some(NirValue::Aggregate { .. }))
            )
        })
        .unwrap();
    let predecessor = block.id;
    let terminator = std::mem::replace(
        &mut block.terminator,
        NirTerminator::Goto(NirEdge {
            target: tail,
            args: Vec::new(),
        }),
    );
    routine.blocks.push(NirBlock {
        id: tail,
        label: "aggregate_return".into(),
        params: Vec::new(),
        ops: Vec::new(),
        terminator,
    });
    let optimized = optimize(&raw);
    let routine = &optimized.routines[0];
    assert!(
        matches!(&routine.blocks.iter().find(|b| b.id == predecessor).unwrap().terminator, NirTerminator::Goto(edge) if edge.target == tail)
    );
    assert!(matches!(
        &routine
            .blocks
            .iter()
            .find(|b| b.id == tail)
            .unwrap()
            .terminator,
        NirTerminator::Return(Some(NirValue::Aggregate { .. }))
    ));
}

#[test]
fn malformed_return_edges_are_rejected_before_forwarding() {
    for wrong_type in [false, true] {
        let mut raw = simple();
        let routine = &mut raw.routines[0];
        let target = routine.blocks[join(routine)].id;
        let edge = routine
            .blocks
            .iter_mut()
            .find_map(|b| match &mut b.terminator {
                NirTerminator::Goto(edge) if edge.target == target => Some(edge),
                _ => None,
            })
            .unwrap();
        if wrong_type {
            edge.args[0] = NirValue::ConstU16(300);
        } else {
            edge.args.clear();
        }
        assert!(nir::optimize_program(&raw).is_err());
    }
}
