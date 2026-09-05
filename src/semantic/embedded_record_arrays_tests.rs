use super::*;
use crate::includes::{ModuleLoadOptions, load_compilation_from_provider};
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::source::{InMemorySourceProvider, SourceOrigin};

fn layout_options(target: TargetId) -> SemanticOptions {
    SemanticOptions {
        embedded_record_arrays: true,
        ..SemanticOptions::modern().with_target(target)
    }
}

fn analyze_layout(source: &str, target: TargetId) -> Result<SemanticModel, Vec<Diagnostic>> {
    let program = parse(&tokenize(source).unwrap()).unwrap();
    analyze_with_options(&program, layout_options(target))
}

#[test]
fn embedded_record_arrays_preserve_element_shape_and_full_extent() {
    let model = analyze_layout(
        "CONST Count=100 TYPE Buffers=[INT ARRAY x(Count),y(Count)] Buffers ARRAY rows(3)",
        TargetId::Atari6502,
    )
    .unwrap();
    let layout = model.layout.record_for_name("Buffers").unwrap();
    assert_eq!(
        (layout.size, layout.alignment, layout.tail_padding),
        (400, 1, 0)
    );
    for (index, field) in layout.fields.iter().enumerate() {
        assert_eq!(
            (field.offset, field.size, field.alignment),
            (index as u16 * 200, 200, 1)
        );
        assert_eq!(field.ty, ValueType::fund(FundType::Int));
        assert_eq!(
            field.storage,
            RecordFieldStorage::InlineArray {
                array_type: ArrayType::new(ValueType::fund(FundType::Int), Some(100)),
                stride: 2,
            }
        );
        assert_eq!(layout.record_type.fields[index].storage, field.storage);
        let source = &model.fields[field.id.0];
        assert_eq!(source.owner, layout.owner);
        assert_eq!(
            (source.offset, source.size, source.alignment),
            (field.offset, field.size, field.alignment)
        );
        assert_eq!(source.storage, field.storage);
    }
    let rows = model
        .symbols
        .lookup(model.symbols.global_scope(), "rows")
        .unwrap();
    let array = model.layout.array_for_symbol(rows).unwrap();
    assert_eq!(
        (array.element_size, array.stride, array.storage_size),
        (400, 400, Some(1200))
    );
}

#[test]
fn embedded_record_arrays_use_target_alignment_and_nested_record_stride() {
    let source = "TYPE Point=[BYTE tag CARD word] \
                  TYPE Packet=[BYTE lead Point ARRAY points(2) INT ARRAY words(3) BYTE tail] \
                  Packet ARRAY rows(2)";
    for (target, point_size, packet_size, points_offset, words_offset, tail_offset, padding) in [
        (TargetId::Atari6502, 3, 14, 1, 7, 13, 0),
        (TargetId::Wdc65816Small, 4, 18, 2, 10, 16, 1),
        (TargetId::Wdc65816Native, 4, 18, 2, 10, 16, 1),
        (TargetId::Motorola68000, 4, 18, 2, 10, 16, 1),
    ] {
        let model = analyze_layout(source, target).unwrap();
        let point = model.layout.record_for_name("Point").unwrap();
        assert_eq!(point.size, point_size, "{target:?}");
        let packet = model.layout.record_for_name("Packet").unwrap();
        assert_eq!(
            (packet.size, packet.tail_padding),
            (packet_size, padding),
            "{target:?}"
        );
        assert_eq!(
            packet
                .fields
                .iter()
                .map(|field| field.offset)
                .collect::<Vec<_>>(),
            [0, points_offset, words_offset, tail_offset],
            "{target:?}"
        );
        assert_eq!(packet.fields[1].size, 2 * point_size);
        assert_eq!(
            packet.fields[1].storage,
            RecordFieldStorage::InlineArray {
                array_type: ArrayType::new(ValueType::record("Point"), Some(2)),
                stride: point_size,
            }
        );
        let rows = model
            .symbols
            .lookup(model.symbols.global_scope(), "rows")
            .unwrap();
        let array = model.layout.array_for_symbol(rows).unwrap();
        assert_eq!(
            (array.stride, array.storage_size),
            (packet_size, Some(u32::from(packet_size) * 2))
        );
    }
}

#[test]
fn embedded_record_arrays_cover_scalar_widths_and_page_sized_extents() {
    for (element, width) in [
        ("BYTE", 1u16),
        ("CHAR", 1),
        ("INT", 2),
        ("CARD", 2),
        ("REAL", 6),
    ] {
        for length in [1u16, 2, 100, 127, 128, 129, 255, 256, 257] {
            let source =
                format!("TYPE Buffer=[BYTE prefix {element} ARRAY values({length}) BYTE suffix]");
            let model = analyze_layout(&source, TargetId::Atari6502).unwrap();
            let record = model.layout.record_for_name("Buffer").unwrap();
            assert_eq!(
                (record.fields[1].offset, record.fields[1].size),
                (1, length * width)
            );
            assert_eq!(record.fields[2].offset, 1 + length * width);
            assert_eq!(record.size, 2 + length * width);
        }
    }
}

