use super::*;

use std::collections::BTreeSet;

use crate::ast::FundType;
use crate::includes::{ModuleLoadOptions, load_compilation_from_provider};
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::semantic::{SemanticOptions, ValueType, analyze_with_options};
use crate::source::{InMemorySourceProvider, SourceOrigin};

fn edge(target: u32) -> NirEdge {
    NirEdge {
        target: BlockId(target),
        args: Vec::new(),
    }
}

fn lower_modern_source(source: &str) -> NirProgram {
    let tokens = tokenize(source).expect("tokenize modern source");
    let program = parse(&tokens).expect("parse modern source");
    let model =
        analyze_with_options(&program, SemanticOptions::modern()).expect("analyze modern source");
    let semir = crate::semantic::ir::lower_program(&program, &model);
    lower_program(&semir)
}

fn lower_modern_source_for_target(source: &str, target: crate::target::TargetId) -> NirProgram {
    let tokens = tokenize(source).expect("tokenize modern source");
    let program = parse(&tokens).expect("parse modern source");
    let model = analyze_with_options(&program, SemanticOptions::modern().with_target(target))
        .expect("analyze modern source");
    let semir = crate::semantic::ir::lower_program(&program, &model);
    lower_program(&semir)
}

fn foreign_relocations(op: &NirOp) -> Option<&[NirForeignRelocation]> {
    let NirOp::ForeignCode { code, .. } = op else {
        return None;
    };
    let NirForeignCodePayload::Bytes { relocations, .. } = &code.payload else {
        return None;
    };
    Some(relocations)
}

fn structured_machine_items(op: &NirOp) -> Option<&[NirMachineItem]> {
    let NirOp::ForeignCode { code, .. } = op else {
        return None;
    };
    let NirForeignCodePayload::Structured(items) = &code.payload else {
        return None;
    };
    Some(items)
}

#[test]
fn runtime_helper_sets_become_verified_program_bindings() {
    let program = lower_modern_source(include_str!(
        "../../fixtures/mir6502/runtime_helper_set_sargs.act"
    ));
    verify_program(&program).expect("runtime helper binding NIR should verify");

    assert!(
        program
            .routines
            .iter()
            .all(|routine| routine.name != "<program>"),
        "a metadata-only SET must not create an executable routine"
    );
    assert_eq!(program.runtime_bindings.len(), 1);
    let binding = &program.runtime_bindings[0];
    assert_eq!(binding.name, "ACTION.RUNTIME.HELPER.SARGS");
    assert_eq!(binding.symbol, runtime_symbol_id(&binding.name));
    assert_eq!(
        binding.target,
        Some(NirRuntimeTarget::Routine(RoutineId(0)))
    );
    assert_eq!(program.routines[0].name, "r_Par");
}

#[test]
fn verifier_rejects_runtime_symbol_identity_mismatches() {
    let mut program = lower_modern_source(include_str!(
        "../../fixtures/mir6502/runtime_helper_set_sargs.act"
    ));
    program.runtime_bindings[0].symbol = RuntimeSymbolId(0);

    let diagnostics = verify_program(&program).expect_err("mismatched runtime id must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("runtime symbol `ACTION.RUNTIME.HELPER.SARGS` has a mismatched stable id")
    }));
}

#[test]
fn verifier_rejects_6502_foreign_code_for_other_targets() {
    for (source, kind) in [
        ("PROC Main() [$60]", "machine block"),
        ("PROC Main()\nASM\n  NOP\nENDASM\nRETURN", "inline assembly"),
    ] {
        let program =
            lower_modern_source_for_target(source, crate::target::TargetId::Motorola68000);
        let diagnostics = verify_program(&program).expect_err("6502 payload must be rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains(kind)
                && diagnostic.message.contains("atari-6502")
                && diagnostic.message.contains("motorola-68000")
                && diagnostic.message.contains("source span")
        }));
    }
}

#[test]
fn record_assignment_lowers_to_verified_copy_bytes() {
    let program = lower_modern_source(
        "TYPE Pair=[BYTE tag CARD word] Pair ARRAY table(2) Pair current PROC Main() current=table(1) RETURN",
    );
    verify_program(&program).expect("record copy NIR should verify");

    let copy = program.routines[0]
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op {
            NirOp::CopyBytes {
                destination,
                source,
                size,
                ..
            } => Some((destination, source, *size)),
            _ => None,
        })
        .expect("record assignment must produce copy_bytes");

    assert_eq!(copy.2, ByteSize::new(3));
    assert_eq!(
        copy.0.ty.as_ref().and_then(|ty| ty.width),
        Some(ByteSize::new(3))
    );
    assert_eq!(
        copy.1.ty.as_ref().and_then(|ty| ty.width),
        Some(ByteSize::new(3))
    );
    assert!(
        !program.routines[0]
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(op, NirOp::Unsupported { .. }))
    );

    let optimized = optimize_program(&program).expect("record copy must remain verifier-clean");
    verify_program(&optimized).expect("optimized record copy NIR should verify");
}

#[test]
fn verifier_rejects_invalid_record_copy_extent() {
    let mut program = lower_modern_source(
        "TYPE Pair=[BYTE tag CARD word] Pair source,destination PROC Main() destination=source RETURN",
    );
    let copy = program.routines[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.ops)
        .find(|op| matches!(op, NirOp::CopyBytes { .. }))
        .expect("record copy");
    let NirOp::CopyBytes { size, .. } = copy else {
        unreachable!()
    };
    *size = ByteSize::ZERO;

    let diagnostics = verify_program(&program).expect_err("zero-sized copy must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("copy_bytes extent must be non-zero")
    }));
}

#[test]
fn verifier_rejects_record_storage_in_the_scalar_operation_lane() {
    let mut program = lower_modern_source(
        "TYPE Pair=[BYTE tag CARD word] Pair source,destination PROC Main() destination=source RETURN",
    );
    let block = &mut program.routines[0].blocks[0];
    let (destination, ty) = match &block.ops[0] {
        NirOp::CopyBytes { destination, .. } => (
            destination.clone(),
            destination.ty.clone().expect("typed record destination"),
        ),
        other => panic!("expected record copy, got {other:?}"),
    };
    block.ops[0] = NirOp::Store {
        place: destination,
        src: NirValue::ConstU8(0),
        ty,
    };

    let diagnostics = verify_program(&program).expect_err("scalar record store must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot use a record in the byte/word scalar lane")
    }));
}

#[test]
fn descending_for_uses_directional_limit_and_underflow_guard() {
    let program = lower_modern_source(
        "BYTE i BYTE count PROC Main() FOR i=1 TO 0 STEP -1 DO count==+1 OD RETURN",
    );
    verify_program(&program).expect("descending FOR NIR should verify");

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let comparisons = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::Compare { op, right, .. } => Some((*op, right)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        comparisons
            .iter()
            .any(|(op, right)| *op == NirCompareOp::Ge && **right == NirValue::ConstU8(0)),
        "descending limit must compare the counter >= the end: {comparisons:?}"
    );
    assert!(
        comparisons
            .iter()
            .any(|(op, right)| *op == NirCompareOp::Lt && **right == NirValue::ConstU8(1)),
        "descending byte loop must exit before subtracting one from zero: {comparisons:?}"
    );
}

#[test]
fn ascending_for_uses_directional_limit_and_overflow_guard() {
    let program =
        lower_modern_source("BYTE i BYTE count PROC Main() FOR i=0 TO 255 DO count==+1 OD RETURN");
    verify_program(&program).expect("ascending FOR NIR should verify");

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let comparisons = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::Compare { op, right, .. } => Some((*op, right)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        comparisons
            .iter()
            .any(|(op, right)| *op == NirCompareOp::Le && **right == NirValue::ConstU8(255)),
        "ascending limit must compare the counter <= the end: {comparisons:?}"
    );
    assert!(
        comparisons
            .iter()
            .any(|(op, right)| *op == NirCompareOp::Gt && **right == NirValue::ConstU8(254)),
        "ascending byte loop must exit before adding one to 255: {comparisons:?}"
    );
}

#[test]
fn pointer_null_comparisons_use_pointer_width_operands() {
    let program = lower_modern_source(
        "BYTE POINTER p BYTE leftResult,rightResult PROC Main() leftResult=p<>0 rightResult=0<>p RETURN",
    );
    verify_program(&program).expect("pointer/null comparison NIR should verify");

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let comparisons = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::Compare {
                operand_ty,
                left,
                right,
                ..
            } => Some((operand_ty, left, right)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(comparisons.len(), 2);
    for (operand_ty, left, right) in comparisons {
        assert!(matches!(operand_ty.kind, NirTypeKind::Pointer { .. }));
        assert_eq!(super::facts::value_width(left), Some(ByteSize::new(2)));
        assert_eq!(super::facts::value_width(right), Some(ByteSize::new(2)));
    }
}

#[test]
fn mixed_scalar_comparison_widens_negative_literal_to_word() {
    let program = lower_modern_source("INT value BYTE result PROC Main() result=value>=-64 RETURN");
    verify_program(&program).expect("mixed scalar comparison NIR should verify");

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let (operand_ty, left, right) = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op {
            NirOp::Compare {
                op: NirCompareOp::Ge,
                operand_ty,
                left,
                right,
                ..
            } => Some((operand_ty, left, right)),
            _ => None,
        })
        .expect("word comparison");
    let NirValue::Temp { ty: left_ty, .. } = left else {
        panic!("expected loaded word operand");
    };
    let NirValue::Temp { ty: right_ty, .. } = right else {
        panic!("expected widened negative word operand");
    };
    assert_eq!(left_ty.width, Some(ByteSize::new(2)));
    assert_eq!(right_ty.width, Some(ByteSize::new(2)));
    assert_eq!(operand_ty.kind, NirTypeKind::I16);

    let optimized = optimize_program(&program).expect("optimize mixed scalar comparison");
    let main = optimized
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let right = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op {
            NirOp::Compare {
                op: NirCompareOp::Ge,
                right,
                ..
            } => Some(right),
            _ => None,
        })
        .expect("optimized word comparison");
    assert_eq!(right, &NirValue::integer_const(0xFFC0, NirIntegerType::I16));
}

