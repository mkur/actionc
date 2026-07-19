use super::*;

use crate::ast::FundType;
use crate::semantic::ValueType;

fn edge(target: u32) -> NirEdge {
    NirEdge {
        target: BlockId(target),
        args: Vec::new(),
    }
}

#[test]
fn formats_labeled_blocks() {
    let program = NirProgram {
        globals: vec![NirGlobal {
            id: SymbolId(0),
            name: "counter".to_string(),
            kind: "Byte".to_string(),
            ty: None,
            storage_size: 1,
            array: None,
            init: None,
            backing: NirGlobalBacking::Ordinary,
        }],
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![NirLocal {
                id: LocalId(0),
                name: "i".to_string(),
                kind: "Byte".to_string(),
                storage: NirStorageClass::Scalar,
                ty: byte_type(),
                backing: NirLocalBacking::Ordinary,
                init: None,
            }],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Store {
                    place: byte_place("i"),
                    src: byte_value(0),
                    ty: byte_type(),
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let formatted = format_program(&program);
    assert!(formatted.contains("nir program"));
    assert!(formatted.contains("global counter: Byte"));
    assert!(formatted.contains("routine Main"));
    assert!(formatted.contains("bb0:"));
    assert!(formatted.contains("store i = 0"));
    assert!(formatted.contains("return"));
}

#[test]
fn nir_type_kind_tracks_semir_value_types() {
    let byte = NirType::from_value(&ValueType::fund(FundType::Byte));
    assert_eq!(byte.kind, NirTypeKind::U8);
    assert_eq!(byte.width, Some(1));
    assert!(!byte.pointer);

    let int = NirType::from_value(&ValueType::fund(FundType::Int));
    assert_eq!(int.kind, NirTypeKind::I16);
    assert_eq!(int.width, Some(2));

    let pointer = NirType::from_value(&ValueType::pointer_to(ValueType::fund(FundType::Byte)));
    assert_eq!(
        pointer.kind,
        NirTypeKind::Ptr16 {
            pointee: Some(Box::new(NirTypeKind::U8))
        }
    );
    assert_eq!(pointer.width, Some(2));
    assert!(pointer.pointer);

    let record = NirType::from_value(&ValueType::record("Pair"));
    assert_eq!(
        record.kind,
        NirTypeKind::Record {
            name: "Pair".to_string(),
            size: None
        }
    );
    assert_eq!(record.width, None);
}

#[test]
fn nir_value_reads_simple_legacy_operands() {
    let byte = NirOperand {
        kind: NirOperandKind::Literal {
            text: "7".to_string(),
            value: Some(7),
        },
        ty: Some(byte_type()),
    };
    assert_eq!(
        NirValue::from_legacy_operand(&byte),
        Some(NirValue::ConstU8(7))
    );

    let card = card_literal_with_value("$1234", 0x1234);
    assert_eq!(
        NirValue::from_legacy_operand(&card),
        Some(NirValue::ConstU16(0x1234))
    );

    let temp = temp_operand(3, byte_type());
    assert_eq!(
        NirValue::from_legacy_operand(&temp),
        Some(temp_value(3, byte_type()))
    );

    let place = NirOperand {
        kind: NirOperandKind::Place(Box::new(byte_place("x"))),
        ty: Some(byte_type()),
    };
    assert_eq!(NirValue::from_legacy_operand(&place), None);
}

#[test]
fn verifier_accepts_valid_targets() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Goto(edge(1)),
                },
                NirBlock {
                    id: BlockId(1),
                    label: "bb1".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    assert_eq!(verify_program(&program), Ok(()));
}

#[test]
fn verifier_accepts_typed_block_arguments_and_printer_keeps_labels_readable() {
    let program = typed_block_argument_program();

    assert_eq!(verify_program(&program), Ok(()));
    let formatted = format_program(&program);
    assert!(formatted.contains("goto join(7, %t0, %t1, &table)"));
    assert!(formatted.contains("join(%t2:Byte, %t3:Byte, %t4:Card, %t5:Byte*):"));
}

#[test]
fn verifier_rejects_block_argument_arity_mismatch() {
    let mut program = typed_block_argument_program();
    let NirTerminator::Goto(edge) = &mut program.routines[0].blocks[0].terminator else {
        panic!("expected goto");
    };
    edge.args.pop();

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("supplies 3 argument(s), expected 4")
    }));
}

#[test]
fn verifier_rejects_block_argument_type_mismatch() {
    let mut program = typed_block_argument_program();
    let NirTerminator::Goto(edge) = &mut program.routines[0].blocks[0].terminator else {
        panic!("expected goto");
    };
    edge.args[1] = temp_value(0, card_type());

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("does not match parameter type Byte")
    }));
}

#[test]
fn verifier_rejects_duplicate_block_parameter_definition() {
    let mut program = typed_block_argument_program();
    program.routines[0].blocks[1].params[1].dest = TempId(2);

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("duplicate block parameter definition `%t2`")
    }));
}

#[test]
fn verifier_rejects_block_parameters_without_predecessor_contributions() {
    let mut program = typed_block_argument_program();
    program.routines[0].blocks[0].params.push(NirBlockParam {
        dest: TempId(6),
        ty: byte_type(),
    });
    program.routines[0]
        .temps
        .push(block_temp_table_entry(6, byte_type(), 0));

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("block parameters require at least one predecessor edge")
    }));
}

