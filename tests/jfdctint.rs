//! The benchmark itself has no VM memory map and lowers for every target.
use actionc::{
    lexer::tokenize,
    nir,
    parser::parse,
    semantic::{self, SemanticOptions},
    target::TargetId,
};

#[test]
fn jfdctint_uses_ordinary_storage_and_lowers_to_all_target_mirs() {
    let source = include_str!("../fixtures/runtime/tacle/jfdctint/jfdctint.act");
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        let model =
            semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
                .unwrap_or_else(|error| panic!("{target:?}: {error:?}"));
        let raw = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        nir::verify_program(&raw).unwrap();
        assert!(
            raw.globals
                .iter()
                .all(|g| g.backing == nir::NirGlobalBacking::Ordinary)
        );
        let block = raw
            .globals
            .iter()
            .find(|g| g.name == "block")
            .unwrap()
            .array
            .as_ref()
            .unwrap();
        assert_eq!(block.length, Some(64));
        assert_eq!(block.elem_size.get(), 4);
        assert!(block.address_initializer.is_none());
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
