//! Nominal aggregate references must remain finite and survive every IR boundary.
use actionc::ast::Program;
use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
use actionc::lexer::tokenize;
use actionc::nir::{self, NirTypeKind};
use actionc::parser::parse;
use actionc::semantic::{
    self, AggregateIdentity, SemanticModel, SemanticOptions, SymbolId, ValueType,
};
use actionc::source::{InMemorySourceProvider, SourceOrigin};
use actionc::target::{TargetId, TargetLayout};

const TARGETS: [TargetId; 4] = [
    TargetId::Atari6502,
    TargetId::Wdc65816Small,
    TargetId::Wdc65816Native,
    TargetId::Motorola68000,
];

fn analyze(source: &str, target: TargetId) -> (Program, SemanticModel) {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap_or_else(|errors| panic!("{source}\n{target:?}: {errors:#?}"));
    (ast, model)
}

fn lower(ast: &Program, model: &SemanticModel) -> nir::NirProgram {
    let semir = semantic::ir::lower_program(ast, model);
    let nir = nir::lower_program(&semir);
    nir::verify_program(&nir).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let optimized = nir::optimize_program(&nir).unwrap();
    match model.target_layout.target {
        TargetId::Motorola68000 => {
            actionc::mir68k::lower_program(&optimized).unwrap();
        }
        TargetId::Wdc65816Small | TargetId::Wdc65816Native => {
            actionc::mir65816::lower_program(&optimized).unwrap();
        }
        TargetId::Atari6502 => {
            actionc::mir6502::lower_program(&optimized).unwrap();
        }
    }
    nir
}

#[test]
fn definition_identity_not_display_spelling_controls_aggregate_compatibility() {
    let ty = |id, name: &str| {
        ValueType::aggregate(AggregateIdentity::resolved(
            SymbolId(id),
            name.into(),
            "test::node".into(),
        ))
    };
    let original = ty(1, "Node");
    let renamed = ty(1, "DisplayAlias");
    let distinct = ty(2, "Node");
    assert_eq!(original, renamed);
    assert!(original.same_record_family(&ValueType::pointer_to(renamed)));
    assert_ne!(original, distinct);
    assert!(!original.same_record_family(&distinct));
    assert_ne!(original, ValueType::record("Node"));
}

#[test]
fn self_pointer_fields_keep_a_finite_definition_reference_on_all_targets() {
    for target in TARGETS {
        let (ast, model) = analyze(
            "TYPE Node=[BYTE value Node POINTER next] Node ARRAY pool(3) Node POINTER p \
             PROC Main() p=pool p.next=p RETURN",
            target,
        );
        let record = model.layout.record_for_name("Node").unwrap();
        let next = &record.fields[1];
        assert_eq!(
            next.ty.as_aggregate_identity().unwrap().symbol,
            Some(record.owner)
        );
        assert_eq!(next.ty.pointee_type(), record.record_type.value_type());
        assert_eq!(
            next.size,
            TargetLayout::for_target(target)
                .data_pointer
                .size_bytes
                .get()
        );
        let nir = lower(&ast, &model);
        let pointer = nir
            .globals
            .iter()
            .find(|global| global.name == "p")
            .unwrap();
        let NirTypeKind::Pointer {
            pointee: Some(pointee),
            ..
        } = &pointer.ty.as_ref().unwrap().kind
        else {
            panic!("record pointer");
        };
        assert!(
            matches!(pointee.as_ref(), NirTypeKind::Record { definition: Some(id), .. } if *id == record.owner)
        );
    }
}

#[test]
fn homonymous_local_record_layouts_do_not_overwrite_each_other() {
    let source = "PROC First() TYPE Pair=[BYTE value] Pair ARRAY rows(3) rows(2).value=7 RETURN \
                  PROC Second() TYPE Pair=[CARD value] Pair ARRAY rows(3) rows(2).value=$1234 RETURN \
                  PROC Main() First() Second() RETURN";
    for target in TARGETS {
        let (ast, model) = analyze(source, target);
        let records = &model.layout.records;
        assert_eq!(records.len(), 2);
        assert_ne!(records[0].owner, records[1].owner);
        assert_ne!(
            records[0].record_type.identity.canonical_name,
            records[1].record_type.identity.canonical_name
        );
        assert_eq!((records[0].size, records[1].size), (1, 2));
        let nir = lower(&ast, &model);
        for (name, expected_size) in [("First", 3), ("Second", 6)] {
            let routine = nir
                .routines
                .iter()
                .find(|routine| routine.name == name)
                .unwrap();
            assert!(
                routine
                    .locals
                    .iter()
                    .any(|local| { local.layout.size.get() == expected_size }),
                "{name}: {:#?}",
                routine.locals
            );
        }
    }
}

