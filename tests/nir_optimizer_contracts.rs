#![allow(dead_code)]

use std::path::Path;

use actionc::nir::{self, NirCallee, NirLocalBacking, NirMemoryAccess, NirOp, NirValue};

mod nir_fixture_support;

use nir_fixture_support::{NIR_FIXTURE_CASES, NirFixtureStage, lower_case};

fn lowered(name: &str) -> nir::NirProgram {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let case = NIR_FIXTURE_CASES
        .iter()
        .find(|case| case.name == name && case.stage == NirFixtureStage::Lowered)
        .unwrap_or_else(|| panic!("missing lowered NIR fixture case `{name}`"));
    lower_case(repo_root, *case)
}

fn optimized(name: &str) -> (nir::NirProgram, nir::NirProgram) {
    let lowered = lowered(name);
    let optimized = nir::optimize_program(&lowered)
        .unwrap_or_else(|diagnostics| panic!("optimize `{name}`: {diagnostics:?}"));
    nir::verify_program(&lowered)
        .unwrap_or_else(|diagnostics| panic!("verify lowered `{name}`: {diagnostics:?}"));
    nir::verify_program(&optimized)
        .unwrap_or_else(|diagnostics| panic!("verify optimized `{name}`: {diagnostics:?}"));
    (lowered, optimized)
}

fn op_count(program: &nir::NirProgram, predicate: impl Fn(&NirOp) -> bool) -> usize {
    program
        .routines
        .iter()
        .flat_map(|routine| &routine.blocks)
        .flat_map(|block| &block.ops)
        .filter(|op| predicate(op))
        .count()
}

#[test]
fn optimizer_snapshots_exercise_transformation_families() {
    let (unary_lowered, unary_optimized) = optimized("unary_cast");
    let unary_before = nir::collect_program_stats(&unary_lowered);
    let unary_after = nir::collect_program_stats(&unary_optimized);
    assert!(unary_after.operations < unary_before.operations);
    assert!(unary_after.temp_definitions < unary_before.temp_definitions);

    let (_, scalar_optimized) = optimized("scalar_assignments");
    assert_eq!(
        op_count(&scalar_optimized, |op| matches!(op, NirOp::Load { .. })),
        0
    );
    assert!(scalar_optimized.routines[0].blocks[0].ops.iter().any(|op| {
        matches!(
            op,
            NirOp::Store {
                src: NirValue::IntegerConst { bits: 1, .. },
                ..
            }
        )
    }));

    let (_, local_optimized) = optimized("optimizer_local_promotion");
    assert!(local_optimized.routines[0].locals.is_empty());
    assert_eq!(local_optimized.routines[0].blocks[0].ops.len(), 1);

    let (_, control_optimized) = optimized("control_flow");
    assert!(
        control_optimized.routines[0]
            .blocks
            .iter()
            .all(|block| { !matches!(block.terminator, nir::NirTerminator::Open) })
    );
}

#[test]
fn optimizer_preserves_calls_volatility_aliases_real_and_foreign_code() {
    let (calls_lowered, calls_optimized) = optimized("call_forms");
    let call_count =
        |program: &nir::NirProgram| op_count(program, |op| matches!(op, NirOp::Call { .. }));
    assert_eq!(call_count(&calls_optimized), call_count(&calls_lowered));
    assert!(
        calls_optimized
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .any(|op| {
                matches!(
                    op,
                    NirOp::Call {
                        callee: NirCallee::Indirect { .. },
                        ..
                    }
                )
            })
    );
    assert!(
        calls_optimized
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .any(|op| {
                matches!(op, NirOp::Call { effects, .. }
            if matches!(effects.memory.reads, NirMemoryAccess::Unknown)
                && matches!(effects.memory.writes, NirMemoryAccess::Unknown)
                && effects.may_call_external
                && effects.opaque)
            })
    );

    let (_, copy_optimized) = optimized("volatile_record_copy");
    assert!(copy_optimized.routines[0].blocks[0].ops.iter().any(|op| {
        matches!(
            op,
            NirOp::CopyBytes {
                source_volatile: true,
                destination_volatile: true,
                ..
            }
        )
    }));

    let (_, views_optimized) = optimized("local_storage_views");
    let main = &views_optimized.routines[0];
    assert!(
        main.locals
            .iter()
            .any(|local| { matches!(local.backing, NirLocalBacking::Alias { .. }) })
    );
    assert_eq!(
        op_count(&views_optimized, |op| matches!(
            op,
            NirOp::VolatileStore { .. }
        )),
        2
    );
    assert_eq!(
        op_count(&views_optimized, |op| matches!(
            op,
            NirOp::VolatileLoad { .. }
        )),
        1
    );
    assert!(op_count(&views_optimized, |op| matches!(op, NirOp::AddrOf { .. })) > 0);
    assert!(op_count(&views_optimized, |op| matches!(op, NirOp::Real(_))) > 0);

    let (machine_lowered, machine_optimized) = optimized("machine_blocks");
    let foreign_count =
        |program: &nir::NirProgram| op_count(program, |op| matches!(op, NirOp::ForeignCode { .. }));
    assert_eq!(
        foreign_count(&machine_optimized),
        foreign_count(&machine_lowered)
    );
    assert_eq!(foreign_count(&machine_optimized), 2);
}