#[test]
fn cartridge_integer_arithmetic_types_survive_semir_and_nir_lowering() {
    let program = lower_modern_source(
        "BYTE a,b,byteProduct,byteSum,byteDifference,comparison CARD cardProduct INT intProduct,negative \
         PROC Main() \
         intProduct=a*b \
         cardProduct=a*b \
         byteProduct=BYTE(a*b) \
         negative=-a \
         byteSum=250+10 \
         byteDifference=BYTE(1-2) \
         comparison=a*b<100 \
         RETURN",
    );
    verify_program(&program).expect("cartridge-compatible arithmetic NIR should verify");

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let ops = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .collect::<Vec<_>>();

    let multiplication_types = ops
        .iter()
        .filter_map(|op| match op {
            NirOp::Binary {
                op: NirBinaryOp::Mul,
                ty,
                ..
            } => Some(&ty.kind),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(multiplication_types.len(), 4);
    assert!(
        multiplication_types
            .iter()
            .all(|ty| **ty == NirTypeKind::I16),
        "multiplications must remain INT in assignment and comparison contexts: {ops:#?}"
    );
    assert!(ops.iter().any(|op| matches!(
        op,
        NirOp::Unary {
            op: NirUnaryOp::Neg,
            ty,
            ..
        } if ty.kind == NirTypeKind::I16
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        NirOp::Compare {
            op: NirCompareOp::Lt,
            operand_ty,
            ..
        } if operand_ty.kind == NirTypeKind::I16
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        NirOp::Cast {
            from,
            to,
            ..
        } if from.kind == NirTypeKind::I16 && to.kind == NirTypeKind::U16
    )));

    let optimized = optimize_program(&program).expect("arithmetic optimization should verify");
    let main = optimized
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let stored_constants = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::Store { src, .. } => match src {
                NirValue::IntegerConst { bits, ty } if *ty == NirIntegerType::U8 => {
                    Some(*bits as u8)
                }
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(stored_constants.contains(&4), "{main:#?}");
    assert!(stored_constants.contains(&u8::MAX), "{main:#?}");
}

#[test]
fn verifier_rejects_pre_cartridge_multiply_and_negation_result_types() {
    let byte = byte_type();
    let multiplication = optimizer_program(
        vec![temp_table_entry(0, byte.clone(), 0, 0)],
        vec![NirBlock {
            id: BlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![NirOp::Binary {
                dest: TempId(0),
                ty: byte.clone(),
                op: NirBinaryOp::Mul,
                left: NirValue::ConstU8(20),
                right: NirValue::ConstU8(20),
            }],
            terminator: NirTerminator::Return(None),
        }],
    );
    let diagnostics = verify_program(&multiplication).expect_err("BYTE multiply must be rejected");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("multiplication must produce cartridge-compatible INT")
    }));

    let negation = optimizer_program(
        vec![temp_table_entry(0, byte.clone(), 0, 0)],
        vec![NirBlock {
            id: BlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![NirOp::Unary {
                dest: TempId(0),
                ty: byte,
                op: NirUnaryOp::Neg,
                src: NirValue::ConstU8(1),
            }],
            terminator: NirTerminator::Return(None),
        }],
    );
    let diagnostics = verify_program(&negation).expect_err("BYTE negation must be rejected");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("negation must produce cartridge-compatible INT")
    }));
}

#[test]
fn verifier_rejects_unwidened_overflowing_constant_byte_arithmetic() {
    for (op, left, right) in [(NirBinaryOp::Add, 250, 10), (NirBinaryOp::Sub, 1, 2)] {
        let byte = byte_type();
        let program = optimizer_program(
            vec![temp_table_entry(0, byte.clone(), 0, 0)],
            vec![NirBlock {
                id: BlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![NirOp::Binary {
                    dest: TempId(0),
                    ty: byte,
                    op,
                    left: NirValue::ConstU8(left),
                    right: NirValue::ConstU8(right),
                }],
                terminator: NirTerminator::Return(None),
            }],
        );
        let diagnostics = verify_program(&program).expect_err("constant BYTE result must widen");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("constant BYTE addition or subtraction must produce INT")
        }));
    }
}

#[test]
fn verifier_rejects_mixed_width_compare_operands() {
    let mut program =
        lower_modern_source("INT value BYTE result PROC Main() result=value>=-64 RETURN");
    let compare = program
        .routines
        .iter_mut()
        .flat_map(|routine| &mut routine.blocks)
        .flat_map(|block| &mut block.ops)
        .find(|op| {
            matches!(
                op,
                NirOp::Compare {
                    op: NirCompareOp::Ge,
                    ..
                }
            )
        })
        .expect("word comparison");
    let NirOp::Compare { right, .. } = compare else {
        unreachable!();
    };
    *right = NirValue::ConstU8(0xC0);

    let diagnostics = verify_program(&program).expect_err("mixed widths must be rejected");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("compare operands width mismatch: left is 2 byte(s), right is 1 byte(s)")
    }));
}

#[test]
fn verifier_rejects_compare_operand_signedness_mismatch() {
    let mut program =
        lower_modern_source("INT value BYTE result PROC Main() result=value>=-64 RETURN");
    let compare = program
        .routines
        .iter_mut()
        .flat_map(|routine| &mut routine.blocks)
        .flat_map(|block| &mut block.ops)
        .find(|op| {
            matches!(
                op,
                NirOp::Compare {
                    op: NirCompareOp::Ge,
                    ..
                }
            )
        })
        .expect("word comparison");
    let NirOp::Compare { operand_ty, .. } = compare else {
        unreachable!();
    };
    operand_ty.kind = NirTypeKind::U16;
    operand_ty.summary = "Card".to_string();

    let diagnostics = verify_program(&program).expect_err("mixed signedness must be rejected");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("operand type Card does not match value type Int")
    }));
}

#[test]
fn optimizer_folds_constant_int_comparisons_as_signed() {
    let program = lower_modern_source(
        "INT value BYTE result PROC Main() value=64 IF value<=-64 THEN result=1 ELSE result=0 FI RETURN",
    );
    let optimized = optimize_program(&program).expect("optimize signed comparison");
    let main = optimized
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let stored = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::Store {
                src:
                    NirValue::IntegerConst {
                        bits: value,
                        ty: NirIntegerType::U8,
                    },
                place:
                    NirPlace {
                        kind: NirPlaceKind::Global { name, .. },
                        ..
                    },
                ..
            } if name == "result" => Some(*value as u8),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(stored, [0]);
}

fn display_name_leaf(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

#[test]
fn program_entry_note_tracks_the_last_proc_instead_of_main_or_a_function() {
    let tokens = tokenize("PROC Main() RETURN PROC Start() RETURN BYTE FUNC Helper() RETURN(0)")
        .expect("tokenize entry source");
    let ast = parse(&tokens).expect("parse entry source");
    let model =
        analyze_with_options(&ast, SemanticOptions::modern()).expect("analyze entry source");
    let mut semir = crate::semantic::ir::lower_program(&ast, &model);
    semir.modules[0].items.reverse();
    let program = lower_program(&semir);
    verify_program(&program).expect("program-entry NIR should verify");

    let entries = program
        .routines
        .iter()
        .filter(|routine| routine.entry.program)
        .map(|routine| routine.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(entries, ["Start"]);
}

#[test]
fn verifier_rejects_more_than_one_program_entry() {
    let mut program = lower_modern_source("PROC First() RETURN PROC Second() RETURN");
    program.routines[0].entry.program = true;

    let diagnostics = verify_program(&program).expect_err("duplicate entry must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("more than one program-entry routine")
    }));
}

#[test]
fn lexical_blocks_lower_shadowed_storage_by_stable_local_id() {
    let program = lower_modern_source(include_str!("../../fixtures/nir/lexical_blocks.act"));
    verify_program(&program).expect("lexical-block NIR should verify");
    let routine = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");

    let value_ids = routine
        .locals
        .iter()
        .filter(|local| display_name_leaf(&local.name).eq_ignore_ascii_case("value"))
        .map(|local| local.id)
        .collect::<Vec<_>>();
    assert_eq!(value_ids, [LocalId(0), LocalId(2), LocalId(3)]);

    let value_store_ids = routine
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::Store {
                place:
                    NirPlace {
                        kind: NirPlaceKind::Local { id, name },
                        ..
                    },
                ..
            } if display_name_leaf(name).eq_ignore_ascii_case("value") => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        value_store_ids,
        [LocalId(0), LocalId(2), LocalId(3), LocalId(2), LocalId(0)]
    );

    let addressed_local =
        routine
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .find_map(|op| match op {
                NirOp::AddrOf {
                    place:
                        NirPlace {
                            kind: NirPlaceKind::Local { id, name },
                            ..
                        },
                    ..
                } if display_name_leaf(name).eq_ignore_ascii_case("value") => Some(*id),
                _ => None,
            });
    assert_eq!(addressed_local, Some(LocalId(3)));

    let assembler_local_ids = routine
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| foreign_relocations(op))
        .flatten()
        .filter_map(|relocation| match relocation {
            NirForeignRelocation {
                target: NirForeignCodeTarget::Storage(NirStorageId::Local(id)),
                ..
            } => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(assembler_local_ids, [LocalId(3), LocalId(3)]);

    let printed = format_program(&program);
    assert!(!printed.to_ascii_lowercase().contains("lexical"));
    assert!(printed.contains("Main::block0::value"));
    assert!(printed.contains("Main::block0::block1::value"));
    crate::mir6502::lower_program(&program).expect("MIR6502 should consume lexical local IDs");

    let mut invalid = program.clone();
    let invalid_place = invalid
        .routines
        .iter_mut()
        .flat_map(|routine| &mut routine.blocks)
        .flat_map(|block| &mut block.ops)
        .find_map(|op| match op {
            NirOp::Store { place, .. } if matches!(place.kind, NirPlaceKind::Local { .. }) => {
                Some(place)
            }
            _ => None,
        })
        .expect("local store place");
    let NirPlaceKind::Local { id, .. } = &mut invalid_place.kind else {
        unreachable!()
    };
    *id = LocalId(99);
    let diagnostics = verify_program(&invalid).expect_err("unknown LocalId must be rejected");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("references unknown local id 99")
    }));
}

