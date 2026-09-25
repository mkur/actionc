use std::path::{Path, PathBuf};
use std::process::Command;

use actionc::compiler::{CompileMode, CompileOptions, compile_file};
use actionc::runtime::Runtime;

fn contract_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("listing")
        .join("mads_contract.act")
}

fn reorigin_contract_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("listing")
        .join("mads_reorigin_contract.act")
}

fn first_load_segment(object: &[u8]) -> (u16, &[u8]) {
    assert!(object.len() >= 6, "load file is missing its first segment");
    assert_eq!(&object[..2], &[0xFF, 0xFF], "load file header");
    let start = u16::from_le_bytes([object[2], object[3]]);
    let end = u16::from_le_bytes([object[4], object[5]]);
    let len = usize::from(end.wrapping_sub(start)) + 1;
    assert!(object.len() >= 6 + len, "first load segment is truncated");
    (start, &object[6..6 + len])
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn emit_listing(profile: &str, backend: &str, mode: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg("--profile")
        .arg(profile)
        .arg("--backend")
        .arg(backend)
        .arg(mode)
        .arg(contract_fixture())
        .output()
        .unwrap_or_else(|error| panic!("run actionc-emit {mode}: {error}"));
    assert!(
        output.status.success(),
        "actionc-emit {mode} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("listing must be UTF-8")
}

fn assembly_statements(listing: &str) -> Vec<&str> {
    listing
        .lines()
        .filter(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with(';')
        })
        .collect()
}

fn synthetic_label_definitions(listing: &str) -> Vec<&str> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("loc_") && line.ends_with(':'))
        .collect()
}

fn has_address_derived_synthetic_label(listing: &str) -> bool {
    listing.lines().map(str::trim).any(|line| {
        line.strip_prefix('L')
            .and_then(|line| line.strip_suffix(':'))
            .is_some_and(|address| {
                !address.is_empty() && address.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    })
}

#[test]
fn contract_fixture_covers_mads_sensitive_encodings_in_every_mode() {
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(contract_fixture(), &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile MADS contract fixture in {mode:?}: {error}"));
        let bytes = compiled.object_bytes();

        for expected in [
            &[0xAD, 0x58, 0x00][..], // LDA absolute below $0100
            &[0xA5, 0x58][..],       // LDA zero page
            &[0xBD, 0x58, 0x00][..], // LDA absolute,X below $0100
            &[0xB6, 0x58][..],       // LDX zero page,Y
            &[0xA1, 0x80][..],       // LDA (zp,X)
            &[0xB1, 0x80][..],       // LDA (zp),Y
            &[0xCA, 0xD0, 0xFD][..], // backward relative branch
            &[0x20, 0x6C, 0xA4][..], // external absolute JSR
        ] {
            assert!(
                bytes
                    .windows(expected.len())
                    .any(|window| window == expected),
                "{mode:?} output lacks encoding {expected:02X?}"
            );
        }

        assert!(bytes.contains(&0x0A), "{mode:?} output lacks ASL A");
        assert!(bytes.contains(&0x4C), "{mode:?} output lacks direct JMP");
        assert_ne!(
            compiled.run_address(),
            compiled.origin(),
            "{mode:?} fixture must cover RUNAD distinct from the segment origin"
        );
    }
}

#[test]
fn reorigin_contract_fixture_separates_internal_references_from_fixed_values() {
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for origin in [0x3000u16, 0x41c7] {
            let compiled = compile_file(
                reorigin_contract_fixture(),
                &CompileOptions::for_mode(mode).with_origin(origin),
            )
            .unwrap_or_else(|error| {
                panic!("compile re-origin contract in {mode:?} at ${origin:04X}: {error}")
            });
            let (segment_origin, payload) = first_load_segment(compiled.object_bytes());
            assert_eq!(segment_origin, origin, "{mode:?} main segment origin");

            // The fixture begins with selected bytes of internal addresses.
            assert_eq!(payload[0], 0x41, "{mode:?} literal initializer byte");
            assert_eq!(
                payload[1],
                origin.wrapping_add(2).to_le_bytes()[0],
                "{mode:?} low-byte initializer relocation"
            );
            assert_eq!(
                payload[2],
                origin.wrapping_add(5).to_le_bytes()[1],
                "{mode:?} high-byte initializer relocation with a negative addend"
            );

            // The routine address appears once as selected bytes and once as
            // a word in the CARD ARRAY backing.
            assert_eq!(
                &payload[3..5],
                &payload[8..10],
                "{mode:?} routine byte selectors and word relocation disagree"
            );
            let handler = u16::from_le_bytes([payload[8], payload[9]]);
            assert!(
                handler >= origin && usize::from(handler - origin) < payload.len(),
                "{mode:?} relocated routine target ${handler:04X} is outside the payload"
            );
            assert_eq!(
                u16::from_le_bytes([payload[10], payload[11]]),
                origin.wrapping_add(8),
                "{mode:?} CARD ARRAY descriptor must point at its relocated backing"
            );

            // These operands are deliberately fixed even when one happens to
            // equal the original program origin.
            assert!(
                contains_bytes(payload, &[0xAD, 0x00, 0x30]),
                "{mode:?} explicit numeric $3000 operand moved at ${origin:04X}"
            );
            assert!(
                contains_bytes(payload, &[0x8D, 0x1A, 0xD0]),
                "{mode:?} fixed hardware address moved at ${origin:04X}"
            );
        }
    }
}

#[test]
fn reorigin_contract_fixture_has_origin_stable_layout() {
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let at_3000 = compile_file(
            reorigin_contract_fixture(),
            &CompileOptions::for_mode(mode).with_origin(0x3000),
        )
        .unwrap_or_else(|error| panic!("compile re-origin baseline in {mode:?}: {error}"));
        let at_41c7 = compile_file(
            reorigin_contract_fixture(),
            &CompileOptions::for_mode(mode).with_origin(0x41c7),
        )
        .unwrap_or_else(|error| panic!("compile re-origin candidate in {mode:?}: {error}"));
        let (_, baseline) = first_load_segment(at_3000.object_bytes());
        let (_, candidate) = first_load_segment(at_41c7.object_bytes());

        assert_eq!(
            baseline.len(),
            candidate.len(),
            "{mode:?} contract fixture changes layout across origins"
        );
        assert_ne!(
            baseline, candidate,
            "{mode:?} contract fixture must contain origin-dependent bytes"
        );
    }
}