#[test]
fn verifier_rejects_edge_value_unavailable_at_predecessor_terminator() {
    let mut program = typed_block_argument_program();
    let NirTerminator::Goto(edge) = &mut program.routines[0].blocks[0].terminator else {
        panic!("expected goto");
    };
    edge.args[1] = temp_value(3, byte_type());

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("edge argument uses temp `%t3` before its definition")
    }));
}

#[test]
fn optimizer_preserves_typed_block_edges_and_rebuilds_parameter_definitions() {
    let optimized = optimize_program(&typed_block_argument_program())
        .expect("optimize verifier-clean block arguments");
    let routine = &optimized.routines[0];
    let NirTerminator::Goto(edge) = &routine.blocks[0].terminator else {
        panic!("expected goto");
    };

    assert_eq!(
        edge.args,
        vec![
            NirValue::ConstU8(7),
            NirValue::ConstU8(3),
            NirValue::ConstU16(3),
            NirValue::StaticAddr {
                id: SymbolId(0),
                name: "table".to_string(),
                ty: byte_pointer_type(),
            },
        ]
    );
    assert_eq!(routine.blocks[1].params.len(), 4);
    assert!(
        routine
            .temps
            .iter()
            .filter(|temp| temp.def.op_index.is_none())
            .all(|temp| temp.def.block == BlockId(1))
    );
    assert_eq!(verify_program(&optimized), Ok(()));
}

#[test]
fn verifier_rejects_open_block() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: NirTerminator::Open,
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("no terminator")),
        "expected open-block diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn routine_local_defines_do_not_lower_to_executable_metadata_ops() {
    let source = "PROC Main() DEFINE NOP=\"$EA\" [ NOP ] RETURN";
    let tokens = crate::lexer::tokenize(source).unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("routine-local DEFINE should be compile-time metadata only");
    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    assert!(
        main.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .all(|op| !matches!(op, NirOp::Define { .. })),
        "{main:#?}"
    );
    assert!(
        main.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(
                op,
                NirOp::MachineBlock { items, .. } if items == &[NirMachineItem::Byte(0xEA)]
            )),
        "{main:#?}"
    );
}

#[test]
fn routine_local_scalar_aliases_global_storage() {
    let source = "BYTE state PROC Main() BYTE high=state+1 high=$42 RETURN";
    let tokens = crate::lexer::tokenize(source).unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let high = main
        .locals
        .iter()
        .find(|local| local.name == "high")
        .expect("high local");
    assert!(matches!(
        high.backing,
        NirLocalBacking::GlobalAlias {
            ref target_name,
            offset: 1,
            ..
        } if target_name == "state"
    ));
    assert!(
        main.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(
                op,
                NirOp::Store {
                    place: NirPlace {
                        kind: NirPlaceKind::Field { offset: 1, .. },
                        ..
                    },
                    ..
                }
            ))
    );
}

#[test]
fn global_scalar_aliases_absolute_backed_global_storage() {
    let source = "SET $E=$CB SET $F=0 BYTE ARRAY line SET $E=$3000 BYTE low=line, high=line+1 PROC Main() RETURN";
    let tokens = crate::lexer::tokenize(source).unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    let line = program
        .globals
        .iter()
        .find(|global| global.name == "line")
        .expect("line global");
    assert_eq!(line.backing, NirGlobalBacking::Absolute(0x00CB));

    for (name, offset) in [("low", 0), ("high", 1)] {
        let alias = program
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap_or_else(|| panic!("{name} global"));
        assert_eq!(
            alias.backing,
            NirGlobalBacking::Alias {
                target: "line".to_string(),
                offset,
            }
        );
    }
}

