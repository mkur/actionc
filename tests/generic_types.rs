use actionc::{
    lexer::tokenize,
    parser::parse,
    semantic::{self, SemanticOptions},
};

fn options() -> SemanticOptions {
    let mut options = SemanticOptions::modern();
    options.algebraic_types.generic_types = true;
    options
}

#[test]
fn generic_types_cache_concrete_instances_and_keep_nominal_arguments() {
    let source = "TYPE Box<T>=[T value] Box<BYTE> first,second Box<CARD> third PROC Main() RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, options()).unwrap();
    assert_eq!(model.generics.instances.len(), 2);
    let types: Vec<_> = ["first", "second", "third"]
        .map(|name| {
            model
                .symbols
                .symbols
                .iter()
                .find(|s| s.name == name)
                .unwrap()
                .ty
                .clone()
        })
        .into();
    assert_eq!(types[0], types[1]);
    assert_ne!(types[0], types[2]);
}

#[test]
fn generic_types_reject_invalid_arguments_inline_cycles_and_expansion() {
    for source in [
        "TYPE Box<T>=[T value] Box data PROC Main() RETURN",
        "TYPE Box<T>=[T value] Box<BYTE,CARD> data PROC Main() RETURN",
        "TYPE Box<T>=[U value] Box<BYTE> data PROC Main() RETURN",
        "TYPE Box<T>=[U value] PROC Main() RETURN",
        "TYPE Box<T,T>=[T value] Box<BYTE,CARD> data PROC Main() RETURN",
        "TYPE Box<T>=[T value] Box<STRING> data PROC Main() RETURN",
        "TYPE Box<T>=[T value] PROC Task() RETURN PROC Main() Box<Task> data RETURN",
        "TYPE Box<T>=[Box<T> child] Box<BYTE> data PROC Main() RETURN",
        "TYPE Box<T>=[T value] TYPE Grow<T>=[Grow<Box<T>> POINTER next] Grow<BYTE> data PROC Main() RETURN",
        "TYPE Box<T>=[T value] Box<BYTE> first Box<CARD> second PROC Main() first=second RETURN",
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        assert!(
            semantic::analyze_with_options(&ast, options()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generic_types_share_instances_across_import_aliases_and_public_calls() {
    use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use actionc::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(
            root.clone(),
            br#"MODULE App
USE Lib AS A USE Lib AS B
A.Option<BYTE> current
BYTE output
PROC Main()
 current=A.Make(9)
 CASE current OF
 WHEN B.Option<BYTE>.NONE THEN
   output=0
 WHEN B.Option<BYTE>.SOME(code) THEN
   output=code
 ESAC
RETURN ENDMODULE"#
                .to_vec(),
        )
        .with_source(
            SourceOrigin::host("project/lib.act"),
            br#"MODULE Lib
PUBLIC TYPE Option<T>=VARIANT [NONE SOME [T value]]
PUBLIC Option<BYTE> FUNC Make(BYTE n) RETURN(Option<BYTE>.SOME(n))
ENDMODULE"#
                .to_vec(),
        );
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model = semantic::analyze_compilation_with_options(&loaded, options()).unwrap();
    assert_eq!(model.generics.instances.len(), 1);
    let semir = semantic::ir::lower_compilation(&loaded, &model);
    let nir = actionc::nir::lower_program(&semir);
    actionc::nir::verify_program(&nir).unwrap();
    actionc::nir::optimize_program(&nir).unwrap();
    actionc::codegen::generate_semir_profile_with_origin(
        &semir,
        0x3000,
        actionc::codegen::CodegenProfile::Modern,
    )
    .unwrap();
}

#[test]
fn generic_types_regular_mutual_recursion_is_finite_and_target_sized() {
    use actionc::target::TargetId;
    let source = "TYPE Left<T>=[Right<T> POINTER next T value] TYPE Right<T>=[Left<T> POINTER next T value] \
        TYPE Box<T>=[T value] Left<BYTE> first Right<BYTE> second Box<BYTE POINTER> third PROC Main() RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let model = semantic::analyze_with_options(&ast, options().with_target(target)).unwrap();
        assert_eq!(model.generics.instances.len(), 3);
        let boxed = model.generics.instances.last().unwrap();
        let layout = model.layout.record_for_owner(boxed.owner).unwrap();
        assert_eq!(
            layout.size,
            model.target_layout.data_pointer.size_bytes.get()
        );
        let nir = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        actionc::nir::verify_program(&nir).unwrap();
    }
}

#[test]
fn generic_types_keep_ordinary_comparisons_and_reject_unbounded_syntax() {
    let source = "TYPE Box<T>=[T value] BYTE a,b,c PROC Main() a=b>=c a=b<c a=b>c RETURN";
    let tokens = tokenize(source).unwrap();
    assert_eq!(
        tokens
            .iter()
            .filter(|t| matches!(t.kind, actionc::lexer::TokenKind::Ge))
            .count(),
        2
    );
    let ast = parse(&tokens).unwrap();
    semantic::analyze_with_options(&ast, options()).unwrap();
    let nested = format!(
        "TYPE Box<T>=[T value] PROC Main() LET {}BYTE{} value=0 RETURN",
        "Box<".repeat(80),
        ">".repeat(80)
    );
    assert!(parse(&tokenize(&nested).unwrap()).is_err());
}

#[test]
fn generic_types_enforce_the_instance_budget_without_expansion() {
    let mut source = String::from("TYPE Box<T>=[T value] ");
    for index in 0..1025 {
        source.push_str(&format!(
            "TYPE T{index}=[BYTE value] Box<T{index}> v{index} "
        ));
    }
    source.push_str("PROC Main() RETURN");
    let ast = parse(&tokenize(&source).unwrap()).unwrap();
    let errors = semantic::analyze_with_options(&ast, options()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("instantiation budget")),
        "{errors:?}"
    );
}

#[test]
fn generic_types_bound_distinct_definition_depth_and_keep_output_deterministic() {
    let mut deep = String::new();
    for index in 0..66 {
        deep.push_str(&format!(
            "TYPE T{index}<A>=[T{}<A> POINTER next] ",
            index + 1
        ));
    }
    deep.push_str("TYPE T66<A>=[A value] T0<BYTE> data PROC Main() RETURN");
    let ast = parse(&tokenize(&deep).unwrap()).unwrap();
    let errors = semantic::analyze_with_options(&ast, options()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("instantiation budget")),
        "{errors:?}"
    );

    let source = "TYPE Option<T>=VARIANT [NONE SOME [T value]] Option<BYTE> a,b Option<CARD> c PROC Main() a=Option<BYTE>.SOME(3) RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let first = semantic::analyze_with_options(&ast, options()).unwrap();
    let second = semantic::analyze_with_options(&ast, options()).unwrap();
    assert_eq!(first, second);
    let first = semantic::ir::lower_program(&ast, &first);
    let second = semantic::ir::lower_program(&ast, &second);
    assert_eq!(
        semantic::ir::format_program(&first),
        semantic::ir::format_program(&second)
    );
}

#[test]
fn generic_types_do_not_collapse_shadowed_definitions_or_same_width_arguments() {
    let source = "TYPE Box<T>=[T value] Box<BYTE> first Box<CHAR> second \
        PROC Main() TYPE Box<T>=[T value] Box<BYTE> third RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, options()).unwrap();
    assert_eq!(model.generics.instances.len(), 3);
    let instances = &model.generics.instances;
    assert_eq!(instances[0].definition, instances[1].definition);
    assert_ne!(instances[0].definition, instances[2].definition);
    assert_ne!(instances[0].owner, instances[1].owner);
    assert!(semantic::analyze_with_options(&ast, SemanticOptions::default()).is_err());
}

#[test]
fn generic_types_constant_specializations_stabilize_and_layout_queries_resolve() {
    let source = "TYPE Anchor<T>=[Anchor<BYTE> POINTER next T value] Anchor<CARD> first \
        CONST Extent=SIZEOF(Anchor<CARD>) PROC Main() RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, options()).unwrap();
    assert_eq!(model.generics.instances.len(), 2);
    let id = model
        .symbols
        .symbols
        .iter()
        .position(|s| s.name == "Extent")
        .unwrap();
    assert_eq!(model.constants[&semantic::SymbolId(id)].bits, 4);
    let nir = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    actionc::nir::verify_program(&nir).unwrap();
}
