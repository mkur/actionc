#[path = "support/variant_values.rs"]
mod support;
use support::{check, check_with_fault};

#[test]
fn variant_construction_snapshot_match_and_argument_order() {
    let source = r#"
TYPE Event=VARIANT [NONE KEY [BYTE code] MOVE [INT x,y]]
Event current
Event ARRAY pool(2)
BYTE ARRAY output=$600
BYTE done=$63F,calls=$63E
BYTE FUNC Next()
  calls==+1
RETURN(calls)
INT FUNC Read()
CASE current OF
WHEN Event.NONE THEN
  RETURN(0)
WHEN Event.KEY(code) THEN
  RETURN(code)
WHEN Event.MOVE(x,y) THEN
  RETURN(x+y)
ESAC
PROC Main()
  calls=0
  current=Event.MOVE(12,-3)
  output(0)=BYTE(Read())
  LET saved=current
  current=Event.NONE
CASE saved OF
WHEN Event.NONE THEN
  output(1)=255
WHEN Event.KEY(_) THEN
  output(1)=254
WHEN Event.MOVE(x,y) THEN
  output(1)=BYTE(x+y)
ESAC
  pool(0)=Event.MOVE(Next(),Next())
  current=pool(0)
  output(2)=BYTE(Read())
CASE Event.KEY(Next()) OF
WHEN Event.KEY(code) THEN
  output(3)=code
ELSE
  output(3)=253
ESAC
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..4].copy_from_slice(&[9, 9, 3, 3]);
    expected[0x3E] = 3;
    expected[0x3F] = 0xA5;
    check(source, &expected);
}

#[test]
fn nested_record_array_payload_snapshots_and_zero_inactive_storage() {
    let source = r#"
TYPE Inner=VARIANT [NONE VALUE [INT number]]
TYPE Payload=[BYTE pre Inner ARRAY items(2) BYTE ARRAY bytes(257) BYTE post]
TYPE Outer=VARIANT [EMPTY DATA [Payload payload Inner nested]]
Payload original
Outer current
BYTE ARRAY output=$600
BYTE done=$63F
BYTE POINTER raw
CARD i
PROC Main()
  original.pre=41 original.post=42
  original.items(0)=Inner.VALUE(-7)
  original.items(1)=Inner.NONE
  FOR i=0 TO 256 DO original.bytes(i)=BYTE(i) OD
  current=Outer.DATA(original,Inner.VALUE(19))
  original.pre=99 original.bytes(256)=99
CASE current OF
WHEN Outer.DATA(payload,nested) THEN
  output(0)=payload.pre output(1)=payload.post output(2)=payload.bytes(256)
  CASE payload.items(0) OF
  WHEN Inner.VALUE(number) THEN
    output(3)=BYTE(number)
  ELSE
    output(3)=255
  ESAC
  CASE nested OF
  WHEN Inner.VALUE(number) THEN
    output(4)=BYTE(number)
  ELSE
    output(4)=255
  ESAC
ELSE
  output(0)=255
ESAC
  current=Outer.EMPTY
  raw=BYTE POINTER(@current)
  output(5)=raw(0)
  output(6)=0
  FOR i=1 TO SIZEOF(Outer)-1 DO output(6)=output(6) OR raw(i) OD
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..7].copy_from_slice(&[41, 42, 0, 249, 19, 1, 0]);
    expected[0x3F] = 0xA5;
    check(source, &expected);
}

#[test]
fn invalid_tags_fault_before_any_arm_copy_or_else_and_never_return() {
    for (setup, consumer) in [
        (
            "",
            "CASE current OF\nWHEN Event.NONE THEN\noutput(1)=1\nELSE\noutput(1)=2\nESAC",
        ),
        (
            "current=Event.VALUE(7) raw=BYTE POINTER(@current) raw(0)=3",
            "CASE current OF\nWHEN Event.NONE THEN\noutput(1)=1\nELSE\noutput(1)=2\nESAC",
        ),
        (
            "current=Event.VALUE(7) raw=BYTE POINTER(@current) raw(0)=0",
            "LET saved=current\ncopy=saved",
        ),
        (
            "current=Event.VALUE(7) raw=BYTE POINTER(@current) raw(0)=255",
            "copy=current",
        ),
    ] {
        let source = format!(
            "TYPE Event=VARIANT [NONE VALUE [CARD number]]\nEvent current,copy\nBYTE POINTER raw\nBYTE ARRAY output=$600\nPROC Main()\n{setup}\noutput(0)=41\n{consumer}\noutput(0)=42 output(1)=99\nDO OD\nRETURN"
        );
        let mut expected = vec![0xCC; 0x500];
        expected[0] = 41;
        check_with_fault(&source, &expected, true);
    }
}

