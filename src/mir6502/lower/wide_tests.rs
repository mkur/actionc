use super::*;
use crate::mir6502::ir::MirReg;
use crate::mir6502::passes::Mir6502Config;
use crate::runtime::Runtime;

fn lower_source(source: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(&semir)).unwrap();
    let mir = crate::mir6502::lower_program(&nir).unwrap();
    crate::mir6502::verify_program(&mir, crate::mir6502::MirPhase::PreMaterialization).unwrap();
    mir
}

#[test]
fn aligned_wide_shifts_use_byte_computation_and_keep_captured_loads() {
    for count in [4, 8, 12, 16, 20, 24, 28] {
        for operator in ["LSH", "RSH"] {
            for ty in ["LONGCARD", "LONGINT"] {
                let mir = lower_source(&format!(
                    "VOLATILE {ty} input=$6E0 {ty} result=$600\n\
                     PROC Main() result=input {operator} {count} DO OD RETURN"
                ));
                let ops: Vec<_> = mir
                    .routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .collect();
                assert_eq!(
                    ops.iter()
                        .filter(|op| matches!(
                            op,
                            MirOp::Load {
                                width: MirWidth::Word,
                                ..
                            }
                        ))
                        .count(),
                    2
                );
                assert!(!ops.iter().any(|op| matches!(
                    op,
                    MirOp::Binary {
                        width: MirWidth::Word,
                        ..
                    } | MirOp::RuntimeHelper { .. }
                )));
                assert!(!ops.iter().any(|op| matches!(
                    op,
                    MirOp::Binary {
                        op: MirBinaryOp::Or,
                        left: MirValue::ConstU8(0),
                        ..
                    } | MirOp::Binary {
                        op: MirBinaryOp::Or,
                        right: MirValue::ConstU8(0),
                        ..
                    }
                )));
                if count % 8 == 0 {
                    assert!(!ops.iter().any(|op| matches!(op, MirOp::Binary { .. })));
                }
                for runtime in [Runtime::Standalone, Runtime::ActionCart] {
                    for config in [
                        Mir6502Config::default(),
                        Mir6502Config::optimized(),
                        Mir6502Config {
                            enable_peepholes: false,
                            ..Mir6502Config::default()
                        },
                    ] {
                        let materialized =
                            crate::mir6502::materialize_program_with_origin_and_runtime(
                                mir.clone(),
                                &config,
                                0x3000,
                                runtime,
                            )
                            .unwrap();
                        crate::mir6502::verify_program(
                            &materialized,
                            crate::mir6502::MirPhase::PreEmission,
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
}

#[test]
fn aligned_wide_shift_narrow_probe_avoids_zero_merge_and_shift_result_staging() {
    let mir = lower_source(
        "LONGCARD input=$6E0 CARD result=$600\n\
         PROC Main() result=CARD(input RSH 12) DO OD RETURN",
    );
    for runtime in [Runtime::Standalone, Runtime::ActionCart] {
        let materialized = crate::mir6502::materialize_program_with_origin_and_runtime(
            mir.clone(),
            &Mir6502Config::optimized(),
            0x3000,
            runtime,
        )
        .unwrap();
        let main = &materialized.routines[0];
        let ops: Vec<_> = main.blocks.iter().flat_map(|b| &b.ops).collect();
        let merges: Vec<_> = ops
            .iter()
            .enumerate()
            .filter(|(_, op)| {
                matches!(
                    op,
                    MirOp::Binary {
                        op: MirBinaryOp::Or,
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(merges.len(), 2);
        for (index, _) in merges {
            assert!(
                matches!(
                    ops[index - 1],
                    MirOp::Binary {
                        op: MirBinaryOp::Rsh,
                        dst: MirDef::Reg(MirReg::A),
                        ..
                    }
                ),
                "{}",
                crate::mir6502::format_program(&materialized)
            );
        }
    }
}

#[test]
fn dynamic_wide_shifts_keep_the_wide_helpers() {
    let mir = lower_source(
        "LONGCARD input=$6E0,count=$6E4,left=$600,right=$604\n\
         PROC Main() left=input LSH count right=input RSH count DO OD RETURN",
    );
    let helpers: Vec<_> = mir
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .filter_map(|op| match op {
            MirOp::RuntimeHelper { helper, .. } => Some(*helper),
            _ => None,
        })
        .collect();
    assert_eq!(helpers, [MirRuntimeHelper::Lsh32, MirRuntimeHelper::Rsh32]);
}
