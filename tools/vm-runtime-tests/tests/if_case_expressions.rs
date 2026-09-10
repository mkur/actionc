#[path = "support/variant_values.rs"]
mod support;

#[test]
fn if_values_execute_only_selected_results_and_preserve_surrounding_values() {
    let source = r#"
BYTE ARRAY out=$600
BYTE calls=$63E,done=$63F
BYTE flag,zero,i
BYTE FUNC Mark(BYTE n)
  calls==+1
RETURN(n)
BYTE FUNC Pick(BYTE f)
RETURN(IF f THEN Mark(11) ELSE Mark(22) FI)
BYTE FUNC Sum(BYTE a,b)
RETURN(a+b)
PROC Main()
  calls=0 flag=1 zero=0
  out(0)=IF flag THEN Mark(7) ELSE Mark(99) FI
  flag=0
  out(1)=IF flag THEN Mark(99) ELSEIF Mark(1) THEN Mark(8) ELSE Mark(99) FI
  out(2)=Sum(Mark(30),IF Mark(0) THEN Mark(99) ELSE Mark(12) FI)
  out(3)=Mark(40)+(IF Mark(1) THEN Mark(2) ELSE Mark(99) FI)
  out(4)=IF flag THEN 1/zero ELSE Mark(9) FI
  out(5)=Pick(1)
  out(6)=Pick(0)
  out(7)=0
  IF IF 1 THEN Mark(0) AND Mark(1) ELSE 1 FI THEN out(7)=99 FI
  IF IF 1 THEN (Mark(2)=1) AND (Mark(3)=3) ELSE 1 FI THEN out(7)=99 FI
  out(16)=IF (Mark(0)=1) AND (Mark(99)=99) THEN 99 ELSE 4 FI
  i=0
  WHILE IF i<3 THEN Mark(1) ELSE Mark(0) FI DO
    out(8+i)=i
    i==+1
  OD
  out(11)=IF flag THEN 99 ELSE IF i=3 THEN 31 ELSE 99 FI FI
  out(12)=10
  out(IF flag THEN 13 ELSE 12 FI)==+IF flag THEN 99 ELSE 5 FI
  FOR i=IF flag THEN 99 ELSE 0 FI TO IF flag THEN 99 ELSE 2 FI DO
    out(13+i)=i+20
  OD
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..16].copy_from_slice(&[7, 8, 42, 42, 9, 11, 22, 0, 0, 1, 2, 31, 15, 20, 21, 22]);
    expected[16] = 4;
    expected[0x3E] = 21;
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

fn semir(source: &str) -> actionc::semantic::ir::SemProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    actionc::semantic::ir::lower_program(&ast, &model)
}

#[test]
fn if_values_preserve_volatile_read_order_and_discarded_result_effects() {
    let program = semir(
        r#"
BYTE ARRAY out=$600
VOLATILE BYTE signal=$610,left=$611,right=$612
BYTE done=$63F
PROC Main()
  signal=1
  out(0)=IF signal THEN left ELSE right FI
  signal=0
  out(1)=IF signal THEN left ELSE right FI
  LET discarded=IF signal THEN left ELSE right FI
  done=$A5
  DO OD
RETURN
"#,
    );
    let mut expected = vec![0xCC; 0x500];
    expected[0x10] = 0;
    expected[0x3F] = 0xA5;
    support::check_semir_watched(
        &program,
        &expected,
        false,
        &[0x610, 0x611, 0x612],
        |path, events| {
            use actionc_vm::BusAccess::{Read, Write};
            let observed: Vec<_> = events
                .iter()
                .map(|e| (e.access, e.address, e.value))
                .collect();
            assert_eq!(
                observed,
                [
                    (Write, 0x610, 1),
                    (Read, 0x610, 1),
                    (Read, 0x611, 0xCC),
                    (Write, 0x610, 0),
                    (Read, 0x610, 0),
                    (Read, 0x612, 0xCC),
                    (Read, 0x610, 0),
                    (Read, 0x612, 0xCC),
                ],
                "{path}"
            );
        },
    );
}