#[test]
fn imported_recursive_types_preserve_definition_identity_through_aliases() {
    for target in TARGETS {
        let root = SourceOrigin::host("project/main.act");
        let provider = InMemorySourceProvider::default()
            .with_source(root.clone(), b"MODULE App USE Lib AS A USE Lib AS B A.Left one B.Left two A.Left POINTER p PROC Main() one=two p=two RETURN ENDMODULE".to_vec())
            .with_source(SourceOrigin::host("project/lib.act"), b"MODULE Lib PUBLIC TYPE Left=[BYTE value Right POINTER next] PUBLIC TYPE Right=[CARD value Left POINTER prev] ENDMODULE".to_vec());
        let loaded =
            load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
        let model = semantic::analyze_compilation_with_options(
            &loaded,
            SemanticOptions::modern().with_target(target),
        )
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
        let left = model.layout.record_for_name("Lib.Left").unwrap();
        let right = model.layout.record_for_name("Lib.Right").unwrap();
        assert_eq!(
            left.fields[1].ty.as_aggregate_identity().unwrap().symbol,
            Some(right.owner)
        );
        assert_eq!(
            right.fields[1].ty.as_aggregate_identity().unwrap().symbol,
            Some(left.owner)
        );
        let one = model
            .symbols
            .symbols
            .iter()
            .find(|symbol| symbol.name == "one")
            .unwrap();
        let two = model
            .symbols
            .symbols
            .iter()
            .find(|symbol| symbol.name == "two")
            .unwrap();
        assert_eq!(one.ty, two.ty);
        let semir = semantic::ir::lower_compilation(&loaded, &model);
        let nir = nir::lower_program(&semir);
        nir::verify_program(&nir).unwrap();
        nir::optimize_program(&nir).unwrap();
    }
}

#[test]
fn record_pointer_callable_parameters_preserve_identity_through_module_aliases() {
    for target in TARGETS {
        let root = SourceOrigin::host("project/main.act");
        let provider = InMemorySourceProvider::default()
            .with_source(root.clone(), b"MODULE App USE Lib AS A USE Lib AS B PROC POINTER callback(A.Node POINTER arg) B.Node POINTER p PROC Main() callback=@B.Accept callback(p) RETURN ENDMODULE".to_vec())
            .with_source(SourceOrigin::host("project/lib.act"), b"MODULE Lib PUBLIC TYPE Node=[BYTE value] PUBLIC PROC Accept(Node POINTER arg) RETURN ENDMODULE".to_vec());
        let loaded =
            load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
        let model = semantic::analyze_compilation_with_options(
            &loaded,
            SemanticOptions::modern().with_target(target),
        )
        .unwrap_or_else(|errors| panic!("{target:?}: {errors:#?}"));
        let semir = semantic::ir::lower_compilation(&loaded, &model);
        let nir = nir::lower_program(&semir);
        nir::verify_program(&nir).unwrap();
        nir::optimize_program(&nir).unwrap();
    }
}

#[test]
fn record_pointer_signatures_are_stable_when_unrelated_declarations_shift_ids() {
    let compile = |prefix: &str| {
        let (ast, model) = analyze(
            &format!(
                "{prefix} TYPE Node=[BYTE value Node POINTER next] PROC Take(Node POINTER p) RETURN PROC Main() RETURN"
            ),
            TargetId::Atari6502,
        );
        let owner = model.layout.record_for_name("Node").unwrap().owner;
        let nir = lower(&ast, &model);
        let signature = nir
            .routines
            .iter()
            .find(|routine| routine.name == "Take")
            .unwrap()
            .signature
            .id;
        (owner, signature)
    };
    let first = compile("");
    let shifted = compile("TYPE Unrelated=[BYTE flag]");
    assert_ne!(first.0, shifted.0);
    assert_eq!(first.1, shifted.1);
}

#[test]
fn nir_rejects_a_name_only_aggregate_pointer_even_when_its_width_is_correct() {
    let (ast, model) = analyze(
        "TYPE Node=[BYTE value] Node POINTER p PROC Main() RETURN",
        TargetId::Atari6502,
    );
    let mut nir = lower(&ast, &model);
    let pointer = nir
        .globals
        .iter_mut()
        .find(|global| global.name == "p")
        .unwrap();
    let NirTypeKind::Pointer {
        pointee: Some(pointee),
        ..
    } = &mut pointer.ty.as_mut().unwrap().kind
    else {
        panic!("pointer");
    };
    let NirTypeKind::Record { definition, .. } = pointee.as_mut() else {
        panic!("record");
    };
    *definition = None;
    let errors = nir::verify_program(&nir).unwrap_err();
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("resolved aggregate definition identity")),
        "{errors:#?}"
    );
}