#[test]
fn focused_lexical_declaration_fixtures_preserve_storage_and_type_facts() {
    let storage_program =
        lower_modern_source(include_str!("../../fixtures/nir/local_storage_views.act"));
    verify_program(&storage_program).expect("local storage-view NIR should verify");
    let storage_routine = storage_program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");

    assert!(storage_routine.locals.iter().all(|local| {
        !matches!(
            local.storage,
            NirStorageClass::Type | NirStorageClass::Record
        )
    }));
    let value = storage_routine
        .locals
        .iter()
        .find(|local| local.name == "value")
        .expect("local value");
    let alias = storage_routine
        .locals
        .iter()
        .find(|local| local.name == "alias")
        .expect("local alias");
    assert!(matches!(
        alias.backing,
        NirLocalBacking::Alias {
            target,
            offset: ByteOffset::ZERO,
            ..
        } if target == value.id
    ));
    assert!(storage_routine.locals.iter().any(|local| {
        local.name == "absolute"
            && matches!(
                local.backing,
                NirLocalBacking::Absolute(address)
                    if address == AddressValue::data(0xD01F)
            )
    }));
    let storage_ops = storage_routine
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .collect::<Vec<_>>();
    assert!(
        storage_ops
            .iter()
            .any(|op| matches!(op, NirOp::VolatileLoad { .. }))
    );
    assert!(
        storage_ops
            .iter()
            .any(|op| matches!(op, NirOp::VolatileStore { .. }))
    );
    assert!(storage_ops.iter().any(|op| matches!(op, NirOp::Real(_))));
    assert!(storage_ops.iter().any(|op| matches!(
        op,
        NirOp::AddrOf {
            place: NirPlace {
                kind: NirPlaceKind::Local { id, .. },
                ..
            },
            ..
        } if *id == value.id
    )));

    let aggregate_program = lower_modern_source(include_str!(
        "../../fixtures/nir/local_aggregate_declarations.act"
    ));
    verify_program(&aggregate_program).expect("local aggregate NIR should verify");
    let aggregate_routine = aggregate_program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let aggregate_value = aggregate_routine
        .locals
        .iter()
        .find(|local| local.name == "value")
        .expect("aggregate address target");
    let addresses = aggregate_routine
        .locals
        .iter()
        .find(|local| local.name == "addresses")
        .expect("local relocation array");
    assert!(matches!(
        &addresses.init,
        Some(NirStorageInit::Descriptor { backing, .. })
            if matches!(
                backing.image.fragments.as_slice(),
                [NirDataFragment::Address {
                    target: NirDataAddressTarget::Storage(NirStorageId::Local(id)),
                    ..
                }] if *id == aggregate_value.id
            )
    ));

    let scopes_program =
        lower_modern_source(include_str!("../../fixtures/nir/lexical_type_scopes.act"));
    verify_program(&scopes_program).expect("lexical type-scope NIR should verify");
    let scopes_routine = scopes_program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let item_sizes = scopes_routine
        .locals
        .iter()
        .filter(|local| display_name_leaf(&local.name).eq_ignore_ascii_case("item"))
        .filter_map(|local| match local.init {
            Some(NirStorageInit::ZeroFill { bytes, .. }) => Some(bytes),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(item_sizes, [ByteSize::ONE, ByteSize::new(2)]);

    let output = crate::mir6502::generate_output(&scopes_program, crate::codegen::CODE_ORIGIN)
        .expect("MIR6502 should emit duplicate lexical display names by stable local ID");
    let item_addresses = output
        .map
        .storage_symbols
        .iter()
        .filter(|symbol| {
            display_name_leaf(&symbol.name).eq_ignore_ascii_case("item")
                && matches!(
                    &symbol.scope,
                    crate::codegen::CodegenSymbolScope::Routine(routine)
                        if routine == "Main"
                )
        })
        .map(|symbol| symbol.address)
        .collect::<BTreeSet<_>>();
    assert_eq!(item_addresses.len(), 2, "item storage addresses");
}

#[test]
fn formats_labeled_blocks() {
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: vec![NirGlobal {
            id: SymbolId(0),
            name: "counter".to_string(),
            kind: "Byte".to_string(),
            ty: None,
            storage_size: ByteSize::ONE,
            array: None,
            init: None,
            backing: NirGlobalBacking::Ordinary,
        }],
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![NirLocal {
                id: LocalId(0),
                name: "i".to_string(),
                kind: "Byte".to_string(),
                purpose: NirLocalPurpose::Storage,
                storage: NirStorageClass::Scalar,
                duration: crate::nir::NirStorageDuration::RoutineStatic,
                layout: crate::nir::NirObjectLayout::byte(),
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
    assert!(formatted.contains("routine r0 Main"));
    assert!(formatted.contains("bb0:"));
    assert!(formatted.contains("store i = 0"));
    assert!(formatted.contains("return"));
}

#[test]
fn named_module_executable_references_lower_to_stable_ids() {
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(
            root.clone(),
            b"MODULE APP\nUSE LIB.ONE AS ONE\nUSE LIB.TWO AS TWO\nBYTE alias=ONE.shared\nCARD ARRAY callback=ONE.Touch\nPROC Main() [<ONE.shared >ONE.shared <ONE.Touch >ONE.Touch] ONE.Touch() TWO.Touch() RETURN\nENDMODULE\n".to_vec(),
        )
        .with_source(
            SourceOrigin::host("project/lib/one.act"),
            b"MODULE LIB.ONE\nPUBLIC BYTE shared\nBYTE hiddenValue\nPUBLIC PROC Touch() RETURN\nPROC HiddenProc() RETURN\nENDMODULE\n".to_vec(),
        )
        .with_source(
            SourceOrigin::host("project/lib/two.act"),
            b"MODULE LIB.TWO\nPUBLIC BYTE shared\nBYTE hiddenValue\nPUBLIC PROC Touch() RETURN\nPROC HiddenProc() RETURN\nENDMODULE\n".to_vec(),
        );
    let loaded = load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default())
        .expect("load named modules");
    let model = crate::semantic::analyze_compilation(&loaded).expect("analyze named modules");
    let semir = crate::semantic::ir::lower_compilation(&loaded, &model);
    let program = lower_program(&semir);
    verify_program(&program).expect("named-module NIR should verify");

    let hidden_names = program
        .globals
        .iter()
        .filter(|global| global.name.contains("_HIDDENVALUE_"))
        .map(|global| global.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(hidden_names.len(), 2);
    assert_ne!(hidden_names[0], hidden_names[1]);

    let alias = program
        .globals
        .iter()
        .find(|global| global.name.contains("APP_ALIAS"))
        .expect("module alias global");
    let NirGlobalBacking::Alias { target, offset } = alias.backing else {
        panic!("expected ID-backed global alias: {:?}", alias.backing);
    };
    assert_eq!(offset, ByteOffset::ZERO);
    assert!(program.globals.iter().any(|global| global.id == target));

    let callback = program
        .globals
        .iter()
        .find(|global| global.name.contains("APP_CALLBACK"))
        .expect("routine-address global");
    assert!(matches!(
        callback.init,
        Some(NirGlobalInit::RoutineAddress { routine, .. })
            if (routine.0 as usize) < program.routines.len()
    ));

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name.contains("APP_MAIN"))
        .expect("main routine");
    let calls = main
        .blocks
        .iter()
        .flat_map(|block| block.ops.iter())
        .filter_map(|op| match op {
            NirOp::Call {
                callee: NirCallee::User { id, .. },
                ..
            } => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0], calls[1]);
    assert!(main.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        structured_machine_items(op).is_some_and(|items| {
            items
                .iter()
                .all(|item| matches!(item, NirMachineItem::Relocation { .. }))
        })
    }));
    crate::mir6502::lower_program(&program).expect("MIR6502 must consume resolved module IDs");
}

#[test]
fn nir_type_kind_tracks_semir_value_types() {
    let byte = NirType::from_value(&ValueType::fund(FundType::Byte));
    assert_eq!(byte.kind, NirTypeKind::U8);
    assert_eq!(byte.width, Some(ByteSize::ONE));
    assert!(!byte.pointer);

    let int = NirType::from_value(&ValueType::fund(FundType::Int));
    assert_eq!(int.kind, NirTypeKind::I16);
    assert_eq!(int.width, Some(ByteSize::new(2)));

    let pointer = NirType::from_value(&ValueType::pointer_to(ValueType::fund(FundType::Byte)));
    assert_eq!(
        pointer.kind,
        NirTypeKind::Pointer {
            pointee: Some(Box::new(NirTypeKind::U8)),
            address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
        }
    );
    assert_eq!(pointer.width, Some(ByteSize::new(2)));
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
fn native_targets_use_layout_driven_data_and_code_pointer_types() {
    let source = "BYTE POINTER data PROC POINTER callback PROC Main() data=0 RETURN";
    for (target, width) in [
        (crate::target::TargetId::Wdc65816Native, 3),
        (crate::target::TargetId::Motorola68000, 4),
    ] {
        let program = lower_modern_source_for_target(source, target);
        verify_program(&program).expect("native pointer NIR should verify");
        let data = program
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case("data"))
            .and_then(|global| global.ty.as_ref())
            .expect("data pointer type");
        assert_eq!(data.width, Some(ByteSize::new(width)));
        assert!(matches!(
            data.kind,
            NirTypeKind::Pointer { address_space, .. }
                if address_space == program.target_layout.data_pointer.address_space
        ));

        let callback = program
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case("callback"))
            .and_then(|global| global.ty.as_ref())
            .expect("callable pointer type");
        assert_eq!(callback.width, Some(ByteSize::new(width)));
        assert!(matches!(
            callback.kind,
            NirTypeKind::Callable { address_space, .. }
                if address_space == program.target_layout.code_pointer.address_space
        ));

        assert!(program.routines.iter().flat_map(|routine| &routine.blocks).flat_map(|block| &block.ops).any(|op| {
            matches!(op, NirOp::Store { src: NirValue::Null { ty }, .. } if ty.width == Some(ByteSize::new(width)))
        }));
    }
}

