#[path = "support/variant_values.rs"]
mod support;

fn check(source: &str, prefix: &[u8], fault: bool) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = actionc::semantic::ir::lower_program(&ast, &model);
    let mut expected = vec![0xCC; 0x500];
    expected[..prefix.len()].copy_from_slice(prefix);
    support::check_semir(&semir, &expected, fault);
}

#[test]
fn final_unmatched_path_catches_invalid_tags_without_exposing_payloads() {
    for tag in [0, 1, 2, 3, 255] {
        let valid = matches!(tag, 1 | 2);
        let source = format!(
            "TYPE Value=VARIANT [NONE SOME [BYTE value]] Value current \
            BYTE POINTER raw BYTE ARRAY output=$600 PROC Main() raw=BYTE POINTER(@current) \
            raw(0)={tag} raw(1)=42 output(0)=41\nCASE current OF\n\
            WHEN Value.NONE THEN\noutput(1)=10\nWHEN Value.SOME(n) THEN\noutput(1)=n\n\
            ESAC\noutput(0)=42 DO OD RETURN"
        );
        if valid {
            check(&source, &[42, if tag == 1 { 10 } else { 42 }], false);
        } else {
            check(&source, &[41], true);
        }
    }
}

#[test]
fn invalid_tags_skip_else_and_both_constructor_and_wildcard_guard_effects() {
    for tag in [0, 1, 2, 3, 255] {
        let source = format!(
            "TYPE Value=VARIANT [NONE SOME [BYTE value]] Value current \
            BYTE POINTER raw BYTE ARRAY output=$600 BYTE FUNC Reject() output(2)=77 RETURN(0) \
            PROC Main() raw=BYTE POINTER(@current) raw(0)={tag} raw(1)=42 output(0)=41\n\
            CASE current OF\nWHEN Value.SOME(n) IF Reject() THEN\noutput(1)=n\n\
            WHEN _ IF Reject() THEN\noutput(1)=11\nELSE\noutput(1)=12\nESAC\n\
            output(0)=42 DO OD RETURN"
        );
        if matches!(tag, 1 | 2) {
            check(&source, &[42, 12, 77], false);
        } else {
            check(&source, &[41], true);
        }
    }
}

#[test]
fn invalid_nested_payloads_still_fault_before_any_guard_or_user_else() {
    for (outer, inner) in [(2, 0), (2, 255), (1, 255)] {
        let source = format!(
            "TYPE Inner=VARIANT [OFF ON] TYPE Value=VARIANT [NONE SOME [Inner value]] \
            Value current BYTE POINTER raw BYTE ARRAY output=$600 \
            BYTE FUNC Touch() output(2)=77 RETURN(1) PROC Main() raw=BYTE POINTER(@current) \
            raw(0)={outer} raw(1)={inner} output(0)=41\nCASE current OF\n\
            WHEN Value.SOME(_) IF Touch() THEN\noutput(1)=1\nELSE\noutput(1)=2\nESAC\n\
            output(0)=42 DO OD RETURN"
        );
        if outer == 1 {
            check(&source, &[42, 2], false);
        } else {
            check(&source, &[41], true);
        }
    }
}