#[test]
fn routine_local_machine_defines_do_not_leak_between_routines() {
    let source =
        "PROC One() DEFINE OP=\"$EA\" [ OP ] RETURN PROC Two() DEFINE OP=\"$60\" [ OP ] RETURN";
    let tokens = crate::lexer::tokenize(source).unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("routine-local DEFINE aliases should verify");
    let routine_machine_bytes = |name: &str| {
        let routine = program
            .routines
            .iter()
            .find(|routine| routine.name == name)
            .unwrap_or_else(|| panic!("missing {name} routine"));
        routine
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .find_map(|op| match op {
                NirOp::MachineBlock { items, .. } => Some(items.as_slice()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {name} machine block"))
            .to_vec()
    };

    assert_eq!(
        routine_machine_bytes("One"),
        vec![NirMachineItem::Byte(0xEA)]
    );
    assert_eq!(
        routine_machine_bytes("Two"),
        vec![NirMachineItem::Byte(0x60)]
    );
}

#[test]
fn empty_machine_blocks_do_not_lower_to_executable_ops() {
    let source = "PROC Cold=$A326()[] PROC Main() Cold() RETURN";
    let tokens = crate::lexer::tokenize(source).unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("empty machine block should not produce executable NIR");
    let cold = program
        .routines
        .iter()
        .find(|routine| routine.name == "Cold")
        .expect("Cold routine");
    assert!(
        cold.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .all(|op| !matches!(op, NirOp::MachineBlock { .. })),
        "{cold:#?}"
    );
}

#[test]
fn verifier_rejects_unknown_terminator() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: NirTerminator::Unknown("unsupported branch shape".to_string()),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown terminator")),
        "expected unknown-terminator diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_executable_error_type() {
    let error = error_type();
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![NirLocal {
                id: LocalId(0),
                name: "bad".to_string(),
                kind: "error".to_string(),
                storage: NirStorageClass::Scalar,
                ty: error.clone(),
                backing: NirLocalBacking::Ordinary,
                init: None,
            }],
            temps: vec![temp_table_entry(0, error.clone(), 0, 0)],
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Load {
                    dest: TempId(0),
                    ty: error,
                    place: NirPlace {
                        kind: NirPlaceKind::Local {
                            id: LocalId(0),
                            name: "bad".to_string(),
                        },
                        ty: Some(error_type()),
                    },
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("load result must not have Error type")),
        "expected Error type diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_missing_branch_target() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: NirTerminator::Branch {
                    condition: temp_value(0, byte_type()),
                    then_edge: edge(1),
                    else_edge: edge(2),
                },
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("does not exist")),
        "expected missing-target diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_non_bool_branch_condition() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Binary {
                        dest: TempId(0),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: byte_value(1),
                        right: byte_value(2),
                    }],
                    terminator: NirTerminator::Branch {
                        condition: temp_value(0, byte_type()),
                        then_edge: edge(1),
                        else_edge: edge(1),
                    },
                },
                NirBlock {
                    id: BlockId(1),
                    label: "bb1".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("branch condition must be")),
        "expected branch-condition diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_duplicate_block_labels() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Fallthrough,
                },
                NirBlock {
                    id: BlockId(1),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate block label")),
        "expected duplicate-label diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_duplicate_block_ids() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Fallthrough,
                },
                NirBlock {
                    id: BlockId(0),
                    label: "bb1".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate block id")),
        "expected duplicate-block-id diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_metadata_ops_in_executable_blocks() {
    let metadata_ops = [
        NirOp::Define {
            name: "SIZE".to_string(),
            value: "1".to_string(),
        },
        NirOp::Declare {
            name: "x".to_string(),
            kind: "Byte".to_string(),
        },
        NirOp::Note {
            text: "local x: Byte".to_string(),
        },
    ];

    for op in metadata_ops {
        let program = NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
                name: "Main".to_string(),
                params: Vec::new(),
                locals: Vec::new(),
                temps: Vec::new(),
                notes: Vec::new(),
                blocks: vec![NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![op],
                    terminator: NirTerminator::Return(None),
                }],
            }],
        };

        let diagnostics = verify_program(&program).expect_err("expected verifier error");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("metadata op")),
            "expected metadata-op diagnostic, got {diagnostics:?}"
        );
    }
}

#[test]
fn verifier_rejects_legacy_assign_ops() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Assign {
                    target: NirPlace {
                        kind: NirPlaceKind::Symbol("x".to_string()),
                        ty: None,
                    },
                    value: NirOperand {
                        kind: NirOperandKind::Literal {
                            text: "1".to_string(),
                            value: Some(1),
                        },
                        ty: None,
                    },
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("legacy Assign op")),
        "expected legacy-assign diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_legacy_set_ops() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Set {
                    address: card_literal_with_value("$491", 0x0491),
                    value: card_literal_with_value("$3000", 0x3000),
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("legacy SET op")),
        "expected legacy-SET diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_store_with_untyped_place() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Store {
                    place: NirPlace {
                        kind: NirPlaceKind::Symbol("x".to_string()),
                        ty: None,
                    },
                    src: byte_value(1),
                    ty: byte_type(),
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("store place has no NIR type")),
        "expected untyped-store-place diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_legacy_compound_assignment_ops() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::CompoundAssign {
                    target: byte_place("x"),
                    op: "Add".to_string(),
                    value: card_literal("$1234"),
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("legacy CompoundAssign op")),
        "expected legacy-CompoundAssign diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_legacy_for_step_compound_assignment() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::CompoundAssign {
                    target: byte_place("i"),
                    op: "ForStep".to_string(),
                    value: byte_literal_with_value("1", 1),
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("legacy CompoundAssign op")),
        "expected legacy-CompoundAssign diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_accepts_literal_that_fits_narrow_store() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Store {
                    place: byte_place("x"),
                    src: NirValue::ConstU16(0x0011),
                    ty: byte_type(),
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    assert_eq!(verify_program(&program), Ok(()));
}

#[test]
fn verifier_accepts_defined_temp_use() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![temp_table_entry(0, byte_type(), 0, 0)],
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Binary {
                        dest: TempId(0),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: byte_value(1),
                        right: byte_value(2),
                    },
                    NirOp::Store {
                        place: byte_place("x"),
                        src: temp_value(0, byte_type()),
                        ty: byte_type(),
                    },
                ],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    assert_eq!(verify_program(&program), Ok(()));
}

#[test]
fn verifier_accepts_store_with_defined_temp_use() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![temp_table_entry(0, byte_type(), 0, 0)],
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Binary {
                        dest: TempId(0),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: byte_value(1),
                        right: byte_value(2),
                    },
                    NirOp::Store {
                        place: byte_place("x"),
                        src: temp_value(0, byte_type()),
                        ty: byte_type(),
                    },
                ],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    assert_eq!(verify_program(&program), Ok(()));
}

#[test]
fn verifier_rejects_store_width_mismatch() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Store {
                    place: byte_place("x"),
                    src: NirValue::ConstU16(0x1234),
                    ty: byte_type(),
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("store width mismatch")),
        "expected store-width diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_undefined_temp_use() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Store {
                    place: byte_place("x"),
                    src: temp_value(0, byte_type()),
                    ty: byte_type(),
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("uses undefined temp `%t0`")),
        "expected undefined-temp diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_string_storage_identity_in_scalar_load() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![temp_table_entry(0, byte_type(), 0, 0)],
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Load {
                    dest: TempId(0),
                    ty: byte_type(),
                    place: NirPlace {
                        kind: NirPlaceKind::Symbol("x".to_string()),
                        ty: Some(byte_type()),
                    },
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("load place uses string storage identity")),
        "expected string-storage diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_accepts_temp_use_from_dominating_block() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![temp_table_entry(0, byte_type(), 0, 0)],
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Binary {
                        dest: TempId(0),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: byte_value(1),
                        right: byte_value(2),
                    }],
                    terminator: NirTerminator::Goto(edge(1)),
                },
                NirBlock {
                    id: BlockId(1),
                    label: "bb1".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Store {
                        place: byte_place("x"),
                        src: temp_value(0, byte_type()),
                        ty: byte_type(),
                    }],
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    assert_eq!(verify_program(&program), Ok(()));
}