#[test]
fn aggregate_layout_facts_reach_nir_without_semir_reconstruction() {
    let source = "TYPE Inner=[BYTE tag CARD word] \
                  TYPE Matrix=[BYTE lead CARD word BYTE POINTER data \
                               PROC POINTER callback Inner nested BYTE tail] \
                  Matrix ARRAY rows(2) Matrix item \
                  PROC Main() item.tail=1 rows(1)=item RETURN";
    for (target, size, tail_offset) in [
        (crate::target::TargetId::Atari6502, 11, 10),
        (crate::target::TargetId::Wdc65816Native, 16, 14),
        (crate::target::TargetId::Wdc65816Small, 14, 12),
        (crate::target::TargetId::Motorola68000, 18, 16),
    ] {
        let program = lower_modern_source_for_target(source, target);
        verify_program(&program).expect("aggregate NIR should verify for every target layout");
        let item = program
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case("item"))
            .expect("record object");
        assert_eq!(item.storage_size, ByteSize::new(size));
        let rows = program
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case("rows"))
            .expect("record array");
        assert_eq!(rows.storage_size, ByteSize::new(size * 2));
        assert_eq!(
            rows.array.as_ref().map(|array| array.elem_size),
            Some(ByteSize::new(size))
        );
        let main = program
            .routines
            .iter()
            .find(|routine| routine.name == "Main")
            .expect("Main");
        assert!(main.blocks.iter().flat_map(|block| &block.ops).any(|op| {
            matches!(op, NirOp::Store { place: NirPlace { kind: NirPlaceKind::Field { offset, .. }, .. }, .. }
                if *offset == ByteOffset::new(tail_offset))
        }));
        assert!(main.blocks.iter().flat_map(|block| &block.ops).any(|op| {
            matches!(op, NirOp::CopyBytes { size: copy_size, .. } if *copy_size == ByteSize::new(size))
        }));
    }
}

#[test]
fn initialized_array_descriptors_follow_the_selected_data_pointer_width() {
    let source = "CARD ARRAY values(2)=[1 2] PROC Main() RETURN";
    for (target, descriptor_size) in [
        (crate::target::TargetId::Atari6502, 4),
        (crate::target::TargetId::Wdc65816Native, 5),
        (crate::target::TargetId::Wdc65816Small, 4),
        (crate::target::TargetId::Motorola68000, 6),
    ] {
        let program = lower_modern_source_for_target(source, target);
        verify_program(&program).expect("target-sized descriptor should verify");
        let values = program
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case("values"))
            .expect("values descriptor");
        assert_eq!(values.storage_size, ByteSize::new(descriptor_size));
        assert!(matches!(
            values.init,
            Some(NirGlobalInit::Descriptor { descriptor_size: size, size_word: Some(0), .. })
                if size == ByteSize::new(descriptor_size)
        ));
    }
}

#[test]
fn callable_types_have_stable_structural_signature_ids() {
    let first = crate::semantic::CallableType::new(
        crate::ast::RoutineKind::Proc,
        [ValueType::fund(FundType::Byte)],
        None,
    );
    let same = first.clone();
    let different = crate::semantic::CallableType::new(
        crate::ast::RoutineKind::Proc,
        [ValueType::fund(FundType::Card)],
        None,
    );
    let id = |callable| {
        let ty = NirType::from_value(&ValueType::callable_pointer(callable));
        match ty.kind {
            NirTypeKind::Callable { signature, .. } => signature,
            other => panic!("expected callable type, got {other:?}"),
        }
    };

    assert_eq!(id(first), id(same));
    assert_ne!(
        id(different),
        id(crate::semantic::CallableType::unknown_proc())
    );
}

#[test]
fn pointer_arithmetic_lowers_to_a_distinct_pointer_offset_operation() {
    let program = lower_modern_source("BYTE POINTER data PROC Main() data=data+1 RETURN");
    verify_program(&program).expect("pointer offset NIR should verify");
    assert!(
        program
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .any(|op| {
                matches!(
                    op,
                    NirOp::PointerOffset {
                        subtract: false,
                        ..
                    }
                )
            })
    );
    assert!(
        !program
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .any(|op| { matches!(op, NirOp::Binary { ty, .. } if ty.kind.is_pointer()) })
    );

    let native = lower_modern_source_for_target(
        "BYTE POINTER data PROC Main() data=data+1 RETURN",
        crate::target::TargetId::Motorola68000,
    );
    verify_program(&native).expect("the same pointer offset must verify with a 32-bit pointer");
}

#[test]
fn native_pointer_integer_conversions_are_explicit_and_checked() {
    let constant = lower_modern_source_for_target(
        "BYTE POINTER data PROC Main() data=$1234 RETURN",
        crate::target::TargetId::Motorola68000,
    );
    verify_program(&constant).expect("a fitting numeric address constant is portable");
    assert!(
        constant
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .any(|op| {
                matches!(op, NirOp::Store { src: NirValue::AddressConst { address, ty }, .. }
            if address.value == 0x1234 && ty.width == Some(ByteSize::new(4)))
            })
    );

    for target in [
        crate::target::TargetId::Wdc65816Native,
        crate::target::TargetId::Wdc65816Small,
        crate::target::TargetId::Motorola68000,
    ] {
        let dynamic = lower_modern_source_for_target(
            "CARD address BYTE POINTER data PROC Main() data=address RETURN",
            target,
        );
        let diagnostics = verify_program(&dynamic)
            .expect_err("dynamic CARD-to-pointer conversion needs a native ADDRESS type");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("native pointer stores require an explicit")
                || diagnostic.message.contains("pointer/integer conversion")
        }));
    }
}

#[test]
fn real_expressions_lower_to_address_based_verified_nir() {
    let program = lower_modern_source(
        "REAL x,y,result BYTE flag PROC Main() x=1.25 y=2 result=x*y+0.5 flag=result>2 RETURN",
    );
    verify_program(&program).expect("address-based REAL NIR should verify");

    assert_eq!(
        program
            .statics
            .iter()
            .map(|static_data| static_data.image.bytes.as_slice())
            .collect::<Vec<_>>(),
        vec![
            [0x40, 0x01, 0x25, 0, 0, 0].as_slice(),
            [0x3F, 0x50, 0, 0, 0, 0].as_slice(),
        ]
    );
    assert!(program.routines.iter().all(|routine| {
        routine
            .temps
            .iter()
            .all(|temp| !matches!(temp.ty.kind, NirTypeKind::Real))
    }));
    assert!(
        program.routines[0]
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(
                op,
                NirOp::Real(NirRealOp::Binary {
                    operation: NirBinaryOp::Mul,
                    left: NirRealSource::Place(_),
                    right: NirRealSource::Place(_),
                    ..
                })
            ))
    );
    assert!(
        program.routines[0]
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(op, NirOp::Real(NirRealOp::Compare { .. })))
    );
    let half_static = program.statics[1].id;
    assert!(
        program.routines[0]
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(
                op,
                NirOp::Real(NirRealOp::Binary {
                    operation: NirBinaryOp::Add,
                    right: NirRealSource::Static { id, .. },
                    ..
                }) if *id == half_static
            ))
    );
    assert!(
        !program.routines[0]
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(
                op,
                NirOp::Real(NirRealOp::Copy {
                    source: NirRealSource::Static { id, .. },
                    ..
                }) if *id == half_static
            )),
        "literal REAL operands must not be staged through a copy"
    );

    let optimized = optimize_program(&program).expect("REAL NIR should remain optimizer-clean");
    verify_program(&optimized).expect("optimized REAL NIR should verify");
    let mir = crate::mir6502::lower_program(&optimized)
        .expect("REAL comparisons should lower after Slice 6");
    crate::mir6502::verify_program(&mir, crate::mir6502::MirPhase::PreMaterialization)
        .expect("lowered REAL comparison MIR should verify");
}

#[test]
fn verifier_rejects_malformed_real_static_data() {
    let mut program = lower_modern_source("REAL value PROC Main() value=1.25 RETURN");
    program.statics[0].image.bytes.pop();

    let diagnostics = verify_program(&program).expect_err("short REAL static must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("must be an immutable six-byte rodata object")
    }));
}