#[test]
fn synthetic_listing_labels_are_origin_independent_ordinals() {
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let at_3000 = compile_file(
            reorigin_contract_fixture(),
            &CompileOptions::for_mode(mode).with_origin(0x3000),
        )
        .unwrap_or_else(|error| panic!("compile re-origin baseline in {mode:?}: {error}"));
        let at_41c7 = compile_file(
            reorigin_contract_fixture(),
            &CompileOptions::for_mode(mode).with_origin(0x41c7),
        )
        .unwrap_or_else(|error| panic!("compile re-origin candidate in {mode:?}: {error}"));
        let baseline = at_3000.source_listing();
        let candidate = at_41c7.source_listing();
        let baseline_labels = synthetic_label_definitions(&baseline);
        let candidate_labels = synthetic_label_definitions(&candidate);

        assert!(
            !baseline_labels.is_empty(),
            "{mode:?} fixture must exercise synthetic labels"
        );
        assert_eq!(
            baseline_labels, candidate_labels,
            "{mode:?} synthetic label names depend on the origin"
        );
        assert!(
            !has_address_derived_synthetic_label(&baseline),
            "{mode:?} baseline listing still contains address-derived labels"
        );
        assert!(
            !has_address_derived_synthetic_label(&candidate),
            "{mode:?} re-origin listing still contains address-derived labels"
        );
    }
}

#[test]
fn reorigin_listing_uses_symbolic_mads_relocations_without_moving_fixed_values() {
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(
            reorigin_contract_fixture(),
            &CompileOptions::for_mode(mode).with_origin(0x3000),
        )
        .unwrap_or_else(|error| panic!("compile re-origin listing in {mode:?}: {error}"));
        let listing = compiled.source_listing();

        assert!(listing.contains("Re-originable MADS assembly listing"));
        assert!(listing.contains("ACTIONC_ORIGIN = $3000"));
        assert!(listing.contains("ORG ACTIONC_ORIGIN"));
        assert_eq!(
            listing
                .lines()
                .filter(|line| line.starts_with("ACTIONC_ORIGIN = "))
                .count(),
            1,
            "{mode:?} listing must have exactly one editable origin definition"
        );
        assert!(listing.contains("DTA L(global_first+2)"));
        assert!(listing.contains("DTA H(global_later-1)"));
        assert!(listing.contains("DTA L(proc_Helper)"));
        assert!(listing.contains("DTA H(proc_Helper)"));
        assert!(listing.contains("DTA A(proc_Helper)"));
        assert!(listing.contains("DTA A(data_generated_1)"));
        assert!(listing.contains("LDA #<(global_first+2)"));
        assert!(listing.contains("LDX #>(global_first+2)"));
        assert!(listing.contains("LDA.A global_first+1"));
        assert!(listing.contains("STA.A local_Main_local"));
        assert!(listing.contains("STA.A local_Main_aliasLow"));
        assert!(listing.contains("STA.A local_Main_aliasHigh"));
        assert!(listing.contains("JSR.A proc_Helper"));
        assert!(listing.contains("LDA.A $3000"));
        assert!(listing.contains("STA.A global_color"));
        assert!(listing.contains("global_color = $D01A"));
        assert!(listing.contains("ORG $02E2\n        DTA A(proc_Main)"));
    }
}

