use actionc::{
    lexer::tokenize,
    parser::parse,
    semantic::{self, AggregateKind, SemanticOptions},
    target::TargetId,
};

fn options() -> SemanticOptions {
    let mut options = SemanticOptions::modern();
    options.algebraic_types.unions = true;
    options
}
fn analyze(source: &str) -> semantic::SemanticModel {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    semantic::analyze_with_options(&ast, options()).unwrap_or_else(|e| panic!("{source}: {e:?}"))
}
fn rejects(source: &str, needle: &str) {
    let ast = parse(&tokenize(source).unwrap()).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    let errors = semantic::analyze_with_options(&ast, options()).expect_err(source);
    assert!(
        errors.iter().any(|e| e.message.contains(needle)),
        "{source}: {errors:?}"
    );
}

#[test]
fn generic_unions_share_the_nominal_cache_and_overlapping_layout_after_substitution() {
    let source = "TYPE Overlay<T,U>=UNION [T first U second] TYPE Pair=[BYTE low CARD high] \
        Overlay<CARD,Pair> first,second Overlay<INT,Pair> third \
        TYPE Box<T>=[T value] Box<Overlay<CARD,Pair>> nested PROC Main() first.second.low=7 second=first RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model = semantic::analyze_with_options(&ast, options().with_target(target)).unwrap();
        assert_eq!(model.generics.instances.len(), 3);
        let types: [_; 3] = ["first", "second", "third"].map(|name| {
            &model
                .symbols
                .symbols
                .iter()
                .find(|s| s.name == name)
                .unwrap()
                .ty
        });
        assert_eq!(types[0], types[1]);
        assert_ne!(types[0], types[2]);
        for instance in &model.generics.instances[..2] {
            let layout = model.layout.record_for_owner(instance.owner).unwrap();
            assert_eq!(layout.kind, AggregateKind::Union);
            assert!(layout.fields.iter().all(|f| f.offset == 0));
            assert_eq!(
                layout.size,
                if target == TargetId::Atari6502 { 3 } else { 4 }
            );
        }
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
fn substituted_members_keep_transitive_restrictions_and_pointer_barriers() {
    for (prefix, argument) in [
        ("", "REAL"),
        ("TYPE Hidden=[REAL ARRAY values(2)]", "Hidden"),
        ("TYPE Event=VARIANT [NONE]", "Event"),
        (
            "TYPE Event=VARIANT [NONE] TYPE Box<T>=[T ARRAY values(2)]",
            "Box<Event>",
        ),
        ("TYPE Hidden=[PROC POINTER callback()]", "Hidden"),
        ("", "PROC POINTER()"),
    ] {
        rejects(
            &format!("{prefix} TYPE View<T>=UNION [T first BYTE low] View<{argument}> value"),
            "UNION members cannot contain inline",
        );
    }
    analyze(
        "TYPE Event=VARIANT [NONE] TYPE View<T>=UNION [T first CARD bits] View<Event POINTER> a View<REAL POINTER> b",
    );
    rejects(
        "TYPE View<T>=UNION [T first BYTE low] View<CARD> value=[1 2]",
        "union-containing storage",
    );
    rejects(
        "TYPE View<T>=UNION [T first] View<CARD> a View<INT> b PROC Main() a=b RETURN",
        "cannot assign record",
    );
    rejects(
        "TYPE View<T>=UNION [T first] View<CARD> FUNC POINTER callback(View<CARD> input) View<INT> FUNC Wrong(View<INT> input) RETURN(input) PROC Main() callback=Wrong RETURN",
        "exact nominal signature",
    );
}

#[test]
fn generic_union_recursion_remains_finite_and_budgets_apply() {
    let model = analyze(
        "TYPE Left<T>=UNION [T value Right<T> POINTER next] TYPE Right<T>=UNION [T value Left<T> POINTER prev] Left<BYTE> leftValue Right<BYTE> rightValue",
    );
    assert_eq!(model.generics.instances.len(), 2);
    for source in [
        "TYPE View<T>=UNION [T first View<T> next] View<BYTE> value",
        "TYPE Box<T>=[T value] TYPE View<T>=UNION [T first View<Box<T>> POINTER next] View<BYTE> value",
        "TYPE View<T>=UNION [T first] View<BYTE,CARD> value",
        "TYPE View<T>=UNION [Missing first]",
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        assert!(
            semantic::analyze_with_options(&ast, options()).is_err(),
            "{source}"
        );
    }
    let mut source = String::from("TYPE View<T>=UNION [T first BYTE low] ");
    for i in 0..1025 {
        source.push_str(&format!("TYPE T{i}=[BYTE value] View<T{i}> v{i} "));
    }
    rejects(&source, "instantiation budget");
    let mut source = String::new();
    for i in 0..66 {
        source.push_str(&format!(
            "TYPE V{i}<T>=UNION [V{}<T> POINTER next T value] ",
            i + 1
        ));
    }
    source.push_str("TYPE V66<T>=UNION [T value] V0<BYTE> rootValue");
    rejects(&source, "instantiation budget");
}

#[test]
fn union_variant_binders_are_immutable_and_have_no_union_patterns() {
    for statement in [
        "saved.word=1",
        "address=@saved",
        "address=@saved.bytes(0)",
        "BEGIN\nCARD alias=saved.word\nEND",
    ] {
        rejects(
            &format!(
                "TYPE View=UNION [CARD word BYTE ARRAY bytes(2)] TYPE Event=VARIANT [DATA [View view]] Event value CARD address PROC Main()\nCASE value OF\nWHEN Event.DATA(saved) THEN\n{statement}\nESAC\nRETURN"
            ),
            "immutable LET",
        );
    }
    rejects(
        "TYPE View=UNION [CARD word] TYPE Event=VARIANT [DATA [View view]] Event value PROC Main()\nCASE value OF\nWHEN Event.DATA(View.word(n)) THEN\nRETURN\nESAC\nRETURN",
        "variant",
    );
}

#[test]
fn imported_generic_union_definitions_share_identity_and_native_value_abis() {
    use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use actionc::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("project/main.act");
    let provider=InMemorySourceProvider::default()
        .with_source(root.clone(),b"MODULE App USE Lib AS A USE Lib AS B A.View<CARD> original B.View<CARD> copy B.View<CARD> FUNC POINTER callback(B.View<CARD> input) PROC Main() callback=A.Copy original.first=7 copy=callback(original) RETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/lib.act"),b"MODULE Lib PUBLIC TYPE View<T>=UNION [T first BYTE ARRAY bytes(3)] PUBLIC View<CARD> FUNC Copy(View<CARD> input) RETURN(input) ENDMODULE".to_vec());
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let model =
            semantic::analyze_compilation_with_options(&loaded, options().with_target(target))
                .unwrap();
        assert_eq!(model.generics.instances.len(), 1);
        let original = &model
            .symbols
            .symbols
            .iter()
            .find(|s| s.name == "original")
            .unwrap()
            .ty;
        let copy = &model
            .symbols
            .symbols
            .iter()
            .find(|s| s.name == "copy")
            .unwrap()
            .ty;
        assert_eq!(original, copy);
        let instance = &model.generics.instances[0];
        assert_eq!(
            model.layout.record_for_owner(instance.owner).unwrap().kind,
            AggregateKind::Union
        );
        let nir = actionc::nir::lower_program(&semantic::ir::lower_compilation(&loaded, &model));
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
