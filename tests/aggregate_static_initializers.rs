use actionc::ast::{Decl, ExprKind, Item};
use actionc::codegen::{
    CodegenOutput, CodegenProfile, CodegenSymbolScope, generate_profile_with_origin,
};
use actionc::lexer::tokenize;
use actionc::parser::parse;
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
