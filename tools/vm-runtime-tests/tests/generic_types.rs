#[path = "support/variant_values.rs"]
mod support;

#[test]
fn generic_types_sample_print_values_match_the_documented_results() {
    let sample = include_str!("../../../samples/generic-types.act");
    let source = sample
        .replace("Option<BYTE> FUNC POINTER callback(BYTE n)",
            "Option<BYTE> FUNC POINTER callback(BYTE n)\nCARD ARRAY observed(3)=$0600\nBYTE captures=$0606")
        .replace("PROC Main()", "PROC Capture(CARD value) observed(captures)=value captures==+1 RETURN\nPROC Main()\n captures=0")
        .replace("PrintBE(", "Capture(").replace("PrintCE(", "Capture(").replace("PrintIE(", "Capture(");
    let end = source.rfind("RETURN").unwrap();
    let source = format!("{}DO OD\nRETURN\n", &source[..end]);
    let mut expected = vec![0xCC; 0x500];
    expected[..7].copy_from_slice(&[7, 0, 0xe8, 3, 12, 0, 3]);
    support::check(&source, &expected);
}

#[test]
fn generic_types_option_result_and_aggregate_callbacks_compose() {
    let source = r#"
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
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    let bytes = [7];
    expected[..bytes.len()].copy_from_slice(&bytes);
    support::check(source, &expected);
}

#[test]
fn generic_types_recursive_pointer_instances_and_inline_record_arrays_execute() {
    let source = r#"
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
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    let bytes = [0xef, 3];
    expected[..bytes.len()].copy_from_slice(&bytes);
    support::check(source, &expected);
}

#[test]
fn generic_types_pointer_arguments_and_typed_pointer_results_execute() {
    let source = r#"
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
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[0] = 12;
    support::check(source, &expected);
}
