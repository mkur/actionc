use actionc::ast::{Decl, ExprKind, Item};
use actionc::codegen::{
    CodegenOutput, CodegenProfile, CodegenSymbolScope, generate_profile_with_origin,
    generate_semir_profile_with_origin,
};
use actionc::compiler::{CompileMode, CompileOptions, compile_file};
use actionc::lexer::tokenize;
use actionc::mir6502::{self, Mir6502Config};
use actionc::nir;
use actionc::parser::parse;
use actionc::semantic::ir::{self, SemItem, SemStaticInitializerValue};
use actionc::semantic::{SemanticModel, analyze};

const ORIGIN: u16 = 0x3000;

fn parse_and_analyze(source: &str) -> (actionc::ast::Program, SemanticModel) {
    let program = parse(&tokenize(source).expect("tokenize aggregate initializer fixture"))
        .expect("parse aggregate initializer fixture");
    let model = analyze(&program).expect("analyze aggregate initializer fixture");
    (program, model)
}

fn classic_output(source: &str) -> CodegenOutput {
    let (program, _) = parse_and_analyze(source);
    generate_profile_with_origin(&program, ORIGIN, CodegenProfile::Compat)
        .expect("generate aggregate initializer characterization fixture")
}

fn global_address(output: &CodegenOutput, name: &str) -> u16 {
    output
        .map
        .storage_symbols
        .iter()
        .find(|symbol| {
            symbol.scope == CodegenSymbolScope::Global && symbol.name.eq_ignore_ascii_case(name)
        })
        .unwrap_or_else(|| panic!("missing global storage symbol `{name}`"))
        .address
}

fn output_bytes(output: &CodegenOutput, address: u16, len: usize) -> &[u8] {
    let offset = usize::from(address.wrapping_sub(output.origin));
    &output.bytes[offset..offset + len]
}

fn record_array_backing<'a>(
    output: &'a CodegenOutput,
    name: &str,
    byte_len: usize,
) -> &'a [u8] {
    let descriptor = global_address(output, name);
    let pointer = u16::from_le_bytes(
        output_bytes(output, descriptor, 2)
            .try_into()
            .expect("record-array descriptor pointer"),
    );
    output_bytes(output, pointer, byte_len)
}

#[test]
fn flat_initializer_list_preserves_one_element_per_source_value() {
    let (program, _) = parse_and_analyze(
        "BYTE ARRAY data(6)=[1 -2 'A TRUE FALSE NIL] PROC Main() RETURN",
    );
    let Item::Declaration(Decl::Var(declaration)) = &program.modules[0].items[0] else {
        panic!("expected array declaration");
    };
    let ExprKind::InitializerList(elements) =
        &declaration.entries[0].initializer.as_ref().unwrap().kind
    else {
        panic!("expected structured initializer list");
    };

    assert_eq!(elements.len(), 6);
}

#[test]
fn semantic_record_layout_is_packed_in_recursive_declaration_order() {
    let (_, model) = parse_and_analyze(
        "TYPE Inner=[BYTE flag CARD address] \
         TYPE Outer=[BYTE tag Inner value BYTE tail] \
         Outer ARRAY table(2) PROC Main() RETURN",
    );
    let inner = model
        .layout
        .records
        .iter()
        .find(|record| record.name.eq_ignore_ascii_case("Inner"))
        .expect("Inner semantic layout");
    let outer = model
        .layout
        .records
        .iter()
        .find(|record| record.name.eq_ignore_ascii_case("Outer"))
        .expect("Outer semantic layout");

    assert_eq!(inner.size, 3);
    assert_eq!(
        inner
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.offset))
            .collect::<Vec<_>>(),
        [("flag", 0), ("address", 1)]
    );
    assert_eq!(outer.size, 5);
    assert_eq!(
        outer
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.offset))
            .collect::<Vec<_>>(),
        [("tag", 0), ("value", 1), ("tail", 4)]
    );
}

#[test]
fn sized_byte_array_zero_fills_after_the_initialized_extent() {
    let output = classic_output("BYTE ARRAY values(4)=[1 2] PROC Main() RETURN");
    let address = global_address(&output, "values");

    assert_eq!(output_bytes(&output, address, 4), [1, 2, 0, 0]);
}

#[test]
fn initializer_longer_than_declared_byte_array_currently_extends_storage() {
    let output = classic_output(
        "BYTE ARRAY values(2)=[1 2 3] BYTE ARRAY sentinel(1)=[$AA] PROC Main() RETURN",
    );
    let values = global_address(&output, "values");
    let sentinel = global_address(&output, "sentinel");

    assert_eq!(output_bytes(&output, values, 3), [1, 2, 3]);
    assert_eq!(sentinel, values + 3);
}

#[test]
fn sized_card_array_zero_fills_its_descriptor_backing() {
    let output = classic_output("CARD ARRAY values(3)=[1] PROC Main() RETURN");
    let descriptor = global_address(&output, "values");
    let pointer = u16::from_le_bytes(
        output_bytes(&output, descriptor, 2)
            .try_into()
            .expect("descriptor pointer bytes"),
    );

    assert_eq!(output_bytes(&output, pointer, 6), [1, 0, 0, 0, 0, 0]);
}

