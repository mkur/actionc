#[path = "support/unions.rs"]
mod support;

#[test]
fn whole_copies_and_let_snapshots_cover_extents_pages_and_both_overlap_directions() {
    for size in [1, 2, 3, 4, 31, 32, 33, 255, 256, 257] {
        let source = format!(
            r#"
TYPE Chunk=UNION [BYTE low BYTE ARRAY bytes({size})]
Chunk POINTER first,second
BYTE ARRAY raw=$700
CARD index
BYTE calls=$600,iteration
BYTE FUNC Pick() calls==+1 RETURN(0)
PROC Main()
 calls=0
 FOR index=0 TO 299 DO raw(index)=BYTE(index) OD
 first=$701 second=$702
 FOR iteration=0 TO 1 DO
   BEGIN
     LET saved=first(Pick())
     first.bytes(0)=99
     second^=saved
     first^=first^
     first^=second^
   END
 OD
 DO OD
RETURN
"#
        );
        let mut expected = vec![0xCC; 0x500];
        expected[0] = 2;
        for i in 0..300 {
            expected[0x100 + i] = i as u8;
        }
        for _ in 0..2 {
            let saved = expected[0x101..0x101 + size].to_vec();
            expected[0x101] = 99;
            expected[0x102..0x102 + size].copy_from_slice(&saved);
            expected.copy_within(0x102..0x102 + size, 0x101);
        }
        support::check(&source, &expected);
    }
}

#[test]
fn nested_unions_and_records_copy_views_but_share_pointer_pointees() {
    let source = r#"
TYPE Link=UNION [BYTE POINTER ptr CARD address]
TYPE Entry=[BYTE tag Link view BYTE tail]
TYPE Envelope=UNION [Entry entry BYTE ARRAY bytes(4)]
Envelope ARRAY items(2)
BYTE ARRAY output=$600
BYTE cell=$680,other=$681
PROC Main()
 cell=3 other=4
 items(0).entry.tag=1 items(0).entry.tail=2 items(0).entry.view.ptr=@cell
 LET saved=items(0)
 items(0).entry.tag=99 items(0).entry.view.ptr=@other
 saved.entry.view.ptr^=7
 items(1)=saved
 output(0)=items(1).entry.tag output(1)=items(1).entry.tail
 output(2)=items(1).entry.view.ptr^ output(3)=items(0).entry.view.ptr^
 output(4)=saved.bytes(1) output(5)=saved.bytes(2)
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..6].copy_from_slice(&[1, 2, 7, 4, 0x80, 6]);
    expected[0x80] = 7;
    expected[0x81] = 4;
    support::check(source, &expected);
}

#[test]
fn value_calls_capture_arguments_callee_and_destination_before_later_effects() {
    let source = r#"
TYPE View=UNION [CARD word BYTE ARRAY bytes(3)]
View source
View ARRAY destinations(2)
View FUNC POINTER callback(View input BYTE n)
BYTE ARRAY output=$600
BYTE calls,order
View FUNC First(View input BYTE n)
 calls==+1 input.bytes(0)==+n input.bytes(2)==+1
RETURN(input)
View FUNC Second(View input BYTE n)
 calls==+1 input.word=99
RETURN(input)
BYTE FUNC Switch()
 order=order*10+2 callback=Second source.word=$EEEE source.bytes(2)=88
RETURN(4)
BYTE FUNC Pick() order=order*10+1 RETURN(1)
View FUNC Forward(View input BYTE n) RETURN(callback(input,n))
PROC Main()
 calls=0 order=0 source.word=$1203 source.bytes(2)=7 callback=First
 destinations(Pick())=callback(source,Switch())
 output(0)=destinations(1).bytes(0) output(1)=destinations(1).bytes(1)
 output(2)=destinations(1).bytes(2) output(3)=order
 callback=First
 LET saved=Forward(destinations(1),2)
 First(saved,10)
 output(4)=saved.bytes(0) output(5)=saved.bytes(2)
 output(6)=destinations(1).bytes(0) output(7)=source.bytes(0) output(8)=calls
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..9].copy_from_slice(&[7, 0x12, 8, 12, 9, 9, 7, 0xEE, 3]);
    support::check(source, &expected);
}
