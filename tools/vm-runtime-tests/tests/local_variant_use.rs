#[path = "support/variant_values.rs"]
mod support;

#[test]
fn local_constructor_names_preserve_payload_order_patterns_and_scope() {
    let source = r#"
TYPE MaybeByte=VARIANT [NONE SOME [BYTE value]]
TYPE BytePair=VARIANT [PAIR [MaybeByte left,right]]
TYPE Other=VARIANT [NONE SOME [BYTE value]]
BYTE ARRAY output=$600
BYTE done=$63F,calls=$63E
BYTE FUNC Next()
  calls==+1
RETURN(calls)
MaybeByte FUNC Make()
  USE ALL FROM MaybeByte
RETURN(SOME(Next()))
PROC Main()
  calls=0
  BEGIN
    USE ALL FROM MaybeByte
    USE ALL FROM BytePair
    LET item=PAIR(Make(),SOME(Next()))
    CASE item OF
    WHEN PAIR(SOME(a),SOME(b)) THEN
      output(0)=a output(1)=b
    ELSE
      output(0)=255
    ESAC
    LET empty=PAIR(NONE,SOME(3))
    CASE empty OF
    WHEN PAIR(NONE,SOME(n)) IF n=3 THEN
      output(2)=n
    ELSE
      output(2)=255
    ESAC
    BEGIN
      BYTE SOME
      SOME=7
      LET inner=MaybeByte.SOME(SOME)
      CASE inner OF
      WHEN MaybeByte.SOME(n) THEN
        output(3)=n
      ELSE
        output(3)=255
      ESAC
    END
    LET restored=SOME(8)
    CASE restored OF
    WHEN SOME(n) THEN
      output(4)=n
    ELSE
      output(4)=255
    ESAC
  END
  BEGIN
    USE ALL FROM Other
    LET item=SOME(9)
    CASE item OF
    WHEN NONE THEN
      output(5)=255
    WHEN SOME(n) THEN
      output(5)=n
    ESAC
  END
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..6].copy_from_slice(&[1, 2, 3, 7, 8, 9]);
    expected[0x3E] = 2;
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

#[test]
fn opened_patterns_still_fault_before_matching_invalid_nested_tags() {
    let source = r#"
TYPE Inner=VARIANT [NONE SOME [BYTE value]]
TYPE Outer=VARIANT [BOX [Inner value]]
Outer item
BYTE POINTER raw
BYTE ARRAY output=$600
PROC Main()
  USE ALL FROM Inner
  USE ALL FROM Outer
  item=BOX(SOME(9))
  raw=BYTE POINTER(@item)
  raw(1)=0
  output(0)=41
  CASE item OF
  WHEN BOX(NONE) THEN
    output(0)=42
  ELSE
    output(0)=43
  ESAC
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[0] = 41;
    support::check_with_fault(source, &expected, true);
}
