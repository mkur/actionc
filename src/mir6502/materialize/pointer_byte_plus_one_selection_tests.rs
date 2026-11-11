use super::*;
use crate::mir6502::{analysis::pointer_byte_plus_one::tests::baseline, ir::MirRoutine};

fn config() -> Mir6502Config {
    Mir6502Config {
        enable_pointer_byte_plus_one_fusion: true,
        ..Mir6502Config::optimized()
    }
}
fn selected(p: MirProgram) -> MirProgram {
    select_costed(p, &config(), 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
}
fn forward(p: &mut MirProgram) -> &mut MirRoutine {
    p.routines.iter_mut().find(|r| r.name == "Forward").unwrap()
}
fn start(r: &MirRoutine) -> usize {
    r.blocks[0]
        .ops
        .iter()
        .position(|o| matches!(o, MirOp::OffsetPointerByIndirectByte { .. }))
        .unwrap()
}

#[test]
fn pointer_byte_plus_one_selection_saves_five_bytes_and_is_stable_with_reporting() {
    let old = baseline();
    let new = selected(old.clone());
    assert_ne!(old, new);
    assert_eq!(
        image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        5
    );
    assert_eq!(selected(new.clone()), new);
    let reported = select_costed(
        old.clone(),
        &Mir6502Config {
            peephole_report: crate::mir6502::passes::MirPeepholeReportMode::PerRoutine,
            ..config()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap();
    assert_eq!(reported, new);
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        if a.name != "Forward" {
            assert_eq!(a, b);
        }
    }
}

#[test]
fn pointer_byte_plus_one_selection_reconstructs_ordered_read_certificates() {
    let mut p = baseline();
    let r = forward(&mut p);
    let i = start(r);
    let original = &r.blocks[0].ops[i..i + 2];
    let s = facts::shape(original).unwrap();
    assert!(facts::entry_read_homes(original, &[s.selected()]).is_some());
    let mut other = s;
    other.offset = 1;
    assert!(facts::entry_read_homes(original, &[other.selected()]).is_none());
    let mut other = s;
    other.dst = MirFixedZpSlot(0xAD);
    assert!(facts::entry_read_homes(original, &[other.selected()]).is_none());
    assert!(facts::entry_read_homes(&original[..1], &[s.selected()]).is_none());
    let removed = crate::mir6502::rewrite::posthome::removed_home_definitions(
        r.blocks[0].id,
        &(i..i + 2),
        &r.blocks[0].ops,
        &[s.selected()],
    );
    assert!(
        crate::mir6502::rewrite::posthome::replacement_home_dependencies(
            original,
            &removed,
            &[s.selected()]
        )
        .is_ok()
    );
    let wrong = facts::Shape { offset: 1, ..s }.selected();
    assert!(
        crate::mir6502::rewrite::posthome::replacement_home_dependencies(
            original,
            &removed,
            &[wrong]
        )
        .is_err()
    );
    let opaque = MirOp::Barrier {
        effects: MirEffects {
            opaque: true,
            ..MirEffects::default()
        },
    };
    assert!(
        crate::mir6502::rewrite::posthome::replacement_home_dependencies(
            original,
            &removed,
            &[opaque]
        )
        .is_err()
    );
}

#[test]
fn pointer_byte_plus_one_selection_preserves_published_x_and_accepts_an_independent_routine() {
    let mut p = baseline();
    let r = forward(&mut p);
    let i = start(r);
    r.blocks[0].ops.truncate(i + 2);
    r.blocks[0].ops.push(MirOp::LoadImm {
        dst: MirDef::Reg(MirReg::A),
        value: 0,
        width: MirWidth::Byte,
    });
    r.blocks[0].ops.push(MirOp::Binary {
        op: MirBinaryOp::Add,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: MirValue::ConstU8(0),
        width: MirWidth::Byte,
        carry_in: Some(MirCarryIn::Clear),
        carry_out: MirCarryOut::Ignore,
    });
    let id = r.id;
    let old_exits = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    assert!(old_exits.get(id).unwrap().preserves_register(MirReg::X));
    let r = p.routines.iter_mut().find(|r| r.name == "PlusTwo").unwrap();
    let i = start(r);
    if let MirOp::AddByteToWordMem { value, .. } = &mut r.blocks[0].ops[i + 1] {
        *value = MirValue::ConstU8(1);
    }
    let old = p.clone();
    let new = selected(p);
    assert_eq!(
        old.routines.iter().find(|r| r.id == id),
        new.routines.iter().find(|r| r.id == id)
    );
    assert_ne!(
        old.routines.iter().find(|r| r.name == "PlusTwo"),
        new.routines.iter().find(|r| r.name == "PlusTwo")
    );
    assert!(
        MirKnownCalleeSummaries::analyze_for_rewriting(&new)
            .get(id)
            .unwrap()
            .preserves_register(MirReg::X)
    );
}

#[test]
fn pointer_byte_plus_one_unknown_cost_keeps_the_entire_baseline() {
    let mut old = baseline();
    let r = old
        .routines
        .iter_mut()
        .find(|r| r.name == "PlusZero")
        .unwrap();
    if let Some(MirOp::Call { target, .. }) = r.blocks[0]
        .ops
        .iter_mut()
        .find(|o| matches!(o, MirOp::Call { .. }))
    {
        *target = MirCallTarget::Routine(RoutineId(u32::MAX));
    }
    assert!(image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).is_none());
    assert_eq!(selected(old.clone()), old);
}
