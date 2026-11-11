use super::*;
use crate::mir6502::{
    analysis::{argument_x_forwarding::tests::baseline, known_callees::MirKnownCalleeSummaries},
    ir::*,
};

fn wrapper(ops: Vec<MirOp>) -> MirRoutine {
    let mut r = baseline()
        .routines
        .into_iter()
        .find(|r| r.name == "ObserveBytes")
        .unwrap();
    r.blocks.truncate(1);
    r.blocks[0].ops = ops;
    r.blocks[0].terminator = MirTerminator::Return;
    r
}
fn boundary() -> MirOp {
    MirOp::Barrier {
        effects: MirEffects::default(),
    }
}
fn load() -> MirOp {
    MirOp::LoadImm {
        dst: MirDef::Reg(MirReg::A),
        value: 0,
        width: MirWidth::Byte,
    }
}
fn mask(bits: u8) -> MirFlagSet {
    MirFlagSet {
        c: bits & 1 != 0,
        z: bits & 2 != 0,
        n: bits & 4 != 0,
        v: bits & 8 != 0,
    }
}

#[test]
fn callee_entry_flags_distinguish_masks_missing_facts_and_nested_inputs() {
    let r = wrapper(vec![load(), boundary()]);
    let summary = MirKnownCalleeEntryFlags::analyze(&r);
    assert_eq!(
        summary,
        MirKnownCalleeEntryFlags::Known {
            may_observe: mask(9)
        }
    );
    for requested in 0..16 {
        assert_eq!(summary.unobserved(mask(requested)), requested & 9 == 0);
    }
    assert_eq!(
        MirKnownCalleeEntryFlags::analyze(&wrapper(vec![boundary()])),
        MirKnownCalleeEntryFlags::Known {
            may_observe: mask(15)
        }
    );
    assert_eq!(
        MirKnownCalleeEntryFlags::analyze(&wrapper(vec![])),
        MirKnownCalleeEntryFlags::Known {
            may_observe: mask(0)
        }
    );
    assert!(!MirKnownCalleeEntryFlags::Unknown.unobserved(mask(6)));
    let mut p = baseline();
    let index = p
        .routines
        .iter()
        .position(|r| r.name == "ObserveBytes")
        .unwrap();
    let id = p.routines[index].id;
    p.routines[index] = r;
    assert!(!MirKnownCalleeSummaries::analyze_for_rewriting(&p).scalar_entry_ignores_flags(id));
}
#[test]
fn callee_entry_flags_do_not_kill_inputs_at_may_clobbers() {
    let op = MirOp::AddByteToWordMem {
        mem: MirMem::FixedZeroPage(MirFixedZpSlot(0xA0)),
        value: MirValue::ConstU8(1),
    };
    let r = wrapper(vec![op, boundary()]);
    assert_eq!(
        MirKnownCalleeEntryFlags::analyze(&r),
        MirKnownCalleeEntryFlags::Known {
            may_observe: mask(15)
        }
    );
    let r = wrapper(vec![
        load(),
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::FromPrevious),
            carry_out: MirCarryOut::Ignore,
        },
        boundary(),
    ]);
    assert_eq!(
        MirKnownCalleeEntryFlags::analyze(&r),
        MirKnownCalleeEntryFlags::Known {
            may_observe: mask(1)
        }
    );
}
#[test]
fn callee_entry_flags_keep_ineligible_and_invalid_routines_unknown() {
    for change in 0..6 {
        let mut r = wrapper(vec![load()]);
        match change {
            0 => r.scalar_signature = None,
            1 => r.effects.opaque = true,
            2 => r.effects.reads.flags = true,
            3 => r.effects.preserves.flags = true,
            4 => r.blocks.clear(),
            _ => {
                r.blocks[0].terminator = MirTerminator::Jump(MirEdge {
                    target: MirBlockId(u32::MAX),
                    args: vec![],
                })
            }
        }
        assert_eq!(
            MirKnownCalleeEntryFlags::analyze(&r),
            MirKnownCalleeEntryFlags::Unknown
        );
    }
}

#[path = "entry_flags_census.rs"]
mod census;
