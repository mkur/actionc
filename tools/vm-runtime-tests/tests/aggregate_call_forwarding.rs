#[path = "support/variant_values.rs"]
mod support;

fn check(source: &str, bytes: &[u8]) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = actionc::semantic::ir::lower_program(&ast, &model);
    let mut expected = vec![0xCC; 0x500];
    expected[..bytes.len()].copy_from_slice(bytes);
    support::check_semir(&semir, &expected, false);
}

#[test]
fn reused_arguments_keep_two_mutable_parameters_independent_and_full_union_tails() {
    for declaration in [
        "TYPE Value=[BYTE ARRAY bytes(3)]",
        "TYPE Value=UNION [CARD word BYTE ARRAY bytes(3)]",
    ] {
        check(
            &format!(
                "{declaration} Value original BYTE ARRAY output=$600 \
            PROC Take(Value left,right) left.bytes(0)=99 \
            output(0)=left.bytes(0) output(1)=right.bytes(0) output(2)=right.bytes(2) RETURN \
            PROC Main() original.bytes(0)=42 original.bytes(1)=43 original.bytes(2)=$A5 \
            LET saved=original Take(saved,saved) output(3)=saved.bytes(0) DO OD RETURN"
            ),
            &[99, 42, 0xA5, 42],
        );
    }
}

#[test]
fn call_order_and_indirect_callee_evaluation_retain_snapshots() {
    check(
        "TYPE Value=[BYTE first] Value original BYTE ARRAY output=$600 \
        PROC POINTER callback(Value input BYTE n) \
        PROC Take(Value input BYTE n) input.first==+1 output(0)=input.first RETURN \
        PROC Wrong(Value input BYTE n) output(0)=255 RETURN \
        BYTE FUNC Mutate() original.first=99 callback=Wrong RETURN(0) \
        PROC Main() original.first=42 callback=Take LET saved=original \
        callback(saved,Mutate()) output(1)=saved.first output(2)=original.first DO OD RETURN",
        &[43, 42, 99],
    );
}

#[test]
fn variant_argument_and_return_images_remain_independent() {
    check(
        "TYPE Value=VARIANT [NONE SOME [BYTE first]] BYTE ARRAY output=$600 \
        Value FUNC Make() LET saved=Value.SOME(42) RETURN(saved) \
        PROC Take(Value left,right) left=Value.NONE\n\
        CASE right OF\nWHEN Value.SOME(n) THEN\noutput(0)=n\nELSE\noutput(0)=255\nESAC\nRETURN \
        PROC Main() LET saved=Make() Take(saved,saved)\n\
        CASE saved OF\nWHEN Value.SOME(n) THEN\noutput(1)=n\nELSE\noutput(1)=254\nESAC\nDO OD RETURN",
        &[42, 42],
    );
}
