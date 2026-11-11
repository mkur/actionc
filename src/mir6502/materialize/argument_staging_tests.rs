use super::*;
use std::sync::OnceLock;

fn baseline() -> MirProgram {
    static BASE: OnceLock<MirProgram> = OnceLock::new();
    BASE.get_or_init(|| {
        let source =
            include_str!("../../../fixtures/runtime/argument_staging.act").replace("\r\n", "\n");
        let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        let config = Mir6502Config {
            enable_argument_staging_reduction: false,
            ..Mir6502Config::optimized()
        };
        crate::mir6502::materialize_program_with_origin_and_runtime(
            crate::mir6502::lower_program(&nir).unwrap(),
            &config,
            0x3000,
            crate::runtime::Runtime::Standalone,
        )
        .unwrap()
    })
    .clone()
}

fn selected(p: MirProgram) -> MirProgram {
    let eligible = p.routines.iter().map(|r| r.id).collect();
    select_costed(
        p,
        &Mir6502Config::optimized(),
        0x3000,
        crate::runtime::Runtime::Standalone,
        &eligible,
    )
    .unwrap()
}

#[test]
fn argument_staging_keeps_all_stores_homes_source_reads_and_calls() {
    let old = baseline();
    let new = selected(old.clone());
    assert_ne!(old, new);
    assert_eq!(new.globals, old.globals);
    assert_eq!(new.machine_blocks, old.machine_blocks);
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        let stores = |r: &MirRoutine| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|o| matches!(o, MirOp::Store { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        let calls = |r: &MirRoutine| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|o| matches!(o, MirOp::Call { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        let source_reads = |r: &MirRoutine| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter_map(|o| match o {
                    MirOp::Load {
                        src: MirAddr::Direct(mem),
                        width,
                        ..
                    } if !peepholes::mem_is_private_scratch(mem) => Some((mem.clone(), *width)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(stores(a), stores(b));
        assert_eq!(calls(a), calls(b));
        assert_eq!(source_reads(a), source_reads(b));
        if ["ForwardAdd", "ForwardSub"].contains(&a.name.as_str()) {
            assert_ne!(a, b);
        } else {
            assert_eq!(a, b);
        }
    }
    assert_eq!(
        image_size(&old, 0x3000, crate::runtime::Runtime::Standalone).unwrap()
            - image_size(&new, 0x3000, crate::runtime::Runtime::Standalone).unwrap(),
        2
    );
    let eligible = old.routines.iter().map(|r| r.id).collect();
    let reported = select_costed(
        old,
        &Mir6502Config {
            peephole_report: crate::mir6502::MirPeepholeReportMode::PerRoutine,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::runtime::Runtime::Standalone,
        &eligible,
    )
    .unwrap();
    assert_eq!(new, reported);
}

#[test]
fn argument_staging_unknown_emission_cost_retains_the_complete_fallback() {
    let mut old = baseline();
    let control = old
        .routines
        .iter_mut()
        .find(|r| r.name == "FourByte")
        .unwrap();
    if let Some(MirOp::Call { target, .. }) = control.blocks[0]
        .ops
        .iter_mut()
        .find(|o| matches!(o, MirOp::Call { .. }))
    {
        *target = super::super::super::ir::MirCallTarget::Routine(RoutineId(u32::MAX));
    }
    assert!(image_size(&old, 0x3000, crate::runtime::Runtime::Standalone).is_none());
    let eligible = old.routines.iter().map(|r| r.id).collect();
    let new = select_costed(
        old.clone(),
        &Mir6502Config::optimized(),
        0x3000,
        crate::runtime::Runtime::Standalone,
        &eligible,
    )
    .unwrap();
    assert_eq!(new, old);
}
