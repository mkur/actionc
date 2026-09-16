use actionc::{
    includes::{ModuleLoadOptions, load_compilation_from_provider},
    lexer::tokenize,
    nir,
    parser::parse,
    semantic::{self, SemanticOptions},
    source::{InMemorySourceProvider, SourceOrigin},
    target::TargetId,
};

#[test]
fn named_pointer_casts_keep_resolved_types_through_modules_and_nir() {
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(
            root.clone(),
            b"MODULE App USE Links AS L L.Link POINTER cursor \
            L.Box<BYTE> POINTER boxed \
            PROC Main() cursor=L.Link POINTER(0) cursor=L.Link POINTER(ADDRESS($1234)) \
            cursor.next=L.Link POINTER(BYTE POINTER(cursor)) cursor=L.Identity(cursor) \
            boxed=L.Box<BYTE> POINTER(cursor) boxed.value=7 \
            RETURN ENDMODULE"
                .to_vec(),
        )
        .with_source(
            SourceOrigin::host("project/links.act"),
            b"MODULE Links \
            PUBLIC TYPE Link=[Link POINTER next] \
            PUBLIC TYPE Box<T>=[T value] \
            PUBLIC Link POINTER FUNC Identity(Link POINTER value) \
            RETURN(Link POINTER(value)) ENDMODULE"
                .to_vec(),
        );
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let model = semantic::analyze_compilation_with_options(
            &loaded,
            SemanticOptions::modern().with_target(target),
        )
        .unwrap();
        let raw = nir::lower_program(&semantic::ir::lower_compilation(&loaded, &model));
        nir::verify_program(&raw).unwrap();
        for program in [raw.clone(), nir::optimize_program(&raw).unwrap()] {
            nir::verify_program(&program).unwrap();
            match target {
                TargetId::Atari6502 => {
                    actionc::mir6502::lower_program(&program).unwrap();
                }
                TargetId::Motorola68000 => {
                    actionc::mir68k::lower_program(&program).unwrap();
                }
                _ => {
                    actionc::mir65816::lower_program(&program).unwrap();
                }
            }
        }
    }
}

#[test]
fn named_pointer_casts_reject_bad_arity_and_unknown_types() {
    for value in ["Link POINTER()", "Link POINTER(0,1)"] {
        let source =
            format!("TYPE Link=[BYTE value] Link POINTER cursor PROC Main() cursor={value} RETURN");
        let errors = parse(&tokenize(&source).unwrap()).unwrap_err();
        assert!(
            errors.iter().any(|e| e
                .message
                .contains("pointer cast requires exactly one argument")),
            "{errors:?}"
        );
    }
    let source = "BYTE POINTER cursor PROC Main() cursor=BYTE POINTER(Missing POINTER(0)) RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    assert!(semantic::analyze_with_options(&ast, SemanticOptions::modern()).is_err());
}
