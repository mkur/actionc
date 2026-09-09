#[path = "support/unions.rs"]
mod unions;
#[path = "support/variant_values.rs"]
mod values;

#[test]
fn mir_wide_views_reinterpret_signed_bits_and_preserve_other_bytes() {
    let source = r#"
TYPE View=UNION [LONGCARD wide LONGINT signedValue CARD word BYTE ARRAY bytes(5)]
View value=$680
BYTE ARRAY output=$600
PROC Main()
 value.bytes(4)=$A5 value.wide=$89ABCDEF
 output(0)=value.bytes(0) output(1)=value.bytes(1)
 output(2)=value.bytes(2) output(3)=value.bytes(3)
 output(4)=value.signedValue<0
 value.word=$1234
 value.bytes(3)=$7F
 output(5)=value.signedValue<0
 value.wide==+1
 output(6)=value.bytes(0) output(7)=value.bytes(4)
 value.word=$FFFF value.word==+1
 output(10)=value.bytes(0) output(11)=value.bytes(1)
 output(12)=value.bytes(2) output(13)=value.bytes(3)
 value.signedValue=-1
 value.bytes(0)=0
 output(8)=value.signedValue=-256
 output(9)=value.wide=$FFFFFF00
 DO OD
RETURN
"#;
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let mut options = actionc::semantic::SemanticOptions::modern();
    options.algebraic_types.unions = true;
    let model = actionc::semantic::analyze_with_options(&ast, options).unwrap();
    let mut expected = vec![0xCC; 0x500];
    expected[..10].copy_from_slice(&[0xEF, 0xCD, 0xAB, 0x89, 1, 0, 0x35, 0xA5, 1, 1]);
    expected[10..14].copy_from_slice(&[0, 0, 0xAB, 0x7F]);
    expected[0x80..0x85].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xA5]);
    values::check_mir_semir(
        &actionc::semantic::ir::lower_program(&ast, &model),
        &expected,
    );
}

#[test]
fn enum_views_allow_every_byte_including_unnamed_values() {
    let source = r#"
TYPE State=ENUM [READY=7 BUSY=9]
TYPE View=UNION [State state BYTE bits]
View value
BYTE ARRAY output=$600
CARD index,otherCount
BYTE errors
PROC Main()
 errors=0 otherCount=0
 FOR index=0 TO 255 DO
   value.bits=BYTE(index)
   IF BYTE(value.state)<>BYTE(index) THEN errors==+1 FI
   CASE value.state OF
   WHEN State.READY THEN
     output(0)=value.bits
   WHEN State.BUSY THEN
     output(1)=value.bits
   ELSE
     otherCount==+1
   ESAC
 OD
 output(2)=errors output(3)=BYTE(otherCount)
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..4].copy_from_slice(&[7, 9, 0, 254]);
    unions::check(source, &expected);
}
