#[path = "support/variant_values.rs"]
mod support;
use actionc_vm::{BusAccess, BusEvent};

fn semir(source: &str) -> actionc::semantic::ir::SemProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let mut options = actionc::semantic::SemanticOptions::modern();
    options.algebraic_types.unions = true;
    let model = actionc::semantic::analyze_with_options(&ast, options).unwrap();
    actionc::semantic::ir::lower_program(&ast, &model)
}

#[test]
fn volatile_union_aliases_preserve_selected_widths_skipped_reads_and_compound_order() {
    let source = r#"
TYPE View=UNION [CARD word BYTE low BYTE ARRAY bytes(3)]
VOLATILE View device=$680
View alias=device
BYTE sink=$600
CARD total=$601
PROC Main()
 device.bytes(0)=1 device.bytes(1)=2 device.bytes(2)=3
 total=alias.word total=alias.word
 alias.low==+1
 IF 0=1 AND alias.low=0 THEN sink=99 FI
 IF 1=1 OR alias.word=0 THEN sink=7 FI
 alias.bytes(1)=9
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..3].copy_from_slice(&[7, 1, 2]);
    expected[0x80..0x83].copy_from_slice(&[2, 9, 3]);
    support::check_semir_watched(
        &semir(source),
        &expected,
        false,
        &[0x680, 0x681, 0x682],
        |path, events| {
            let actual: Vec<_> = events
                .iter()
                .map(|e| (e.access, e.address, e.value))
                .collect();
            use BusAccess::{Read as R, Write as W};
            assert_eq!(actual.len(), 10, "{path}: {actual:?}");
            assert_eq!(
                actual[..3],
                [(W, 0x680, 1), (W, 0x681, 2), (W, 0x682, 3)],
                "{path}"
            );
            // Existing backends may load a word high-first or low-first. Require
            // each selected byte once per source read, without imposing atomicity.
            for chunk in actual[3..7].chunks_exact(2) {
                assert!(
                    chunk.contains(&(R, 0x680, 1)) && chunk.contains(&(R, 0x681, 2)),
                    "{path}: {chunk:?}"
                );
            }
            assert_eq!(
                actual[7..],
                [(R, 0x680, 1), (W, 0x680, 2), (W, 0x681, 9)],
                "{path}"
            );
        },
    );
}

#[test]
fn volatile_union_snapshots_and_copies_access_the_complete_extent_once() {
    let source = r#"
TYPE View=UNION [CARD word BYTE ARRAY bytes(3)]
VOLATILE View source=$680,destination=$690
View alias=source
BYTE phase=$610
BYTE result=$600
PROC Main()
 source.bytes(0)=1 source.bytes(1)=2 source.bytes(2)=3
 phase=1
 LET saved=alias
 phase=2
 source.word=$9080
 phase=3
 destination=saved
 phase=4
 destination=alias
 phase=5
 result=saved.bytes(0)
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[0] = 1;
    expected[0x10] = 5;
    expected[0x80..0x83].copy_from_slice(&[0x80, 0x90, 3]);
    expected[0x90..0x93].copy_from_slice(&[0x80, 0x90, 3]);
    support::check_semir_watched(
        &semir(source),
        &expected,
        false,
        &[0x610, 0x680, 0x681, 0x682, 0x690, 0x691, 0x692],
        |path, events| {
            let mut phases: Vec<Vec<&BusEvent>> = vec![vec![]; 6];
            let mut phase = 0;
            for event in events {
                if event.address == 0x610 {
                    assert_eq!(event.access, BusAccess::Write, "{path}");
                    phase = usize::from(event.value);
                } else {
                    phases[phase].push(event);
                }
            }
            use BusAccess::{Read as R, Write as W};
            let expected = [
                vec![(W, 0x680, 1), (W, 0x681, 2), (W, 0x682, 3)],
                vec![(R, 0x680, 1), (R, 0x681, 2), (R, 0x682, 3)],
                vec![(W, 0x680, 0x80), (W, 0x681, 0x90)],
                vec![(W, 0x690, 1), (W, 0x691, 2), (W, 0x692, 3)],
                vec![
                    (R, 0x680, 0x80),
                    (R, 0x681, 0x90),
                    (R, 0x682, 3),
                    (W, 0x690, 0x80),
                    (W, 0x691, 0x90),
                    (W, 0x692, 3),
                ],
                vec![],
            ];
            for (phase, (events, expected)) in phases.iter().zip(expected).enumerate() {
                let mut actual: Vec<_> = events
                    .iter()
                    .map(|e| (e.access == W, e.address, e.value))
                    .collect();
                actual.sort();
                let mut expected: Vec<_> = expected
                    .into_iter()
                    .map(|(a, b, c)| (a == W, b, c))
                    .collect();
                expected.sort();
                assert_eq!(actual, expected, "{path}, phase {phase}");
            }
        },
    );
}

#[test]
fn absolute_union_aliases_and_static_member_addresses_are_storage_bindings() {
    let source = r#"
TYPE View=UNION [CARD word BYTE ARRAY bytes(3)]
View original=$680
View alias=original
BYTE POINTER member=[@original.bytes(1)]
View ARRAY views(2)=$690
BYTE ARRAY output=$600
PROC Main()
 original.word=$1234 original.bytes(2)=7
 member^=$56 alias.bytes(0)=$78
 views(0)=alias views(1)=views(0)
 output(0)=original.bytes(0) output(1)=original.bytes(1)
 output(2)=views(1).bytes(2)
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..3].copy_from_slice(&[0x78, 0x56, 7]);
    expected[0x80..0x83].copy_from_slice(&[0x78, 0x56, 7]);
    expected[0x90..0x96].copy_from_slice(&[0x78, 0x56, 7, 0x78, 0x56, 7]);
    support::check_semir(&semir(source), &expected, false);
}
