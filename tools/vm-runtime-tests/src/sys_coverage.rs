use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const EXECUTED: &[(&str, &str)] = &[
    ("Zero", "standalone_sys_memory.act"),
    ("SetBlock", "standalone_sys_blocks.act"),
    ("MoveBlock", "standalone_sys_blocks.act"),
    ("SCompare", "standalone_sys_strings_runtime.act"),
    ("SCopy", "standalone_sys_strings_runtime.act"),
    ("SCopyS", "standalone_sys_strings_runtime.act"),
    ("SAssign", "resident_memory_strings.act"),
    ("StrB", "resident_numeric_strings.act"),
    ("StrC", "resident_numeric_strings.act"),
    ("StrI", "resident_numeric_strings.act"),
    ("Rand", "resident_hardware_helpers.act"),
    ("Sound", "resident_hardware_helpers.act"),
    ("SndRst", "resident_hardware_helpers.act"),
    ("Paddle", "resident_hardware_helpers.act"),
    ("PTrig", "resident_hardware_helpers.act"),
    ("Stick", "resident_hardware_helpers.act"),
    ("STrig", "resident_hardware_helpers.act"),
    ("Peek", "resident_memory_strings.act"),
    ("PeekC", "resident_memory_strings.act"),
    ("Poke", "resident_memory_strings.act"),
    ("PokeC", "resident_memory_strings.act"),
    ("Error", "resident_error.act"),
    ("Break", "resident_break.act"),
    ("Graphics", "resident_graphics_io.act"),
    ("Position", "standalone_sys_graphics_runtime.act"),
    ("DrawTo", "resident_graphics_io.act"),
    ("Locate", "resident_graphics_io.act"),
    ("Plot", "resident_graphics_io.act"),
    ("SetColor", "resident_graphics_io.act"),
    ("Fill", "resident_graphics_io.act"),
    ("Put", "resident_numeric_output.act"),
    ("PutE", "resident_printh.act"),
    ("PutD", "resident_formatted_output.act"),
    ("PutDE", "resident_device_io.act"),
    ("Print", "standalone_sys_output_runtime.act"),
    ("PrintE", "resident_formatted_output.act"),
    ("PrintD", "resident_device_io.act"),
    ("PrintDE", "resident_formatted_output.act"),
    ("PrintF", "resident_formatted_output.act"),
    ("PrintH", "resident_printh.act"),
    ("GetD", "resident_console_input.act"),
    ("InputS", "resident_console_input.act"),
    ("InputSD", "resident_console_input.act"),
    ("InputMD", "resident_console_input.act"),
    ("InputD", "resident_console_input.act"),
    ("Open", "resident_device_io.act"),
    ("Close", "resident_device_io.act"),
    ("XIO", "resident_device_io.act"),
    ("Note", "resident_device_io.act"),
    ("Point", "resident_device_io.act"),
    ("PrintB", "resident_numeric_output.act"),
    ("PrintBE", "resident_numeric_output.act"),
    ("PrintBD", "resident_numeric_output.act"),
    ("PrintBDE", "resident_numeric_output.act"),
    ("PrintC", "resident_numeric_output.act"),
    ("PrintCE", "resident_numeric_output.act"),
    ("PrintCD", "resident_numeric_output.act"),
    ("PrintCDE", "resident_numeric_output.act"),
    ("PrintI", "resident_numeric_output.act"),
    ("PrintIE", "resident_numeric_output.act"),
    ("PrintID", "resident_numeric_output.act"),
    ("PrintIDE", "resident_numeric_output.act"),
    ("InputB", "resident_console_input.act"),
    ("InputBD", "resident_console_input.act"),
    ("InputC", "resident_console_input.act"),
    ("InputCD", "resident_console_input.act"),
    ("InputI", "resident_console_input.act"),
    ("InputID", "resident_console_input.act"),
    ("ValB", "resident_numeric_values.act"),
    ("ValC", "resident_numeric_values.act"),
    ("ValI", "resident_numeric_values.act"),
];

const DEFERRED: &[(&str, &str)] = &[];

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sys_interface_names(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| {
            let words = line.split_ascii_whitespace().collect::<Vec<_>>();
            let routine = words.iter().position(|word| {
                word.eq_ignore_ascii_case("PROC") || word.eq_ignore_ascii_case("FUNC")
            })?;
            (words.get(..routine) == Some(&["PUBLIC", "EXTERNAL"][..])
                || words.get(..routine).is_some_and(|prefix| {
                    prefix.len() == 3
                        && prefix[0].eq_ignore_ascii_case("PUBLIC")
                        && prefix[1].eq_ignore_ascii_case("EXTERNAL")
                }))
            .then(|| {
                words[routine + 1]
                    .split('(')
                    .next()
                    .expect("routine name")
                    .to_ascii_uppercase()
            })
        })
        .collect()
}

fn source_calls(source: &str, routine: &str) -> bool {
    let source = source.as_bytes();
    let routine = routine.as_bytes();
    source
        .windows(routine.len())
        .enumerate()
        .any(|(start, word)| {
            if !word.eq_ignore_ascii_case(routine) {
                return false;
            }
            let before_is_identifier = start > 0
                && (source[start - 1].is_ascii_alphanumeric() || source[start - 1] == b'_');
            if before_is_identifier {
                return false;
            }
            source[start + routine.len()..]
                .iter()
                .find(|byte| !byte.is_ascii_whitespace())
                == Some(&b'(')
        })
}

#[test]
fn every_public_sys_routine_has_vm_execution_or_an_explicit_deferral() {
    let root = repository_root();
    let interface_source = std::fs::read_to_string(root.join("embedded/modules/sys.act"))
        .expect("read embedded SYS interface");
    let interface = sys_interface_names(&interface_source);
    let harness_source = include_str!("lib.rs");
    let mut inventory = BTreeMap::new();

    for (routine, fixture) in EXECUTED {
        let key = routine.to_ascii_uppercase();
        assert!(
            inventory
                .insert(key, format!("fixture {fixture}"))
                .is_none(),
            "duplicate coverage entry for SYS.{routine}"
        );
        let fixture_path = root.join("fixtures/runtime").join(fixture);
        let source = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));
        assert!(
            source_calls(&source, routine),
            "{fixture} does not call SYS.{routine}"
        );
        assert!(
            harness_source.contains(&format!("\"{fixture}\"")),
            "{fixture} is not referenced by the VM execution harness"
        );
    }

    for (routine, reason) in DEFERRED {
        assert!(!reason.trim().is_empty(), "SYS.{routine} needs a reason");
        assert!(
            inventory
                .insert(routine.to_ascii_uppercase(), format!("deferred: {reason}"))
                .is_none(),
            "duplicate coverage entry for SYS.{routine}"
        );
    }

    assert_eq!(
        inventory.keys().cloned().collect::<BTreeSet<_>>(),
        interface,
        "update the SYS VM execution coverage ledger"
    );
}
