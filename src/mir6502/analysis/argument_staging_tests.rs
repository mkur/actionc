use super::*;
use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
use crate::mir6502::analysis::sites::MirRoutineGeneration;
use crate::mir6502::rewrite::driver::MirPreHomeRewriteDriver;

fn program() -> MirProgram {
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
    let mut mir = crate::mir6502::lower_program(&nir).unwrap();
    for routine in &mut mir.routines {
        MirPreHomeRewriteDriver::default()
            .run_fixed_point(
                routine,
                crate::mir6502::rewrite::pilots::discover_call_arg_producers,
            )
            .unwrap();
    }
    mir
}

fn routine() -> MirRoutine {
    program()
        .routines
        .into_iter()
        .find(|r| r.name == "ForwardAdd")
        .unwrap()
}

fn fact(routine: &MirRoutine) -> Result<ArgumentStaging, Blocker> {
    let snapshot = PreHomeAnalysisSnapshot::new(routine, MirRoutineGeneration::initial()).unwrap();
    let context = PreHomeRewriteContext::new(&snapshot);
    let block = &routine.blocks[0];
    let op_index = block
        .ops
        .iter()
        .position(|o| matches!(o, MirOp::Call { .. }))
        .unwrap();
    context.argument_staging(
        routine,
        context.point(MirSite::Op {
            block: block.id,
            op_index,
        }),
    )
}

#[test]
fn argument_staging_facts_preserve_complete_captures_and_canonical_sources() {
    let p = program();
    let layout = MaterializeLayout::new(&p, 0x3000);
    for r in &p.routines {
        if ["ForwardAdd", "ForwardSub"].contains(&r.name.as_str()) {
            let f = fact(r).unwrap();
            assert_eq!(f.definitions[0].lane.byte, 0);
            assert_eq!(f.definitions[1].lane.byte, 1);
            assert_eq!(f.definitions[0].site, f.producer);
            assert_eq!(
                f.sources
                    .iter()
                    .map(|(_, w)| if *w == MirWidth::Word { 2 } else { 1 })
                    .sum::<u8>(),
                5
            );
            assert!(f.placed(r.id, &layout));
        } else if ["FourByte", "FivePlain"].contains(&r.name.as_str()) {
            assert_eq!(fact(r), Err(Blocker::Shape));
        }
    }
}

#[test]
fn argument_staging_facts_reject_extra_uses_partial_words_and_unsafe_storage() {
    let mut extra = routine();
    let captured = fact(&extra).unwrap().captured;
    extra.blocks[0].ops.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::Absolute(0x680)),
        src: MirValue::Def(MirDef::VTemp(captured)),
        width: MirWidth::Word,
    });
    assert_eq!(fact(&extra), Err(Blocker::Capture));
    let mut partial = routine();
    let i = partial.blocks[0]
        .ops
        .iter()
        .position(|o| matches!(o, MirOp::Binary { .. }))
        .unwrap();
    if let MirOp::Binary { width, .. } = &mut partial.blocks[0].ops[i] {
        *width = MirWidth::Byte;
    }
    assert_eq!(fact(&partial), Err(Blocker::Shape));
    let mut fixed = routine();
    fixed.frame.params[1].base = MirStorageBase::Absolute(0xA3);
    assert_eq!(fact(&fixed), Err(Blocker::Storage));
    let mut opaque = routine();
    if let Some(MirOp::Call { effects, .. }) = opaque.blocks[0].ops.last_mut() {
        effects.opaque = true;
    }
    assert_eq!(fact(&opaque), Err(Blocker::Callee));
}

#[test]
fn argument_staging_facts_bound_the_window_and_reject_stale_points() {
    for extra in [MAX_SETUP_OPS - 3, MAX_SETUP_OPS - 2] {
        let mut r = routine();
        let index = r.blocks[0]
            .ops
            .iter()
            .position(|o| matches!(o, MirOp::Binary { .. }))
            .unwrap();
        let next = r.temps.iter().map(|t| t.id.0).max().unwrap() + 1;
        for n in 0..extra {
            let id = MirTempId(next + n as u32);
            r.temps.push(MirTemp { id });
            r.blocks[0].ops.insert(
                index,
                MirOp::LoadImm {
                    dst: MirDef::VTemp(id),
                    value: 7,
                    width: MirWidth::Byte,
                },
            );
        }
        assert_eq!(fact(&r).is_ok(), extra == MAX_SETUP_OPS - 3);
    }
    let r = routine();
    let old = fact(&r).unwrap().call;
    let snapshot =
        PreHomeAnalysisSnapshot::new(&r, MirRoutineGeneration::initial().next()).unwrap();
    let context = PreHomeRewriteContext::new(&snapshot);
    assert_eq!(context.argument_staging(&r, old), Err(Blocker::Site));
}
