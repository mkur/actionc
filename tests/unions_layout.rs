//! Slice 1 tests deliberately opt into an internal layout capability. Public
//! profiles stay closed until access, effects and value-boundary acceptance.
use actionc::ast::{Decl, Item, TypeDefinition};
use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
use actionc::semantic::{self, AggregateKind, SemanticModel, SemanticOptions};
use actionc::source::{InMemorySourceProvider, SourceOrigin};
use actionc::target::{TargetId, TargetLayout};
use actionc::{lexer::tokenize, nir, parser::parse};

const TARGETS: [TargetId; 4] = [
    TargetId::Atari6502,
    TargetId::Wdc65816Small,
    TargetId::Wdc65816Native,
    TargetId::Motorola68000,
];

fn options(target: TargetId) -> SemanticOptions {
    let mut options = SemanticOptions::modern().with_target(target);
    options.algebraic_types.unions = true;
    options
}

fn analyze(source: &str, target: TargetId) -> SemanticModel {
    let ast =
        parse(&tokenize(source).unwrap()).unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    semantic::analyze_with_options(&ast, options(target))
        .unwrap_or_else(|errors| panic!("{source}\n{target:?}: {errors:#?}"))
}

fn rejects(source: &str, needle: &str) {
    let ast =
        parse(&tokenize(source).unwrap()).unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    let errors = semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).unwrap_err();
    assert!(
        errors.iter().any(|error| error.message.contains(needle)),
        "{source}\nexpected {needle:?}: {errors:#?}"
    );
}

#[test]
fn union_syntax_reuses_record_members_without_reserving_the_name() {
    let source = "TYPE View=UNION [BYTE first,second CARD word BYTE ARRAY bytes(3)]";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let Item::Declaration(Decl::Type(decl)) = &ast.modules[0].items[0] else {
        panic!("TYPE");
    };
    let TypeDefinition::Union(fields) = &decl.definition else {
        panic!("UNION");
    };
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[0].entries.len(), 2);
    assert_eq!(fields[2].entries[0].name, "bytes");
    for source in [
        "TYPE Union=[BYTE value] Union object PROC Main() object.value=7 RETURN",
        "PROC Union() RETURN PROC Main() Union() RETURN",
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    }
    for source in ["TYPE V=UNION", "TYPE V=UNION [", "TYPE V=UNION [BYTE value"] {
        assert!(parse(&tokenize(source).unwrap()).is_err(), "{source}");
    }
}