#[test]
fn mir_if_value_joins_preserve_wide_signed_and_unsigned_results() {
    let program = semir(
        r#"
BYTE ARRAY out=$600
BYTE flag,done=$63F
LONGINT signed
LONGCARD unsigned
PROC Main()
  flag=0
  signed=IF flag THEN LONGINT(42) ELSE LONGINT(-65537) FI
  unsigned=IF flag THEN LONGCARD(7) ELSE LONGCARD(65537) FI
  out(0)=BYTE(signed)
  out(1)=BYTE(signed RSH 8)
  out(2)=BYTE(signed RSH 16)
  out(3)=BYTE(signed RSH 24)
  out(4)=BYTE(unsigned)
  out(5)=BYTE(unsigned RSH 8)
  out(6)=BYTE(unsigned RSH 16)
  out(7)=BYTE(unsigned RSH 24)
  done=$A5
  DO OD
RETURN
"#,
    );
    let mut expected = vec![0xCC; 0x500];
    expected[..8].copy_from_slice(&[255, 255, 254, 255, 1, 0, 1, 0]);
    expected[0x3F] = 0xA5;
    support::check_mir_semir(&program, &expected);
}

#[test]
fn case_values_capture_selectors_and_preserve_ordered_guards_and_surrounding_calls() {
    let source = r#"
BYTE ARRAY out=$600,trace=$680
BYTE ARRAY fixed(4)=$620
BYTE calls=$63E,done=$63F,input,zero
BYTE FUNC Mark(BYTE n)
  trace(calls)=n calls==+1
RETURN(n)
BYTE FUNC Read()
  Mark(10)
RETURN(input)
BYTE FUNC Reject()
  input=9 Mark(11)
RETURN(0)
BYTE FUNC Accept()
  input=8 Mark(12)
RETURN(1)
BYTE FUNC Sum(BYTE a,b)
RETURN(a+b)
PROC Main()
  calls=0 input=3 zero=0
  out(0)=CASE Read() OF
  WHEN 0 IF 1/zero THEN
    Mark(99)
  WHEN 3 IF Reject() THEN
    Mark(99)
  WHEN 2 TO 4 IF Accept() THEN
    Mark(13)
  ELSE
    Mark(99)
  ESAC
  out(1)=input
  out(2)=CASE Mark(7) OF
  WHEN 7 IF Mark(0) THEN
    Mark(99)
  WHEN _ IF Mark(1) THEN
    Mark(14)
  ELSE
    Mark(99)
  ESAC
  out(3)=Mark(30)+(CASE Mark(1) OF
  WHEN 1 THEN
    Mark(12)
  ELSE
    1/zero
  ESAC)
  out(4)=Sum(Mark(40),CASE Mark(0) OF
  WHEN 1 THEN
    1/zero
  ELSE
    Mark(2)
  ESAC)
  out(5)=IF input=8 THEN CASE Mark(5) OF
  WHEN 5 THEN
    CASE Mark(6) OF
    WHEN 6 THEN
      Mark(15)
    ELSE
      1/zero
    ESAC
  ELSE
    1/zero
  ESAC ELSE 1/zero FI
  LET unused=CASE Mark(8) OF
  WHEN 8 THEN
    Mark(16)
  ELSE
    Mark(99)
  ESAC
  out(6)=IF input THEN Mark(17) ELSE Mark(99) FI
  input=0
  fixed(input)=CASE Mark(1) OF
  WHEN 1 THEN
    Mark(18)
  ELSE
    Mark(99)
  ESAC
  done=$A5
  DO OD
RETURN
"#;
    let trace = [
        10, 11, 12, 13, 7, 0, 1, 14, 30, 1, 12, 40, 0, 2, 5, 6, 15, 8, 16, 17, 1, 18,
    ];
    let mut expected = vec![0xCC; 0x500];
    expected[..7].copy_from_slice(&[13, 8, 14, 42, 42, 15, 17]);
    expected[0x20] = 18;
    expected[0x80..0x80 + trace.len()].copy_from_slice(&trace);
    expected[0x3E] = trace.len() as u8;
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

#[test]
fn case_results_keep_value_operator_semantics_and_repeat_at_runtime_consumer_sites() {
    let source = r#"
BYTE ARRAY out=$600
BYTE calls=$63E,done=$63F,i
BYTE FUNC Mark(BYTE n)
  calls==+1
RETURN(n)
BYTE FUNC Pick(BYTE n)
RETURN(CASE n OF
WHEN 0,2 THEN
  10
ELSE
  20
ESAC)
PROC Main()
  calls=0 out(0)=0
  IF CASE Mark(1) OF
  WHEN 1 THEN
    (Mark(0)=1) AND (Mark(2)=2)
  ELSE
    1
  ESAC THEN out(0)=99 FI
  out(1)=CASE 1 OF
  WHEN 1 IF (Mark(0)=1) AND (Mark(99)=99) THEN
    99
  ELSE
    7
  ESAC
  i=0
  WHILE CASE Mark(i) OF
  WHEN 0 TO 2 THEN
    1
  ELSE
    0
  ESAC DO
    out(2+i)=Pick(i)
    i==+1
  OD
  out(CASE i OF
  WHEN 3 THEN
    5
  ELSE
    99
  ESAC)=10
  out(5)==+CASE i OF
  WHEN 3 THEN
    2
  ELSE
    99
  ESAC
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..6].copy_from_slice(&[0, 7, 10, 20, 10, 12]);
    expected[0x3E] = 8;
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

#[test]
fn case_values_preserve_enum_fallback_signed_ranges_and_outer_conversions() {
    let source = r#"
TYPE E=ENUM [FIRST=17 LAST=255]
BYTE ARRAY out=$600
BYTE done=$63F
E state
INT n
PROC Main()
  state=E.FIRST
  state=CASE state OF
  WHEN E.FIRST THEN
    E.LAST
  ELSE
    E.FIRST
  ESAC
  out(0)=BYTE(state)
  state=E(99)
  LET BYTE narrowed=CASE state OF
  WHEN E.FIRST, E.LAST THEN
    CARD(5)
  ELSE
    CARD(258)
  ESAC
  out(1)=narrowed
  n=-257
  n=CASE n OF
  WHEN -300 TO -256 THEN
    -258
  ELSE
    INT(0)
  ESAC
  out(2)=BYTE(n)
  out(3)=BYTE(n RSH 8)
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..4].copy_from_slice(&[255, 2, 254, 254]);
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

#[test]
fn case_values_read_volatile_selectors_once_even_after_mutating_guards() {
    let program = semir(
        r#"
BYTE ARRAY out=$600
VOLATILE BYTE input=$610,left=$611,right=$612
BYTE done=$63F
BYTE FUNC Reject()
  input=9
RETURN(0)
PROC Main()
  input=1
  out(0)=CASE input OF
  WHEN 1 IF Reject() THEN
    right
  WHEN 1 THEN
    left
  ELSE
    right
  ESAC
  LET unused=CASE input OF
  WHEN 1 THEN
    left
  ELSE
    right
  ESAC
  done=$A5
  DO OD
RETURN
"#,
    );
    let mut expected = vec![0xCC; 0x500];
    expected[0x10] = 9;
    expected[0x3F] = 0xA5;
    support::check_semir_watched(
        &program,
        &expected,
        false,
        &[0x610, 0x611, 0x612],
        |path, events| {
            use actionc_vm::BusAccess::{Read, Write};
            let observed: Vec<_> = events
                .iter()
                .map(|e| (e.access, e.address, e.value))
                .collect();
            assert_eq!(
                observed,
                [
                    (Write, 0x610, 1),
                    (Read, 0x610, 1),
                    (Write, 0x610, 9),
                    (Read, 0x611, 0xCC),
                    (Read, 0x610, 9),
                    (Read, 0x612, 0xCC)
                ],
                "{path}"
            );
        },
    );
}

#[test]
fn mir_case_values_preserve_wide_labels_ranges_and_joins() {
    let program = semir(
        r#"
BYTE ARRAY out=$600
BYTE done=$63F
LONGCARD n
LONGINT signed
PROC Main()
  n=4294967295
  n=CASE n OF
  WHEN LONGCARD(2147483648) TO LONGCARD(4294967295) THEN
    LONGCARD(65537)
  ELSE
    LONGCARD(0)
  ESAC
  signed=-2147483648
  signed=CASE signed OF
  WHEN -2147483648 TO -65537 THEN
    LONGINT(-65537)
  ELSE
    LONGINT(0)
  ESAC
  out(0)=BYTE(n)
  out(1)=BYTE(n RSH 8)
  out(2)=BYTE(n RSH 16)
  out(3)=BYTE(n RSH 24)
  out(4)=BYTE(signed)
  out(5)=BYTE(signed RSH 8)
  out(6)=BYTE(signed RSH 16)
  out(7)=BYTE(signed RSH 24)
  done=$A5
  DO OD
RETURN
"#,
    );
    let mut expected = vec![0xCC; 0x500];
    expected[..8].copy_from_slice(&[1, 0, 1, 0, 255, 255, 254, 255]);
    expected[0x3F] = 0xA5;
    support::check_mir_semir(&program, &expected);
}

#[test]
fn if_values_preserve_enum_identity_signed_values_and_outer_narrowing() {
    let source = r#"
TYPE E=ENUM [FIRST=17 LAST=255]
BYTE ARRAY out=$600
BYTE done=$63F,flag
CARD c
INT n
E state
PROC Main()
  flag=1
  state=IF flag THEN E.LAST ELSE E.FIRST FI
  out(0)=BYTE(state)
  flag=0
  out(1)=BYTE(IF flag THEN E.LAST ELSE E.FIRST FI)
  c=IF flag THEN CARD(5) ELSE CARD(511) FI
  out(2)=BYTE(c)
  out(3)=BYTE(c RSH 8)
  n=IF flag THEN INT(42) ELSE -257 FI
  out(4)=BYTE(n)
  out(5)=BYTE(n RSH 8)
  LET BYTE narrowed=IF flag THEN CARD(5) ELSE CARD(258) FI
  out(6)=narrowed
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..7].copy_from_slice(&[255, 17, 255, 1, 255, 254, 2]);
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

#[test]
fn variant_case_values_capture_once_and_keep_binders_across_mutating_guards_and_results() {
    let source = r#"
TYPE MaybeByte=VARIANT [NONE SOME [BYTE value]]
MaybeByte current
BYTE ARRAY out=$600,trace=$680
BYTE calls=$63E,done=$63F
BYTE FUNC Mark(BYTE n)
  trace(calls)=n calls==+1
RETURN(n)
MaybeByte FUNC Read()
  Mark(10)
RETURN(current)
BYTE FUNC Reject(BYTE n)
  Mark(n) current=MaybeByte.NONE
RETURN(0)
BYTE FUNC Change()
  current=MaybeByte.SOME(99)
RETURN(Mark(1))
BYTE FUNC Sum(BYTE a,b)
RETURN(a+b)
PROC Main()
  USE ALL FROM MaybeByte
  calls=0
  current=SOME(7)
  out(0)=CASE Read() OF
  WHEN SOME(n) IF Reject(n) THEN
    Mark(255)
  WHEN SOME(n) IF Change() THEN
    Mark(n)
  ELSE
    Mark(254)
  ESAC
  out(1)=Sum(Mark(30),CASE current OF
  WHEN SOME(n) THEN
    Mark(12)
  WHEN NONE THEN
    Mark(253)
  ESAC)
  out(2)=CASE SOME(Mark(5)) OF
  WHEN SOME(n) THEN
    n+(CASE SOME(Mark(8)) OF
    WHEN SOME(n) THEN
      n
    ELSE
      0
    ESAC)+n
  ELSE
    0
  ESAC
  out(3)=Mark(40)+(CASE current OF
  WHEN SOME(n) THEN
    Mark(2)
  ELSE
    0
  ESAC)
  LET unused=CASE current OF
  WHEN SOME(n) THEN
    Mark(16)
  ELSE
    Mark(255)
  ESAC
  done=$A5
  DO OD
RETURN
"#;
    let trace = [10, 7, 1, 7, 30, 12, 5, 8, 40, 2, 16];
    let mut expected = vec![0xCC; 0x500];
    expected[..4].copy_from_slice(&[7, 42, 18, 42]);
    expected[0x80..0x80 + trace.len()].copy_from_slice(&trace);
    expected[0x3E] = trace.len() as u8;
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

#[test]
fn nested_variant_case_values_preserve_aggregate_binders_and_open_generic_constructors() {
    let source = r#"
TYPE Payload=[BYTE value]
TYPE Option<T>=VARIANT [NONE SOME [T value]]
TYPE Outer=VARIANT [EMPTY PAIR [Option<Payload> first,second]]
Outer current
Payload original
BYTE ARRAY out=$600
BYTE done=$63F
BYTE FUNC Change()
  original.value=99 current=Outer.EMPTY
RETURN(1)
BYTE FUNC Pick()
  USE ALL FROM Outer
RETURN(CASE current OF
WHEN PAIR(Option<Payload>.SOME(saved),Option<Payload>.NONE) IF Change() THEN
  saved.value
WHEN PAIR(first,second) THEN
  CASE first OF
  WHEN Option<Payload>.SOME(saved) THEN
    saved.value
  WHEN Option<Payload>.NONE THEN
    CASE second OF
    WHEN Option<Payload>.SOME(saved) THEN
      saved.value+10
    WHEN Option<Payload>.NONE THEN
      30
    ESAC
  ESAC
WHEN EMPTY THEN
  40
ESAC)
PROC Main()
  original.value=7
  current=Outer.PAIR(Option<Payload>.SOME(original),Option<Payload>.NONE)
  out(0)=Pick()
  original.value=8
  current=Outer.PAIR(Option<Payload>.NONE,Option<Payload>.SOME(original))
  out(1)=Pick()
  current=Outer.PAIR(Option<Payload>.NONE,Option<Payload>.NONE)
  out(2)=Pick()
  current=Outer.EMPTY
  out(3)=Pick()
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..4].copy_from_slice(&[7, 18, 30, 40]);
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

#[test]
fn variant_case_values_repeat_in_loops_indexes_and_guards_with_value_context_results() {
    let source = r#"
TYPE V=VARIANT [NONE SOME [BYTE value]]
V item
BYTE ARRAY out=$600
BYTE calls=$63E,done=$63F,i
BYTE FUNC Mark(BYTE n)
  calls==+1
RETURN(n)
PROC Main()
  USE ALL FROM V
  calls=0 out(0)=0 item=SOME(0)
  IF CASE item OF
  WHEN SOME(n) THEN
    (Mark(n)=1) AND (Mark(2)=2)
  ELSE
    1
  ESAC THEN out(0)=255 FI
  out(1)=CASE item OF
  WHEN SOME(n) IF (Mark(n)=1) AND (Mark(255)=255) THEN
    255
  WHEN _ IF CASE SOME(1) OF
  WHEN SOME(n) THEN
    n
  ELSE
    0
  ESAC THEN
    7
  ELSE
    255
  ESAC
  i=0
  WHILE CASE SOME(i) OF
  WHEN SOME(n) THEN
    n<3
  ELSE
    0
  ESAC DO
    out(2+i)=i
    i==+1
  OD
  out(CASE SOME(i) OF
  WHEN SOME(n) THEN
    n+2
  ELSE
    99
  ESAC)=10
  out(5)==+CASE SOME(2) OF
  WHEN SOME(n) THEN
    n
  ELSE
    99
  ESAC
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..6].copy_from_slice(&[0, 7, 0, 1, 2, 12]);
    expected[0x3E] = 3;
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

#[test]
fn invalid_variant_case_value_tags_fault_before_guards_results_and_user_else() {
    for tag in [0, 3, 255] {
        for arms in [
            "WHEN V.SOME(n) IF Mark(n) THEN\nn\nWHEN _ IF Mark(1) THEN\nMark(2)\nELSE\nMark(3)",
            "WHEN V.NONE THEN\nMark(1)\nWHEN V.SOME(n) THEN\nMark(n)",
            "WHEN V.NONE THEN\nMark(1)\nELSE\nMark(3)",
        ] {
            let source = format!(
                r#"
TYPE V=VARIANT [NONE SOME [BYTE value]]
V item
BYTE POINTER bytes
BYTE ARRAY out=$600
BYTE calls=$63E
BYTE FUNC Mark(BYTE n)
  calls==+1
RETURN(n)
PROC Main()
  calls=0 out(0)=41 item=V.SOME(7)
  bytes=BYTE POINTER(@item) bytes(0)={tag}
  out(1)=CASE item OF
  {arms}
  ESAC
  out(0)=42
  DO OD
RETURN
"#
            );
            let mut expected = vec![0xCC; 0x500];
            expected[0] = 41;
            expected[0x3E] = 0;
            support::check_with_fault(&source, &expected, true);
        }
    }
}

#[test]
fn nested_variant_case_values_validate_active_payloads_before_matching() {
    for (outer_tag, inner_tag, fault) in
        [(2, 0, true), (2, 3, true), (2, 1, false), (1, 255, false)]
    {
        let source = format!(
            r#"
TYPE Inner=VARIANT [NONE SOME [BYTE value]]
TYPE Outer=VARIANT [EMPTY WRAP [Inner value]]
Outer item
BYTE POINTER bytes
BYTE ARRAY out=$600
BYTE calls=$63E
BYTE FUNC Mark()
  calls==+1
RETURN(1)
PROC Main()
  calls=0 out(0)=41 item=Outer.WRAP(Inner.SOME(7))
  bytes=BYTE POINTER(@item) bytes(0)={outer_tag} bytes(1)={inner_tag}
  out(1)=CASE item OF
  WHEN _ IF Mark() THEN
    8
  ELSE
    9
  ESAC
  out(0)=42
  DO OD
RETURN
"#
        );
        let mut expected = vec![0xCC; 0x500];
        expected[0] = if fault { 41 } else { 42 };
        expected[0x3E] = if fault { 0 } else { 1 };
        if !fault {
            expected[1] = 8;
        }
        support::check_with_fault(&source, &expected, fault);
    }
}

#[test]
fn variant_case_values_preserve_volatile_constructor_reads_and_discarded_results() {
    let program = semir(
        r#"
TYPE V=VARIANT [NONE SOME [BYTE value]]
VOLATILE BYTE input=$610,left=$611,right=$612
BYTE ARRAY out=$600
BYTE done=$63F
BYTE FUNC Reject()
  input=9
RETURN(0)
PROC Main()
  input=7
  out(0)=CASE V.SOME(input) OF
  WHEN V.SOME(n) IF Reject() THEN
    right
  WHEN V.SOME(n) THEN
    n+left
  ELSE
    right
  ESAC
  LET unused=CASE V.NONE OF
  WHEN V.SOME(n) THEN
    left
  WHEN V.NONE THEN
    right
  ESAC
  done=$A5
  DO OD
RETURN
"#,
    );
    let mut expected = vec![0xCC; 0x500];
    expected[0] = 0xD3;
    expected[0x10] = 9;
    expected[0x3F] = 0xA5;
    support::check_semir_watched(
        &program,
        &expected,
        false,
        &[0x610, 0x611, 0x612],
        |path, events| {
            use actionc_vm::BusAccess::{Read, Write};
            let observed = events
                .iter()
                .map(|e| (e.access, e.address, e.value))
                .collect::<Vec<_>>();
            assert_eq!(
                observed,
                [
                    (Write, 0x610, 7),
                    (Read, 0x610, 7),
                    (Write, 0x610, 9),
                    (Read, 0x611, 0xCC),
                    (Read, 0x612, 0xCC)
                ],
                "{path}"
            );
        },
    );
}

#[test]
fn variant_case_expression_copies_allocate_scratch_without_statement_copies() {
    let source = r#"
TYPE Payload=[BYTE value]
TYPE V=VARIANT [NONE PAIR [Payload first,second]]
TYPE E=ENUM [FIRST=17 LAST=255]
Payload original
BYTE ARRAY out=$600
BYTE done=$63F
E result
PROC Main()
  original.value=7
  out(0)=CASE V.PAIR(original,original) OF
  WHEN V.PAIR(first,second) THEN
    first.value+second.value
  ELSE
    0
  ESAC
  result=CASE V.NONE OF
  WHEN V.PAIR(first,second) THEN
    E.FIRST
  WHEN V.NONE THEN
    E.LAST
  ESAC
  out(1)=BYTE(result)
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..2].copy_from_slice(&[14, 255]);
    expected[0x3F] = 0xA5;
    support::check(source, &expected);
}

#[test]
fn mir_variant_case_values_join_wide_payloads_and_keep_selected_faults_terminal() {
    let program = semir(
        r#"
TYPE Option<T>=VARIANT [NONE SOME [T value]]
BYTE ARRAY out=$600
BYTE done=$63F
LONGINT signed
LONGCARD unsigned
PROC Main()
  signed=CASE Option<LONGINT>.SOME(-65537) OF
  WHEN Option<LONGINT>.SOME(n) THEN
    n
  WHEN Option<LONGINT>.NONE THEN
    LONGINT(0)
  ESAC
  unsigned=CASE Option<LONGCARD>.SOME(65537) OF
  WHEN Option<LONGCARD>.SOME(n) THEN
    n
  WHEN Option<LONGCARD>.NONE THEN
    LONGCARD(0)
  ESAC
  out(0)=BYTE(signed) out(1)=BYTE(signed RSH 8)
  out(2)=BYTE(signed RSH 16) out(3)=BYTE(signed RSH 24)
  out(4)=BYTE(unsigned) out(5)=BYTE(unsigned RSH 8)
  out(6)=BYTE(unsigned RSH 16) out(7)=BYTE(unsigned RSH 24)
  done=$A5
  DO OD
RETURN
"#,
    );
    let mut expected = vec![0xCC; 0x500];
    expected[..8].copy_from_slice(&[255, 255, 254, 255, 1, 0, 1, 0]);
    expected[0x3F] = 0xA5;
    support::check_mir_semir(&program, &expected);

    let source = "TYPE V=VARIANT [NONE SOME [BYTE value]] BYTE ARRAY out=$600 BYTE zero\nPROC Main()\nzero=0 out(0)=41\nout(1)=CASE V.SOME(7) OF\nWHEN V.SOME(n) THEN\nn/zero\nWHEN V.NONE THEN\n0\nESAC\nout(0)=42\nDO OD\nRETURN";
    let mut expected = vec![0xCC; 0x500];
    expected[0] = 41;
    support::check_with_error_code(source, &expected, Some(101));
}
