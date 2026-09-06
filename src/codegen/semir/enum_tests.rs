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

#[test]
fn enum_routines_and_indirect_results_execute_with_byte_abi() {
    let source = r#"
TYPE E=ENUM [ZERO=0 TOP=255]
BYTE input=$0600,count=$0601,selected=$0602,converted=$0604
E result=$0603
E FUNC Echo(E value)
  count==+1
RETURN(value)
E FUNC Read()
  count==+1
RETURN(E(input))
E FUNC POINTER callback
PROC Main()
  count=0
  callback=@Read
  result=Echo(Echo(callback()))
  converted=BYTE(result)
  CASE Echo(result) OF
  WHEN E.ZERO THEN
    selected=1
  WHEN E.TOP THEN
    selected=2
  ELSE
    selected=3
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
            let memory = execute(&output, |memory| memory[0x600] = input);
            assert_eq!(
                &memory[0x600..0x605],
                &[
                    input,
                    4,
                    match input {
                        0 => 1,
                        255 => 2,
                        _ => 3,
                    },
                    input,
                    input
                ],
                "{mode}/{input}"
            );
        }
    }
}

#[test]
fn enum_arrays_records_and_pointers_execute_without_neighbor_writes() {
    let source = r#"
TYPE E=ENUM [OFF=0 ON=17]
TYPE Nonzero=ENUM [ONE=1]
Nonzero ARRAY zeros(2)
CONST Alias=E.ON
TYPE Pair=[E first,second]
TYPE Packet=[BYTE ARRAY pad(257) E state E ARRAY states(2) BYTE tail]
Pair defaults=[E.ON E.OFF]
Packet data=$5001
E initial=[Alias]
E ARRAY table(3)=[E.ON E.OFF Alias]
E POINTER ep
BYTE input=$0600,result=$0601,initialized=$0602,localResult=$0603,zeroResult=$0604
PROC SetFirst(E ARRAY a E item)
  a(0)=item
RETURN
PROC Main()
  E local=[Alias]
  data.state=E(input)
  ep=@data.states
  ep^=table(0)
  SetFirst(data.states,E.OFF)
  data.states(1)=defaults.first
  result=BYTE(data.state)
  initialized=BYTE(initial)+BYTE(table(2))
  localResult=BYTE(local)
  zeroResult=BYTE(zeros(1))
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
            let memory = execute(&output, |memory| {
                memory[0x5000..0x5107].fill(0xA5);
                memory[0x600] = input;
            });
            let mut expected = [0xA5u8; 0x107];
            expected[0x102..0x105].copy_from_slice(&[input, 0, 17]);
            assert_eq!(&memory[0x5000..0x5107], &expected, "{mode}/{input}");
            assert_eq!(
                &memory[0x600..0x605],
                &[input, input, 34, 17, 0],
                "{mode}/{input}"
            );
        }
    }
}

#[test]
fn enum_modules_execute_after_selective_linking_and_keep_type_metadata() {
    use crate::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use crate::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), b"MODULE App USE Lib AS API API.E value=[API.E.ON] BYTE out=$0600 PROC Main() out=BYTE(API.Echo(value)) RETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/lib.act"), b"MODULE Lib PUBLIC TYPE E=ENUM [OFF ON=17] PUBLIC E FUNC Echo(E arg) RETURN(arg) BYTE FUNC Unused() RETURN(99) ENDMODULE".to_vec());
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model = crate::semantic::analyze_compilation_with_options(
        &loaded,
        SemanticOptions {
            enum_types: true,
            ..SemanticOptions::modern()
        },
    )
    .unwrap();
    let ir = crate::semantic::ir::lower_compilation(&loaded, &model);
    let selected =
        crate::linker::select_semir(&ir, crate::linker::SemLinkPolicy::EntryReachable).unwrap();
    assert!(selected.modules.iter().flat_map(|m| &m.items).any(|item| matches!(item, crate::semantic::ir::SemItem::Declaration(d) if matches!(d.storage, crate::semantic::ir::SemDeclarationStorage::Enum { .. }))));
    assert!(!selected.modules.iter().flat_map(|m| &m.items).any(|item| matches!(item, crate::semantic::ir::SemItem::Routine(r) if r.symbol.name.ends_with("Unused"))));
    for (mode, output) in super::array_execution_tests::outputs_from_semir(&selected) {
        let memory = execute(&output, |_| {});
        assert_eq!(memory[0x600], 17, "{mode}");
        assert!(
            output
                .map
                .storage_symbols
                .iter()
                .any(|symbol| symbol.name.to_ascii_lowercase().contains("value"))
        );
    }
}
