#[path = "support/variant_values.rs"]
mod support;
use support::check;

#[test]
fn fixed_arena_binary_search_tree_executes_iteratively() {
    let sample = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/variant-tree.act"),
    )
    .unwrap();
    let source = format!(
        "{sample}\nBYTE ARRAY output=$600 BYTE done=$63F\nPROC Probe()\nBYTE i\nBuildDemoTree()\nFOR i=0 TO count-1 DO output(i)=BYTE(sorted(i)) OD\noutput(8)=used output(9)=count output(10)=overflow\noutput(11)=arena.before output(12)=arena.after\ndone=$A5\nDO OD\nRETURN"
    );
    let mut expected = vec![0xCC; 0x500];
    expected[..13].copy_from_slice(&[1, 2, 3, 5, 6, 7, 8, 9, 8, 8, 0, 41, 42]);
    expected[0x3F] = 0xA5;
    check(&source, &expected);
}

#[test]
fn shared_subtrees_nil_and_non_page_aligned_pointer_pool_are_explicit() {
    let source = r#"
CONST CARD NIL=0
TYPE Tree=VARIANT [EMPTY NODE [INT value Tree POINTER left,right]]
Tree empty
Tree POINTER pool,first,second
BYTE ARRAY output=$600
BYTE done=$63F
PROC Main()
  empty=Tree.EMPTY
  pool=$681
  pool(0)=Tree.NODE(3,@empty,NIL)
  pool(1)=Tree.NODE(5,@pool(0),@pool(0))
  first=@pool(1) second=first
  CASE first^ OF
  WHEN Tree.NODE(value,left,right) THEN
    output(0)=BYTE(value)
    IF left=right THEN output(1)=1 FI
    left^=Tree.NODE(7,@empty,NIL)
  ELSE
    output(0)=255
  ESAC
  CASE second^ OF
  WHEN Tree.NODE(_,_,right) THEN
    CASE right^ OF
    WHEN Tree.NODE(value,_,leaf) THEN
      output(2)=BYTE(value)
      IF leaf=NIL THEN output(3)=1 FI
    ELSE
      output(2)=255
    ESAC
  ELSE
    output(2)=254
  ESAC
  done=$A5
  DO OD
RETURN
"#;
    // Pointer bytes for the compiler-owned sentinel vary with code size.
    // Copy the pointed-to nodes back into a zero-payload alternative before
    // checking the entire guarded pool, leaving addresses unobserved.
    let source = source.replace("done=$A5", "pool(0)=Tree.EMPTY pool(1)=Tree.EMPTY done=$A5");
    let mut expected = vec![0xCC; 0x500];
    expected[..4].copy_from_slice(&[5, 1, 7, 1]);
    expected[0x81..0x8F].fill(0);
    expected[0x81] = 1;
    expected[0x88] = 1;
    expected[0x3F] = 0xA5;
    check(&source, &expected);
}

#[test]
fn mutually_recursive_payload_pointers_share_the_original_objects() {
    let source = r#"
TYPE Left=VARIANT [END LINK [Right POINTER next]]
TYPE Right=VARIANT [END LINK [INT value Left POINTER next]]
Left first
Right second
BYTE ARRAY output=$600
BYTE done=$63F
PROC Main()
  first=Left.LINK(@second)
  second=Right.LINK(17,@first)
  CASE first OF
  WHEN Left.LINK(peer) THEN
    CASE peer^ OF
    WHEN Right.LINK(value,back) THEN
      output(0)=BYTE(value)
      IF back=@first THEN output(1)=1 FI
      back^=Left.END
    ELSE
      output(0)=255
    ESAC
  ELSE
    output(0)=254
  ESAC
  CASE first OF
  WHEN Left.END THEN
    output(2)=1
  ELSE
    output(2)=255
  ESAC
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..3].copy_from_slice(&[17, 1, 1]);
    expected[0x3F] = 0xA5;
    check(source, &expected);
}

#[test]
fn byte_and_named_pointer_results_preserve_complete_addresses_across_calls() {
    let source = r#"
TYPE Node=[BYTE value]
Node original
BYTE datum
Node POINTER FUNC NodeAddress() RETURN(@original)
BYTE POINTER FUNC ByteAddress() RETURN(@datum)
Node POINTER FUNC POINTER callback()
BYTE ARRAY output=$600
BYTE done=$63F
PROC Main()
  BYTE POINTER b
  callback=@NodeAddress
  LET p=callback()
  p.value=9
  b=ByteAddress() b^=17
  IF p=@original THEN output(0)=original.value FI
  IF @datum=b THEN output(1)=datum FI
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..2].copy_from_slice(&[9, 17]);
    expected[0x3F] = 0xA5;
    check(source, &expected);
}