#[test]
fn listing_variants_share_one_mads_compatible_assembly_syntax() {
    for (mode, profile, backend) in [
        (CompileMode::Compatibility, "legacy", "classic"),
        (CompileMode::Optimized, "modern", "classic"),
        (CompileMode::Mir6502, "modern", "mir6502"),
    ] {
        let listing = emit_listing(profile, backend, "--emit-listing");
        let source_listing = emit_listing(profile, backend, "--emit-source-listing");
        let compiled = compile_file(contract_fixture(), &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile MADS contract fixture in {mode:?}: {error}"));

        assert!(listing.is_ascii(), "{mode:?} listing is not ASCII");
        assert!(
            source_listing.is_ascii(),
            "{mode:?} source listing is not ASCII"
        );
        assert!(listing.contains("Re-originable MADS assembly listing"));
        assert!(listing.contains(&format!("ACTIONC_ORIGIN = ${:04X}", compiled.origin())));
        assert!(listing.contains("ORG ACTIONC_ORIGIN"));
        assert!(listing.contains("ORG $02E2"));
        assert!(listing.contains("DTA A(proc_Main)"));
        assert!(listing.contains("global_source = $0058"));
        assert!(listing.contains("global_ptr = $0080"));
        assert!(listing.contains("proc_Helper:"));
        assert!(listing.contains("proc_Main:"));
        assert!(listing.contains("LDA.A global_source"));
        assert!(listing.contains("LDA.Z global_source"));
        assert!(listing.contains("LDX.Z global_source,Y"));
        assert!(listing.contains("LDA (global_ptr,X)"));
        assert!(listing.contains("LDA (global_ptr),Y"));
        assert!(listing.contains("JSR.A proc_Helper"));
        assert!(
            listing
                .lines()
                .any(|line| line.trim_start().starts_with("ASL "))
        );
        assert!(!listing.contains("ASL A"));
        assert!(source_listing.contains("\\u{C3}\\u{A9}"));
        assert_eq!(
            assembly_statements(&listing),
            assembly_statements(&source_listing),
            "{mode:?} listing variants differ in assembly statements"
        );
        assert!(listing.lines().all(|line| {
            let bytes = line.as_bytes();
            bytes.len() < 5
                || !bytes[..4].iter().all(u8::is_ascii_hexdigit)
                || !bytes[4].is_ascii_whitespace()
        }));
    }
}

#[test]
fn storage_rows_and_runtime_names_are_consistent_across_modes() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/listing/mads_storage_runtime.act");
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                &fixture,
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|error| panic!("{mode:?}/{runtime:?}: {error}"));
            let listing = compiled.source_listing();
            let lines = listing.lines().collect::<Vec<_>>();
            let (origin, payload) = first_load_segment(compiled.object_bytes());
            let mut array_offset = 0;
            for (name, size) in [("x", 192), ("y", 192), ("c", 192), ("markers", 11)] {
                let label = format!("global_{name}:");
                let start = lines
                    .iter()
                    .position(|line| *line == label)
                    .expect("array label");
                let mut bytes = Vec::new();
                for row in lines[start + 1..]
                    .iter()
                    .take_while(|line| line.trim_start().starts_with(".BYTE "))
                {
                    let (directive, comment) = row.split_once(';').expect("byte comment");
                    let values = directive
                        .trim()
                        .strip_prefix(".BYTE ")
                        .unwrap()
                        .split(',')
                        .map(|value| u8::from_str_radix(value.trim_start_matches('$'), 16).unwrap())
                        .collect::<Vec<_>>();
                    assert_eq!(
                        values.len(),
                        (size - bytes.len()).min(8),
                        "{mode:?}/{runtime:?}: {row}"
                    );
                    assert!(comment.trim_start().starts_with(&format!(
                        "${:04X}:",
                        usize::from(origin) + array_offset + bytes.len()
                    )));
                    bytes.extend(values);
                }
                assert_eq!(bytes.len(), size, "{mode:?}/{runtime:?}: {name}");
                assert_eq!(bytes, payload[array_offset..array_offset + size]);
                array_offset += size;
            }

            // The first deferred array starts exactly at the saved segment end.
            // Its label must be outside the last routine's visual boundary.
            let last_end = lines
                .iter()
                .rposition(|line| line.starts_with("; ===== END PROC "))
                .unwrap();
            let deferred = lines[last_end + 1..]
                .iter()
                .find(|line| !line.is_empty())
                .unwrap();
            assert!(deferred.ends_with(':'), "{mode:?}/{runtime:?}: {deferred}");
            assert!(deferred.starts_with("global_work") || deferred.starts_with("data_generated_"));
            assert!(listing.contains("; ===== PROC MainCase "));
            assert!(listing.contains("; ===== END PROC MainCase ====="));

            assert!(
                !listing.contains("M_ACTION_RUNTIME_"),
                "{mode:?}/{runtime:?}: mangled runtime name"
            );
            for spelling in [
                "::multi",
                "::MULTI",
                "proc_syslib_multi",
                "proc_syslib_MULTI",
                "::divi",
                "::DIVI",
                "; Runtime binding: divi ->",
                "; Runtime binding: DIVI ->",
            ] {
                assert!(
                    !listing.contains(spelling),
                    "{mode:?}/{runtime:?}: {spelling}"
                );
            }
            assert!(listing.contains("; Runtime binding: DivI -> "));
            assert!(listing.contains("proc_actionc_DivI:"));
            if runtime == Runtime::Standalone {
                assert!(listing.contains("; Runtime binding: MultI -> "));
                for helper in ["SMOps", "SetSign", "SS1"] {
                    assert!(listing.contains(&format!("proc_syslib_{helper}:")));
                    assert!(listing.contains(&format!("ACTION.RUNTIME.SYSLIB::{helper}")));
                }
                assert!(listing.contains("loc_syslib_MultB_"));
                for helper in ["MultI", "MultB"] {
                    assert!(listing.contains(&format!("proc_syslib_{helper}:")));
                    assert!(
                        listing.contains(&format!("; ===== PROC ACTION.RUNTIME.SYSLIB::{helper} "))
                    );
                    assert!(listing.contains(&format!(
                        "; ===== END PROC ACTION.RUNTIME.SYSLIB::{helper} ====="
                    )));
                    assert!(
                        lines
                            .iter()
                            .any(|line| line.contains(&format!("JSR.A proc_syslib_{helper}"))
                                && line.ends_with(&format!("; ACTION.RUNTIME.SYSLIB::{helper}")))
                    );
                }
            }
        }
    }
}

