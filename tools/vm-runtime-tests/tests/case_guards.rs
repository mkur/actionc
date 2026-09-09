#[path = "support/variant_values.rs"]
mod support;
use support::{check, check_with_fault};

#[test]
fn guarded_nested_patterns_capture_bindings_before_guard_calls_and_keep_selector_values() {
    let source = r#"
TYPE Payload=[BYTE value]
TYPE Inner=VARIANT [NONE DATA [Payload object BYTE code]]
TYPE Outer=VARIANT [EMPTY WRAP [Inner value]]
Payload original
Outer current
BYTE ARRAY output=$600
BYTE reads=$610,guards=$611,zero=$612
Outer FUNC Read()
 reads==+1
RETURN(current)
BYTE FUNC Reject(Payload value BYTE code)
 guards==+1 current=Outer.EMPTY
 output(2)=value.value+code value.value=99
RETURN(0)
PROC Main()
 reads=0 guards=0 zero=0 original.value=9
 current=Outer.WRAP(Inner.DATA(original,7))
 CASE Read() OF
 WHEN Outer.EMPTY IF 1/zero THEN
   output(0)=255
 WHEN Outer.WRAP(Inner.DATA(saved,7)) IF Reject(saved,7) THEN
   output(0)=254
 WHEN Outer.WRAP(Inner.DATA(saved,7)) IF saved.value=9 THEN
   output(0)=saved.value
 WHEN _ IF 1/zero THEN
   output(0)=253
 ELSE
   output(0)=252
 ESAC
 CASE current OF
 WHEN Outer.EMPTY THEN
   output(1)=1
 ELSE
   output(1)=0
 ESAC
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..3].copy_from_slice(&[9, 1, 16]);
    expected[0x10..0x13].copy_from_slice(&[1, 1, 0]);
    check(source, &expected);
}

#[test]
fn scalar_enum_and_wildcard_guards_preserve_calls_volatile_order_and_loop_exit() {
    let source = r#"
TYPE Mode=ENUM [IDLE RUN]
Mode state
BYTE ARRAY output=$600
VOLATILE BYTE input=$610,zero=$612
BYTE calls=$611,i=$613
BYTE FUNC Reject()
 calls==+1 input=9
RETURN(0)
BYTE FUNC Select()
 CASE input OF
 WHEN 0 IF 1/zero THEN
   RETURN(255)
 WHEN 1 TO 3 IF Reject() THEN
   RETURN(254)
 WHEN 2 IF zero<>0 AND 10/zero>0 THEN
   RETURN(253)
 WHEN 2 THEN
   RETURN(7)
 WHEN _ IF Reject() THEN
   RETURN(252)
 ELSE
   RETURN(251)
 ESAC
PROC Main()
 input=2 zero=0 calls=0
 output(0)=Select()
 state=Mode.RUN
 CASE state OF
 WHEN Mode.RUN IF Reject() THEN
   output(1)=255
 WHEN Mode.RUN THEN
   output(1)=8
 ELSE
   output(1)=254
 ESAC
 state=Mode(200)
 CASE state OF
 WHEN Mode.IDLE IF 1/zero THEN
   output(2)=255
 WHEN _ IF calls=2 THEN
   output(2)=9
 ELSE
   output(2)=254
 ESAC
 i=0
 DO
   i==+1
   CASE i OF
   WHEN 1 IF Reject() THEN
     EXIT
   WHEN 1 THEN
     output(3)=i
   WHEN _ IF i=2 THEN
     EXIT
   ESAC
 OD
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..4].copy_from_slice(&[7, 8, 9, 1]);
    expected[0x10..0x14].copy_from_slice(&[9, 3, 0, 2]);
    check(source, &expected);
}

#[test]
fn a_selected_guard_faults_after_prior_stores_and_never_reaches_fallback() {
    let source = r#"
BYTE output=$600,zero=$601
PROC Main()
 output=41 zero=0
 CASE 7 OF
 WHEN 7 IF 1/zero THEN
   output=42
 ELSE
   output=43
 ESAC
 output=44
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[0] = 41;
    expected[1] = 0;
    check_with_fault(source, &expected, true);
    let invalid = r#"
TYPE V=VARIANT [KEY [BYTE code]]
V current
BYTE output=$600,calls=$601
BYTE FUNC Count() calls==+1 RETURN(1)
PROC Main()
 output=41 calls=0
 CASE current OF
 WHEN _ IF Count() THEN
   output=42
 ELSE
   output=43
 ESAC
 DO OD
RETURN
"#;
    check_with_fault(invalid, &expected, true);
}

#[test]
fn real_enum_and_pointer_bindings_remain_typed_in_short_circuit_guards() {
    let source = r#"
TYPE Shade=ENUM [RED BLUE]
TYPE Value=VARIANT [NONE DATA [REAL number Shade color BYTE POINTER address]]
Value current
BYTE output=$600,cell=$601
PROC Main()
 cell=7 current=Value.DATA(1.5,Shade.BLUE,@cell)
 CASE current OF
 WHEN Value.DATA(r,c,p) IF r>2.0 THEN
   output=255
 WHEN Value.DATA(r,c,p) IF r>=1.0 AND c=Shade.BLUE AND p^=7 THEN
   output=9
 ELSE
   output=254
 ESAC
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..2].copy_from_slice(&[9, 7]);
    check(source, &expected);
}
