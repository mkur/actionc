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
fn branch_merges_and_nested_union_tails_keep_values_on_both_paths() {
    for flag in [0, 1] {
        for mutate in [false, true] {
            let write = if mutate {
                "original.child.bytes(1)=99"
            } else {
                "original.tail=99"
            };
            check(
                &format!(
                    "TYPE Value=UNION [CARD word BYTE ARRAY bytes(3)] TYPE Box=[Value child BYTE tail] Box original BYTE ARRAY output=$600 BYTE flag \
                PROC Main() BYTE a,b,c flag={flag} original.child.bytes(0)=42 original.child.bytes(1)=43 original.child.bytes(2)=$A5 \
                LET saved=original.child IF flag THEN {write} FI a=saved.bytes(0) b=saved.bytes(1) c=saved.bytes(2) output(0)=a output(1)=b output(2)=c DO OD RETURN"
                ),
                &[42, 43, 0xA5],
            );
        }
    }
}

#[test]
fn nested_variant_binders_and_mutating_guards_preserve_snapshots() {
    check(
        "TYPE Payload=[BYTE value] TYPE Event=VARIANT [NONE DATA [Payload item]] Payload value Event original BYTE ARRAY output=$600 \
        BYTE FUNC Reject() value.value=99 original=Event.DATA(value) RETURN(0) \
        PROC Main() value.value=42 original=Event.DATA(value)\n\
        CASE original OF\nWHEN Event.DATA(saved) IF Reject() THEN\noutput(0)=255\n\
        WHEN Event.DATA(saved) THEN\noutput(0)=saved.value\nELSE\noutput(0)=254\nESAC\n\
        CASE original OF\nWHEN Event.DATA(saved) THEN\noutput(1)=saved.value\nELSE\noutput(1)=253\nESAC\nDO OD RETURN",
        &[42, 99],
    );
}
