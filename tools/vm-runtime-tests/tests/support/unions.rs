#[path = "variant_values.rs"]
mod values;

pub fn check(source: &str, expected: &[u8]) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let mut options = actionc::semantic::SemanticOptions::modern();
    options.algebraic_types.unions = true;
    let model = actionc::semantic::analyze_with_options(&ast, options)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:?}"));
    values::check_semir(
        &actionc::semantic::ir::lower_program(&ast, &model),
        expected,
        false,
    );
}
