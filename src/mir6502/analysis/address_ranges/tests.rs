use super::*;
use crate::compiler::Runtime;
use crate::mir6502::{self, Mir6502Config};

fn lower(source: &str) -> nir::NirProgram {
    let tokens = crate::lexer::tokenize(source).unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model))
}
fn fixture() -> String {
    include_str!("../../../../fixtures/runtime/checked_pointer_ranges.act").replace("\r\n", "\n")
}

#[test]
fn checked_words_reach_complete_materialized_abi_carriers() {
    let raw = lower(&fixture());
    let optimized = nir::optimize_program(&raw).unwrap();
    for source in [&raw, &optimized] {
        let lowered = mir6502::lower_program(source).unwrap();
        let materialized = mir6502::materialize_program_with_origin_and_runtime(
            lowered.clone(),
            &Mir6502Config::optimized(),
            0x3000,
            Runtime::ActionCart,
        )
        .unwrap();
        let ranges = MirAddressRanges::project(source, &lowered, &materialized).unwrap();
        let copy = materialized
            .routines
            .iter()
            .find(|r| r.name == "CopyBytes")
            .unwrap()
            .id;
        for name in ["Guarded", "FromReturn"] {
            let r = materialized
                .routines
                .iter()
                .find(|r| r.name == name)
                .unwrap();
            let (_, words) = ranges
                .for_call(r, copy, MirRoutineGeneration::initial())
                .unwrap_or_else(|| {
                    panic!(
                        "missing {name}: source CFG {:?}, final CFG {:?}",
                        cfg_shape(lowered.routines.iter().find(|s| s.id == r.id).unwrap()),
                        cfg_shape(r)
                    )
                });
            assert_eq!(
                words,
                [
                    CoherentWordRange {
                        lo: AbiByte::Register(MirReg::A),
                        hi: AbiByte::Register(MirReg::X),
                        min: 0x2104,
                        max: 0x280B
                    },
                    CoherentWordRange {
                        lo: AbiByte::Register(MirReg::Y),
                        hi: AbiByte::Cell(0xA3),
                        min: 0x2104,
                        max: 0x270C
                    }
                ]
            );
            assert!(
                ranges
                    .for_call(r, copy, MirRoutineGeneration::initial().next())
                    .is_none()
            );
            let mut stale = r.clone();
            stale.blocks[0].ops.insert(
                0,
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 0,
                    width: MirWidth::Byte,
                },
            );
            assert!(
                ranges
                    .for_call(&stale, copy, MirRoutineGeneration::initial())
                    .is_none()
            );
        }
        let generic = materialized
            .routines
            .iter()
            .find(|r| r.name == "Unrestricted")
            .unwrap();
        assert!(
            ranges
                .for_call(generic, copy, MirRoutineGeneration::initial())
                .is_none()
        );
    }
}

#[test]
fn repeated_calls_and_changed_caller_generations_are_not_guessed() {
    let text = fixture().replace(
        "  CopyBytes(dst+dst^+1,src+1,src^)\n",
        "  CopyBytes(dst+dst^+1,src+1,src^)\n  CopyBytes(dst+dst^+1,src+1,src^)\n",
    );
    let source = lower(&text);
    let lowered = mir6502::lower_program(&source).unwrap();
    let materialized = mir6502::materialize_program_with_origin_and_runtime(
        lowered.clone(),
        &Mir6502Config::optimized(),
        0x3000,
        Runtime::Standalone,
    )
    .unwrap();
    let ranges = MirAddressRanges::project(&source, &lowered, &materialized).unwrap();
    let copy = materialized
        .routines
        .iter()
        .find(|r| r.name == "CopyBytes")
        .unwrap()
        .id;
    for r in &materialized.routines {
        assert!(
            ranges
                .for_call(r, copy, MirRoutineGeneration::initial())
                .is_none()
        );
    }
}
