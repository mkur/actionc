//! Byte-image and source-time oracles for bounded NIR snapshot forwarding.
#[path = "support/variant_values.rs"]
mod support;

fn check(source: &str, values: &[(usize, u8)]) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = actionc::semantic::ir::lower_program(&ast, &model);
    let mut expected = vec![0xCC; 0x500];
    for &(offset, value) in values {
        expected[offset] = value;
    }
    support::check_semir(&semir, &expected, false);
}

#[test]
fn record_and_union_inline_array_captures_preserve_the_complete_image() {
    for declaration in [
        "TYPE Value=[BYTE ARRAY bytes(3)]",
        "TYPE Value=UNION [CARD word BYTE ARRAY bytes(3)]",
    ] {
        check(
            &format!(
                "{declaration} Value original BYTE ARRAY output=$600 \
            PROC Main() BYTE a,b,c original.bytes(0)=42 original.bytes(1)=255 original.bytes(2)=$A5 \
            LET first=original LET saved=first \
            a=saved.bytes(0) b=saved.bytes(1) c=saved.bytes(2) \
            original.bytes(2)=9 output(0)=a output(1)=b output(2)=c output(3)=original.bytes(2) DO OD RETURN"
            ),
            &[(0, 42), (1, 255), (2, 0xA5), (3, 9)],
        );
    }
}

#[test]
fn later_reads_keep_the_snapshot_across_mutation_calls_and_unknown_writes() {
    for change in ["original.second=99", "Mutate()", "p=@original.second p^=99"] {
        check(
            &format!(
                "TYPE Value=[BYTE first,second] Value original BYTE POINTER p BYTE ARRAY output=$600 \
            PROC Mutate() original.second=99 RETURN \
            PROC Main() BYTE a,b original.first=42 original.second=43 LET saved=original \
            a=saved.first {change} b=saved.second output(0)=a output(1)=b output(2)=original.second DO OD RETURN"
            ),
            &[(0, 42), (1, 43), (2, 99)],
        );
    }
}

#[test]
fn repeated_initialization_uses_this_iteration_and_preserves_untaken_paths() {
    check(
        "TYPE Value=[BYTE first,second] Value original BYTE ARRAY output=$600 \
        PROC Main() BYTE i,a original.second=100 \
        FOR i=0 TO 3 DO\nBEGIN\noriginal.first=i LET saved=original a=saved.first+saved.second output(i)=a\nEND\nOD\n\
        IF i=0 THEN\nBEGIN\nLET skipped=original output(4)=skipped.first\nEND\nFI\nDO OD RETURN",
        &[(0, 100), (1, 101), (2, 102), (3, 103)],
    );
}

#[test]
fn variant_guard_mutation_keeps_the_original_case_image() {
    check(
        "TYPE Value=VARIANT [NONE SOME [BYTE first,second]] Value original BYTE ARRAY output=$600 \
        BYTE FUNC Mutate() original=Value.SOME(99,100) RETURN(0) \
        PROC Main() original=Value.SOME(42,43) LET saved=original\n\
        CASE saved OF\nWHEN Value.SOME(a,b) IF Mutate() THEN\noutput(0)=255\n\
        WHEN Value.SOME(a,b) THEN\noutput(0)=a output(1)=b\nELSE\noutput(0)=254\nESAC\n\
        CASE original OF\nWHEN Value.SOME(a,b) THEN\noutput(2)=a output(3)=b\nELSE\noutput(2)=253\nESAC\n\
        DO OD RETURN",
        &[(0, 42), (1, 43), (2, 99), (3, 100)],
    );
}
