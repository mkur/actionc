use super::array_execution_tests::{execute, outputs_with_options};
use crate::semantic::SemanticOptions;

fn options() -> SemanticOptions {
    let mut options = SemanticOptions::modern();
    options.algebraic_types.aggregate_calls = true;
    options
}

#[test]
fn aggregate_direct_parameter_addresses_refer_to_mutable_callee_copies() {
    let source = r#"
TYPE Pair=[BYTE x,y]
Pair original
BYTE result=$0600
PROC Mutate(Pair input)
  Pair POINTER alias=[@input]
  alias.x=9
  result=input.x
RETURN
PROC Main()
  original.x=3 original.y=4
  Mutate(original)
  result==+original.x
RETURN
"#;
    for (mode, output) in outputs_with_options(source, options()) {
        let memory = execute(&output, |memory| memory[0x11] = 0xff);
        assert_eq!(memory[0x600], 12, "{mode}");
    }
}

#[test]
fn aggregate_direct_calls_copy_parameters_and_return_independent_values() {
    let source = r#"
TYPE Pair=[BYTE x CARD y]
Pair original,second
BYTE result=$0600
CARD total=$0601
Pair FUNC Change(Pair input BYTE amount)
  input.x==+amount
  input.y==+1
RETURN(input)
PROC Main()
  original.x=7 original.y=1000
  LET first=Change(original,3)
  second=Change(Change(original,4),5)
  result=original.x+first.x+second.x
  total=original.y+first.y+second.y
RETURN
"#;
    for (mode, output) in outputs_with_options(source, options()) {
        // SArgs checks the Atari BREAK flag; a running machine sets it.
        let memory = execute(&output, |memory| memory[0x11] = 0xff);
        assert_eq!(memory[0x600], 33, "{mode}");
        assert_eq!(
            u16::from_le_bytes([memory[0x601], memory[0x602]]),
            3003,
            "{mode}"
        );
    }
}

#[test]
fn aggregate_direct_argument_capture_precedes_later_effects() {
    let source = r#"
TYPE Pair=[BYTE x,y]
Pair original
BYTE result=$0600,calls=$0601
BYTE FUNC Mutate()
  original.x=99 calls==+1
RETURN(4)
BYTE FUNC Sum(Pair input BYTE n)
  input.y=8
RETURN(input.x+n)
PROC Main()
  original.x=3 original.y=5 calls=0
  result=10+Sum(original,Mutate())
  result==+original.y
RETURN
"#;
    for (mode, output) in outputs_with_options(source, options()) {
        let memory = execute(&output, |_| {});
        assert_eq!(&memory[0x600..0x602], &[22, 1], "{mode}");
    }
}

#[test]
fn aggregate_direct_variant_producers_ignored_results_and_case_consumers() {
    let source = r#"
TYPE Event=VARIANT [NONE KEY [BYTE code]]
BYTE result=$0600,calls=$0601
Event FUNC Make(BYTE code)
  calls==+1
  IF code=0 THEN RETURN(Event.NONE) FI
RETURN(Event.KEY(code))
Event FUNC Forward(Event input BYTE code)
  input=Event.NONE
RETURN(Make(code))
PROC Main()
  calls=0 result=0
  LET first=Make(3)
  Make(9)
  CASE Make(4) OF
  WHEN Event.NONE THEN
    result=90
  WHEN Event.KEY(code) THEN
    result=code
  ESAC
  LET second=Forward(first,5)
  CASE first OF
  WHEN Event.NONE THEN
    result=91
  WHEN Event.KEY(code) THEN
    result==+code
  ESAC
  CASE second OF
  WHEN Event.NONE THEN
    result=92
  WHEN Event.KEY(code) THEN
    result==+code
  ESAC
RETURN
"#;
    for (mode, output) in outputs_with_options(source, options()) {
        let memory = execute(&output, |memory| memory[0x11] = 0xff);
        assert_eq!(&memory[0x600..0x602], &[12, 4], "{mode}");
    }
}

#[test]
fn aggregate_direct_assignment_captures_destination_before_result_call() {
    let source = r#"
TYPE Buffer=[BYTE ARRAY data(8)]
Buffer first,second
Buffer POINTER selected
BYTE result=$0600,calls=$0601
Buffer FUNC Make(Buffer input)
  selected=second
  input.data(7)=9 calls==+1
RETURN(input)
PROC Main()
  first.data(0)=3 first.data(7)=1
  second.data(0)=7 second.data(7)=2
  selected=first calls=0
  selected^=Make(selected^)
  result=first.data(0)+first.data(7)+second.data(0)+second.data(7)
RETURN
"#;
    for (mode, output) in outputs_with_options(source, options()) {
        let memory = execute(&output, |memory| memory[0x11] = 0xff);
        assert_eq!(&memory[0x600..0x602], &[21, 1], "{mode}");
    }
}