#[test]
fn verifier_rejects_temp_use_from_non_dominating_block() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![temp_table_entry(0, byte_type(), 1, 0)],
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Branch {
                        condition: byte_value(1),
                        then_edge: edge(1),
                        else_edge: edge(2),
                    },
                },
                NirBlock {
                    id: BlockId(1),
                    label: "bb1".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Binary {
                        dest: TempId(0),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: byte_value(1),
                        right: byte_value(2),
                    }],
                    terminator: NirTerminator::Goto(edge(2)),
                },
                NirBlock {
                    id: BlockId(2),
                    label: "bb2".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Store {
                        place: byte_place("x"),
                        src: temp_value(0, byte_type()),
                        ty: byte_type(),
                    }],
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("store source uses temp `%t0` before its definition")),
        "expected cross-block temp diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_missing_static_addr() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: NirTerminator::Return(Some(NirValue::StaticAddr {
                    id: SymbolId(99),
                    name: "__missing".to_string(),
                    ty: byte_pointer_type(),
                })),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("missing static data id `99`")),
        "expected missing-static diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn verifier_rejects_duplicate_temp_definition() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Binary {
                        dest: TempId(0),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: byte_value(1),
                        right: byte_value(2),
                    },
                    NirOp::Binary {
                        dest: TempId(0),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: byte_value(3),
                        right: byte_value(4),
                    },
                ],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let diagnostics = verify_program(&program).expect_err("expected verifier error");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("duplicate temp definition `%t0`")),
        "expected duplicate-temp diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn optimizer_removes_unreachable_blocks() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
                NirBlock {
                    id: BlockId(1),
                    label: "dead".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    assert_eq!(optimized.routines[0].blocks.len(), 1);
    assert_eq!(optimized.routines[0].blocks[0].label, "bb0");
}

#[test]
fn optimizer_folds_constants_and_simplifies_branches() {
    let condition = NirType {
        kind: NirTypeKind::Bool,
        summary: "condition".to_string(),
        width: Some(1),
        pointer: false,
    };
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![temp_table_entry(0, condition.clone(), 0, 0)],
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "bb0".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Compare {
                        dest: TempId(0),
                        ty: condition.clone(),
                        op: NirCompareOp::Eq,
                        left: byte_value(1),
                        right: byte_value(1),
                    }],
                    terminator: NirTerminator::Branch {
                        condition: temp_value(0, condition.clone()),
                        then_edge: edge(1),
                        else_edge: edge(2),
                    },
                },
                NirBlock {
                    id: BlockId(1),
                    label: "then".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
                NirBlock {
                    id: BlockId(2),
                    label: "else".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    let routine = &optimized.routines[0];
    assert!(routine.blocks[0].ops.is_empty());
    assert_eq!(routine.blocks[0].terminator, NirTerminator::Goto(edge(1)));
    assert!(routine.blocks.iter().all(|block| block.label != "else"));
    assert!(routine.temps.is_empty());
}

#[test]
fn optimizer_eliminates_dead_pure_temps_but_keeps_loads() {
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![
                temp_table_entry(0, byte_type(), 0, 0),
                temp_table_entry(1, byte_type(), 0, 1),
            ],
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Load {
                        dest: TempId(0),
                        ty: byte_type(),
                        place: byte_place("hw"),
                    },
                    NirOp::Binary {
                        dest: TempId(1),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: byte_value(1),
                        right: byte_value(2),
                    },
                ],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    assert_eq!(optimized.routines[0].blocks[0].ops.len(), 1);
    assert!(matches!(
        optimized.routines[0].blocks[0].ops[0],
        NirOp::Load { .. }
    ));
}

#[test]
fn optimizer_keeps_pure_temp_used_in_successor_block() {
    let ty = byte_type();
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![
                temp_table_entry(0, ty.clone(), 0, 0),
                temp_table_entry(1, ty.clone(), 0, 1),
            ],
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        NirOp::Load {
                            dest: TempId(0),
                            ty: ty.clone(),
                            place: byte_place("input"),
                        },
                        NirOp::Binary {
                            dest: TempId(1),
                            ty: ty.clone(),
                            op: NirBinaryOp::Add,
                            left: temp_value(0, ty.clone()),
                            right: byte_value(1),
                        },
                    ],
                    terminator: NirTerminator::Goto(edge(1)),
                },
                NirBlock {
                    id: BlockId(1),
                    label: "use".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Store {
                        place: byte_place("output"),
                        src: temp_value(1, ty.clone()),
                        ty: ty.clone(),
                    }],
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    assert!(matches!(
        optimized.routines[0].blocks[0].ops.as_slice(),
        [
            NirOp::Load {
                dest: TempId(0),
                ..
            },
            NirOp::Binary {
                dest: TempId(1),
                ..
            }
        ]
    ));
}