#[test]
fn validity_checks_active_nested_values_but_ignores_inactive_payloads() {
    let prefix="TYPE Inner=VARIANT [NONE VALUE [CARD number]]\nTYPE Outer=VARIANT [EMPTY DATA [Inner value]]\nOuter current\nBYTE POINTER raw\nBYTE ARRAY output=$600\nPROC Main()\n";
    let mut expected=vec![0xCC;0x500];
    expected[0]=41;
    let invalid=format!("{prefix}current=Outer.DATA(Inner.VALUE(7)) raw=BYTE POINTER(@current) raw(1)=0\noutput(0)=41\nLET saved=current\noutput(0)=42\nDO OD\nRETURN");
    check_with_fault(&invalid,&expected,true);
    let inactive=format!("{prefix}current=Outer.EMPTY raw=BYTE POINTER(@current) raw(1)=255\nLET saved=current\nCASE saved OF\nWHEN Outer.EMPTY THEN\noutput(0)=41\nELSE\noutput(0)=42\nESAC\nDO OD\nRETURN");
    check(&inactive,&expected);
}

#[test]
fn real_enum_and_pointer_payloads_keep_their_resolved_types() {
    let source = r#"
TYPE Mode=ENUM [IDLE READY=7]
TYPE Reading=VARIANT [NONE VALUE [REAL number,second BYTE code Mode mode CARD POINTER link]]
Reading current
CARD value
BYTE ARRAY output=$600
BYTE done=$63F
PROC Main()
  value=10
  current=Reading.VALUE(4.0,19.0,9,Mode.READY,@value)
CASE current OF
WHEN Reading.VALUE(number,second,code,mode,link) THEN
  output(0)=BYTE(number)
  output(1)=code
  output(2)=BYTE(mode)
  link^=13
  output(3)=BYTE(value)
  output(4)=BYTE(second)
ELSE
  output(0)=255
ESAC
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..5].copy_from_slice(&[4, 9, 7, 13, 19]);
    expected[0x3F] = 0xA5;
    check(source, &expected);
}

#[test]
fn variant_copies_stage_both_overlap_directions_and_capture_destination_first() {
    let source = r#"
TYPE Item=VARIANT [NONE VALUE [CARD number BYTE code]]
Item POINTER first,second
BYTE calls=$63E,done=$63F
BYTE FUNC Pick()
  calls==+1
RETURN(0)
BYTE FUNC ChangeDestination()
  first=second
RETURN(7)
PROC Main()
  calls=0
  first=$681 second=$682
  first(Pick())=Item.VALUE($1234,ChangeDestination())
  first=$681
  second^=first^
  first^=second^
  second^=first^
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[0x81..0x85].copy_from_slice(&[2, 0x34, 0x12, 7]);
    expected.copy_within(0x81..0x85, 0x82);
    expected.copy_within(0x82..0x86, 0x81);
    expected.copy_within(0x81..0x85, 0x82);
    expected[0x3E] = 1;
    expected[0x3F] = 0xA5;
    check(source, &expected);
}

#[test]
fn maximum_tag_and_routine_static_lifetime_are_preserved() {
    let alternatives = (0..255)
        .map(|i| format!("C{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!(
        "TYPE Many=VARIANT [{alternatives}]\nBYTE calls=$63E,done=$63F\nBYTE ARRAY output=$600\nPROC Visit()\nMany local\nIF calls=0 THEN local=Many.C254 FI\nCASE local OF\nWHEN Many.C254 THEN\noutput(calls)=19\nELSE\noutput(calls)=255\nESAC\ncalls==+1\nRETURN\nPROC Main()\ncalls=0 Visit() Visit() done=$A5\nDO OD\nRETURN"
    );
    let mut expected = vec![0xCC; 0x500];
    expected[..2].copy_from_slice(&[19, 19]);
    expected[0x3E] = 2;
    expected[0x3F] = 0xA5;
    check(&source, &expected);
}
