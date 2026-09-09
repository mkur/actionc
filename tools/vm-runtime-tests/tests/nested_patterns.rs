#[path = "support/variant_values.rs"]
mod support;
use support::{check, check_with_fault};

#[test]
fn nested_patterns_preserve_order_bindings_and_selector_snapshots() {
    let source = r#"
TYPE Option<T>=VARIANT [NONE SOME [T value]]
TYPE Pair=VARIANT [EMPTY BOTH [Option<INT> first,second]]
Pair current
BYTE ARRAY output=$600
BYTE calls=$610
Pair FUNC Read()
 calls==+1
RETURN(current)
BYTE FUNC Choose()
 CASE Read() OF
 WHEN Pair.BOTH(Option<INT>.SOME(-1),Option<INT>.SOME(x)) THEN
   current=Pair.EMPTY
   RETURN(BYTE(x))
 WHEN Pair.BOTH(Option<INT>.SOME(x),_) THEN
   RETURN(BYTE(x+10))
 WHEN Pair.BOTH(_,Option<INT>.SOME(y)) THEN
   RETURN(BYTE(y+20))
 WHEN Pair.BOTH(_,_) THEN
   RETURN(30)
 WHEN Pair.EMPTY THEN
   RETURN(40)
 ESAC
PROC Main()
 calls=0
 current=Pair.BOTH(Option<INT>.SOME(-1),Option<INT>.SOME(7))
 output(0)=Choose()
 current=Pair.BOTH(Option<INT>.SOME(2),Option<INT>.NONE)
 output(1)=Choose()
 current=Pair.BOTH(Option<INT>.NONE,Option<INT>.SOME(3))
 output(2)=Choose()
 current=Pair.BOTH(Option<INT>.NONE,Option<INT>.NONE)
 output(3)=Choose()
 current=Pair.EMPTY
 output(4)=Choose()
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..5].copy_from_slice(&[7, 12, 23, 30, 40]);
    expected[0x10] = 5;
    check(source, &expected);
}

#[test]
fn nested_patterns_keep_aggregate_and_shadowed_bindings_immutable_snapshots() {
    let source = r#"
TYPE Payload=[BYTE value]
TYPE Inner=VARIANT [NONE DATA [Payload object]]
TYPE Outer=VARIANT [EMPTY WRAP [Inner value]]
Payload original
Outer current
BYTE ARRAY output=$600
PROC Main()
 original.value=7
 current=Outer.WRAP(Inner.DATA(original))
 CASE current OF
 WHEN Outer.WRAP(Inner.DATA(saved)) THEN
   original.value=99 current=Outer.EMPTY
   output(0)=saved.value
   CASE Inner.DATA(original) OF
   WHEN Inner.DATA(saved) THEN
     output(1)=saved.value
   ELSE
     output(1)=255
   ESAC
   output(2)=saved.value
 ELSE
   output(0)=255
 ESAC
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..3].copy_from_slice(&[7, 99, 7]);
    check(source, &expected);
}

#[test]
fn nested_patterns_validate_active_values_before_else_but_not_inactive_payloads() {
    for (outer_tag, tag, fault) in [(2, 0, true), (2, 3, true), (2, 1, false), (1, 255, false)] {
        let source = format!(
            r#"
TYPE Inner=VARIANT [NONE SOME [BYTE value]]
TYPE Outer=VARIANT [EMPTY WRAP [Inner value]]
Outer current
BYTE POINTER bytes
BYTE output=$600
PROC Main()
 output=41
 current=Outer.WRAP(Inner.SOME(7))
 bytes=BYTE POINTER(@current)
 bytes(0)={outer_tag}
 bytes(1)={tag}
 CASE current OF
 WHEN Outer.WRAP(Inner.SOME(7)) THEN
   output=99
 ELSE
   output=42
 ESAC
 DO OD
RETURN
"#
        );
        let mut expected = vec![0xCC; 0x500];
        expected[0] = if fault { 41 } else { 42 };
        check_with_fault(&source, &expected, fault);
    }
}