#[test]
fn verifier_rejects_malformed_direct_real_operand_static_data() {
    let mut program = lower_modern_source("REAL value PROC Main() value=value+1.25 RETURN");
    program.statics[0].image.bytes.pop();

    let diagnostics =
        verify_program(&program).expect_err("short direct REAL operand static must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("does not name six-byte REAL static data")
    }));
}

#[test]
fn verifier_rejects_malformed_real_temporary_purpose() {
    let mut program = lower_modern_source("REAL value PROC Main() value=value+1.25 RETURN");
    let temporary = program.routines[0]
        .locals
        .iter_mut()
        .find(|local| matches!(local.purpose, NirLocalPurpose::RealTemporary))
        .expect("REAL evaluation temporary");
    temporary.storage = NirStorageClass::Array;

    let diagnostics = verify_program(&program).expect_err("malformed REAL temporary must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("must be ordinary, uninitialized, scalar six-byte REAL storage")
    }));
}

#[test]
fn verifier_rejects_real_in_the_scalar_operation_lane() {
    let mut program = lower_modern_source("REAL value PROC Main() value=1.25 RETURN");
    let block = &mut program.routines[0].blocks[0];
    let destination = match &block.ops[0] {
        NirOp::Real(NirRealOp::Copy { destination, .. }) => destination.clone(),
        other => panic!("expected REAL copy, got {other:?}"),
    };
    block.ops[0] = NirOp::Store {
        place: destination,
        src: NirValue::ConstU8(0),
        ty: NirType::from_value(&ValueType::real()),
    };

    let diagnostics = verify_program(&program).expect_err("scalar REAL store must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot use REAL in the byte/word scalar lane")
    }));
}

#[test]
fn verifier_rejects_non_real_real_operation_places() {
    let mut program =
        lower_modern_source("REAL value BYTE flag PROC Main() value=1.25 flag=0 RETURN");
    let source = match &program.routines[0].blocks[0].ops[0] {
        NirOp::Real(NirRealOp::Copy { source, .. }) => source.clone(),
        other => panic!("expected REAL copy, got {other:?}"),
    };
    program.routines[0].blocks[0].ops.insert(
        1,
        NirOp::Real(NirRealOp::Copy {
            destination: NirPlace {
                kind: NirPlaceKind::Global {
                    id: program.globals[1].id,
                    name: program.globals[1].name.clone(),
                },
                ty: program.globals[1].ty.clone(),
            },
            source,
        }),
    );

    let diagnostics = verify_program(&program).expect_err("non-REAL destination must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("REAL copy destination must be a six-byte REAL place")
    }));
}

#[test]
fn verifier_rejects_real_to_integer_with_a_non_integer_result() {
    let mut program = lower_modern_source(
        "REAL value INT result PROC Main() value=1.25 result=INT(value) RETURN",
    );
    let operation = program.routines[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.ops)
        .find_map(|op| match op {
            NirOp::Real(NirRealOp::RealToInteger { result_type, .. }) => Some(result_type),
            _ => None,
        })
        .expect("REAL-to-integer conversion");
    *operation = NirType::from_value(&ValueType::real());

    let diagnostics = verify_program(&program).expect_err("REAL result lane must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("REAL-to-integer conversion result must be an integer")
    }));
}

#[test]
fn verifier_rejects_real_index_with_scalar_element_stride() {
    let mut program = lower_modern_source(
        "REAL ARRAY values(2) BYTE index PROC Main() index=1 values(index)=1.25 RETURN",
    );
    let destination = program.routines[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.ops)
        .find_map(|op| match op {
            NirOp::Real(NirRealOp::Copy { destination, .. })
                if matches!(destination.kind, NirPlaceKind::Index { .. }) =>
            {
                Some(destination)
            }
            _ => None,
        })
        .expect("indexed REAL destination");
    let NirPlaceKind::Index { elem_size, .. } = &mut destination.kind else {
        unreachable!()
    };
    *elem_size = ByteSize::ONE;

    let diagnostics = verify_program(&program).expect_err("scalar REAL stride must fail");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("must index six-byte REAL elements")
    }));
}

#[test]
fn identical_real_literals_share_one_immutable_static_across_routines() {
    let program = lower_modern_source(
        "REAL left,right PROC First() left=1.25 RETURN PROC Second() right=1.25 RETURN",
    );

    assert_eq!(program.statics.len(), 1);
    assert_eq!(program.statics[0].image.bytes, [0x40, 0x01, 0x25, 0, 0, 0]);
    assert!(!program.statics[0].mutable);
    assert_eq!(program.statics[0].section, "rodata");
    let ids = program
        .routines
        .iter()
        .flat_map(|routine| &routine.blocks)
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::Real(NirRealOp::Copy {
                source: NirRealSource::Static { id, .. },
                ..
            }) => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, [program.statics[0].id, program.statics[0].id]);
    verify_program(&program).expect("deduplicated REAL statics verify");
}

#[test]
fn identical_direct_real_operands_share_one_static_across_routines() {
    let program = lower_modern_source(
        "REAL left,right PROC First() left=left+1.25 RETURN PROC Second() right=right+1.25 RETURN",
    );

    assert_eq!(program.statics.len(), 1);
    let static_id = program.statics[0].id;
    let ids = program
        .routines
        .iter()
        .flat_map(|routine| &routine.blocks)
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::Real(NirRealOp::Binary {
                right: NirRealSource::Static { id, .. },
                ..
            }) => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, [static_id, static_id]);
    verify_program(&program).expect("deduplicated direct REAL operand statics verify");
}

#[test]
fn verifier_accepts_valid_targets() {
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
fn optimizer_folds_uniform_typed_block_arguments_and_rebuilds_temp_definitions() {
    let optimized = optimize_program(&typed_block_argument_program())
        .expect("optimize verifier-clean block arguments");
    let routine = &optimized.routines[0];
    let NirTerminator::Goto(edge) = &routine.blocks[0].terminator else {
        panic!("expected goto");
    };

    assert!(edge.args.is_empty());
    assert!(routine.blocks[1].params.is_empty());
    assert!(routine.temps.is_empty());
    assert_eq!(verify_program(&optimized), Ok(()));
}

#[test]
fn verifier_rejects_open_block() {
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
            .any(|op| structured_machine_items(op) == Some(&[NirMachineItem::Byte(0xEA)])),
        "{main:#?}"
    );
}

#[test]
fn const_declarations_lower_to_typed_literals_without_nir_storage_or_metadata() {
    let source = "CONST Base=4, Limit=Base*2 PROC Main() CONST Local=Limit+1 BYTE x x=Local RETURN";
    let tokens = crate::lexer::tokenize(source).unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let formatted_semir = crate::semantic::ir::format_program(&semir);

    assert!(formatted_semir.contains("const Base#"), "{formatted_semir}");
    assert!(
        formatted_semir.contains("const Local#"),
        "{formatted_semir}"
    );
    assert!(formatted_semir.contains("$0009:INT"), "{formatted_semir}");

    let program = lower_program(&semir);
    verify_program(&program).expect("CONST values should leave verifier-clean NIR");
    assert!(program.globals.iter().all(|global| global.name != "Base"));

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    assert!(main.locals.iter().all(|local| local.name != "Local"));
    assert!(
        main.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(
                op,
                NirOp::Cast {
                    src: NirValue::IntegerConst { bits: 9, ty: NirIntegerType::U16 },
                    from,
                    to,
                    ..
                } if from.kind == NirTypeKind::I16 && to.kind == NirTypeKind::U8
            ))
    );

    optimize_program(&program).expect("CONST narrowing should optimize cleanly");
}

#[test]
fn lowers_self_and_forward_addresses_to_nir_data_relocations() {
    let source = "BYTE ARRAY dlist(4)=[$41 <dlist+2 >later $70] BYTE ARRAY later(1)=[$60]";
    let tokens = crate::lexer::tokenize(source).expect("tokenize source");
    let ast = crate::parser::parse(&tokens).expect("parse source");
    let model = crate::semantic::analyze(&ast).expect("analyze source");
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("relocatable data image should verify");
    let dlist = program
        .globals
        .iter()
        .find(|global| global.name == "dlist")
        .expect("dlist global");
    let later = program
        .globals
        .iter()
        .find(|global| global.name == "later")
        .expect("later global");
    let Some(NirGlobalInit::Bytes { image, .. }) = &dlist.init else {
        panic!("expected initialized data image, got {:#?}", dlist.init);
    };

    assert_eq!(image.bytes, [0x41, 0, 0, 0x70]);
    assert!(matches!(
        image.fragments.as_slice(),
        [
            NirDataFragment::Address {
                offset,
                encoding: NirDataAddressEncoding::TargetByte {
                    target: crate::target::TargetId::Atari6502,
                    byte_index: 0,
                },
                target: NirDataAddressTarget::Storage(NirStorageId::Global(first)),
                addend: 2,
                ..
            },
            NirDataFragment::Address {
                offset: high_offset,
                encoding: NirDataAddressEncoding::TargetByte {
                    target: crate::target::TargetId::Atari6502,
                    byte_index: 1,
                },
                target: NirDataAddressTarget::Storage(NirStorageId::Global(second)),
                ..
            }
        ] if *offset == ByteOffset::new(1)
            && *high_offset == ByteOffset::new(2)
            && *first == dlist.id
            && *second == later.id
    ));
    let formatted = format_program(&program);
    assert!(
        formatted.contains("fragments=[1:atari-6502-byte0(g"),
        "{formatted}"
    );
    assert!(formatted.contains("+2)"), "{formatted}");
}

