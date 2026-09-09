#[path = "support/variant_values.rs"]
mod support;

fn expected(prefix: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xCC; 0x500];
    bytes[..prefix.len()].copy_from_slice(prefix);
    bytes
}

fn semir(source: &str) -> actionc::semantic::ir::SemProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    actionc::semantic::ir::lower_program(&ast, &model)
}

#[test]
fn fresh_nullary_payload_and_nested_constructors_choose_the_original_arms() {
    let source = r#"
TYPE Inner=VARIANT [OFF ON [BYTE n]]
TYPE Value=VARIANT [NONE SOME [BYTE n] NEST [Inner child]]
BYTE ARRAY output=$600
PROC Main()
 LET empty=Value.NONE
 CASE empty OF
 WHEN Value.NONE THEN
 output(0)=1
 ELSE
 output(0)=255
 ESAC
 LET item=Value.SOME(42)
 CASE item OF
 WHEN Value.SOME(n) THEN
 output(1)=n
 ELSE
 output(1)=254
 ESAC
 LET nested=Value.NEST(Inner.ON(7))
 CASE nested OF
 WHEN Value.NEST(Inner.ON(n)) THEN
 output(2)=n
 ELSE
 output(2)=253
 ESAC
 output(3)=99
 DO OD
RETURN
"#;
    support::check(source, &expected(&[1, 42, 7, 99]));
}

#[test]
fn false_mutating_guards_preserve_captured_selector_and_payload_values() {
    let source = r#"
TYPE Value=VARIANT [NONE SOME [BYTE n]]
Value current
BYTE ARRAY output=$600
BYTE guards=$610
BYTE FUNC Reject()
 guards==+1 current=Value.NONE
RETURN(0)
PROC Main()
 guards=0
 LET fresh=Value.SOME(42)
 CASE fresh OF
 WHEN Value.SOME(n) IF Reject() THEN
 output(0)=255
 WHEN Value.SOME(n) THEN
 output(0)=n
 ELSE
 output(0)=254
 ESAC
 current=Value.SOME(9)
 LET saved=current
 current=Value.NONE
 CASE saved OF
 WHEN Value.SOME(n) IF Reject() THEN
 output(1)=253
 WHEN Value.SOME(n) THEN
 output(1)=n
 ELSE
 output(1)=252
 ESAC
 output(2)=99
 DO OD
RETURN
"#;
    let mut oracle = expected(&[42, 9, 99]);
    oracle[0x10] = 2;
    support::check(source, &oracle);
}

#[test]
fn repeated_constructor_initialization_and_loop_branches_do_not_reuse_old_payloads() {
    let source = r#"
TYPE Value=VARIANT [NONE SOME [BYTE n]]
BYTE ARRAY output=$600
BYTE i
PROC Main()
 FOR i=0 TO 3 DO
  BEGIN
   LET item=Value.SOME(i)
   CASE item OF
   WHEN Value.SOME(n) THEN
 output(i)=n
   ELSE
 output(i)=255
   ESAC
   IF i<>1 THEN output(8)=i FI
  END
 OD
 output(9)=99
 DO OD
RETURN
"#;
    let mut oracle = expected(&[0, 1, 2, 3]);
    oracle[8..10].copy_from_slice(&[3, 99]);
    support::check(source, &oracle);
}

#[test]
fn volatile_payload_reads_and_guard_writes_execute_once_in_source_order() {
    let source = r#"
TYPE Value=VARIANT [NONE SOME [BYTE n]]
VOLATILE BYTE input=$610
BYTE ARRAY output=$600
BYTE FUNC Reject() input=9 RETURN(0)
PROC Main()
 input=7
 LET item=Value.SOME(input)
 CASE item OF
 WHEN Value.SOME(n) IF Reject() THEN
 output(0)=255
 WHEN Value.SOME(n) THEN
 output(0)=n
 ELSE
 output(0)=254
 ESAC
 output(1)=input
 DO OD
RETURN
"#;
    let mut oracle = expected(&[7, 9]);
    oracle[0x10] = 9;
    support::check_semir_watched(&semir(source), &oracle, false, &[0x610], |path, events| {
        use actionc_vm::BusAccess::{Read, Write};
        let observed: Vec<_> = events
            .iter()
            .map(|e| (e.access, e.address, e.value))
            .collect();
        assert_eq!(
            observed,
            [
                (Write, 0x610, 7),
                (Read, 0x610, 7),
                (Write, 0x610, 9),
                (Read, 0x610, 9)
            ],
            "{path}"
        );
    });
}

#[test]
fn faults_before_and_after_construction_stop_even_when_error_returns() {
    for before in [true, false] {
        let initialize = "LET item=Value.SOME(42)";
        let fail = "output(1)=10/zero";
        let source = format!(
            "TYPE Value=VARIANT [NONE SOME [BYTE n]] BYTE zero BYTE ARRAY output=$600 PROC Main() zero=0 output(0)=41 {} {}\nCASE item OF\nWHEN Value.SOME(n) THEN
 output(2)=n\nELSE
 output(2)=255\nESAC\noutput(0)=42 DO OD RETURN",
            if before { fail } else { initialize },
            if before { initialize } else { fail }
        );
        support::check_with_error_code(&source, &expected(&[41]), Some(101));
    }
}

#[test]
fn unknown_nested_tags_fault_before_outer_publication_and_guard_effects() {
    let source = r#"
TYPE Inner=VARIANT [OFF ON]
TYPE Outer=VARIANT [NONE SOME [Inner child]]
Inner corrupt
BYTE POINTER raw
BYTE ARRAY output=$600
BYTE FUNC Guard() output(2)=99 RETURN(1)
PROC Main()
 raw=BYTE POINTER(@corrupt) raw(0)=255 output(0)=41
 LET item=Outer.SOME(corrupt)
 CASE item OF
 WHEN Outer.SOME(_) IF Guard() THEN
 output(1)=1
 ELSE
 output(1)=2
 ESAC
 output(0)=42
 DO OD
RETURN
"#;
    support::check_with_fault(source, &expected(&[41]), true);
}