#[test]
fn optimizer_eliminates_dead_pure_temp_chain_across_blocks_to_fixed_point() {
    let ty = byte_type();
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![
                temp_table_entry(0, ty.clone(), 0, 0),
                temp_table_entry(1, ty.clone(), 0, 1),
                temp_table_entry(2, ty.clone(), 1, 0),
            ],
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        NirOp::Load {
                            dest: TempId(0),
                            ty: ty.clone(),
                            place: byte_place("input"),
                        },
                        NirOp::Binary {
                            dest: TempId(1),
                            ty: ty.clone(),
                            op: NirBinaryOp::Add,
                            left: temp_value(0, ty.clone()),
                            right: byte_value(1),
                        },
                    ],
                    terminator: NirTerminator::Goto(edge(1)),
                },
                NirBlock {
                    id: BlockId(1),
                    label: "dead".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Binary {
                        dest: TempId(2),
                        ty: ty.clone(),
                        op: NirBinaryOp::Add,
                        left: temp_value(1, ty.clone()),
                        right: byte_value(1),
                    }],
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    assert!(matches!(
        optimized.routines[0].blocks[0].ops.as_slice(),
        [NirOp::Load {
            dest: TempId(0),
            ..
        }]
    ));
    assert!(optimized.routines[0].blocks[1].ops.is_empty());
    assert_eq!(
        optimized.routines[0]
            .temps
            .iter()
            .map(|temp| temp.id)
            .collect::<Vec<_>>(),
        vec![TempId(0)]
    );
}

#[test]
fn optimizer_propagates_folded_constant_to_successor_block() {
    let ty = byte_type();
    let program = optimizer_program(
        vec![temp_table_entry(0, ty.clone(), 0, 0)],
        vec![
            NirBlock {
                id: BlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Binary {
                    dest: TempId(0),
                    ty: ty.clone(),
                    op: NirBinaryOp::Add,
                    left: byte_value(1),
                    right: byte_value(2),
                }],
                terminator: NirTerminator::Goto(edge(1)),
            },
            NirBlock {
                id: BlockId(1),
                label: "use".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Store {
                    place: byte_place("output"),
                    src: temp_value(0, ty.clone()),
                    ty: ty.clone(),
                }],
                terminator: NirTerminator::Return(None),
            },
        ],
    );

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    assert!(optimized.routines[0].blocks[0].ops.is_empty());
    assert!(matches!(
        &optimized.routines[0].blocks[1].ops[0],
        NirOp::Store {
            src: NirValue::ConstU8(3),
            ..
        }
    ));
    assert!(optimized.routines[0].temps.is_empty());
}

#[test]
fn optimizer_propagates_common_alias_through_diamond_join() {
    let byte = byte_type();
    let condition = NirType {
        kind: NirTypeKind::Bool,
        summary: "condition".to_string(),
        width: Some(1),
        pointer: false,
    };
    let program = optimizer_program(
        vec![
            temp_table_entry(0, byte.clone(), 0, 0),
            temp_table_entry(1, byte.clone(), 0, 1),
            temp_table_entry(2, condition.clone(), 0, 2),
        ],
        vec![
            NirBlock {
                id: BlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Load {
                        dest: TempId(0),
                        ty: byte.clone(),
                        place: byte_place("input"),
                    },
                    NirOp::Binary {
                        dest: TempId(1),
                        ty: byte.clone(),
                        op: NirBinaryOp::Add,
                        left: temp_value(0, byte.clone()),
                        right: byte_value(0),
                    },
                    NirOp::Compare {
                        dest: TempId(2),
                        ty: condition.clone(),
                        op: NirCompareOp::Ne,
                        left: temp_value(0, byte.clone()),
                        right: byte_value(0),
                    },
                ],
                terminator: NirTerminator::Branch {
                    condition: temp_value(2, condition),
                    then_edge: edge(1),
                    else_edge: edge(2),
                },
            },
            NirBlock {
                id: BlockId(1),
                label: "left".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: NirTerminator::Goto(edge(3)),
            },
            NirBlock {
                id: BlockId(2),
                label: "right".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: NirTerminator::Goto(edge(3)),
            },
            NirBlock {
                id: BlockId(3),
                label: "join".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Store {
                    place: byte_place("output"),
                    src: temp_value(1, byte.clone()),
                    ty: byte.clone(),
                }],
                terminator: NirTerminator::Return(None),
            },
        ],
    );

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    assert!(matches!(
        &optimized.routines[0].blocks[3].ops[0],
        NirOp::Store {
            src: NirValue::Temp { id: TempId(0), .. },
            ..
        }
    ));
    assert!(
        optimized.routines[0]
            .temps
            .iter()
            .all(|temp| temp.id != TempId(1))
    );
}

#[test]
fn optimizer_cancels_constant_offsets_across_blocks() {
    let ty = byte_type();
    let program = optimizer_program(
        vec![
            temp_table_entry(0, ty.clone(), 0, 0),
            temp_table_entry(1, ty.clone(), 0, 1),
            temp_table_entry(2, ty.clone(), 1, 0),
        ],
        vec![
            NirBlock {
                id: BlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Load {
                        dest: TempId(0),
                        ty: ty.clone(),
                        place: byte_place("input"),
                    },
                    NirOp::Binary {
                        dest: TempId(1),
                        ty: ty.clone(),
                        op: NirBinaryOp::Add,
                        left: temp_value(0, ty.clone()),
                        right: byte_value(5),
                    },
                ],
                terminator: NirTerminator::Goto(edge(1)),
            },
            NirBlock {
                id: BlockId(1),
                label: "cancel".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Binary {
                        dest: TempId(2),
                        ty: ty.clone(),
                        op: NirBinaryOp::Sub,
                        left: temp_value(1, ty.clone()),
                        right: byte_value(5),
                    },
                    NirOp::Store {
                        place: byte_place("output"),
                        src: temp_value(2, ty.clone()),
                        ty: ty.clone(),
                    },
                ],
                terminator: NirTerminator::Return(None),
            },
        ],
    );

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    assert!(matches!(
        optimized.routines[0].blocks[0].ops.as_slice(),
        [NirOp::Load {
            dest: TempId(0),
            ..
        }]
    ));
    assert!(matches!(
        optimized.routines[0].blocks[1].ops.as_slice(),
        [NirOp::Store {
            src: NirValue::Temp { id: TempId(0), .. },
            ..
        }]
    ));
}

