use super::*;
use crate::semantic::ir::{SemItem, SemStmt};

#[test]
fn compounds_reuse_ordinary_binary_types_and_record_store_conversion() {
    for target_ty in ["BYTE", "CHAR", "INT", "CARD"] {
        for value_ty in ["BYTE", "CHAR", "INT", "CARD"] {
            for operator in ["+", "-", "*", "/", "MOD", "&", "%", "XOR", "LSH", "RSH"] {
                let source = format!(
                    "{target_ty} target {value_ty} value \
                     PROC Main() target=={operator} value RETURN(target {operator} value)"
                );
                // Use a function of the ordinary result type to avoid imposing
                // an assignment's expected type on the expression under test.
                let op = match operator {
                    "+" => BinaryOp::Add,
                    "-" => BinaryOp::Sub,
                    "*" => BinaryOp::Mul,
                    "/" => BinaryOp::Div,
                    "MOD" => BinaryOp::Mod,
                    "&" => BinaryOp::And,
                    "%" => BinaryOp::Or,
                    "XOR" => BinaryOp::Xor,
                    "LSH" => BinaryOp::Lsh,
                    _ => BinaryOp::Rsh,
                };
                let scalar = |ty| match ty {
                    "BYTE" => ScalarType::Byte,
                    "CHAR" => ScalarType::Char,
                    "INT" => ScalarType::Int,
                    _ => ScalarType::Card,
                };
                let result =
                    ScalarType::arithmetic_result(op, scalar(target_ty), scalar(value_ty), None);
                let result_name = match result {
                    ScalarType::Byte => "BYTE",
                    ScalarType::Char => "CHAR",
                    ScalarType::Int => "INT",
                    ScalarType::Card => "CARD",
                };
                let source = source.replace("PROC Main", &format!("{result_name} FUNC Main"));
                let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
                let model = analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
                let semir = ir::lower_program(&ast, &model);
                let routine = semir
                    .modules
                    .iter()
                    .flat_map(|module| &module.items)
                    .find_map(|item| match item {
                        SemItem::Routine(routine) => Some(routine),
                        _ => None,
                    })
                    .unwrap();
                let SemStmt::CompoundAssign {
                    target, operation, ..
                } = &routine.body[0]
                else {
                    panic!()
                };
                let SemStmt::Return {
                    value: Some(value), ..
                } = &routine.body[1]
                else {
                    panic!()
                };
                assert_eq!(operation.result_type, value.ty, "{source}");
                assert_eq!(operation.result_type, ValueType::scalar(result), "{source}");
                assert_eq!(
                    operation.store_conversion,
                    (operation.result_type != target.ty).then(|| target.ty.clone()),
                    "{source}"
                );
                crate::nir::verify_program(&crate::nir::lower_program(&semir)).unwrap();
            }
        }
    }
}

#[test]
fn compound_nir_computes_at_word_width_then_truncates_the_store() {
    use crate::nir::{NirBinaryOp, NirOp, NirTypeKind, NirValue};
    for (operator, op, kind) in [
        ("*", NirBinaryOp::Mul, NirTypeKind::I16),
        ("/", NirBinaryOp::Div, NirTypeKind::U16),
        ("MOD", NirBinaryOp::Mod, NirTypeKind::U16),
    ] {
        let source = format!("BYTE item CARD value PROC Main() item=={operator} value RETURN");
        let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let model = analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
        let nir = crate::nir::lower_program(&ir::lower_program(&ast, &model));
        crate::nir::verify_program(&nir).unwrap();
        let ops: Vec<_> = nir
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .collect();
        let (result, result_ty) = ops
            .iter()
            .find_map(|operation| match operation {
                NirOp::Binary {
                    dest,
                    ty,
                    op: operation,
                    ..
                } if *operation == op => Some((*dest, ty)),
                _ => None,
            })
            .unwrap();
        assert_eq!(result_ty.kind, kind);
        assert!(ops.iter().any(|operation| matches!(operation,
            NirOp::Cast { src: NirValue::Temp { id, .. }, from, to, .. }
            if *id == result && from.kind == kind && to.kind == NirTypeKind::U8)));
    }
}