#[test]
fn lowers_word_routine_addresses_to_descriptor_backing_relocations() {
    let source = "CARD ARRAY handlers(1)=[@Draw] PROC Draw() RETURN";
    let tokens = crate::lexer::tokenize(source).expect("tokenize source");
    let ast = crate::parser::parse(&tokens).expect("parse source");
    let model = crate::semantic::analyze(&ast).expect("analyze source");
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("routine relocation should verify");
    let handlers = program
        .globals
        .iter()
        .find(|global| global.name == "handlers")
        .expect("handlers global");
    let Some(NirGlobalInit::Descriptor { backing, .. }) = &handlers.init else {
        panic!("expected descriptor-backed word array");
    };
    assert_eq!(backing.image.bytes, [0, 0]);
    assert!(matches!(
        backing.image.fragments.as_slice(),
        [NirDataFragment::Address {
            offset: ByteOffset::ZERO,
            encoding: NirDataAddressEncoding::Pointer {
                address_space: crate::target::TargetLayout::CODE_ADDRESS_SPACE,
                width,
            },
            target: NirDataAddressTarget::Routine(RoutineId(0)),
            addend: 0,
            ..
        }] if *width == ByteSize::new(2)
    ));
}

#[test]
fn typed_integer_static_data_projects_with_target_endianness() {
    for (target, endian, expected) in [
        (
            crate::target::TargetId::Wdc65816Native,
            crate::target::Endian::Little,
            [0x34, 0x12],
        ),
        (
            crate::target::TargetId::Motorola68000,
            crate::target::Endian::Big,
            [0x12, 0x34],
        ),
    ] {
        let program = lower_modern_source_for_target(
            "CARD ARRAY values(1)=[$1234] PROC Main() RETURN",
            target,
        );
        verify_program(&program).expect("logical integer initializer should verify");
        let word = program
            .globals
            .iter()
            .find(|global| global.name == "values")
            .unwrap();
        let Some(NirGlobalInit::Descriptor { backing, .. }) = &word.init else {
            panic!("expected word initializer");
        };
        let image = &backing.image;
        assert_eq!(image.bytes, [0, 0]);
        assert!(matches!(
            image.fragments.as_slice(),
            [NirDataFragment::Integer { offset: ByteOffset::ZERO, width, value: 0x1234 }]
                if *width == ByteSize::new(2)
        ));
        assert_eq!(
            image.project_constants(endian).as_deref(),
            Some(expected.as_slice())
        );
    }
}

#[test]
fn static_address_fragments_use_data_and_code_pointer_layouts() {
    let source = "TYPE DataRec=[CARD word BYTE POINTER ptr PROC POINTER callback] \
                  BYTE target DataRec value=[$1234 @target @Draw] \
                  PROC Draw() RETURN";
    for (target, width, data_offset, code_offset) in [
        (crate::target::TargetId::Wdc65816Native, 3, 2, 5),
        (crate::target::TargetId::Motorola68000, 4, 2, 6),
    ] {
        let program = lower_modern_source_for_target(source, target);
        verify_program(&program).expect("typed address initializer should verify");
        let value = program
            .globals
            .iter()
            .find(|global| global.name == "value")
            .unwrap();
        let Some(NirGlobalInit::Bytes { image, .. }) = &value.init else {
            panic!("expected record initializer");
        };
        assert!(image.fragments.iter().any(|fragment| matches!(
            fragment,
            NirDataFragment::Address {
                offset,
                encoding: NirDataAddressEncoding::Pointer { address_space, width: fragment_width },
                target: NirDataAddressTarget::Storage(_),
                ..
            } if *offset == ByteOffset::new(data_offset)
                && *address_space == program.target_layout.data_pointer.address_space
                && *fragment_width == ByteSize::new(width)
        )));
        assert!(image.fragments.iter().any(|fragment| matches!(
            fragment,
            NirDataFragment::Address {
                offset,
                encoding: NirDataAddressEncoding::Pointer { address_space, width: fragment_width },
                target: NirDataAddressTarget::Routine(_),
                ..
            } if *offset == ByteOffset::new(code_offset)
                && *address_space == program.target_layout.code_pointer.address_space
                && *fragment_width == ByteSize::new(width)
        )));
    }
}

#[test]
fn target_specific_address_byte_selectors_are_rejected_on_other_targets() {
    let program = lower_modern_source_for_target(
        "BYTE ARRAY bytes(1)=[<bytes] PROC Main() RETURN",
        crate::target::TargetId::Motorola68000,
    );
    let diagnostics = verify_program(&program).expect_err("Atari byte selector on 68k");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("address-byte selector for target `atari-6502`")
    }));
}

#[test]
fn unsized_string_initializer_is_inline_array_storage_not_a_pointer_cell() {
    let source = "BYTE ARRAY text=\"ABC\"";
    let tokens = crate::lexer::tokenize(source).expect("tokenize source");
    let ast = crate::parser::parse(&tokens).expect("parse source");
    let model = crate::semantic::analyze(&ast).expect("analyze source");
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("string-initialized array should verify");
    let text = program
        .globals
        .iter()
        .find(|global| global.name == "text")
        .expect("text global");
    assert!(matches!(text.init, Some(NirGlobalInit::Bytes { .. })));
    assert!(
        text.array
            .as_ref()
            .is_some_and(|array| !array.pointer_backed),
        "inline string bytes must decay to their storage address: {text:#?}"
    );
}

#[test]
fn data_relocation_targets_are_address_observable_storage_roots() {
    let source = "PROC Main() BYTE value BYTE ARRAY refs(2)=[<value >value] value=1 RETURN";
    let tokens = crate::lexer::tokenize(source).expect("tokenize source");
    let ast = crate::parser::parse(&tokens).expect("parse source");
    let model = crate::semantic::analyze(&ast).expect("analyze source");
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("local data relocation should verify");
    let analysis = analyze_program_storage(&program);
    let value = analysis
        .routine("Main")
        .and_then(|routine| routine.storage_by_name("value"))
        .expect("value storage facts");
    assert!(value.address_taken);
    assert!(value.blockers.contains(&NirPromotionBlocker::AddressTaken));
}

#[test]
fn verifier_rejects_out_of_bounds_and_overlapping_data_relocations() {
    let mut program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: vec![NirGlobal {
            id: SymbolId(0),
            name: "data".to_string(),
            kind: "Byte Array".to_string(),
            ty: Some(byte_type()),
            storage_size: ByteSize::new(2),
            array: None,
            init: Some(NirGlobalInit::Bytes {
                image: NirDataImage {
                    bytes: vec![0, 0],
                    fragments: vec![NirDataFragment::Address {
                        offset: ByteOffset::new(1),
                        encoding: NirDataAddressEncoding::Pointer {
                            address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
                            width: ByteSize::new(2),
                        },
                        target: NirDataAddressTarget::Storage(NirStorageId::Global(SymbolId(0))),
                        addend: 0,
                        span: crate::source::Span::new(0, 0),
                    }],
                },
                zero_fill: ByteSize::ZERO,
                mutable: true,
                section: "global".to_string(),
            }),
            backing: NirGlobalBacking::Ordinary,
        }],
        statics: Vec::new(),
        routines: Vec::new(),
    };
    let diagnostics = verify_program(&program).expect_err("out-of-bounds relocation");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("exceeds 2 initialized bytes"))
    );

    let Some(NirGlobalInit::Bytes { image, .. }) = &mut program.globals[0].init else {
        unreachable!();
    };
    let NirDataFragment::Address { offset, .. } = &mut image.fragments[0] else {
        unreachable!();
    };
    *offset = ByteOffset::ZERO;
    image.fragments.push(NirDataFragment::Address {
        offset: ByteOffset::new(1),
        encoding: NirDataAddressEncoding::TargetByte {
            target: crate::target::TargetId::Atari6502,
            byte_index: 1,
        },
        target: NirDataAddressTarget::Routine(RoutineId(9)),
        addend: 0,
        span: crate::source::Span::new(0, 0),
    });
    let diagnostics = verify_program(&program).expect_err("overlap and unknown target");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("overlapping data fragment"))
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown routine id 9"))
    );
}

#[test]
fn verifier_rejects_nonzero_relocation_placeholders_and_oversized_data_extents() {
    let mut program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: vec![NirGlobal {
            id: SymbolId(0),
            name: "data".to_string(),
            kind: "Byte Array".to_string(),
            ty: Some(byte_type()),
            storage_size: ByteSize::new(2),
            array: None,
            init: Some(NirGlobalInit::Bytes {
                image: NirDataImage {
                    bytes: vec![1, 0],
                    fragments: vec![NirDataFragment::Address {
                        offset: ByteOffset::ZERO,
                        encoding: NirDataAddressEncoding::Pointer {
                            address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
                            width: ByteSize::new(2),
                        },
                        target: NirDataAddressTarget::Storage(NirStorageId::Global(SymbolId(0))),
                        addend: 0,
                        span: crate::source::Span::new(0, 0),
                    }],
                },
                zero_fill: ByteSize::ZERO,
                mutable: true,
                section: "global".to_string(),
            }),
            backing: NirGlobalBacking::Ordinary,
        }],
        statics: Vec::new(),
        routines: Vec::new(),
    };

    let diagnostics = verify_program(&program).expect_err("nonzero relocation placeholder");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("placeholder bytes must be zero")
    }));

    let Some(NirGlobalInit::Bytes {
        image, zero_fill, ..
    }) = &mut program.globals[0].init
    else {
        unreachable!();
    };
    image.bytes = vec![0];
    image.fragments.clear();
    *zero_fill = ByteSize::new(u32::MAX);
    program.globals[0].storage_size = ByteSize::new(u32::MAX);

    let diagnostics = verify_program(&program).expect_err("oversized initialized extent");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("initialized extent exceeds the NIR storage range")
    }));
}