#[test]
fn user_names_follow_declarations_in_every_mode_and_runtime() {
    for (fixture, scope) in [("mads_user_names", ""), ("mads_module_names", "DrawDemo_")] {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/listing/{fixture}.act"));
        for mode in [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let compiled =
                    compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime))
                        .unwrap_or_else(|error| panic!("{fixture}/{mode:?}/{runtime:?}: {error}"));
                let listing = compiled.source_listing();
                let statements = assembly_statements(&listing).join("\n");
                for expected in [
                    format!("proc_{scope}DrawLine:"),
                    format!("proc_{scope}MainLoop:"),
                    format!("global_{scope}PlayerX = $0600"),
                    format!("param_{scope}DrawLine_StartX"),
                    format!("local_{scope}DrawLine_TempValue"),
                    format!("loc_{scope}DrawLine_"),
                    format!(".A proc_{scope}DrawLine"),
                    format!("DTA A(proc_{scope}MainLoop)"),
                ] {
                    assert!(
                        statements.contains(&expected),
                        "{fixture}/{mode:?}/{runtime:?}: missing {expected}\n{listing}"
                    );
                }
                for wrong in [
                    "proc_drawline",
                    "proc_DRAWLINE",
                    "global_PLAYERX",
                    "param_drawline",
                    "_TEMPVALUE",
                ] {
                    assert!(
                        !statements.contains(wrong),
                        "{fixture}/{mode:?}/{runtime:?}: {wrong}\n{listing}"
                    );
                }
                assert!(!listing.contains("M_DRAWDEMO_"), "{listing}");
                if mode == CompileMode::Compatibility {
                    assert!(
                        listing
                            .contains("; Parameter frame follows: StartX, StepSize, Shade, Flags"),
                        "{listing}"
                    );
                }
                if scope.is_empty() {
                    assert!(statements.contains("global_PixelData:"), "{listing}");
                    assert!(
                        statements.contains("global_PlayerX__2 = $0602"),
                        "{listing}"
                    );
                    assert!(statements.contains("STA.A global_PlayerX__2"), "{listing}");
                }
            }
        }
    }
}

#[test]
fn lexical_names_keep_spelling_and_distinct_scopes() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/listing/mads_lexical_names.act");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled =
                compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
            let listing = compiled.source_listing();
            for name in ["TempValue", "block0_TempValue", "block0_block1_TempValue"] {
                let label = format!("local_ScopedDemo_MainLoop_{name}");
                assert!(
                    listing.contains(&format!("{label}:")),
                    "{mode:?}/{runtime:?}: {label}\n{listing}"
                );
                assert!(
                    listing.contains(&format!("STA.A {label}")),
                    "{mode:?}/{runtime:?}: {label}\n{listing}"
                );
            }
        }
    }
}