#[test]
fn optimizer_propagates_constant_through_loop_backedge() {
    let ty = byte_type();
    let program = optimizer_program(
        vec![temp_table_entry(0, ty.clone(), 0, 0)],
        vec![
            NirBlock {
                id: BlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Binary {
                    dest: TempId(0),
                    ty: ty.clone(),
                    op: NirBinaryOp::Add,
                    left: byte_value(1),
                    right: byte_value(2),
                }],
                terminator: NirTerminator::Goto(edge(1)),
            },
            NirBlock {
                id: BlockId(1),
                label: "loop".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Store {
                    place: byte_place("output"),
                    src: temp_value(0, ty.clone()),
                    ty: ty.clone(),
                }],
                terminator: NirTerminator::Goto(edge(1)),
            },
        ],
    );

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    assert!(matches!(
        &optimized.routines[0].blocks[1].ops[0],
        NirOp::Store {
            src: NirValue::ConstU8(3),
            ..
        }
    ));
}

#[test]
fn optimizer_aliases_algebraic_identity_temps() {
    let ty = byte_type();
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: (0..7)
                .map(|id| temp_table_entry(id, ty.clone(), 0, id as usize))
                .collect(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Load {
                        dest: TempId(0),
                        ty: ty.clone(),
                        place: byte_place("x"),
                    },
                    NirOp::Binary {
                        dest: TempId(1),
                        ty: ty.clone(),
                        op: NirBinaryOp::Add,
                        left: temp_value(0, ty.clone()),
                        right: byte_value(0),
                    },
                    NirOp::Binary {
                        dest: TempId(2),
                        ty: ty.clone(),
                        op: NirBinaryOp::Add,
                        left: byte_value(0),
                        right: temp_value(1, ty.clone()),
                    },
                    NirOp::Binary {
                        dest: TempId(3),
                        ty: ty.clone(),
                        op: NirBinaryOp::Sub,
                        left: temp_value(2, ty.clone()),
                        right: byte_value(0),
                    },
                    NirOp::Binary {
                        dest: TempId(4),
                        ty: ty.clone(),
                        op: NirBinaryOp::Or,
                        left: temp_value(3, ty.clone()),
                        right: byte_value(0),
                    },
                    NirOp::Binary {
                        dest: TempId(5),
                        ty: ty.clone(),
                        op: NirBinaryOp::Xor,
                        left: temp_value(4, ty.clone()),
                        right: byte_value(0),
                    },
                    NirOp::Binary {
                        dest: TempId(6),
                        ty: ty.clone(),
                        op: NirBinaryOp::And,
                        left: temp_value(5, ty.clone()),
                        right: byte_value(0xFF),
                    },
                    NirOp::Store {
                        place: byte_place("out"),
                        src: temp_value(6, ty.clone()),
                        ty: ty.clone(),
                    },
                ],
                terminator: NirTerminator::Return(None),
            }],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    let ops = &optimized.routines[0].blocks[0].ops;
    assert_eq!(ops.len(), 2);
    assert!(matches!(
        ops[0],
        NirOp::Load {
            dest: TempId(0),
            ..
        }
    ));
    assert!(matches!(
        &ops[1],
        NirOp::Store {
            src: NirValue::Temp { id: TempId(0), .. },
            ..
        }
    ));
    assert_eq!(optimized.routines[0].temps.len(), 1);
}

#[test]
fn optimizer_aliases_word_all_ones_identity() {
    let ty = card_type();
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![
                temp_table_entry(0, ty.clone(), 0, 0),
                temp_table_entry(1, ty.clone(), 0, 1),
            ],
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Load {
                        dest: TempId(0),
                        ty: ty.clone(),
                        place: byte_place("x"),
                    },
                    NirOp::Binary {
                        dest: TempId(1),
                        ty: ty.clone(),
                        op: NirBinaryOp::And,
                        left: temp_value(0, ty.clone()),
                        right: NirValue::ConstU16(0xFFFF),
                    },
                ],
                terminator: NirTerminator::Return(Some(temp_value(1, ty.clone()))),
            }],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    let ops = &optimized.routines[0].blocks[0].ops;
    assert_eq!(ops.len(), 1);
    assert_eq!(
        optimized.routines[0].blocks[0].terminator,
        NirTerminator::Return(Some(temp_value(0, ty)))
    );
}

