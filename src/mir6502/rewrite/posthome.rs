use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::mir6502::analysis::effects::{MirHomeByte, classify_op};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirAddr, MirBlockId, MirMem, MirOp, MirRoutine, MirWidth};
use crate::mir6502::rewrite::context::{MirExitStateChange, MirProof, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::{
    MirChangeSet, MirPostHomeRewritePlan, MirRemovedHomeDefinition,
};

/// Constructs a post-home plan only when every concrete home definition that
/// disappears is dead outside the proposed transaction. Replacement writes
/// preserve the latest original definitions for the same home; any excess
/// earlier writes are transient definitions and must be proven dead.
pub(in crate::mir6502) fn structural_plan(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    block: MirBlockId,
    range: Range<usize>,
    replacement: Vec<MirOp>,
    mut exit_state_change: MirExitStateChange,
    stat: &'static str,
    family_priority: u16,
) -> Option<MirPostHomeRewritePlan> {
    let block_ops = &routine
        .blocks
        .iter()
        .find(|candidate| candidate.id == block)?
        .ops;
    if range.start >= range.end || range.end > block_ops.len() {
        return None;
    }
    let removed_homes = removed_home_definitions(block, &range, block_ops, &replacement);
    let original_write_counts = home_write_counts(&block_ops[range.clone()]);
    for (home, replacement_count) in home_write_counts(&replacement) {
        if replacement_count > original_write_counts.get(&home).copied().unwrap_or(0) {
            exit_state_change.homes.insert(home);
        }
    }
    let end = context.point(MirSite::Op {
        block,
        op_index: range.end - 1,
    });
    for removed in &removed_homes {
        if let MirProof::Blocked(blocker) =
            context.home_definition_dead_after(removed.home, context.point(removed.store), end)
        {
            context.record_blocker(stat, block, range.start, &blocker);
            return None;
        }
    }
    if let MirProof::Blocked(blocker) =
        context.exit_state_change_is_unobservable(&exit_state_change, end)
    {
        context.record_blocker(stat, block, range.start, &blocker);
        return None;
    }

    let original_cost = estimated_6502_cost(&block_ops[range.clone()]);
    let replacement_cost = estimated_6502_cost(&replacement);

    Some(MirPostHomeRewritePlan {
        generation: context.generation(),
        block,
        range,
        replacement,
        removed_homes,
        exit_state_change,
        change_set: MirChangeSet::posthome_operation_change(),
        stat,
        observations: Vec::new(),
        family_priority,
        estimated_byte_saving: original_cost.0.saturating_sub(replacement_cost.0),
        estimated_cycle_saving: original_cost.1.saturating_sub(replacement_cost.1),
    })
}

/// A deliberately small 6502 cost model used only to rank already-legal
/// competing plans and report estimated gains. Exact bytes remain a listing
/// measurement; barriers and machine blocks are neutral because their payload
/// cost is not represented by one MIR operation.
fn estimated_6502_cost(ops: &[MirOp]) -> (u16, u16) {
    ops.iter().fold((0u16, 0u16), |(bytes, cycles), op| {
        let (op_bytes, op_cycles) = estimated_op_cost(op);
        (
            bytes.saturating_add(op_bytes),
            cycles.saturating_add(op_cycles),
        )
    })
}

fn estimated_op_cost(op: &MirOp) -> (u16, u16) {
    match op {
        MirOp::LoadImm { width, .. } => width_cost(*width, (2, 2), (4, 4)),
        MirOp::Load { src, width, .. } => address_cost(src, *width, false),
        MirOp::Store { dst, width, .. } => address_cost(dst, *width, true),
        MirOp::Move { width, .. } => width_cost(*width, (1, 2), (4, 6)),
        MirOp::LeaAddr { .. } => (4, 4),
        MirOp::Extend { .. } | MirOp::Truncate { .. } => (3, 4),
        MirOp::Unary { width, .. } => width_cost(*width, (1, 2), (4, 6)),
        MirOp::Binary { width, .. } => width_cost(*width, (2, 2), (8, 12)),
        MirOp::UpdateMem { mem, width, .. } => {
            let zp = mem_is_zero_page(mem);
            match (width, zp) {
                (MirWidth::Byte, true) => (2, 5),
                (MirWidth::Byte, false) => (3, 6),
                (MirWidth::Word, true) => (6, 10),
                (MirWidth::Word, false) => (8, 12),
            }
        }
        MirOp::UpdateIndexedMem { .. } => (3, 7),
        MirOp::AddByteToWordMem { .. } | MirOp::SubByteFromWordMem { .. } => (8, 12),
        MirOp::Compare { width, .. } => width_cost(*width, (2, 2), (6, 8)),
        MirOp::CompareIndirectBytes { .. } => (6, 12),
        MirOp::Call { .. } | MirOp::RuntimeHelper { .. } => (3, 6),
        MirOp::MaterializeAddress { .. } => (6, 8),
        MirOp::MaterializeIndexedAddress { consumer, .. } if consumer.uses_scaled_y() => (8, 12),
        MirOp::MaterializeIndexedAddress { .. } => (12, 18),
        MirOp::AdvanceAddress { .. } => (8, 12),
        MirOp::LoadIndirect { .. } => (2, 5),
        MirOp::StoreIndirect { .. } => (2, 6),
        MirOp::IndirectByteCompound { .. } => (8, 12),
        MirOp::Barrier { .. } | MirOp::MachineBlock { .. } => (0, 0),
    }
}

fn address_cost(addr: &MirAddr, width: MirWidth, store: bool) -> (u16, u16) {
    let (bytes, cycles) = match addr {
        MirAddr::Direct(mem) if mem_is_zero_page(mem) => (2, 3),
        MirAddr::Direct(_) => (3, 4),
        MirAddr::AbsoluteIndexedX { .. } | MirAddr::AbsoluteIndexedY { .. } => {
            (3, if store { 5 } else { 4 })
        }
        MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => (2, if store { 6 } else { 5 }),
        MirAddr::Label(_) => (3, 4),
        MirAddr::ComputedIndex { .. }
        | MirAddr::PointerCell { .. }
        | MirAddr::PointerIndex { .. }
        | MirAddr::Deref { .. } => (8, 12),
    };
    match width {
        MirWidth::Byte => (bytes, cycles),
        MirWidth::Word => (bytes.saturating_mul(2), cycles.saturating_mul(2)),
    }
}

fn mem_is_zero_page(mem: &MirMem) -> bool {
    matches!(mem, MirMem::ZeroPage(_) | MirMem::FixedZeroPage(_))
}

fn width_cost(width: MirWidth, byte: (u16, u16), word: (u16, u16)) -> (u16, u16) {
    match width {
        MirWidth::Byte => byte,
        MirWidth::Word => word,
    }
}

fn removed_home_definitions(
    block: MirBlockId,
    range: &Range<usize>,
    block_ops: &[MirOp],
    replacement: &[MirOp],
) -> Vec<MirRemovedHomeDefinition> {
    let mut replacement_writes = BTreeMap::<MirHomeByte, usize>::new();
    for op in replacement {
        for home in written_homes(op) {
            *replacement_writes.entry(home).or_default() += 1;
        }
    }

    let mut preserved = BTreeMap::<MirHomeByte, usize>::new();
    let mut removed = Vec::new();
    for op_index in range.clone().rev() {
        for home in written_homes(&block_ops[op_index]) {
            let keep = replacement_writes.get(&home).copied().unwrap_or(0);
            let already_preserved = preserved.entry(home).or_default();
            if *already_preserved < keep {
                *already_preserved += 1;
            } else {
                removed.push(MirRemovedHomeDefinition {
                    home,
                    store: MirSite::Op { block, op_index },
                });
            }
        }
    }
    removed.sort();
    removed
}

fn written_homes(op: &MirOp) -> BTreeSet<MirHomeByte> {
    let effects = classify_op(op);
    effects
        .homes
        .writes
        .into_iter()
        .chain(effects.addresses.pair_writes)
        .collect()
}

fn home_write_counts(ops: &[MirOp]) -> BTreeMap<MirHomeByte, usize> {
    let mut counts = BTreeMap::new();
    for op in ops {
        for home in written_homes(op) {
            *counts.entry(home).or_default() += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
    use crate::mir6502::analysis::sites::MirRoutineGeneration;
    use crate::mir6502::ir::{
        MirAddr, MirBlock, MirDef, MirEffects, MirFixedZpSlot, MirFrame, MirMem, MirReg,
        MirRoutineAbi, MirSpillId, MirTerminator, MirValue, MirWidth, RoutineId,
    };

    fn store(id: u32) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill {
                id: MirSpillId(id),
                offset: 0,
            }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }
    }

    fn load(id: u32) -> MirOp {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Spill {
                id: MirSpillId(id),
                offset: 0,
            }),
            width: MirWidth::Byte,
        }
    }

    fn store_fixed(slot: u8) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }
    }

    fn load_fixed(slot: u8) -> MirOp {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
            width: MirWidth::Byte,
        }
    }

    fn routine(ops: Vec<MirOp>, terminator: MirTerminator) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "posthome".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops,
                terminator,
            }],
            effects: MirEffects::default(),
        }
    }

    #[test]
    fn structural_plan_blocks_a_removed_home_definition_used_after_window() {
        let routine = routine(vec![store(0), load(0)], MirTerminator::Return);
        let snapshot =
            PostHomeAnalysisSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
        let context = PostHomeRewriteContext::new(&snapshot);
        assert!(
            structural_plan(
                &routine,
                &context,
                MirBlockId(0),
                0..1,
                Vec::new(),
                MirExitStateChange::default(),
                "remove-store",
                0,
            )
            .is_none()
        );
        let blocked = context.take_blocked_sites();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].stat, "remove-store");
        assert_eq!(blocked[0].reason, "home-definition-live");
        assert!(context.take_blocked_sites().is_empty());
    }

    #[test]
    fn structural_plan_accepts_a_transient_home_definition_overwritten_later() {
        let routine = routine(vec![store(0), store(0)], MirTerminator::Return);
        let snapshot =
            PostHomeAnalysisSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
        let context = PostHomeRewriteContext::new(&snapshot);
        let plan = structural_plan(
            &routine,
            &context,
            MirBlockId(0),
            0..1,
            Vec::new(),
            MirExitStateChange::default(),
            "remove-store",
            0,
        )
        .unwrap();
        assert_eq!(plan.removed_homes.len(), 1);
        assert!(plan.estimated_byte_saving > 0);
        assert!(plan.estimated_cycle_saving > 0);
    }

    #[test]
    fn structural_plan_blocks_retargeting_when_old_or_new_pointer_home_is_live() {
        for later_read in [0xA8, 0xAC] {
            let routine = routine(
                vec![store_fixed(0xA8), load_fixed(later_read)],
                MirTerminator::Return,
            );
            let snapshot =
                PostHomeAnalysisSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
            let context = PostHomeRewriteContext::new(&snapshot);
            assert!(
                structural_plan(
                    &routine,
                    &context,
                    MirBlockId(0),
                    0..1,
                    vec![store_fixed(0xAC)],
                    MirExitStateChange::default(),
                    "retarget-pointer-home",
                    0,
                )
                .is_none(),
                "later read of ${later_read:02X} must block pointer-home retargeting"
            );
        }
    }
}
