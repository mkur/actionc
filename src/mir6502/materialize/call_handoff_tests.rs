use super::*;
use crate::mir6502::analysis::{prehome::PreHomeAnalysisSnapshot, sites::MirRoutineGeneration};
use crate::mir6502::{MirParamBinding, MirParamHandoff, MirParamHome};

fn wrapper() -> MirRoutine {
    let text = include_str!("../../../fixtures/runtime/parameter_argument_handoff.act")
        .replace("\r\n", "\n");
    let ast = crate::parser::parse(&crate::lexer::tokenize(&text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    let mut r = crate::mir6502::lower_program(&nir)
        .unwrap()
        .routines
        .into_iter()
        .find(|r| r.name == "CopyCounted")
        .unwrap();
    r.frame.parameter_homes = Some(
        r.frame
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let MirStorageBase::Param(param) = p.base else {
                    panic!()
                };
                MirParamBinding {
                    param,
                    home: MirParamHome::ZeroPage(MirFixedZpSlot(0xa0 + 2 * i as u8)),
                }
            })
            .collect(),
    );
    r.frame
        .fixed_zero_page
        .extend((0xa0..=0xa3).map(MirFixedZpSlot));
    let MirOp::Call {
        target: MirCallTarget::Routine(target),
        ..
    } = r.blocks[0].ops.last().unwrap()
    else {
        panic!()
    };
    r.frame.parameter_handoff = Some(MirParamHandoff {
        target: *target,
        argument_bytes: 5,
    });
    r
}
fn plan(r: &MirRoutine) -> Option<MirRewritePlan> {
    let snapshot = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
    let context = PreHomeRewriteContext::new(&snapshot);
    resident_parameter_uses_plan(r, r.blocks[0].id, &r.blocks[0].ops, 1, &context)
}
#[test]
fn resident_consumer_keeps_the_read_and_rejects_mutated_or_redefined_values() {
    let r = wrapper();
    let p = plan(&r).unwrap();
    assert_eq!(p.range, 1..5);
    assert!(matches!(
        p.replacement[0],
        MirOp::Load {
            src: MirAddr::Deref { .. },
            ..
        }
    ));
    assert!(matches!(p.replacement[1], MirOp::Binary { .. }));
    assert!(matches!(p.replacement[2], MirOp::Call { .. }));
    let MirOp::Load {
        dst: MirDef::VTemp(temp),
        src: MirAddr::Direct(mem),
        ..
    } = &r.blocks[0].ops[1]
    else {
        panic!()
    };
    // Direct-only call consumers keep the existing repeated-argument strategy.
    let mut direct_only = r.clone();
    direct_only.blocks[0].ops[2] = MirOp::LoadImm {
        dst: match &r.blocks[0].ops[2] {
            MirOp::Load { dst, .. } => dst.clone(),
            _ => panic!(),
        },
        value: 7,
        width: MirWidth::Byte,
    };
    assert!(plan(&direct_only).is_none());
    let mut mutated = r.clone();
    mutated.blocks[0].ops.insert(
        2,
        MirOp::Store {
            dst: MirAddr::Direct(mem.clone()),
            src: MirValue::ConstU8(1),
            width: MirWidth::Byte,
        },
    );
    assert!(plan(&mutated).is_none());
    let mut redefined = r.clone();
    redefined.blocks[0].ops.insert(
        2,
        MirOp::LoadImm {
            dst: MirDef::VTempByte { id: *temp, byte: 0 },
            value: 1,
            width: MirWidth::Byte,
        },
    );
    assert!(plan(&redefined).is_none());
    let mut extra_use = r.clone();
    extra_use.blocks[0].ops.insert(
        2,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x600)),
            src: MirValue::Def(MirDef::VTemp(*temp)),
            width: MirWidth::Word,
        },
    );
    assert!(plan(&extra_use).is_none());
}
