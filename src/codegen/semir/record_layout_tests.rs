//! Canonical classic-layout projection; experimental arrays remain publicly gated.
use super::*;
use crate::codegen::{CodegenOutput, CodegenProfile};
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::semantic::{self, SemanticOptions};

fn lower(source: &str) -> SemProgram {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        SemanticOptions {
            embedded_record_arrays: true,
            ..SemanticOptions::modern()
        },
    )
    .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    semantic::ir::lower_program(&ast, &model)
}

fn generate(program: &SemProgram, runtime: Runtime) -> CodegenOutput {
    match runtime {
        Runtime::ActionCart => crate::codegen::generate_semir_profile_at_origin(
            program,
            0x3000,
            CodegenProfile::Modern,
        ),
        Runtime::Standalone => crate::codegen::generate_semir_standalone_profile_at_origin(
            program,
            0x3000,
            CodegenProfile::Modern,
        ),
    }
    .unwrap_or_else(|errors| panic!("{runtime:?}: {errors:#?}"))
}

#[test]
fn canonical_classic_layout_keeps_inline_extents_offsets_and_nested_record_ids() {
    let semir = lower(
        "CONST Count=100 TYPE Pair=[BYTE tag CARD value] \
        TYPE Buffer=[INT ARRAY x(Count),y(Count) Pair ARRAY pairs(2)] \
        TYPE Envelope=[BYTE lead Buffer payload BYTE tail] \
        Envelope data PROC Main() RETURN",
    );
    let records = semir_to_projection(&semir).unwrap().record_layouts;
    let (pair_id, pair) = records.get("Pair").unwrap();
    assert_eq!(pair.size, 3);
    let (buffer_id, buffer) = records.get("Buffer").unwrap();
    assert_eq!(buffer.size, 406);
    assert_eq!(
        (buffer.fields["Y"].offset, buffer.fields["Y"].size),
        (200, 200)
    );
    assert_eq!(
        buffer.fields["Y"].array,
        Some(super::super::storage::RecordArrayField {
            length: 100,
            stride: 2,
        })
    );
    assert_eq!(buffer.fields["PAIRS"].record, Some(pair_id));
    assert_eq!(buffer.fields["PAIRS"].array.unwrap().stride, 3);
    let (_, envelope) = records.get("Envelope").unwrap();
    assert_eq!(envelope.size, 408);
    assert_eq!(envelope.fields["PAYLOAD"].record, Some(buffer_id));
    assert_eq!(envelope.fields["TAIL"].offset, 407);
}

#[test]
fn canonical_classic_layout_controls_allocation_without_recomputing_ast_bounds() {
    let semir = lower(
        "TYPE Buffer=[BYTE lead INT ARRAY x(100),y(100) BYTE tail] \
        Buffer data BYTE sentinel PROC Main() data.lead=11 data.tail=42 sentinel=99 RETURN",
    );
    let mut stale_syntax = semir.clone();
    for item in stale_syntax
        .modules
        .iter_mut()
        .flat_map(|module| &mut module.items)
    {
        if let SemItem::Declaration(SemDeclaration {
            storage: SemDeclarationStorage::Type { fields, .. },
            ..
        }) = item
        {
            for field in fields {
                if let SemDeclarationStorage::Array { length, .. } = &mut field.storage {
                    // Deliberately remove projected bound syntax. Canonical
                    // RecordType, not this AST-facing scaffolding, owns layout.
                    *length = None;
                }
            }
        }
    }
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let expected = generate(&semir, runtime);
        let actual = generate(&stale_syntax, runtime);
        assert_eq!(actual.bytes, expected.bytes, "{runtime:?}");
        let data = actual
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name.eq_ignore_ascii_case("data"))
            .unwrap();
        let sentinel = actual
            .map
            .storage_symbols
            .iter()
            .find(|symbol| symbol.name.eq_ignore_ascii_case("sentinel"))
            .unwrap();
        assert_eq!(data.size, 402);
        assert_eq!(sentinel.address, data.address + 402);
    }
}

#[test]
fn canonical_classic_layout_merging_rebases_nested_record_references() {
    let app = lower("TYPE AppInner=[BYTE tag] TYPE AppOuter=[AppInner inner] PROC Main() RETURN");
    let resident = lower(
        "TYPE RuntimeInner=[CARD word] \
        TYPE RuntimeOuter=[RuntimeInner ARRAY items(3)] PROC Helper() RETURN",
    );
    let mut records = semir_to_projection(&app).unwrap().record_layouts;
    records.extend(semir_to_projection(&resident).unwrap().record_layouts);
    let runtime_inner = records.get("RuntimeInner").unwrap().0;
    let runtime_outer = records.get("RuntimeOuter").unwrap().1;
    assert_eq!(runtime_inner, 2);
    assert_eq!(runtime_outer.fields["ITEMS"].record, Some(runtime_inner));
    assert_eq!(runtime_outer.fields["ITEMS"].size, 6);
    assert_eq!(
        records.get("AppOuter").unwrap().1.fields["INNER"].record,
        Some(0)
    );
}

#[test]
fn canonical_classic_layout_rejects_inconsistent_field_shapes() {
    let mut semir = lower("TYPE Buffer=[INT ARRAY x(100)] Buffer data PROC Main() RETURN");
    let SemItem::Declaration(SemDeclaration {
        storage: SemDeclarationStorage::Type { record_type, .. },
        ..
    }) = &mut semir.modules[0].items[0]
    else {
        panic!("type");
    };
    record_type.size = 199;
    let errors = semir_to_projection(&semir)
        .err()
        .expect("truncated record must fail projection");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("canonical layout"))
    );
}

#[test]
fn canonical_classic_layout_does_not_enable_unsupported_pointer_fields() {
    let semir = lower(
        "TYPE Cell=[BYTE value] TYPE Link=[Cell POINTER next] \
        Link item PROC Main() item.next.value=1 RETURN",
    );
    let errors = semir_to_projection(&semir)
        .err()
        .expect("pointer field needs a typed carrier");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("pointer-valued record fields"))
    );
}

#[test]
fn canonical_classic_layout_preserves_module_owned_names_and_bounds() {
    use crate::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use crate::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(
            root.clone(),
            b"MODULE App USE Small USE Large \
            Small.Buffer a Large.Buffer b PROC Main() a.tail=1 b.tail=2 RETURN ENDMODULE"
                .to_vec(),
        )
        .with_source(
            SourceOrigin::host("project/small.act"),
            b"MODULE Small PUBLIC CONST Count=2 \
            PUBLIC TYPE Buffer=[INT ARRAY values(Count) BYTE tail] ENDMODULE"
                .to_vec(),
        )
        .with_source(
            SourceOrigin::host("project/large.act"),
            b"MODULE Large PUBLIC CONST Count=100 \
            PUBLIC TYPE Buffer=[INT ARRAY values(Count) BYTE tail] ENDMODULE"
                .to_vec(),
        );
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model = semantic::analyze_compilation_with_options(
        &loaded,
        SemanticOptions {
            embedded_record_arrays: true,
            ..SemanticOptions::modern()
        },
    )
    .unwrap();
    let semir = semantic::ir::lower_compilation(&loaded, &model);
    let projection = semir_to_projection(&semir).unwrap();
    let mut sizes = projection
        .record_layouts
        .layouts
        .iter()
        .map(|record| record.size)
        .collect::<Vec<_>>();
    sizes.sort();
    assert_eq!(sizes, [5, 201]);
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        generate(&semir, runtime);
    }
}
