use actionc::target::{TargetId, TargetLayout};
use actionc::{
    lexer::tokenize,
    parser::parse,
    semantic::{self, SemanticOptions},
};

#[test]
fn recursive_variants_are_finite_and_target_sized_through_pointer_barriers() {
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let source = "TYPE Tree=VARIANT [EMPTY NODE [INT value Tree POINTER left,right]]\nTYPE Left=VARIANT [END LINK [Right POINTER next]]\nTYPE Right=VARIANT [END LINK [Left POINTER next]]\nTYPE Arena=[BYTE guard Tree ARRAY nodes(8)]\nTree POINTER p PROC Main() RETURN";
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model =
            semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
                .unwrap();
        let tree = model
            .variants
            .types
            .values()
            .find(|ty| ty.identity.name == "Tree")
            .unwrap();
        let pointer = TargetLayout::for_target(target)
            .data_pointer
            .size_bytes
            .get();
        let packed = target == TargetId::Atari6502;
        assert_eq!(tree.payload_offset, if packed { 1 } else { 2 });
        assert_eq!(
            tree.size,
            if packed {
                3 + 2 * pointer
            } else {
                4 + 2 * pointer
            }
        );
        let owner = tree.identity.symbol.unwrap();
        for field in &tree.constructors[1].fields[1..] {
            assert_eq!(
                model.fields[field.0]
                    .ty
                    .as_aggregate_identity()
                    .unwrap()
                    .symbol,
                Some(owner)
            );
            assert_eq!(model.fields[field.0].size, pointer);
        }
        let arena = model.layout.record_for_name("Arena").unwrap();
        assert_eq!(arena.fields[1].offset, if packed { 1 } else { 2 });
        let nir = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        actionc::nir::verify_program(&nir).unwrap();
        let nir = actionc::nir::optimize_program(&nir).unwrap();
        match target {
            TargetId::Atari6502 => {
                actionc::mir6502::lower_program(&nir).unwrap();
            }
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(&nir).unwrap();
            }
            _ => {
                actionc::mir65816::lower_program(&nir).unwrap();
            }
        }
    }
}

#[test]
fn inline_recursive_variant_cycles_remain_errors() {
    for source in [
        "TYPE Tree=VARIANT [EMPTY NODE [Tree child]]",
        "TYPE First=VARIANT [END LINK [Second next]] TYPE Second=VARIANT [END LINK [First next]]",
        "TYPE Box=[Tree child] TYPE Tree=VARIANT [END NODE [Box value]]",
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let errors = semantic::analyze_with_options(&ast, SemanticOptions::modern())
            .err()
            .expect(source);
        assert!(
            errors.iter().any(|e| e.message.contains("cyclic")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn named_pointer_results_and_callable_declarations_remain_distinct() {
    let source = "TYPE Node=[BYTE value]\nNode original\nBYTE datum\nNode POINTER FUNC NodeAddress() RETURN(@original)\nBYTE POINTER FUNC ByteAddress() RETURN(@datum)\nNode POINTER FUNC POINTER callback()\nPROC Main()\nNode POINTER p\nBYTE POINTER b\ncallback=@NodeAddress p=callback() b=ByteAddress()\nIF p=@original THEN p.value=9 FI\nRETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model =
            semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
                .unwrap();
        let semir = semantic::ir::lower_program(&ast, &model);
        let nir = actionc::nir::lower_program(&semir);
        actionc::nir::verify_program(&nir).unwrap();
        let nir = actionc::nir::optimize_program(&nir).unwrap();
        match target {
            TargetId::Atari6502 => {
                actionc::codegen::generate_semir_profile_with_origin(
                    &semir,
                    0x3000,
                    actionc::codegen::CodegenProfile::Modern,
                )
                .unwrap();
                actionc::mir6502::lower_program(&nir).unwrap();
            }
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(&nir).unwrap();
            }
            _ => {
                actionc::mir65816::lower_program(&nir).unwrap();
            }
        }
    }
}
