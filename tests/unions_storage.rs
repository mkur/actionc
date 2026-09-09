use actionc::{
    lexer::tokenize,
    parser::parse,
    semantic::{self, SemanticOptions},
    target::TargetId,
};

#[test]
fn volatile_alias_and_copy_facts_survive_normalization_and_optimization() {
    let source = "TYPE View=UNION [LONGCARD wide CARD word BYTE low] \
        VOLATILE View device=$680,destination=$690 View alias=device LONGCARD result \
        PROC Main() result=alias.wide alias.word=7 alias.low==+1 destination=alias RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let mut options = SemanticOptions::modern().with_target(target);
        options.algebraic_types.unions = true;
        let model = semantic::analyze_with_options(&ast, options).unwrap();
        assert!(
            model
                .symbols
                .symbols
                .iter()
                .find(|s| s.name == "alias")
                .unwrap()
                .is_volatile
        );
        let raw = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        actionc::nir::verify_program(&raw).unwrap();
        for nir in [raw.clone(), actionc::nir::optimize_program(&raw).unwrap()] {
            actionc::nir::verify_program(&nir).unwrap();
            let ops: Vec<_> = nir
                .routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .collect();
            use actionc::nir::NirOp;
            assert_eq!(
                ops.iter()
                    .filter(|op| matches!(op, NirOp::VolatileLoad { .. }))
                    .count(),
                2
            );
            assert_eq!(
                ops.iter()
                    .filter(|op| matches!(op, NirOp::VolatileStore { .. }))
                    .count(),
                2
            );
            assert!(ops.iter().any(|op| matches!(
                op,
                NirOp::CopyBytes {
                    source_volatile: true,
                    destination_volatile: true,
                    ..
                }
            )));
        }
    }
}

#[test]
fn union_initializer_rejection_is_transitive_in_local_and_nested_storage() {
    for declaration in [
        "View value=[1 2]",
        "View value=\"AB\"",
        "View ARRAY values(2)=[1 2 3 4]",
        "TYPE Box=[View ARRAY views(2)] Box value=[1 2 3 4]",
        "TYPE Inner=[View view] TYPE Outer=[Inner ARRAY values(2)] Outer value=[1 2]",
        "TYPE Box<T>=[T value] Box<View> value=[1 2]",
    ] {
        for local in [false, true] {
            let head = "TYPE View=UNION [CARD word BYTE ARRAY bytes(2)]";
            let source = if local {
                format!("{head} PROC Main()\nBEGIN\n{declaration}\nEND\nRETURN")
            } else {
                format!("{head} {declaration} PROC Main() RETURN")
            };
            let ast = parse(&tokenize(&source).unwrap()).unwrap();
            let mut options = SemanticOptions::modern();
            options.algebraic_types.unions = true;
            let errors = semantic::analyze_with_options(&ast, options).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|e| e.message.contains("union-containing storage")),
                "{source}: {errors:?}"
            );
        }
    }
}

#[test]
fn union_storage_keeps_volatile_qualifier_and_alignment_restrictions() {
    for (source, needle) in [
        (
            "TYPE View=UNION [VOLATILE CARD word]",
            "VOLATILE record fields",
        ),
        (
            "TYPE View=UNION [CARD word] VOLATILE View POINTER ptr",
            "VOLATILE pointer",
        ),
        (
            "TYPE View=UNION [CARD word] PROC Take(VOLATILE View value) RETURN",
            "VOLATILE parameters",
        ),
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let mut options = SemanticOptions::modern();
        options.algebraic_types.unions = true;
        let errors = semantic::analyze_with_options(&ast, options).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(needle)),
            "{errors:?}"
        );
    }
    let source =
        "TYPE View=UNION [CARD word BYTE ARRAY bytes(3)] View value=$680 PROC Main() RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let mut options = SemanticOptions::modern().with_target(target);
        options.algebraic_types.unions = true;
        let model = semantic::analyze_with_options(&ast, options).unwrap();
        let layout = model.layout.record_for_name("View").unwrap();
        assert_eq!(
            (layout.size, layout.alignment),
            if target == TargetId::Atari6502 {
                (3, 1)
            } else {
                (4, 2)
            }
        );
        let nir = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        actionc::nir::verify_program(&nir).unwrap();
    }
}