#[test]
fn optimizer_cancels_local_constant_offsets() {
    let ty = card_type();
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![
                temp_table_entry(0, ty.clone(), 0, 0),
                temp_table_entry(1, ty.clone(), 0, 1),
                temp_table_entry(2, ty.clone(), 0, 2),
            ],
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Load {
                        dest: TempId(0),
                        ty: ty.clone(),
                        place: byte_place("x"),
                    },
                    NirOp::Binary {
                        dest: TempId(1),
                        ty: ty.clone(),
                        op: NirBinaryOp::Add,
                        left: temp_value(0, ty.clone()),
                        right: NirValue::ConstU16(2),
                    },
                    NirOp::Binary {
                        dest: TempId(2),
                        ty: ty.clone(),
                        op: NirBinaryOp::Sub,
                        left: temp_value(1, ty.clone()),
                        right: NirValue::ConstU16(2),
                    },
                ],
                terminator: NirTerminator::Return(Some(temp_value(2, ty.clone()))),
            }],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    let ops = &optimized.routines[0].blocks[0].ops;
    assert_eq!(ops.len(), 1);
    assert_eq!(
        optimized.routines[0].blocks[0].terminator,
        NirTerminator::Return(Some(temp_value(0, ty)))
    );
}

#[test]
fn optimizer_canonicalizes_local_constant_offset_chains() {
    let ty = card_type();
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![
                temp_table_entry(0, ty.clone(), 0, 0),
                temp_table_entry(1, ty.clone(), 0, 1),
                temp_table_entry(2, ty.clone(), 0, 2),
            ],
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Load {
                        dest: TempId(0),
                        ty: ty.clone(),
                        place: byte_place("x"),
                    },
                    NirOp::Binary {
                        dest: TempId(1),
                        ty: ty.clone(),
                        op: NirBinaryOp::Add,
                        left: temp_value(0, ty.clone()),
                        right: NirValue::ConstU16(2),
                    },
                    NirOp::Binary {
                        dest: TempId(2),
                        ty: ty.clone(),
                        op: NirBinaryOp::Add,
                        left: temp_value(1, ty.clone()),
                        right: NirValue::ConstU16(3),
                    },
                ],
                terminator: NirTerminator::Return(Some(temp_value(2, ty.clone()))),
            }],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    let ops = &optimized.routines[0].blocks[0].ops;
    assert_eq!(ops.len(), 2);
    assert!(matches!(
        ops[0],
        NirOp::Load {
            dest: TempId(0),
            ..
        }
    ));
    assert_eq!(
        ops[1],
        NirOp::Binary {
            dest: TempId(2),
            ty: ty.clone(),
            op: NirBinaryOp::Add,
            left: temp_value(0, ty.clone()),
            right: NirValue::ConstU16(5),
        }
    );
}

#[test]
fn optimizer_keeps_non_identity_subtraction_and_pointer_arithmetic() {
    let byte = byte_type();
    let pointer = byte_pointer_type();
    let program = NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![
                temp_table_entry(0, byte.clone(), 0, 0),
                temp_table_entry(1, byte.clone(), 0, 1),
                temp_table_entry(2, pointer.clone(), 0, 3),
                temp_table_entry(3, pointer.clone(), 0, 4),
            ],
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Load {
                        dest: TempId(0),
                        ty: byte.clone(),
                        place: byte_place("x"),
                    },
                    NirOp::Binary {
                        dest: TempId(1),
                        ty: byte.clone(),
                        op: NirBinaryOp::Sub,
                        left: byte_value(0),
                        right: temp_value(0, byte.clone()),
                    },
                    NirOp::Store {
                        place: byte_place("out"),
                        src: temp_value(1, byte.clone()),
                        ty: byte.clone(),
                    },
                    NirOp::AddrOf {
                        dest: TempId(2),
                        ty: pointer.clone(),
                        place: byte_place("x"),
                    },
                    NirOp::Binary {
                        dest: TempId(3),
                        ty: pointer.clone(),
                        op: NirBinaryOp::Add,
                        left: temp_value(2, pointer.clone()),
                        right: NirValue::ConstU16(0),
                    },
                ],
                terminator: NirTerminator::Return(Some(temp_value(3, pointer.clone()))),
            }],
        }],
    };

    let optimized = optimize_program(&program).expect("optimize verifier-clean NIR");
    let ops = &optimized.routines[0].blocks[0].ops;
    assert!(ops.iter().any(|op| matches!(
        op,
        NirOp::Binary {
            dest: TempId(1),
            op: NirBinaryOp::Sub,
            ..
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        NirOp::Binary {
            dest: TempId(3),
            op: NirBinaryOp::Add,
            ..
        }
    )));
}

#[test]
fn verifier_and_printer_expose_structured_memory_effect_regions() {
    let program = memory_effect_program(NirMemoryRegion {
        kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(0))),
        offset: 0,
        size: 1,
    });

    verify_program(&program).expect("valid exact local effect region");
    assert!(format_program(&program).contains("writes:local0+0:1"));
}

#[test]
fn verifier_rejects_missing_and_malformed_memory_effect_regions() {
    let missing = verify_program(&memory_effect_program(NirMemoryRegion {
        kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(9))),
        offset: 0,
        size: 1,
    }))
    .expect_err("missing effect-region storage must fail verification");
    assert!(
        missing
            .iter()
            .any(|diagnostic| diagnostic.message.contains("missing storage identity"))
    );

    let zero_size = verify_program(&memory_effect_program(NirMemoryRegion {
        kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(0))),
        offset: 0,
        size: 0,
    }))
    .expect_err("zero-size effect region must fail verification");
    assert!(
        zero_size
            .iter()
            .any(|diagnostic| diagnostic.message.contains("zero-size region"))
    );
}