#[test]
fn semir_plans_mixed_width_record_array_writes_at_leaf_offsets() {
    let (program, model) = parse_and_analyze(
        "TYPE Pair=[BYTE tag CARD word] \
         Pair ARRAY pairs(2)=[1 $2345 2 $6789] PROC Main() RETURN",
    );
    let semir = ir::lower_program(&program, &model);
    let declaration = semir.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            SemItem::Declaration(declaration)
                if declaration.symbol.name.eq_ignore_ascii_case("pairs") =>
            {
                Some(declaration)
            }
            _ => None,
        })
        .expect("pairs SemIR declaration");
    let plan = declaration
        .static_initializer
        .as_ref()
        .expect("layout-resolved static initializer");

    assert_eq!(plan.initialized_extent, 6);
    assert_eq!(
        plan.writes
            .iter()
            .map(|write| (write.offset, write.width, write.display_path.as_str()))
            .collect::<Vec<_>>(),
        [
            (0, 1, "pairs(0).tag"),
            (1, 2, "pairs(0).word"),
            (3, 1, "pairs(1).tag"),
            (4, 2, "pairs(1).word"),
        ]
    );
}

#[test]
fn semir_rounds_partial_inferred_record_array_to_a_complete_element() {
    let (program, model) = parse_and_analyze(
        "TYPE Pair=[BYTE tag CARD word] \
         Pair ARRAY pairs=[1 $2345 2] PROC Main() RETURN",
    );
    let semir = ir::lower_program(&program, &model);
    let plan = semir.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            SemItem::Declaration(declaration)
                if declaration.symbol.name.eq_ignore_ascii_case("pairs") =>
            {
                declaration.static_initializer.as_ref()
            }
            _ => None,
        })
        .expect("pairs static initializer plan");

    assert_eq!(plan.initialized_extent, 6);
    assert_eq!(
        plan.writes.iter().map(|write| write.offset).collect::<Vec<_>>(),
        [0, 1, 3]
    );
}

#[test]
fn semantic_address_width_checks_use_the_record_leaf_destination() {
    let (program, model) = parse_and_analyze(
        "TYPE Pair=[BYTE low CARD word] BYTE target \
         Pair ARRAY pairs(1)=[<target @target] PROC Main() RETURN",
    );
    let semir = ir::lower_program(&program, &model);
    let plan = semir.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            SemItem::Declaration(declaration)
                if declaration.symbol.name.eq_ignore_ascii_case("pairs") =>
            {
                declaration.static_initializer.as_ref()
            }
            _ => None,
        })
        .expect("pairs relocation plan");

    assert!(matches!(
        plan.writes[0].value,
        SemStaticInitializerValue::Address {
            selector: Some(_),
            ..
        }
    ));
    assert!(matches!(
        plan.writes[1].value,
        SemStaticInitializerValue::Address { selector: None, .. }
    ));
    assert_eq!(
        plan.writes.iter().map(|write| write.width).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn global_mixed_width_record_array_has_identical_backing_in_all_backends() {
    let source = "TYPE Pair=[BYTE tag CARD word] \
                  Pair ARRAY pairs(2)=[1 $2345 2 $6789] \
                  PROC Main() RETURN";
    let (program, model) = parse_and_analyze(source);
    let semir = ir::lower_program(&program, &model);
    let compatibility =
        generate_semir_profile_with_origin(&semir, ORIGIN, CodegenProfile::Compat)
            .expect("compatibility/classic aggregate initializer");
    let modern = generate_semir_profile_with_origin(&semir, ORIGIN, CodegenProfile::Modern)
        .expect("modern/classic aggregate initializer");
    let nir = nir::lower_program(&semir);
    let nir = nir::optimize_program(&nir).expect("verify aggregate initializer NIR");
    let mir6502 = mir6502::generate_output_with_config(
        &nir,
        ORIGIN,
        &Mir6502Config::optimized(),
    )
    .expect("MIR6502 aggregate initializer");

    for (backend, output) in [
        ("compatibility/classic", compatibility),
        ("modern/classic", modern),
        ("MIR6502", mir6502),
    ] {
        assert_eq!(
            record_array_backing(&output, "pairs", 6),
            [1, 0x45, 0x23, 2, 0x89, 0x67],
            "{backend} record backing"
        );
    }
}

#[test]
fn compiler_api_accepts_global_record_arrays_in_every_mode() {
    let source = "TYPE Pair=[BYTE tag CARD word] \
                  Pair ARRAY pairs(2)=[1 $2345 2 $6789] \
                  PROC Main() RETURN";
    let path = std::env::temp_dir().join(format!(
        "actionc-aggregate-initializer-{}.act",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write aggregate initializer compiler fixture");

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        compile_file(&path, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile aggregate initializer in {mode:?}: {error}"));
    }

    std::fs::remove_file(path).expect("remove aggregate initializer compiler fixture");
}