fn staged_options(target: TargetId) -> SemanticOptions {
    let mut options = SemanticOptions::modern().with_target(target);
    options.algebraic_types.aggregate_values = true;
    options
}

#[test]
fn staged_type_scopes_resolve_forward_layouts_and_mutual_pointer_cycles() {
    let declarations = "TYPE Left=[BYTE value Right POINTER next] \
                        TYPE Right=[CARD value Left POINTER prev] \
                        TYPE Outer=[Later ARRAY rows(2)] TYPE Later=[BYTE x,y] \
                        Left a Right b Outer data";
    for source in [
        format!("{declarations} PROC Main() RETURN"),
        format!("PROC Main() {declarations} RETURN"),
        format!("PROC Main()\nBEGIN\n{declarations}\nEND\nRETURN"),
    ] {
        for target in TARGETS {
            let ast = parse(&tokenize(&source).unwrap()).unwrap();
            let model = semantic::analyze_with_options(&ast, staged_options(target))
                .unwrap_or_else(|errors| panic!("{source}\n{target:?}: {errors:#?}"));
            let records = &model.layout.records;
            assert_eq!(records.len(), 4);
            let left = records
                .iter()
                .find(|record| model.symbols.symbols[record.owner.0].name == "Left")
                .unwrap();
            let right = records
                .iter()
                .find(|record| model.symbols.symbols[record.owner.0].name == "Right")
                .unwrap();
            let outer = records
                .iter()
                .find(|record| model.symbols.symbols[record.owner.0].name == "Outer")
                .unwrap();
            assert_eq!(
                left.fields[1].ty.as_aggregate_identity().unwrap().symbol,
                Some(right.owner)
            );
            assert_eq!(
                right.fields[1].ty.as_aggregate_identity().unwrap().symbol,
                Some(left.owner)
            );
            assert_eq!(outer.size, 4);
            lower(&ast, &model);
        }
    }
}

#[test]
fn staged_type_scopes_reject_inline_cycles_with_a_dependency_path() {
    for declarations in [
        "TYPE A=[A self]",
        "TYPE A=[B b] TYPE B=[A a]",
        "TYPE A=[B ARRAY rows(2)] TYPE B=[A a]",
    ] {
        for source in [
            declarations.to_string(),
            format!("PROC Main() {declarations} RETURN"),
            format!("PROC Main()\nBEGIN\n{declarations}\nEND\nRETURN"),
        ] {
            let ast = parse(&tokenize(&source).unwrap()).unwrap();
            let errors = semantic::analyze_with_options(&ast, staged_options(TargetId::Atari6502))
                .unwrap_err();
            let cycles = errors
                .iter()
                .filter(|error| {
                    error
                        .message
                        .contains("cyclic constant/record layout dependency")
                })
                .collect::<Vec<_>>();
            assert_eq!(cycles.len(), 1, "{source}\n{errors:#?}");
            assert!(cycles[0].message.contains(" -> "), "{errors:#?}");
        }
    }
}

#[test]
fn staged_type_visibility_does_not_enable_forward_constants_variables_or_routines() {
    for (source, needle) in [
        (
            "TYPE A=[BYTE ARRAY bytes(Count)] CONST Count=3",
            "undefined symbol `Count`",
        ),
        (
            "PROC Main() value=1 RETURN BYTE value",
            "undefined symbol `value`",
        ),
        (
            "PROC Main() Later() RETURN PROC Later() RETURN",
            "undefined symbol `Later`",
        ),
        (
            "TYPE A=[BYTE first] TYPE A=[BYTE second]",
            "duplicate symbol `A`",
        ),
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let errors =
            semantic::analyze_with_options(&ast, staged_options(TargetId::Atari6502)).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(needle)),
            "{source}\n{errors:#?}"
        );
    }
}

#[test]
fn new_type_scope_visibility_is_modern_only() {
    let ast = parse(&tokenize("TYPE A=[B POINTER next] TYPE B=[A POINTER prev]").unwrap()).unwrap();
    semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    for options in [SemanticOptions::default()] {
        let errors = semantic::analyze_with_options(&ast, options).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("unknown type `B`")),
            "{errors:#?}"
        );
    }
}
