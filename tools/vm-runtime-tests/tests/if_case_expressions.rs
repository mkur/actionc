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
