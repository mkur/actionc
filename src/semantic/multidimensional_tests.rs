use super::*;
use crate::includes::{ModuleLoadOptions, load_compilation_from_provider};
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::source::{InMemorySourceProvider, SourceOrigin};

fn options(target: TargetId) -> SemanticOptions {
    SemanticOptions {
        multidimensional_arrays: true,
        ..SemanticOptions::modern().with_target(target)
    }
}

fn check(source: &str, target: TargetId) -> Result<SemanticModel, Vec<Diagnostic>> {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    analyze_with_options(&ast, options(target))
}

#[test]
fn multidimensional_shapes_preserve_grouped_and_rank_one_declarations() {
    let ast = parse(&tokenize("BYTE ARRAY grid(2,3),line(4),volume(2,3,5)").unwrap()).unwrap();
    let Item::Declaration(Decl::Var(decl)) = &ast.modules[0].items[0] else {
        panic!()
    };
    assert_eq!(
        decl.entries
            .iter()
            .map(|e| (e.dimensions.len(), e.size.is_some()))
            .collect::<Vec<_>>(),
        [(2, false), (0, true), (3, false)]
    );
    let model = analyze_with_options(&ast, options(TargetId::Atari6502)).unwrap();
    for (name, dimensions, count) in [
        ("grid", vec![2, 3], 6),
        ("line", vec![4], 4),
        ("volume", vec![2, 3, 5], 30),
    ] {
        let symbol = model
            .symbols
            .lookup(model.symbols.global_scope(), name)
            .unwrap();
        let layout = model.layout.array_for_symbol(symbol).unwrap();
        assert_eq!(layout.shape.dimensions(), dimensions);
        assert_eq!(
            (layout.length, layout.storage_size),
            (Some(count), Some(count))
        );
    }
}

#[test]
fn multidimensional_shapes_use_target_record_stride_and_local_constants() {
    let source = "CONST Rows=2 TYPE Point=[BYTE tag CARD value] \
        TYPE Tile=[BYTE tag Point ARRAY pixels(Rows,3)] Tile tileData \
        Point ARRAY points(Rows,3) PROC Main() CONST Rows=4 BYTE ARRAY local(Rows,SIZEOF(CARD)) RETURN";
    for (target, stride, size) in [
        (TargetId::Atari6502, 3, 19),
        (TargetId::Wdc65816Small, 4, 26),
        (TargetId::Wdc65816Native, 4, 26),
        (TargetId::Motorola68000, 4, 26),
    ] {
        let model = check(source, target).unwrap();
        let tile = model.layout.record_for_name("Tile").unwrap();
        assert_eq!(tile.size, size, "{target:?}");
        let RecordFieldStorage::InlineArray {
            array_type,
            stride: field_stride,
        } = &tile.fields[1].storage
        else {
            panic!()
        };
        assert_eq!(array_type.shape().dimensions(), [2, 3]);
        assert_eq!(*field_stride, stride);
        let symbol = model
            .symbols
            .lookup(model.symbols.global_scope(), "points")
            .unwrap();
        let layout = model.layout.array_for_symbol(symbol).unwrap();
        assert_eq!(
            (layout.stride, layout.storage_size),
            (stride, Some(6 * stride))
        );
        let local = model
            .symbols
            .lookup(model.routine_scopes[0].scope, "local")
            .unwrap();
        assert_eq!(
            model
                .layout
                .array_for_symbol(local)
                .unwrap()
                .shape
                .dimensions(),
            [4, 2]
        );
    }
}

#[test]
fn multidimensional_shapes_resolve_imported_constants() {
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(
            root.clone(),
            "MODULE App USE Limits CARD ARRAY data(Limits.Rows,3) PROC Main() RETURN ENDMODULE",
        )
        .with_source(
            SourceOrigin::host("project/limits.act"),
            "MODULE Limits PUBLIC CONST Rows=4 ENDMODULE",
        );
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model = analyze_compilation_with_options(&loaded, options(TargetId::Atari6502)).unwrap();
    let layout = model
        .layout
        .arrays
        .iter()
        .find(|a| a.name == "data")
        .unwrap();
    assert_eq!(
        (layout.shape.dimensions(), layout.storage_size),
        (&[4, 3][..], Some(24))
    );
}

#[test]
fn multidimensional_shapes_diagnose_bad_bounds_and_checked_extents() {
    for (source, expected, target) in [
        ("BYTE ARRAY a(0,2)", "positive", TargetId::Atari6502),
        ("BYTE ARRAY a(-1,2)", "positive", TargetId::Atari6502),
        (
            "BYTE ARRAY a(1.5,2)",
            "integer constant",
            TargetId::Atari6502,
        ),
        (
            "CARD n BYTE ARRAY a(n,2)",
            "integer constant",
            TargetId::Atari6502,
        ),
        (
            "BYTE ARRAY a(1/0,2)",
            "division by zero",
            TargetId::Atari6502,
        ),
        ("BYTE ARRAY a(256,256)", "SIZE limit", TargetId::Atari6502),
        (
            "LONGINT ARRAY a(128,128)",
            "object limit",
            TargetId::Atari6502,
        ),
        (
            "BYTE ARRAY a(LONGCARD(65536),LONGCARD(65536))",
            "overflows",
            TargetId::Motorola68000,
        ),
        (
            "LONGINT ARRAY a(LONGCARD(65536),LONGCARD(16384))",
            "overflows",
            TargetId::Motorola68000,
        ),
        ("BYTE ARRAY a(2,3)=$FFFE", "endpoint", TargetId::Atari6502),
        ("BYTE a(2,3)", "ARRAY declaration", TargetId::Atari6502),
        (
            "PROC Test(BYTE ARRAY a(2,3)) RETURN",
            "parameters are not supported",
            TargetId::Atari6502,
        ),
        ("TYPE Bad=[Bad ARRAY a(2,3)]", "cyclic", TargetId::Atari6502),
    ] {
        let errors = check(source, target).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{source}: {errors:?}"
        );
    }
    // Native counts must not inherit the Atari descriptor's 16-bit count limit.
    let model = check("BYTE ARRAY big(256,256)", TargetId::Motorola68000).unwrap();
    assert_eq!(model.layout.arrays.iter().find(|a| a.name == "big").unwrap().storage_size, Some(65536));
}

#[test]
fn multidimensional_declarations_require_modern_profile_and_every_bound() {
    let ast = parse(&tokenize("BYTE ARRAY a(2,3)").unwrap()).unwrap();
    for options in [SemanticOptions::default()] {
        let errors = analyze_with_options(&ast, options).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("multidimensional arrays require"))
        );
    }
    analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    for source in [
        "BYTE ARRAY a(,3)",
        "BYTE ARRAY a(2,)",
        "BYTE ARRAY a()",
        "PROC POINTER handler(BYTE ARRAY a(2,3))",
    ] {
        assert!(parse(&tokenize(source).unwrap()).is_err(), "{source}");
    }
    assert!(ArrayShape::fixed(vec![2]).is_err());
    assert!(ArrayShape::fixed(vec![2, 0]).is_err());
    assert!(ArrayShape::fixed(vec![u32::MAX, 2]).is_err());
}
