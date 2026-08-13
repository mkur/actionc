use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, compile_file};

fn contract_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("listing")
        .join("mads_contract.act")
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
                bytes.windows(expected.len()).any(|window| window == expected),
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