#[test]
fn embedded_record_arrays_resolve_local_constants_without_losing_field_identity() {
    let model = analyze_layout(
        "CONST Count=100 TYPE Buffer=[INT ARRAY values(Count)] \
         PROC Main() CONST Count=3 TYPE Local=[BYTE tag INT ARRAY values(Count+1)] Local row RETURN",
        TargetId::Atari6502,
    ).unwrap();
    let global = model.layout.record_for_name("Buffer").unwrap();
    let scope = model.routine_scopes[0].scope;
    let local_id = model.symbols.lookup(scope, "Local").unwrap();
    let local = model.layout.record_for_owner(local_id).unwrap();
    assert_eq!(global.size, 200);
    assert_eq!(local.size, 9);
    assert_ne!(global.fields[0].id, local.fields[1].id);
    assert_eq!(
        local.fields[1].storage,
        RecordFieldStorage::InlineArray {
            array_type: ArrayType::new(ValueType::fund(FundType::Int), Some(4)),
            stride: 2,
        }
    );
}

#[test]
fn embedded_record_arrays_reject_invalid_bounds_and_incomplete_layouts() {
    for (source, message) in [
        ("TYPE Bad=[BYTE ARRAY values]", "explicit constant bound"),
        ("TYPE Bad=[BYTE ARRAY values(0)]", "positive constant"),
        ("TYPE Bad=[BYTE ARRAY values(-1)]", "positive constant"),
        (
            "TYPE Bad=[BYTE ARRAY values(INT($8000))]",
            "positive constant",
        ),
        (
            "CARD count TYPE Bad=[BYTE ARRAY values(count)]",
            "scalar constant",
        ),
        (
            "TYPE Bad=[BYTE ARRAY values(Count)] CONST Count=4",
            "undefined symbol",
        ),
        ("TYPE Bad=[BYTE ARRAY values(1/0)]", "scalar constant"),
        ("TYPE Bad=[BYTE ARRAY values(1.5)]", "scalar constant"),
        (
            "TYPE Bad=[BYTE ARRAY values(4)=[1 2 3 4]]",
            "record fields must be fundamental variables",
        ),
        ("TYPE Bad=[Bad ARRAY values(2)]", "complete, non-recursive"),
        ("TYPE Bad=[Bad value]", "complete, non-recursive"),
    ] {
        let errors = analyze_layout(source, TargetId::Atari6502).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(message)),
            "{source}: {errors:?}"
        );
    }
    // Pointer recursion has a complete machine representation and is legal.
    let model = analyze_layout(
        "TYPE Node=[Node POINTER next BYTE ARRAY data(4)]",
        TargetId::Atari6502,
    )
    .unwrap();
    assert_eq!(model.layout.record_for_name("Node").unwrap().size, 6);
}

#[test]
fn embedded_record_array_extent_and_record_tail_padding_never_wrap() {
    for (source, message) in [
        (
            "TYPE Bad=[INT ARRAY values(CARD(32768))]",
            "embedded array storage extent",
        ),
        (
            "TYPE Bad=[BYTE ARRAY values(CARD(65535)) BYTE tail]",
            "record storage extent",
        ),
        (
            "TYPE Inner=[BYTE ARRAY values(CARD(40000))] TYPE Bad=[Inner ARRAY values(2)]",
            "embedded array storage extent",
        ),
    ] {
        let errors = analyze_layout(source, TargetId::Atari6502).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(message)),
            "{source}: {errors:?}"
        );
    }
    let source = "TYPE Limit=[CARD head BYTE ARRAY values(CARD(65533))]";
    let packed = analyze_layout(source, TargetId::Atari6502).unwrap();
    assert_eq!(packed.layout.record_for_name("Limit").unwrap().size, 65535);
    let errors = analyze_layout(source, TargetId::Motorola68000).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("aligned record storage extent"))
    );
}

#[test]
fn embedded_record_arrays_preserve_module_owned_types_and_bounds() {
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), b"MODULE App USE Data Data.Buffers rows PROC Main() RETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/data.act"),
            b"MODULE Data USE Limits PUBLIC TYPE Buffers=[INT ARRAY x(Limits.Count),y(Limits.Count)] ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/limits.act"),
            b"MODULE Limits PUBLIC CONST Count=100 ENDMODULE".to_vec());
    let compilation =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model = analyze_compilation_with_options(&compilation, layout_options(TargetId::Atari6502))
        .unwrap();
    let module = model
        .modules
        .iter()
        .find(|module| module.path.display_name() == "Data")
        .unwrap();
    let owner = module.public_symbol("Buffers").unwrap();
    let layout = model.layout.record_for_owner(owner).unwrap();
    assert_eq!(layout.size, 400);
    assert!(
        layout
            .fields
            .iter()
            .all(|field| model.fields[field.id.0].owner == owner)
    );
}

#[test]
fn embedded_record_arrays_require_the_modern_semantic_profile() {
    let program =
        parse(&tokenize("TYPE Buffer=[INT ARRAY values(100)] PROC Main() RETURN").unwrap())
            .unwrap();
    for options in [SemanticOptions::default(), SemanticOptions::modern()] {
        if options.embedded_record_arrays {
            analyze_with_options(&program, options).unwrap();
            continue;
        }
        let errors = analyze_with_options(&program, options).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("record fields must be fundamental variables")
        }));
    }
}
