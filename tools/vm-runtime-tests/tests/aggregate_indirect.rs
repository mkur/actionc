#[path = "support/variant_values.rs"]
mod support;

#[test]
fn aggregate_indirect_captures_callee_before_argument_effects() {
    let source = r#"
TYPE Pair=[BYTE x CARD y]
Pair source
Pair FUNC POINTER callback(Pair input BYTE n)
BYTE result=$0600
CARD total=$0601
Pair FUNC First(Pair input BYTE n)
 input.x==+n input.y==+1
RETURN(input)
Pair FUNC Second(Pair input BYTE n)
 input.x=99
RETURN(input)
BYTE FUNC Switch() callback=Second source.x=88 RETURN(4)
PROC Main()
 Pair FUNC POINTER initial(Pair input BYTE n)=[@First]
 Pair FUNC POINTER empty(Pair input BYTE n)=[NIL]
 source.x=3 source.y=1000 callback=initial
 LET value=callback(source,Switch())
 result=value.x total=value.y
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    let bytes = [7, 0xe9, 3];
    expected[..bytes.len()].copy_from_slice(&bytes);
    support::check(source, &expected);
}

#[test]
fn aggregate_indirect_field_targets_and_proc_parameters() {
    let source = r#"
TYPE Pair=[BYTE x,y]
TYPE Handler=[PROC POINTER call(Pair input BYTE n)]
Handler handlers
Pair source
BYTE result=$0600
PROC Take(Pair input BYTE n) input.x==+n result=input.x RETURN
PROC Invoke(PROC POINTER action(Pair input BYTE n) Pair input)
 action(input,4)
RETURN
PROC Main()
 source.x=3 source.y=2 handlers.call=Take
 handlers.call(source,5)
 Invoke(Take,source)
 result==+source.x
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    let bytes = [10];
    expected[..bytes.len()].copy_from_slice(&bytes);
    support::check(source, &expected);
}

#[test]
fn aggregate_indirect_variants_forward_results_and_preserve_live_values() {
    let source = r#"
TYPE Event=VARIANT [NONE KEY [BYTE code]]
Event FUNC POINTER callback(Event input BYTE n)
BYTE result=$0600,calls=$0601
Event FUNC Make(Event input BYTE n)
 calls==+1 input=Event.NONE
RETURN(Event.KEY(n))
Event FUNC Forward(Event input BYTE n) RETURN(callback(input,n))
PROC Main()
 calls=0 result=0 callback=Make
 LET first=Forward(Event.NONE,3)
 callback(first,9)
 CASE callback(first,4) OF
 WHEN Event.NONE THEN
   result=99
 WHEN Event.KEY(code) THEN
   result=code
 ESAC
 CASE first OF
 WHEN Event.NONE THEN
   result=98
 WHEN Event.KEY(code) THEN
   result==+code
 ESAC
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    let bytes = [7, 3];
    expected[..bytes.len()].copy_from_slice(&bytes);
    support::check(source, &expected);
}
