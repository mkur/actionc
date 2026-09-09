use super::array_execution_tests::{execute, outputs_with_options};
use crate::semantic::SemanticOptions;

fn check(source: &str, expected: &[u8]) {
    let mut options = SemanticOptions::modern();
    options.algebraic_types.generic_types = true;
    for (mode, output) in outputs_with_options(source, options) {
        let memory = execute(&output, |memory| memory[0x11] = 0xff);
        assert_eq!(&memory[0x600..0x600 + expected.len()], expected, "{mode}");
    }
}

#[test]
fn generic_types_option_result_and_aggregate_callbacks_compose() {
    check(
        r#"
TYPE Option<T>=VARIANT [NONE SOME [T value]]
TYPE Result<T,E>=VARIANT [OK [T value] ERROR [E error]]
Option<BYTE> FUNC POINTER callback(BYTE n)
BYTE output=$0600
Option<BYTE> FUNC Make(BYTE n) RETURN(Option<BYTE>.SOME(n))
Result<Option<BYTE>,CARD> FUNC Wrap(Option<BYTE> input)
RETURN(Result<Option<BYTE>,CARD>.OK(input))
PROC Main()
 callback=Make output=0
 LET first=callback(7)
 LET Result<Option<BYTE>,CARD> second=Wrap(first)
 CASE second OF
 WHEN Result<Option<BYTE>,CARD>.OK(value) THEN
   CASE value OF
   WHEN Option<BYTE>.NONE THEN
     output=99
   WHEN Option<BYTE>.SOME(n) THEN
     output=n
   ESAC
 WHEN Result<Option<BYTE>,CARD>.ERROR(error) THEN
   output=98
 ESAC
RETURN
"#,
        &[7],
    );
}

#[test]
fn generic_types_recursive_pointer_instances_and_inline_record_arrays_execute() {
    check(
        r#"
TYPE Buffer<T>=[T ARRAY values(3)]
TYPE TreeOf<T>=VARIANT [EMPTY NODE [T value TreeOf<T> POINTER left,right]]
Buffer<CARD> data
TreeOf<BYTE> empty,root
TreeOf<BYTE> POINTER link
CARD output=$0600
PROC Main()
 data.values(2)=1000
 empty=TreeOf<BYTE>.EMPTY
 root=TreeOf<BYTE>.NODE(7,@empty,@empty)
 link=@root
 CASE link^ OF
 WHEN TreeOf<BYTE>.EMPTY THEN
   output=99
 WHEN TreeOf<BYTE>.NODE(value,left,right) THEN
   output=data.values(2)+value
 ESAC
RETURN
"#,
        &[0xef, 3],
    );
}

#[test]
fn generic_types_pointer_arguments_and_typed_pointer_results_execute() {
    check(
        r#"
TYPE Box<T>=[T value]
TYPE Option<T>=VARIANT [NONE SOME [T value]]
Box<BYTE> data
Box<BYTE> POINTER FUNC POINTER address()
Box<BYTE FUNC POINTER(BYTE n)> dispatch
BYTE output=$0600
Box<BYTE> POINTER FUNC Get() RETURN(@data)
BYTE FUNC Add(BYTE n) RETURN(n+1)
PROC Main()
 address=Get
 dispatch.value=Add
 LET view=address()
 view.value=9
 LET saved=Option<Box<BYTE> POINTER>.SOME(view)
 CASE saved OF
 WHEN Option<Box<BYTE> POINTER>.NONE THEN
   output=99
 WHEN Option<Box<BYTE> POINTER>.SOME(p) THEN
   output=p.value+dispatch.value(2)
 ESAC
RETURN
"#,
        &[12],
    );
}

#[test]
fn generic_types_embed_caller_defined_record_values() {
    check(
        r#"
TYPE Pair=[BYTE x CARD y]
TYPE Box<T>=[T value]
TYPE Option<T>=VARIANT [NONE SOME [T value]]
Box<Pair> wrapped
BYTE output=$0600
CARD total=$0601
PROC Main()
 wrapped.value.x=7 wrapped.value.y=1000
 LET snapshot=wrapped
 LET item=Option<Pair>.SOME(wrapped.value)
 wrapped.value.x=9
 CASE item OF
 WHEN Option<Pair>.NONE THEN
   output=99
 WHEN Option<Pair>.SOME(pair) THEN
   output=snapshot.value.x+pair.x
   total=pair.y
 ESAC
RETURN
"#,
        &[14, 0xe8, 3],
    );
}