#[test]
fn verifier_keeps_large_nir_sizes_without_card_truncation() {
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::motorola_68000(),
        runtime_bindings: Vec::new(),
        globals: vec![NirGlobal {
            id: SymbolId(0),
            name: "large_buffer".to_string(),
            kind: "Byte Array".to_string(),
            ty: Some(byte_type()),
            storage_size: ByteSize::new(0x1_0001),
            array: None,
            init: None,
            backing: NirGlobalBacking::Ordinary,
        }],
        statics: Vec::new(),
        routines: Vec::new(),
    };

    verify_program(&program).expect("NIR object sizes are not limited to CARD");
    assert_eq!(program.globals[0].storage_size.get(), 0x1_0001);
}

#[test]
fn verifier_checks_absolute_extents_against_the_selected_target() {
    let absolute_global = |address| NirGlobal {
        id: SymbolId(0),
        name: "buffer".to_string(),
        kind: "Byte Array".to_string(),
        ty: Some(byte_type()),
        storage_size: ByteSize::new(2),
        array: None,
        init: None,
        backing: NirGlobalBacking::Absolute(address),
    };
    let program_for = |target_layout, address| NirProgram {
        target_layout,
        runtime_bindings: Vec::new(),
        globals: vec![absolute_global(address)],
        statics: Vec::new(),
        routines: Vec::new(),
    };

    let native = program_for(
        crate::target::TargetLayout::motorola_68000(),
        AddressValue::data(0x1_0000),
    );
    verify_program(&native).expect("68k absolute addresses may exceed $FFFF");

    let atari = program_for(
        crate::target::TargetLayout::atari_6502(),
        AddressValue::data(0x1_0000),
    );
    let diagnostics = verify_program(&atari).expect_err("Atari addresses remain 16-bit");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("outside the selected target address range")
    }));

    let crossing = program_for(
        crate::target::TargetLayout::atari_6502(),
        AddressValue::data(0xFFFF),
    );
    assert!(
        verify_program(&crossing).is_err(),
        "the complete extent must fit"
    );

    let wrong_space = program_for(
        crate::target::TargetLayout::motorola_68000(),
        AddressValue::code(0x1000),
    );
    let diagnostics =
        verify_program(&wrong_space).expect_err("data storage cannot use the code address space");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("outside the selected target address range")
    }));
}

#[test]
fn verifier_requires_global_byte_initializers_to_match_their_storage_extent() {
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: vec![NirGlobal {
            id: SymbolId(0),
            name: "data".to_string(),
            kind: "Byte Array".to_string(),
            ty: Some(byte_type()),
            storage_size: ByteSize::new(2),
            array: None,
            init: Some(NirGlobalInit::Bytes {
                image: NirDataImage::literal(vec![0]),
                zero_fill: ByteSize::ZERO,
                mutable: true,
                section: "global".to_string(),
            }),
            backing: NirGlobalBacking::Ordinary,
        }],
        statics: Vec::new(),
        routines: Vec::new(),
    };

    let diagnostics = verify_program(&program).expect_err("mismatched initialized extent");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("init payload does not match storage size 2")
    }));
}

#[test]
fn compile_time_sets_do_not_lower_to_executable_stores() {
    let source =
        "SET $22F=0 SET $E=$E6 SET $F=0 BYTE POINTER screen SET $E=$3000 PROC Main() RETURN";
    let tokens = crate::lexer::tokenize(source).expect("tokenize source");
    let ast = crate::parser::parse(&tokens).expect("parse source");
    let model = crate::semantic::analyze(&ast).expect("analyze source");
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("compile-time SET should leave verifier-clean NIR");
    assert_eq!(
        program
            .globals
            .iter()
            .find(|global| global.name == "screen")
            .expect("screen global")
            .backing,
        NirGlobalBacking::Absolute(AddressValue::data(0x00E6))
    );
    assert!(
        program
            .routines
            .iter()
            .all(|routine| routine.name != "<program>"),
        "{program:#?}"
    );
    assert!(
        program
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .all(|block| block.ops.iter().all(|op| !matches!(
                op,
                NirOp::Store {
                    place: NirPlace {
                        kind: NirPlaceKind::Absolute(_),
                        ..
                    },
                    ..
                }
            )))
    );
}

