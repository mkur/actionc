use std::path::{Path, PathBuf};
use std::process::Command;

use actionc::compiler::{CompileMode, CompileOptions, compile_file};

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
        assert!(listing.contains("Fixed-origin MADS assembly listing"));
        assert!(listing.contains(&format!("ORG ${:04X}", compiled.origin())));
        assert!(listing.contains("ORG $02E2"));
        assert!(listing.contains("DTA A(proc_main)"));
        assert!(listing.contains("global_source = $0058"));
        assert!(listing.contains("global_ptr = $0080"));
        assert!(listing.contains("proc_helper:"));
        assert!(listing.contains("proc_main:"));
        assert!(listing.contains("LDA.A global_source"));
        assert!(listing.contains("LDA.Z global_source"));
        assert!(listing.contains("LDX.Z global_source,Y"));
        assert!(listing.contains("LDA (global_ptr,X)"));
        assert!(listing.contains("LDA (global_ptr),Y"));
        assert!(listing.contains("JSR.A proc_helper"));
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
