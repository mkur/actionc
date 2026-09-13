//! The decoder has no VM memory map and lowers for every target.
use actionc::{
    includes::{ModuleLoadOptions, load_compilation},
    nir,
    semantic::{self, SemanticOptions},
    target::TargetId,
};
use std::path::Path;

#[test]
fn adpcm_dec_uses_ordinary_storage_and_lowers_to_all_target_mirs() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/tacle/adpcm_dec/adpcm_dec.act");
    let loaded = load_compilation(&path, &ModuleLoadOptions::default()).unwrap();
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        let model = semantic::analyze_compilation_with_options(
            &loaded,
            SemanticOptions::modern().with_target(target),
        )
        .unwrap_or_else(|error| panic!("{target:?}: {error:?}"));
        let raw = nir::lower_program(&semantic::ir::lower_compilation(&loaded, &model));
        nir::verify_program(&raw).unwrap();
        assert!(
            raw.globals
                .iter()
                .all(|g| g.backing == nir::NirGlobalBacking::Ordinary)
        );
        assert!(
            raw.globals
                .iter()
                .filter_map(|g| g.array.as_ref())
                .all(|a| a.address_initializer.is_none())
        );
        let optimized = nir::optimize_program(&raw).unwrap();
        nir::verify_program(&optimized).unwrap();
        for program in [&raw, &optimized] {
            match target {
                TargetId::Atari6502 => {
                    actionc::mir6502::lower_program(program).unwrap();
                }
                TargetId::Motorola68000 => {
                    actionc::mir68k::lower_program(program).unwrap();
                }
                _ => {
                    actionc::mir65816::lower_program(program).unwrap();
                }
            }
        }
    }
}