#[test]
fn two_term_logical_if_lowers_to_short_circuit_cfg() {
    for (operator, expected_first_then_rhs) in [("AND", true), ("OR", false)] {
        let source = format!(
            "BYTE a,b,out PROC Main() IF a#0 {operator} b#0 THEN out=1 ELSE out=2 FI RETURN"
        );
        let tokens = crate::lexer::tokenize(&source).expect("tokenize source");
        let ast = crate::parser::parse(&tokens).expect("parse source");
        let model = crate::semantic::analyze(&ast).expect("analyze source");
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        let program = lower_program(&semir);

        verify_program(&program).expect("logical IF NIR should verify");
        let main = program
            .routines
            .iter()
            .find(|routine| routine.name == "Main")
            .expect("Main routine");
        let branches = main
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator {
                NirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => Some((block, then_edge, else_edge)),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(branches.len(), 2, "{operator}: {main:#?}");
        let rhs = branches[1].0;
        let rhs_id = rhs.id;
        assert_eq!(branches[0].1.target == rhs_id, expected_first_then_rhs);
        assert_eq!(branches[0].2.target == rhs_id, !expected_first_then_rhs);
        assert!(rhs.ops.iter().any(|op| matches!(
            op,
            NirOp::Load {
                place: NirPlace {
                    kind: NirPlaceKind::Global { name, .. },
                    ..
                },
                ..
            } if name == "b"
        )));
        assert!(main.blocks.iter().flat_map(|block| &block.ops).all(|op| {
            !matches!(
                op,
                NirOp::Binary {
                    op: NirBinaryOp::And | NirBinaryOp::Or,
                    ..
                }
            )
        }));
    }
}

#[test]
fn nested_mixed_logical_if_keeps_call_in_reached_rhs_block() {
    let source = "BYTE a,b,out BYTE FUNC Next(BYTE value) RETURN(value) PROC Main() IF (a=1 OR b=2) AND Next(a)=3 THEN out=1 ELSE out=2 FI RETURN";
    let tokens = crate::lexer::tokenize(source).expect("tokenize source");
    let ast = crate::parser::parse(&tokens).expect("parse source");
    let model = crate::semantic::analyze(&ast).expect("analyze source");
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("nested logical IF NIR should verify");
    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let branch_blocks = main
        .blocks
        .iter()
        .filter(|block| matches!(block.terminator, NirTerminator::Branch { .. }))
        .collect::<Vec<_>>();

    assert_eq!(branch_blocks.len(), 3, "{main:#?}");
    assert!(
        branch_blocks[0]
            .ops
            .iter()
            .all(|op| !matches!(op, NirOp::Call { .. }))
    );
    assert!(
        branch_blocks[1]
            .ops
            .iter()
            .all(|op| !matches!(op, NirOp::Call { .. }))
    );
    assert!(
        branch_blocks[2]
            .ops
            .iter()
            .any(|op| matches!(op, NirOp::Call { .. }))
    );
    assert!(main.blocks.iter().flat_map(|block| &block.ops).all(|op| {
        !matches!(
            op,
            NirOp::Binary {
                op: NirBinaryOp::And | NirBinaryOp::Or,
                ..
            }
        )
    }));
}

#[test]
fn logical_while_and_until_lower_to_short_circuit_cfg() {
    let source = "BYTE a,b,out PROC Main() WHILE a=1 AND b=2 DO out=1 EXIT OD DO out=2 UNTIL a=3 OR b=4 OD RETURN";
    let tokens = crate::lexer::tokenize(source).expect("tokenize source");
    let ast = crate::parser::parse(&tokens).expect("parse source");
    let model = crate::semantic::analyze(&ast).expect("analyze source");
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("logical loop NIR should verify");
    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");

    assert_eq!(
        main.blocks
            .iter()
            .filter(|block| matches!(block.terminator, NirTerminator::Branch { .. }))
            .count(),
        4,
        "{main:#?}"
    );
    assert!(main.blocks.iter().flat_map(|block| &block.ops).all(|op| {
        !matches!(
            op,
            NirOp::Binary {
                op: NirBinaryOp::And | NirBinaryOp::Or,
                ..
            }
        )
    }));
}

#[test]
fn raw_machine_items_lower_to_explicit_unsupported_ops() {
    for (item, expected_note) in [
        ("+", "machine block item `+` is not a byte-stream item"),
        ("=", "unsupported raw machine block item `=`"),
    ] {
        let source = format!("PROC Main() [{item}] RETURN");
        let tokens = crate::lexer::tokenize(&source).expect("tokenize raw machine item");
        let ast = crate::parser::parse(&tokens).expect("parse raw machine item");
        let model = crate::semantic::analyze(&ast).expect("analyze raw machine item");
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        let program = lower_program(&semir);

        verify_program(&program).expect("unsupported operation remains verifier-clean");
        let ops = program.routines[0]
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .collect::<Vec<_>>();
        assert!(
            ops.iter().any(|op| matches!(
                op,
                NirOp::Unsupported { note } if note.contains(expected_note)
            )),
            "expected `{expected_note}` for `{item}`, got {ops:#?}"
        );
        assert!(
            ops.iter().all(|op| structured_machine_items(op).is_none()),
            "raw machine item must not enter an executable machine block: {ops:#?}"
        );
    }
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
            offset,
            ..
        } if target_name == "state" && offset == ByteOffset::new(1)
    ));
    assert!(
        main.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(
                op,
                NirOp::Store {
                    place: NirPlace {
                        kind: NirPlaceKind::Field { offset, .. },
                        ..
                    },
                    ..
                } if *offset == ByteOffset::new(1)
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
    assert_eq!(
        line.backing,
        NirGlobalBacking::Absolute(AddressValue::data(0x00CB))
    );

    for (name, offset) in [("low", 0u16), ("high", 1u16)] {
        let alias = program
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap_or_else(|| panic!("{name} global"));
        assert_eq!(
            alias.backing,
            NirGlobalBacking::Alias {
                target: line.id,
                offset: ByteOffset::from(offset),
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
            .find_map(|op| structured_machine_items(op))
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
fn named_module_machine_defines_expand_by_resolved_identity() {
    let root = SourceOrigin::host("project/runtime.act");
    let provider = InMemorySourceProvider::default().with_source(
        root.clone(),
        b"MODULE RUNTIME\nDEFINE EOL=\"$9B\"\nPROC PutE=*()[$A9 EOL]\nENDMODULE\n".to_vec(),
    );
    let loaded = load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default())
        .expect("load named runtime module");
    let model = crate::semantic::analyze_compilation(&loaded).expect("analyze runtime module");
    let semir = crate::semantic::ir::lower_compilation(&loaded, &model);
    let program = lower_program(&semir);

    verify_program(&program).expect("named-module machine DEFINE should verify");
    let put_e = program
        .routines
        .iter()
        .find(|routine| routine.name.to_ascii_uppercase().contains("PUTE"))
        .expect("PutE routine");
    let items = put_e
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| structured_machine_items(op))
        .expect("PutE machine block");

    assert_eq!(
        items,
        [NirMachineItem::Byte(0xA9), NirMachineItem::Byte(0x9B)]
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
            .all(|op| structured_machine_items(op).is_none()),
        "{cold:#?}"
    );
}

#[test]
fn verifier_rejects_executable_error_type() {
    let error = error_type();
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![NirLocal {
                id: LocalId(0),
                name: "bad".to_string(),
                kind: "error".to_string(),
                purpose: NirLocalPurpose::Storage,
                storage: NirStorageClass::Scalar,
                duration: crate::nir::NirStorageDuration::RoutineStatic,
                layout: crate::nir::NirObjectLayout::byte(),
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
fn verifier_rejects_store_with_untyped_place() {
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
                        kind: NirPlaceKind::Absolute(AddressValue::data(0)),
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
fn verifier_accepts_literal_that_fits_narrow_store() {
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
fn verifier_accepts_temp_use_from_dominating_block() {
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
        width: Some(crate::target::ByteSize::ONE),
        pointer: false,
    };
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
                        operand_ty: byte_type(),
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
fn optimizer_threads_repeated_param_predicates_from_both_edges() {
    let source = "BYTE result PROC Main(BYTE value) IF value=0 THEN IF value=0 THEN result=1 ELSE result=2 FI ELSE IF value=0 THEN result=3 ELSE result=4 FI FI RETURN";
    let tokens = crate::lexer::tokenize(source).expect("tokenize source");
    let ast = crate::parser::parse(&tokens).expect("parse source");
    let model = crate::semantic::analyze(&ast).expect("analyze source");
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let optimized = optimize_program(&lower_program(&semir)).expect("optimize verifier-clean NIR");
    let main = optimized
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let compares = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter(|op| matches!(op, NirOp::Compare { .. }))
        .count();
    let stores = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::Store {
                src:
                    NirValue::IntegerConst {
                        bits: value,
                        ty: NirIntegerType::U8,
                    },
                ..
            } => Some(*value as u8),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(compares, 1, "{main:#?}");
    assert_eq!(stores, vec![1, 4], "{main:#?}");
}

#[test]
fn optimizer_eliminates_dead_pure_temps_but_keeps_loads() {
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
fn volatile_accesses_survive_nir_optimization_in_source_order() {
    let source = "VOLATILE BYTE VCOUNT=$D40B, COLBAK=$D01A \
                  VOLATILE BYTE ARRAY POKEY(16)=$D200 \
                  BYTE sink, index \
                  PROC Main() \
                    sink=VCOUNT sink=VCOUNT \
                    COLBAK=0 COLBAK=0 \
                    sink=POKEY(index) POKEY(index)=sink \
                  RETURN";
    let tokens = crate::lexer::tokenize(source).expect("tokenize volatile source");
    let ast = crate::parser::parse(&tokens).expect("parse volatile source");
    let model = crate::semantic::analyze(&ast).expect("analyze volatile source");
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let lowered = lower_program(&semir);
    let optimized = optimize_program(&lowered).expect("optimize verifier-clean volatile NIR");
    let main = optimized
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let volatile_ops = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            NirOp::VolatileLoad { .. } => Some("load"),
            NirOp::VolatileStore { .. } => Some("store"),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        volatile_ops,
        vec!["load", "load", "store", "store", "load", "store"],
        "{}",
        format_program(&optimized)
    );
    let formatted = format_program(&optimized);
    assert_eq!(formatted.matches("load volatile").count(), 3, "{formatted}");
    assert_eq!(
        formatted.matches("store volatile").count(),
        3,
        "{formatted}"
    );
}

#[test]
fn optimizer_keeps_pure_temp_used_in_successor_block() {
    let ty = byte_type();
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
            src: NirValue::IntegerConst { bits: 3, .. },
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
        width: Some(crate::target::ByteSize::ONE),
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
                        operand_ty: byte.clone(),
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
            src: NirValue::IntegerConst { bits: 3, .. },
            ..
        }
    ));
}

#[test]
fn optimizer_aliases_algebraic_identity_temps() {
    let ty = byte_type();
    let program = NirProgram {
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
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
                    NirOp::PointerOffset {
                        dest: TempId(3),
                        ty: pointer.clone(),
                        base: temp_value(2, pointer.clone()),
                        offset: NirValue::ConstU16(0),
                        subtract: false,
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
        NirOp::PointerOffset {
            dest: TempId(3),
            subtract: false,
            ..
        }
    )));
}

#[test]
fn verifier_and_printer_expose_structured_memory_effect_regions() {
    let program = memory_effect_program(NirMemoryRegion {
        kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(0))),
        offset: ByteOffset::ZERO,
        size: ByteSize::ONE,
    });

    verify_program(&program).expect("valid exact local effect region");
    assert!(format_program(&program).contains("writes:local0+0:1"));
}

#[test]
fn verifier_rejects_missing_and_malformed_memory_effect_regions() {
    let missing = verify_program(&memory_effect_program(NirMemoryRegion {
        kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(9))),
        offset: ByteOffset::ZERO,
        size: ByteSize::ONE,
    }))
    .expect_err("missing effect-region storage must fail verification");
    assert!(
        missing
            .iter()
            .any(|diagnostic| diagnostic.message.contains("missing storage identity"))
    );

    let zero_size = verify_program(&memory_effect_program(NirMemoryRegion {
        kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(0))),
        offset: ByteOffset::ZERO,
        size: ByteSize::ZERO,
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![NirLocal {
                id: LocalId(0),
                name: "x".to_string(),
                kind: "Byte".to_string(),
                purpose: NirLocalPurpose::Storage,
                storage: NirStorageClass::Scalar,
                duration: crate::nir::NirStorageDuration::RoutineStatic,
                layout: crate::nir::NirObjectLayout::byte(),
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
                    signature: Some(NirCallableSignature::empty_proc(NirCallConvention::Runtime)),
                    effects: NirCallEffects {
                        memory: NirMemoryEffects {
                            reads: NirMemoryAccess::None,
                            writes: NirMemoryAccess::Regions(vec![region]),
                        },
                        may_call_external: false,
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

fn byte_local() -> NirLocal {
    NirLocal {
        id: LocalId(0),
        name: "storage".to_string(),
        kind: "Byte".to_string(),
        purpose: NirLocalPurpose::Storage,
        storage: NirStorageClass::Scalar,
        duration: crate::nir::NirStorageDuration::External,
        layout: crate::nir::NirObjectLayout::byte(),
        ty: byte_type(),
        backing: NirLocalBacking::Absolute(AddressValue::data(0xD000)),
        init: None,
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: vec![NirStaticData {
            id: SymbolId(0),
            name: "table".to_string(),
            ty: byte.clone(),
            image: NirDataImage::literal(vec![0]),
            display: "table".to_string(),
            alignment: ByteSize::ONE,
            mutable: true,
            section: "data".to_string(),
        }],
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
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
        target_layout: crate::target::TargetLayout::atari_6502(),
        runtime_bindings: Vec::new(),
        globals: Vec::new(),
        statics: Vec::new(),
        routines: vec![NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![byte_local()],
            temps,
            notes: Vec::new(),
            blocks,
        }],
    }
}

fn byte_type() -> NirType {
    NirType {
        kind: NirTypeKind::U8,
        summary: "Byte".to_string(),
        width: Some(crate::target::ByteSize::ONE),
        pointer: false,
    }
}

fn card_type() -> NirType {
    NirType {
        kind: NirTypeKind::U16,
        summary: "Card".to_string(),
        width: Some(crate::target::ByteSize::new(2)),
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
        kind: NirTypeKind::Pointer {
            pointee: Some(Box::new(NirTypeKind::U8)),
            address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
        },
        summary: "Byte*".to_string(),
        width: Some(crate::target::ByteSize::new(2)),
        pointer: true,
    }
}
