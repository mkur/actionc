#[path = "support/variant_values.rs"]
mod support;

fn check(source: &str, expected: &[u8]) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let mut options = actionc::semantic::SemanticOptions::modern();
    options.algebraic_types.unions = true;
    let model = actionc::semantic::analyze_with_options(&ast, options).unwrap();
    support::check_semir(
        &actionc::semantic::ir::lower_program(&ast, &model),
        expected,
        false,
    );
}

#[test]
fn overlapping_typed_views_preserve_bits_and_selected_extents() {
    let source = r#"
TYPE State=ENUM [READY=7 BUSY=9]
TYPE Pair=[BYTE first,second]
TYPE View=UNION [CARD word INT signedWord BYTE ARRAY bytes(3) Pair parts State state]
View value
BYTE ARRAY output=$0600
PROC Main()
 value.bytes(2)=$A5
 value.word=$1234
 output(0)=value.bytes(0) output(1)=value.parts.second
 value.bytes(0)=$78
 output(2)=value.word&$FF output(3)=value.word RSH 8
 value.parts.second=$FF
 output(4)=value.signedWord<0
 value.word==+1
 output(5)=value.parts.first output(6)=value.parts.second
 value.signedWord==-1
 output(7)=value.bytes(0)
 value.state=State.READY
 output(8)=value.parts.first output(9)=value.parts.second
 output(10)=value.bytes(2)
 CASE value.parts.first OF
 WHEN 7 THEN
   output(11)=1
 ELSE
   output(11)=2
 ESAC
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..12].copy_from_slice(&[
        0x34, 0x12, 0x78, 0x12, 1, 0x79, 0xFF, 0x78, 7, 0xFF, 0xA5, 1,
    ]);
    check(source, &expected);
}

#[test]
fn pointer_call_machine_and_join_writes_invalidate_overlapping_views() {
    let source = r#"
TYPE View=UNION [CARD word BYTE ARRAY bytes(2) BYTE POINTER link]
View value=$0680
BYTE ARRAY output=$0600
BYTE POINTER ptr
BYTE flag=$0684
PROC Change(View POINTER input) input.bytes(1)=$56 RETURN
PROC Main()
 flag=1 value.word=$1234
 output(0)=value.word RSH 8
 ptr=@value.bytes(0) ptr^=$78
 output(1)=value.word&$FF
 Change(value)
 output(2)=value.word RSH 8
 [ $A9 $9A $8D $80 $06 ]
 output(3)=value.word&$FF
 IF flag THEN value.bytes(1)=$BC ELSE value.word=0 FI
 output(4)=value.word RSH 8
 value.link=$0690 value.bytes(0)=$91 value.link^=$42
 output(5)=value.word&$FF
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..6].copy_from_slice(&[0x12, 0x78, 0x56, 0x9A, 0xBC, 0x91]);
    expected[0x80..0x82].copy_from_slice(&[0x91, 6]);
    expected[0x84] = 1;
    expected[0x91] = 0x42;
    check(source, &expected);
}

#[test]
fn nested_high_offset_access_evaluates_index_and_rhs_once_in_order() {
    let source = r#"
TYPE View=UNION [CARD word BYTE ARRAY bytes(2)]
TYPE Container=[BYTE ARRAY prefix(257) View ARRAY views(2)]
Container buffer
BYTE ARRAY output=$0600
BYTE count,order
BYTE FUNC Index() count==+1 order=order*10+1 RETURN(1)
BYTE FUNC Value() count==+1 order=order*10+2 RETURN(3)
PROC Main()
 count=0 order=0 buffer.views(1).word=$1204
 buffer.views(Index()).bytes(0)==+Value()
 output(0)=buffer.views(1).word&$FF output(1)=buffer.views(1).word RSH 8
 output(2)=count output(3)=order
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..4].copy_from_slice(&[7, 0x12, 2, 12]);
    check(source, &expected);
}