fn memory_effect_program(region: NirMemoryRegion) -> NirProgram {
    NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![NirLocal {
                id: LocalId(0),
                name: "x".to_string(),
                kind: "Byte".to_string(),
                storage: NirStorageClass::Scalar,
                ty: byte_type(),
                backing: NirLocalBacking::Ordinary,
                init: None,
            }],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Call {
                    callee: NirCallee::Builtin("Touch".to_string()),
                    args: Vec::new(),
                    result: None,
                    signature: None,
                    effects: NirCallEffects {
                        memory: NirMemoryEffects {
                            reads: NirMemoryAccess::None,
                            writes: NirMemoryAccess::Regions(vec![region]),
                        },
                        may_call_os: false,
                        opaque: false,
                    },
                }],
                terminator: NirTerminator::Return(None),
            }],
        }],
    }
}

fn byte_place(name: &str) -> NirPlace {
    NirPlace {
        kind: NirPlaceKind::Local {
            id: LocalId(0),
            name: name.to_string(),
        },
        ty: Some(byte_type()),
    }
}

fn card_literal(value: &str) -> NirOperand {
    card_literal_with_value(value, 0x1234)
}

fn card_literal_with_value(text: &str, value: u16) -> NirOperand {
    NirOperand {
        kind: NirOperandKind::Literal {
            text: text.to_string(),
            value: Some(value),
        },
        ty: Some(card_type()),
    }
}

fn byte_literal_with_value(text: &str, value: u16) -> NirOperand {
    NirOperand {
        kind: NirOperandKind::Literal {
            text: text.to_string(),
            value: Some(value),
        },
        ty: Some(byte_type()),
    }
}

fn byte_value(value: u8) -> NirValue {
    NirValue::ConstU8(value)
}

fn temp_value(id: u32, ty: NirType) -> NirValue {
    NirValue::Temp { id: TempId(id), ty }
}

fn temp_table_entry(id: u32, ty: NirType, block: u32, op_index: usize) -> NirTemp {
    NirTemp {
        id: TempId(id),
        ty,
        def: NirTempDef {
            block: BlockId(block),
            op_index: Some(op_index),
        },
    }
}

fn block_temp_table_entry(id: u32, ty: NirType, block: u32) -> NirTemp {
    NirTemp {
        id: TempId(id),
        ty,
        def: NirTempDef {
            block: BlockId(block),
            op_index: None,
        },
    }
}

fn typed_block_argument_program() -> NirProgram {
    let byte = byte_type();
    let card = card_type();
    let pointer = byte_pointer_type();
    NirProgram {
        globals: Vec::new(),
        statics: vec![NirStaticData {
            id: SymbolId(0),
            name: "table".to_string(),
            ty: byte.clone(),
            bytes: vec![0],
            display: "table".to_string(),
            alignment: 1,
            mutable: true,
            section: "data".to_string(),
        }],
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![
                temp_table_entry(0, byte.clone(), 0, 0),
                temp_table_entry(1, card.clone(), 0, 1),
                block_temp_table_entry(2, byte.clone(), 1),
                block_temp_table_entry(3, byte.clone(), 1),
                block_temp_table_entry(4, card.clone(), 1),
                block_temp_table_entry(5, pointer.clone(), 1),
            ],
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: vec![
                        NirOp::Binary {
                            dest: TempId(0),
                            ty: byte.clone(),
                            op: NirBinaryOp::Add,
                            left: NirValue::ConstU8(1),
                            right: NirValue::ConstU8(2),
                        },
                        NirOp::Binary {
                            dest: TempId(1),
                            ty: card.clone(),
                            op: NirBinaryOp::Add,
                            left: NirValue::ConstU16(1),
                            right: NirValue::ConstU16(2),
                        },
                    ],
                    terminator: NirTerminator::Goto(NirEdge {
                        target: BlockId(1),
                        args: vec![
                            NirValue::ConstU8(7),
                            temp_value(0, byte.clone()),
                            temp_value(1, card.clone()),
                            NirValue::StaticAddr {
                                id: SymbolId(0),
                                name: "table".to_string(),
                                ty: pointer.clone(),
                            },
                        ],
                    }),
                },
                NirBlock {
                    id: BlockId(1),
                    label: "join".to_string(),
                    params: vec![
                        NirBlockParam {
                            dest: TempId(2),
                            ty: byte.clone(),
                        },
                        NirBlockParam {
                            dest: TempId(3),
                            ty: byte,
                        },
                        NirBlockParam {
                            dest: TempId(4),
                            ty: card,
                        },
                        NirBlockParam {
                            dest: TempId(5),
                            ty: pointer,
                        },
                    ],
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
            ],
        }],
    }
}

fn optimizer_program(temps: Vec<NirTemp>, blocks: Vec<NirBlock>) -> NirProgram {
    NirProgram {
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps,
            notes: Vec::new(),
            blocks,
        }],
    }
}

fn temp_operand(id: u32, ty: NirType) -> NirOperand {
    NirOperand {
        kind: NirOperandKind::Temp(TempId(id)),
        ty: Some(ty),
    }
}

fn byte_type() -> NirType {
    NirType {
        kind: NirTypeKind::U8,
        summary: "Byte".to_string(),
        width: Some(1),
        pointer: false,
    }
}

fn card_type() -> NirType {
    NirType {
        kind: NirTypeKind::U16,
        summary: "Card".to_string(),
        width: Some(2),
        pointer: false,
    }
}

fn error_type() -> NirType {
    NirType {
        kind: NirTypeKind::Error,
        summary: "error".to_string(),
        width: None,
        pointer: false,
    }
}

fn byte_pointer_type() -> NirType {
    NirType {
        kind: NirTypeKind::Ptr16 {
            pointee: Some(Box::new(NirTypeKind::U8)),
        },
        summary: "Byte*".to_string(),
        width: Some(2),
        pointer: true,
    }
}