#[test]
fn union_declarations_remain_closed_in_all_public_profiles() {
    for target in TARGETS {
        for profile in [SemanticOptions::default(), SemanticOptions::modern()] {
            assert!(!profile.algebraic_types.unions);
            for source in [
                "TYPE View=UNION [CARD word] PROC Main() RETURN",
                "PROC Main() TYPE View=UNION [CARD word] RETURN",
                "TYPE View<T>=UNION [T value] PROC Main() RETURN",
            ] {
                let ast = parse(&tokenize(source).unwrap()).unwrap();
                let errors =
                    semantic::analyze_with_options(&ast, profile.with_target(target)).unwrap_err();
                assert!(
                    errors.iter().any(|e| e.message.contains("not enabled")
                        || e.message.contains("modern generic-types")),
                    "{source}\n{target:?}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn union_extents_alignment_and_queries_use_canonical_target_layouts() {
    let source = "TYPE Pair=[BYTE low CARD high] \
        TYPE View=UNION [CARD word INT signedWord BYTE ARRAY bytes(7) Pair pair CARD POINTER link] \
        TYPE Wrapper=[BYTE lead View payload BYTE tail] \
        TYPE Tagged=VARIANT [NONE VALUE [CARD payload]] \
        View ARRAY items(2) \
        CONST Extent=SIZEOF(View) CONST Alignment=ALIGNOF(View) CONST MemberOffset=OFFSETOF(View,link) \
        BYTE result PROC Main() result=Extent+Alignment+MemberOffset RETURN";
    for target in TARGETS {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model = analyze(source, target);
        let packed = target == TargetId::Atari6502;
        let view = model.layout.record_for_name("View").unwrap();
        assert_eq!(view.kind, AggregateKind::Union);
        assert_eq!(
            (view.size, view.alignment, view.tail_padding),
            if packed { (7, 1, 0) } else { (8, 2, 1) }
        );
        assert_eq!(view.fields.len(), 5);
        assert!(view.fields.iter().all(|field| field.offset == 0));
        assert_eq!(
            view.fields[4].size,
            TargetLayout::for_target(target)
                .data_pointer
                .size_bytes
                .get()
        );
        let wrapper = model.layout.record_for_name("Wrapper").unwrap();
        assert_eq!(wrapper.kind, AggregateKind::Record);
        assert_eq!(wrapper.fields[1].offset, if packed { 1 } else { 2 });
        assert_eq!(wrapper.fields[2].offset, if packed { 8 } else { 10 });
        assert_eq!(wrapper.size, if packed { 9 } else { 12 });
        assert_eq!(
            model.layout.record_for_name("Tagged").unwrap().kind,
            AggregateKind::Variant
        );
        let items = model
            .layout
            .arrays
            .iter()
            .find(|a| a.name == "items")
            .unwrap();
        assert_eq!(items.stride, view.size);
        assert_eq!(items.storage_size, Some(view.size * 2));
        for (name, expected) in [
            ("Extent", view.size),
            ("Alignment", view.alignment),
            ("MemberOffset", 0),
        ] {
            let id = model
                .symbols
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap();
            assert_eq!(
                model.constants[&semantic::SymbolId(id)].bits,
                u64::from(expected)
            );
        }
        let semir = semantic::ir::lower_program(&ast, &model);
        let raw = nir::lower_program(&semir);
        nir::verify_program(&raw).unwrap();
        let optimized = nir::optimize_program(&raw).unwrap();
        match target {
            TargetId::Atari6502 => {
                actionc::mir6502::lower_program(&optimized).unwrap();
            }
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(&optimized).unwrap();
            }
            _ => {
                actionc::mir65816::lower_program(&optimized).unwrap();
            }
        }
    }
}

#[test]
fn grouped_members_overlap_but_nested_record_fields_remain_sequential() {
    let model = analyze(
        "TYPE Pair=[BYTE first,second] TYPE View=UNION [BYTE first,second, Pair parts]",
        TargetId::Atari6502,
    );
    let view = model.layout.record_for_name("View").unwrap();
    assert_eq!(view.size, 2);
    assert_eq!(
        view.fields.iter().map(|f| f.offset).collect::<Vec<_>>(),
        [0, 0, 0]
    );
    assert_ne!(view.fields[0].id, view.fields[1].id);
    let pair = model.layout.record_for_name("Pair").unwrap();
    assert_eq!(
        pair.fields.iter().map(|f| f.offset).collect::<Vec<_>>(),
        [0, 1]
    );
}

#[test]
fn unions_reuse_nominal_identity_and_pointer_recursion_barriers() {
    for target in TARGETS {
        let model = analyze(
            "TYPE First=UNION [CARD word Second POINTER next] \
            TYPE Second=UNION [CARD word First POINTER prev] \
            TYPE Other=UNION [CARD word Second POINTER next]",
            target,
        );
        let first = model.layout.record_for_name("First").unwrap();
        let second = model.layout.record_for_name("Second").unwrap();
        let other = model.layout.record_for_name("Other").unwrap();
        assert_eq!(
            first.fields[1].ty.as_aggregate_identity().unwrap().symbol,
            Some(second.owner)
        );
        assert_eq!(
            second.fields[1].ty.as_aggregate_identity().unwrap().symbol,
            Some(first.owner)
        );
        assert_ne!(first.record_type.identity, other.record_type.identity);
        assert!(
            !first
                .record_type
                .value_type()
                .same_record_family(&other.record_type.value_type())
        );
    }
    for source in [
        "TYPE Loop=UNION [Loop value]",
        "TYPE First=UNION [Second value] TYPE Second=[First value]",
        "TYPE First=[Second value] TYPE Second=UNION [First value]",
    ] {
        rejects(source, "cyclic");
    }
}

#[test]
fn invalid_union_definitions_fail_before_lowering() {
    for (source, needle) in [
        ("TYPE Bad=UNION []", "at least one"),
        ("TYPE Bad=UNION [CARD word BYTE]", "requires a name"),
        ("TYPE Bad=UNION [BYTE x CARD X]", "duplicate UNION member"),
        (
            "TYPE Bad=UNION [BYTE ARRAY bytes]",
            "explicit constant bound",
        ),
        ("TYPE Bad=UNION [BYTE ARRAY bytes(0)]", "positive constant"),
        ("TYPE Bad=UNION [CARD ARRAY words($8000)]", "extent"),
        ("TYPE Empty=[] TYPE Bad=UNION [Empty value]", "non-zero"),
        ("TYPE Bad=UNION [Missing value]", "unknown type"),
        (
            "TYPE Bad=UNION [VOLATILE BYTE value]",
            "VOLATILE record fields",
        ),
        ("TYPE Bad=UNION [BYTE value=[7]]", "record fields"),
    ] {
        rejects(source, needle);
    }
    let source = "TYPE View=UNION [BYTE ARRAY raw($FFFF) CARD word]";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    assert_eq!(
        analyze(source, TargetId::Atari6502)
            .layout
            .record_for_name("View")
            .unwrap()
            .size,
        65535
    );
    assert!(semantic::analyze_with_options(&ast, options(TargetId::Wdc65816Small)).is_err());
    assert_eq!(
        analyze(source, TargetId::Wdc65816Native)
            .layout
            .record_for_name("View")
            .unwrap()
            .size,
        65536
    );
}

#[test]
fn union_member_restrictions_follow_inline_records_arrays_and_instances() {
    for source in [
        "TYPE Bad=UNION [REAL value]",
        "TYPE Bad=UNION [PROC POINTER callback()]",
        "TYPE Event=VARIANT [NONE] TYPE Bad=UNION [Event value]",
        "TYPE Payload=[REAL ARRAY values(2)] TYPE Bad=UNION [Payload value]",
        "TYPE Event=VARIANT [NONE] TYPE Box<T>=[T value] TYPE Bad=UNION [Box<Event> value]",
        "TYPE Inner=[PROC POINTER callback()] TYPE Bad=UNION [Inner ARRAY values(2)]",
    ] {
        rejects(source, "UNION members cannot contain inline");
    }
    analyze(
        "TYPE Event=VARIANT [NONE] TYPE Box=[REAL value] \
        TYPE View=UNION [Event POINTER eventPtr Box POINTER recordPtr REAL POINTER realValue CARD word]",
        TargetId::Atari6502,
    );
    rejects(
        "TYPE Overlay<T,U>=UNION [T first U second]",
        "generic UNION definitions are not enabled",
    );
}

#[test]
fn positional_union_initializers_cannot_reach_the_record_leaf_walker() {
    for declaration in [
        "View value=[1 2]",
        "View ARRAY values(2)=[1 2 3 4]",
        "TYPE Wrapper=[BYTE lead View value] Wrapper wrapper=[1 2 3]",
        "TYPE Wrapper=[View ARRAY values(2)] Wrapper wrapper=[1 2 3 4]",
        "TYPE Box<T>=[T value] Box<View> wrapper=[1 2]",
        "View value=\"AB\"",
    ] {
        rejects(
            &format!("TYPE View=UNION [CARD word BYTE ARRAY bytes(2)] {declaration}"),
            "union-containing storage",
        );
    }
    // A pointer's storage contains an address, not an inline union image.
    analyze(
        "TYPE View=UNION [CARD word BYTE ARRAY bytes(2)] View POINTER p=[NIL]",
        TargetId::Atari6502,
    );
}

#[test]
fn named_module_aliases_share_one_canonical_union_layout() {
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), b"MODULE App USE Lib AS A USE Lib AS B A.View one B.View two PROC Main() RETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/lib.act"), b"MODULE Lib PUBLIC TYPE View=UNION [CARD word BYTE ARRAY bytes(2)] ENDMODULE".to_vec());
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    for target in TARGETS {
        let model = semantic::analyze_compilation_with_options(&loaded, options(target)).unwrap();
        let view = model.layout.record_for_name("Lib.View").unwrap();
        assert_eq!(view.kind, AggregateKind::Union);
        let one = model
            .symbols
            .symbols
            .iter()
            .find(|s| s.name == "one")
            .unwrap();
        let two = model
            .symbols
            .symbols
            .iter()
            .find(|s| s.name == "two")
            .unwrap();
        assert_eq!(one.ty, two.ty);
        assert_eq!(
            one.ty
                .as_ref()
                .unwrap()
                .as_aggregate_identity()
                .unwrap()
                .symbol,
            Some(view.owner)
        );
        let raw = nir::lower_program(&semantic::ir::lower_compilation(&loaded, &model));
        nir::verify_program(&raw).unwrap();
        nir::optimize_program(&raw).unwrap();
    }
}

#[test]
fn raw_ast_codegen_cannot_reconstruct_a_union_as_a_sequential_record() {
    for source in [
        "TYPE View=UNION [CARD word BYTE ARRAY bytes(2)] View value PROC Main() RETURN",
        "PROC Main() TYPE View=UNION [CARD word BYTE low] View value RETURN",
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let errors = actionc::codegen::generate_with_origin(&ast, 0x3000).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("requires semantic lowering")),
            "{errors:?}"
        );
    }
}
