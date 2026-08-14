use std::path::{Path, PathBuf};
use std::process::Command;

use actionc::compiler::{CompileMode, CompileOptions, compile_file};

fn contract_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("listing")
        .join("mads_contract.act")
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
