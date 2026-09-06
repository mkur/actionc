use super::array_execution_tests::{execute, outputs_with_options};
use crate::semantic::SemanticOptions;

#[test]
fn enum_values_casts_comparisons_and_case_execute_for_all_bytes() {
    let source = r#"
TYPE Code=ENUM [LOW=0 MID=128 HIGH=255]
CONST Alias=Code.MID
CONST Code Unnamed=Code(17)
Code value=$0600,copy=$0601
BYTE result=$0602,comparison=$0603,converted=$0604,raw=$0605
CARD widened=$0606
BYTE constantResult=$0608
PROC Main()
  value=Code(raw)
  copy=value
  converted=BYTE(value)
  widened=CARD(value)
  comparison=value>=Code.MID
  constantResult=BYTE(Unnamed)
  CASE value OF
  WHEN Code.LOW, Alias THEN
    result=1
  WHEN Code.HIGH THEN
    result=2
  ELSE
    result=3
  ESAC
RETURN
"#;
    for (mode, output) in outputs_with_options(
        source,
        SemanticOptions {
            enum_types: true,
            ..SemanticOptions::modern()
        },
    ) {
        for input in 0..=255u8 {
            let memory = execute(&output, |memory| memory[0x605] = input);
            assert_eq!(
                &memory[0x600..0x609],
                &[
                    input,
                    input,
                    match input {
                        0 | 128 => 1,
                        255 => 2,
                        _ => 3,
                    },
                    u8::from(input >= 128),
                    input,
                    input,
                    input,
                    0,
                    17
                ],
                "{mode}/{input}"
            );
        }
    }
}

#[test]
fn enum_wide_and_signed_conversions_keep_all_representation_values_defined() {
    let source = r#"
TYPE Code=ENUM [ZERO=0 HIGH=255]
CONST Code Negative=Code(-1),Wrapped=Code(256)
INT input=$0600,widened=$0604
Code value=$0602
BYTE constantHigh=$0606,constantZero=$0607,matched=$0608
PROC Main()
  value=Code(input)
  widened=INT(value)
  constantHigh=BYTE(Negative) constantZero=BYTE(Wrapped)
  matched=71
  CASE value OF
  WHEN Code.ZERO THEN
    matched=72
  WHEN Code.HIGH THEN
    matched=73
  ESAC
RETURN
"#;
    for (mode, output) in outputs_with_options(
        source,
        SemanticOptions {
            enum_types: true,
            ..SemanticOptions::modern()
        },
    ) {
        for input in [0u16, 1, 17, 128, 255, 256, 257, 32767, 32768, 65534, 65535] {
            let memory = execute(&output, |memory| {
                memory[0x600..0x602].copy_from_slice(&input.to_le_bytes());
                memory[0x603] = 0xCC;
            });
            assert_eq!(
                &memory[0x602..0x609],
                &[
                    input as u8,
                    0xCC,
                    input as u8,
                    0,
                    255,
                    0,
                    match input as u8 {
                        0 => 72,
                        255 => 73,
                        _ => 71,
                    }
                ],
                "{mode}/{input}"
            );
        }
    }
}

#[test]
fn enum_constant_materialization_preserves_defining_scope_under_shadowing() {
    let source = "TYPE E=ENUM [A=1]\nCONST Original=E.A\nPROC Main()\nBEGIN\nTYPE E=ENUM [A=2]\nCONST Local=E.A\nBYTE result\nresult=BYTE(Original)+BYTE(Local)\nEND\nRETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let options = SemanticOptions {
        enum_types: true,
        ..SemanticOptions::modern()
    };
    let model = crate::semantic::analyze_with_options(&ast, options).unwrap();
    let materialized = crate::semantic::materialize::materialize_constants(&ast, &model);
    let model = crate::semantic::analyze_with_options(&materialized, options).unwrap();
    let mut values = model
        .enums
        .constants
        .values()
        .map(|value| value.bits)
        .collect::<Vec<_>>();
    values.sort();
    assert_eq!(values, [1, 2]);
    let identities = model
        .enums
        .constants
        .values()
        .map(|value| value.identity.symbol)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(identities.len(), 2);
}
