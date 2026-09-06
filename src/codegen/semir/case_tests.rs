use super::array_execution_tests::{execute, outputs_with_options};
use crate::semantic::SemanticOptions;

fn options() -> SemanticOptions {
    SemanticOptions {
        case_statements: true,
        ..SemanticOptions::modern()
    }
}

#[test]
fn case_dispatch_captures_once_and_executes_all_byte_inputs_in_both_backends_and_runtimes() {
    let source = r#"
BYTE input=$0600,result=$0601,calls=$0602
BYTE FUNC Selector()
  calls==+1
RETURN(input)
PROC Mark() result=40 RETURN
PROC Main()
  calls=0 result=77
  CASE Selector() OF
  WHEN 0 THEN
    result=10
  WHEN 1,3 THEN
    IF input=1 THEN result=20 ELSE result=30 FI
  WHEN 4 THEN
    Mark()
  ELSE
    result=90
  ESAC
RETURN
"#;
    for (mode, output) in outputs_with_options(source, options()) {
        for input in 0..=255u8 {
            let memory = execute(&output, |memory| memory[0x600] = input);
            let expected = match input {
                0 => 10,
                1 => 20,
                3 => 30,
                4 => 40,
                _ => 90,
            };
            assert_eq!(
                (memory[0x601], memory[0x602]),
                (expected, 1),
                "{mode}/{input}"
            );
        }
    }
}

#[test]
fn case_nested_blocks_and_exit_preserve_loop_target_and_local_scope() {
    let source = r#"
BYTE input=$0600,result=$0601,iteration=$0602
PROC Main()
  FOR iteration=0 TO 2 DO
    CASE iteration OF
    WHEN 0 THEN
      CASE input OF
      WHEN 0 THEN
        BEGIN
          BYTE value
          value=11
          result=value
        END
      ELSE
        result=12
      ESAC
    WHEN 1 THEN
      EXIT
    ELSE
      result=99
    ESAC
  OD
RETURN
"#;
    for (mode, output) in outputs_with_options(source, options()) {
        for input in [0, 1] {
            let memory = execute(&output, |memory| memory[0x600] = input);
            assert_eq!(
                (memory[0x601], memory[0x602]),
                (11 + input, 1),
                "{mode}/{input}"
            );
        }
    }
}

#[test]
fn case_word_selectors_returns_and_missing_default_execute() {
    for (ty, first, first_bits) in [("INT", "-1", 65535u16), ("CARD", "$FFFF", 65535)] {
        let source = format!(
            r#"
{ty} input=$0600
BYTE result=$0602,untouched=$0603
BYTE FUNC Select()
  CASE input OF
  WHEN {first} THEN
    RETURN(11)
  WHEN 0 THEN
    RETURN(12)
  ELSE
    RETURN(13)
  ESAC
PROC Main()
  result=Select() untouched=71
  CASE input OF
  WHEN {first} THEN
    untouched=72
  ESAC
RETURN
"#
        );
        for (mode, output) in outputs_with_options(&source, options()) {
            for input in [first_bits, 0, 32768, 255, 256] {
                let memory = execute(&output, |memory| {
                    memory[0x600..0x602].copy_from_slice(&input.to_le_bytes())
                });
                assert_eq!(
                    memory[0x602],
                    if input == first_bits {
                        11
                    } else if input == 0 {
                        12
                    } else {
                        13
                    },
                    "{ty}/{mode}/{input}"
                );
                assert_eq!(
                    memory[0x603],
                    if input == first_bits { 72 } else { 71 },
                    "{ty}/{mode}/{input}"
                );
            }
        }
    }
}

#[test]
fn case_linking_retains_calls_that_only_appear_in_arms() {
    let source = "BYTE input\nPROC Used() RETURN\nPROC Unused() RETURN\nPROC Main()\nCASE input OF\nWHEN 0 THEN\nUsed()\nELSE\nUsed()\nESAC\nRETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(&ast, options()).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let linked =
        crate::linker::select_semir(&semir, crate::linker::SemLinkPolicy::EntryReachable).unwrap();
    let names = linked
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| {
            if let crate::semantic::ir::SemItem::Routine(routine) = item {
                Some(routine.symbol.name.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(names.contains(&"Used"));
    assert!(!names.contains(&"Unused"));
}
