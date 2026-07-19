use super::dead_spills::block_successor_indices;
use super::defs::op_def;
use super::spills::op_may_clobber_reg;
use super::stats::MirPeepholeStats;
use super::temp_liveness::{MirTempLiveSet, MirTempLiveness};
use super::temp_widths::collect_temp_widths;
use crate::mir6502::ir::{
    MirAddr, MirArgHome, MirCallTarget, MirCarryIn, MirCarryOut, MirCond, MirCondDest, MirDef,
    MirOp, MirProgram, MirReg, MirResultHome, MirRoutine, MirTempId, MirTerminator, MirValue,
    MirWidth,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TempLane {
    id: MirTempId,
    byte: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DefSite {
    block: usize,
    op: usize,
    natural_reg: Option<MirReg>,
    coupled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UseSite {
    block: usize,
    op: Option<usize>,
    accepts_a: bool,
}

#[derive(Debug, Default)]
struct LaneFacts {
    defs: Vec<DefSite>,
    uses: Vec<UseSite>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct HomeDemandCensus {
    pub(super) temp_lanes: usize,
    pub(super) definitions: usize,
    pub(super) uses: usize,
    pub(super) single_use_lanes: usize,
    pub(super) multi_use_lanes: usize,
    pub(super) same_block_lanes: usize,
    pub(super) cross_block_lanes: usize,
    pub(super) terminator_lanes: usize,
    pub(super) join_live_lanes: usize,
    pub(super) backedge_live_lanes: usize,
    pub(super) call_live_lanes: usize,
    pub(super) machine_live_lanes: usize,
    pub(super) barrier_live_lanes: usize,
    pub(super) natural_a_lanes: usize,
    pub(super) natural_x_lanes: usize,
    pub(super) natural_y_lanes: usize,
    pub(super) coupled_lanes: usize,
    pub(super) blocked_clobber_lanes: usize,
    pub(super) unsupported_consumer_lanes: usize,
    pub(super) same_block_a_candidates: usize,
    pub(super) retained_unused_lanes: usize,
    pub(super) retained_non_single_def_lanes: usize,
    pub(super) retained_multi_use_lanes: usize,
    pub(super) retained_terminator_lanes: usize,
    pub(super) retained_coupled_lanes: usize,
    pub(super) retained_call_live_lanes: usize,
    pub(super) retained_machine_live_lanes: usize,
    pub(super) retained_barrier_live_lanes: usize,
    pub(super) retained_backedge_live_lanes: usize,
    pub(super) retained_join_live_lanes: usize,
    pub(super) retained_cross_block_lanes: usize,
    pub(super) retained_non_accumulator_lanes: usize,
    pub(super) retained_clobber_lanes: usize,
    pub(super) retained_unsupported_consumer_lanes: usize,
    pub(super) gross_store_instructions: usize,
    pub(super) gross_reload_instructions: usize,
    pub(super) gross_absolute_code_bytes: usize,
    pub(super) gross_storage_bytes: usize,
}

pub(super) fn record_home_demand_census(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
    stats: &mut MirPeepholeStats,
) {
    let census = scan_home_demand_census(routine, liveness);
    let routine_id = routine.id;
    stats.record_many(
        routine_id,
        "home-demand-preexisting-virtual-zp-cells",
        routine.frame.virtual_zero_page.len(),
    );
    for (name, count) in [
        ("home-demand-temp-lanes", census.temp_lanes),
        ("home-demand-definitions", census.definitions),
        ("home-demand-uses", census.uses),
        ("home-demand-single-use-lanes", census.single_use_lanes),
        ("home-demand-multi-use-lanes", census.multi_use_lanes),
        ("home-demand-same-block-lanes", census.same_block_lanes),
        ("home-demand-cross-block-lanes", census.cross_block_lanes),
        ("home-demand-terminator-lanes", census.terminator_lanes),
        ("home-demand-join-live-lanes", census.join_live_lanes),
        (
            "home-demand-backedge-live-lanes",
            census.backedge_live_lanes,
        ),
        ("home-demand-call-live-lanes", census.call_live_lanes),
        ("home-demand-machine-live-lanes", census.machine_live_lanes),
        ("home-demand-barrier-live-lanes", census.barrier_live_lanes),
        ("home-demand-natural-a-lanes", census.natural_a_lanes),
        ("home-demand-natural-x-lanes", census.natural_x_lanes),
        ("home-demand-natural-y-lanes", census.natural_y_lanes),
        ("home-demand-coupled-lanes", census.coupled_lanes),
        (
            "home-demand-blocked-accumulator-clobber-lanes",
            census.blocked_clobber_lanes,
        ),
        (
            "home-demand-unsupported-consumer-lanes",
            census.unsupported_consumer_lanes,
        ),
        (
            "home-demand-same-block-a-candidates",
            census.same_block_a_candidates,
        ),
        (
            "home-demand-retained-unused-lanes",
            census.retained_unused_lanes,
        ),
        (
            "home-demand-retained-non-single-def-lanes",
            census.retained_non_single_def_lanes,
        ),
        (
            "home-demand-retained-multi-use-lanes",
            census.retained_multi_use_lanes,
        ),
        (
            "home-demand-retained-terminator-lanes",
            census.retained_terminator_lanes,
        ),
        (
            "home-demand-retained-coupled-lanes",
            census.retained_coupled_lanes,
        ),
        (
            "home-demand-retained-call-live-lanes",
            census.retained_call_live_lanes,
        ),
        (
            "home-demand-retained-machine-live-lanes",
            census.retained_machine_live_lanes,
        ),
        (
            "home-demand-retained-barrier-live-lanes",
            census.retained_barrier_live_lanes,
        ),
        (
            "home-demand-retained-backedge-live-lanes",
            census.retained_backedge_live_lanes,
        ),
        (
            "home-demand-retained-join-live-lanes",
            census.retained_join_live_lanes,
        ),
        (
            "home-demand-retained-cross-block-lanes",
            census.retained_cross_block_lanes,
        ),
        (
            "home-demand-retained-non-accumulator-lanes",
            census.retained_non_accumulator_lanes,
        ),
        (
            "home-demand-retained-clobber-lanes",
            census.retained_clobber_lanes,
        ),
        (
            "home-demand-retained-unsupported-consumer-lanes",
            census.retained_unsupported_consumer_lanes,
        ),
        (
            "home-demand-gross-store-instructions",
            census.gross_store_instructions,
        ),
        (
            "home-demand-gross-reload-instructions",
            census.gross_reload_instructions,
        ),
        (
            "home-demand-gross-absolute-code-bytes",
            census.gross_absolute_code_bytes,
        ),
        (
            "home-demand-gross-storage-bytes",
            census.gross_storage_bytes,
        ),
    ] {
        stats.record_many(routine_id, name, count);
    }
}

pub(super) fn record_final_home_allocations(program: &MirProgram, stats: &mut MirPeepholeStats) {
    for routine in &program.routines {
        let ram = routine.frame.spills.len();
        let zp = routine.frame.virtual_zero_page.len();
        let preexisting_zp =
            stats.count_for(routine.id, "home-demand-preexisting-virtual-zp-cells");
        let new_zp = zp.saturating_sub(preexisting_zp);
        stats.record_many(routine.id, "home-demand-final-ram-spill-cells", ram);
        stats.record_many(routine.id, "home-demand-final-virtual-zp-cells", zp);
        stats.record_many(routine.id, "home-demand-final-new-virtual-zp-cells", new_zp);
        stats.record_many(
            routine.id,
            "home-demand-final-temp-home-cells",
            ram.saturating_add(new_zp),
        );
        stats.record_many(
            routine.id,
            "home-demand-final-frame-storage-cells",
            ram.saturating_add(zp),
        );
    }
}

pub(super) fn scan_home_demand_census(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
) -> HomeDemandCensus {
    let widths = routine_temp_widths(routine);
    let mut facts = BTreeMap::<TempLane, LaneFacts>::new();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            record_op_defs(op, block_index, op_index, &mut facts);
            record_op_uses(op, block_index, op_index, &widths, &mut facts);
        }
        record_terminator_uses(&block.terminator, block_index, &widths, &mut facts);
    }

    let keys = facts.keys().copied().collect::<BTreeSet<_>>();
    let predecessors = predecessor_counts(routine);
    let backedge_live = live_across_cycle_edges(routine, liveness, &keys);
    let (call_live, machine_live, barrier_live) =
        live_across_barriers(routine, liveness, &widths, &keys);

    let mut census = HomeDemandCensus::default();
    census.temp_lanes = facts.len();
    for (lane, lane_facts) in &facts {
        census.definitions = census.definitions.saturating_add(lane_facts.defs.len());
        census.uses = census.uses.saturating_add(lane_facts.uses.len());
        census.gross_store_instructions = census
            .gross_store_instructions
            .saturating_add(lane_facts.defs.len());
        census.gross_reload_instructions = census
            .gross_reload_instructions
            .saturating_add(lane_facts.uses.len());
        census.gross_storage_bytes = census.gross_storage_bytes.saturating_add(1);

        match lane_facts.uses.len() {
            1 => census.single_use_lanes = census.single_use_lanes.saturating_add(1),
            2.. => census.multi_use_lanes = census.multi_use_lanes.saturating_add(1),
            _ => {}
        }

        let same_block = lane_facts.defs.len() == 1
            && lane_facts
                .uses
                .iter()
                .all(|site| site.block == lane_facts.defs[0].block);
        if same_block {
            census.same_block_lanes = census.same_block_lanes.saturating_add(1);
        } else if !lane_facts.uses.is_empty() {
            census.cross_block_lanes = census.cross_block_lanes.saturating_add(1);
        }

        let terminator_use = lane_facts.uses.iter().any(|site| site.op.is_none());
        if terminator_use {
            census.terminator_lanes = census.terminator_lanes.saturating_add(1);
        }
        if lane_live_at_join(routine, liveness, &predecessors, *lane) {
            census.join_live_lanes = census.join_live_lanes.saturating_add(1);
        }
        if backedge_live.contains(lane) {
            census.backedge_live_lanes = census.backedge_live_lanes.saturating_add(1);
        }
        if call_live.contains(lane) {
            census.call_live_lanes = census.call_live_lanes.saturating_add(1);
        }
        if machine_live.contains(lane) {
            census.machine_live_lanes = census.machine_live_lanes.saturating_add(1);
        }
        if barrier_live.contains(lane) {
            census.barrier_live_lanes = census.barrier_live_lanes.saturating_add(1);
        }

        let natural_reg = (lane_facts.defs.len() == 1)
            .then(|| lane_facts.defs[0].natural_reg)
            .flatten();
        match natural_reg {
            Some(MirReg::A) => census.natural_a_lanes = census.natural_a_lanes.saturating_add(1),
            Some(MirReg::X) => census.natural_x_lanes = census.natural_x_lanes.saturating_add(1),
            Some(MirReg::Y) => census.natural_y_lanes = census.natural_y_lanes.saturating_add(1),
            None => {}
        }

        let coupled = widths.get(&lane.id) == Some(&MirWidth::Word)
            || lane_facts.defs.iter().any(|site| site.coupled);
        if coupled {
            census.coupled_lanes = census.coupled_lanes.saturating_add(1);
        }

        if lane_facts.uses.is_empty() {
            census.retained_unused_lanes = census.retained_unused_lanes.saturating_add(1);
            continue;
        }
        if lane_facts.defs.len() != 1 {
            census.retained_non_single_def_lanes =
                census.retained_non_single_def_lanes.saturating_add(1);
            continue;
        }
        if lane_facts.uses.len() != 1 {
            census.retained_multi_use_lanes = census.retained_multi_use_lanes.saturating_add(1);
            continue;
        }
        if terminator_use {
            census.retained_terminator_lanes = census.retained_terminator_lanes.saturating_add(1);
            continue;
        }
        if coupled {
            census.retained_coupled_lanes = census.retained_coupled_lanes.saturating_add(1);
            continue;
        }
        if call_live.contains(lane) {
            census.retained_call_live_lanes = census.retained_call_live_lanes.saturating_add(1);
            continue;
        }
        if machine_live.contains(lane) {
            census.retained_machine_live_lanes =
                census.retained_machine_live_lanes.saturating_add(1);
            continue;
        }
        if barrier_live.contains(lane) {
            census.retained_barrier_live_lanes =
                census.retained_barrier_live_lanes.saturating_add(1);
            continue;
        }
        if backedge_live.contains(lane) {
            census.retained_backedge_live_lanes =
                census.retained_backedge_live_lanes.saturating_add(1);
            continue;
        }
        if lane_live_at_join(routine, liveness, &predecessors, *lane) {
            census.retained_join_live_lanes = census.retained_join_live_lanes.saturating_add(1);
            continue;
        }
        if !same_block {
            census.retained_cross_block_lanes = census.retained_cross_block_lanes.saturating_add(1);
            continue;
        }
        if natural_reg != Some(MirReg::A) {
            census.retained_non_accumulator_lanes =
                census.retained_non_accumulator_lanes.saturating_add(1);
            continue;
        }

        let def = lane_facts.defs[0];
        let use_site = lane_facts.uses[0];
        let Some(use_op) = use_site.op else {
            continue;
        };
        if accumulator_clobbered_between(routine, def.block, def.op, use_op) {
            census.blocked_clobber_lanes = census.blocked_clobber_lanes.saturating_add(1);
            census.retained_clobber_lanes = census.retained_clobber_lanes.saturating_add(1);
        } else if !use_site.accepts_a {
            census.unsupported_consumer_lanes = census.unsupported_consumer_lanes.saturating_add(1);
            census.retained_unsupported_consumer_lanes =
                census.retained_unsupported_consumer_lanes.saturating_add(1);
        } else {
            census.same_block_a_candidates = census.same_block_a_candidates.saturating_add(1);
        }
    }
    census.gross_absolute_code_bytes = census
        .gross_store_instructions
        .saturating_add(census.gross_reload_instructions)
        .saturating_mul(3);
    census
}

fn routine_temp_widths(routine: &MirRoutine) -> BTreeMap<MirTempId, MirWidth> {
    let mut widths = BTreeMap::new();
    for block in &routine.blocks {
        for (id, width) in collect_temp_widths(&block.ops) {
            note_width(&mut widths, id, width);
        }
        for op in &block.ops {
            if let MirOp::LoadIndirect { dst, .. } = op {
                note_def_width(&mut widths, dst, MirWidth::Byte);
            }
        }
    }
    widths
}

fn note_def_width(widths: &mut BTreeMap<MirTempId, MirWidth>, def: &MirDef, width: MirWidth) {
    match def {
        MirDef::VTemp(id) => note_width(widths, *id, width),
        MirDef::VTempByte { id, byte } => note_width(
            widths,
            *id,
            if *byte == 0 {
                MirWidth::Byte
            } else {
                MirWidth::Word
            },
        ),
        MirDef::Reg(_) => {}
    }
}

fn note_width(widths: &mut BTreeMap<MirTempId, MirWidth>, id: MirTempId, width: MirWidth) {
    widths
        .entry(id)
        .and_modify(|existing| {
            if width == MirWidth::Word {
                *existing = MirWidth::Word;
            }
        })
        .or_insert(width);
}

fn record_op_defs(
    op: &MirOp,
    block: usize,
    op_index: usize,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    let (def, width) = match op {
        MirOp::LoadImm { dst, width, .. }
        | MirOp::Load { dst, width, .. }
        | MirOp::Move { dst, width, .. }
        | MirOp::LeaAddr { dst, width, .. }
        | MirOp::Unary { dst, width, .. }
        | MirOp::Binary { dst, width, .. } => (Some(dst), Some(*width)),
        MirOp::Extend { dst, to_width, .. } => (Some(dst), Some(*to_width)),
        MirOp::Truncate { dst, to_width, .. } => (Some(dst), Some(*to_width)),
        MirOp::LoadIndirect { dst, .. } => (Some(dst), Some(MirWidth::Byte)),
        MirOp::Call {
            result: Some(result),
            ..
        } => (Some(&result.dst), Some(result.width)),
        _ => (None, None),
    };
    if let (Some(def), Some(width)) = (def, width) {
        for lane in def_lanes(def, width) {
            facts.entry(lane).or_default().defs.push(DefSite {
                block,
                op: op_index,
                natural_reg: natural_result_reg(op, lane.byte),
                coupled: op_couples_result_lanes(op),
            });
        }
    }
    if let MirOp::Compare {
        dst: MirCondDest::Temp(id),
        ..
    } = op
    {
        facts
            .entry(TempLane { id: *id, byte: 0 })
            .or_default()
            .defs
            .push(DefSite {
                block,
                op: op_index,
                natural_reg: None,
                coupled: false,
            });
    }
}

fn def_lanes(def: &MirDef, width: MirWidth) -> Vec<TempLane> {
    match def {
        MirDef::VTemp(id) => lanes_for_width(*id, width),
        MirDef::VTempByte { id, byte } => vec![TempLane {
            id: *id,
            byte: *byte,
        }],
        MirDef::Reg(_) => Vec::new(),
    }
}

fn lanes_for_width(id: MirTempId, width: MirWidth) -> Vec<TempLane> {
    match width {
        MirWidth::Byte => vec![TempLane { id, byte: 0 }],
        MirWidth::Word => vec![TempLane { id, byte: 0 }, TempLane { id, byte: 1 }],
    }
}

fn natural_result_reg(op: &MirOp, byte: u8) -> Option<MirReg> {
    match op {
        MirOp::LoadImm { width, .. }
        | MirOp::Load { width, .. }
        | MirOp::Unary { width, .. }
        | MirOp::Binary { width, .. }
            if *width == MirWidth::Byte && byte == 0 =>
        {
            Some(MirReg::A)
        }
        MirOp::Move {
            src: MirValue::Def(MirDef::Reg(reg)),
            width: MirWidth::Byte,
            ..
        } if byte == 0 => Some(*reg),
        MirOp::Move {
            width: MirWidth::Byte,
            ..
        }
        | MirOp::LoadIndirect { .. }
            if byte == 0 =>
        {
            Some(MirReg::A)
        }
        MirOp::Call {
            result: Some(result),
            ..
        } => result_home_reg(&result.home, byte),
        _ => None,
    }
}

fn result_home_reg(home: &MirResultHome, byte: u8) -> Option<MirReg> {
    match home {
        MirResultHome::Reg(reg) if byte == 0 => Some(*reg),
        MirResultHome::RegisterPair { lo, hi } => match byte {
            0 => Some(*lo),
            1 => Some(*hi),
            _ => None,
        },
        _ => None,
    }
}

fn op_couples_result_lanes(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::Binary {
            carry_in: Some(MirCarryIn::FromPrevious),
            ..
        } | MirOp::Binary {
            carry_out: MirCarryOut::Produce,
            ..
        } | MirOp::Extend {
            to_width: MirWidth::Word,
            ..
        }
    )
}

fn record_op_uses(
    op: &MirOp,
    block: usize,
    op_index: usize,
    widths: &BTreeMap<MirTempId, MirWidth>,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    match op {
        MirOp::Load { src, .. } => record_addr_uses(src, block, op_index, widths, facts),
        MirOp::Store { dst, src, width } => {
            record_addr_uses(dst, block, op_index, widths, facts);
            record_op_value_use(
                src,
                *width,
                *width == MirWidth::Byte,
                block,
                op_index,
                facts,
            );
        }
        MirOp::Move { src, width, .. } | MirOp::Unary { src, width, .. } => record_op_value_use(
            src,
            *width,
            *width == MirWidth::Byte,
            block,
            op_index,
            facts,
        ),
        MirOp::Extend {
            src, from_width, ..
        }
        | MirOp::Truncate {
            src, from_width, ..
        } => record_op_value_use(
            src,
            *from_width,
            *from_width == MirWidth::Byte,
            block,
            op_index,
            facts,
        ),
        MirOp::Binary {
            left, right, width, ..
        }
        | MirOp::Compare {
            left, right, width, ..
        } => {
            record_op_value_use(
                left,
                *width,
                *width == MirWidth::Byte,
                block,
                op_index,
                facts,
            );
            record_op_value_use(right, *width, false, block, op_index, facts);
        }
        MirOp::AddByteToWordMem { value, .. } | MirOp::SubByteFromWordMem { value, .. } => {
            record_op_value_use(value, MirWidth::Byte, false, block, op_index, facts)
        }
        MirOp::Call { target, args, .. } => {
            if let MirCallTarget::Indirect { target, width } = target {
                record_op_value_use(target, *width, false, block, op_index, facts);
            }
            for arg in args {
                record_op_value_use(
                    &arg.value,
                    arg.width,
                    matches!(arg.home, MirArgHome::Reg(MirReg::A)) && arg.width == MirWidth::Byte,
                    block,
                    op_index,
                    facts,
                );
            }
        }
        MirOp::MaterializeAddress { value, .. } => {
            record_op_value_use(value, MirWidth::Word, false, block, op_index, facts)
        }
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            record_op_value_use(base, MirWidth::Word, false, block, op_index, facts);
            record_op_value_use(
                index,
                inferred_value_width(index, widths),
                false,
                block,
                op_index,
                facts,
            );
        }
        MirOp::AdvanceAddress { index, .. } => record_op_value_use(
            index,
            inferred_value_width(index, widths),
            true,
            block,
            op_index,
            facts,
        ),
        MirOp::StoreIndirect { src, .. } => {
            record_op_value_use(src, MirWidth::Byte, true, block, op_index, facts)
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn record_op_value_use(
    value: &MirValue,
    width: MirWidth,
    accepts_a: bool,
    block: usize,
    op: usize,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    record_value_uses(value, width, block, Some(op), accepts_a, facts);
}

fn record_addr_uses(
    addr: &MirAddr,
    block: usize,
    op: usize,
    widths: &BTreeMap<MirTempId, MirWidth>,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            record_value_uses(base, MirWidth::Word, block, Some(op), false, facts);
            record_value_uses(
                index,
                inferred_value_width(index, widths),
                block,
                Some(op),
                false,
                facts,
            );
        }
        MirAddr::PointerIndex { index, .. } => record_value_uses(
            index,
            inferred_value_width(index, widths),
            block,
            Some(op),
            false,
            facts,
        ),
        MirAddr::Deref { ptr, .. } => {
            record_value_uses(ptr, MirWidth::Word, block, Some(op), false, facts)
        }
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::PointerCell { .. } => {}
    }
}

fn inferred_value_width(value: &MirValue, widths: &BTreeMap<MirTempId, MirWidth>) -> MirWidth {
    match value {
        MirValue::ConstU16(_)
        | MirValue::Word { .. }
        | MirValue::Def(MirDef::VTempByte { byte: 1, .. }) => MirWidth::Word,
        MirValue::Def(MirDef::VTemp(id)) => widths.get(id).copied().unwrap_or(MirWidth::Byte),
        _ => MirWidth::Byte,
    }
}

fn record_value_uses(
    value: &MirValue,
    width: MirWidth,
    block: usize,
    op: Option<usize>,
    accepts_a: bool,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    match value {
        MirValue::Def(MirDef::VTemp(id)) => {
            for lane in lanes_for_width(*id, width) {
                facts.entry(lane).or_default().uses.push(UseSite {
                    block,
                    op,
                    accepts_a,
                });
            }
        }
        MirValue::Def(MirDef::VTempByte { id, byte }) => {
            facts
                .entry(TempLane {
                    id: *id,
                    byte: *byte,
                })
                .or_default()
                .uses
                .push(UseSite {
                    block,
                    op,
                    accepts_a,
                });
        }
        MirValue::Word { lo, hi } => {
            record_value_uses(lo, MirWidth::Byte, block, op, accepts_a, facts);
            record_value_uses(hi, MirWidth::Byte, block, op, accepts_a, facts);
        }
        _ => {}
    }
}

fn record_terminator_uses(
    terminator: &MirTerminator,
    block: usize,
    _widths: &BTreeMap<MirTempId, MirWidth>,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    if let MirTerminator::Branch {
        cond: MirCond::BoolValue(value),
        ..
    } = terminator
    {
        record_value_uses(value, MirWidth::Byte, block, None, false, facts);
    }
}

fn predecessor_counts(routine: &MirRoutine) -> Vec<usize> {
    let mut predecessors = vec![0usize; routine.blocks.len()];
    for block in &routine.blocks {
        for successor in block_successor_indices(routine, &block.terminator) {
            predecessors[successor] = predecessors[successor].saturating_add(1);
        }
    }
    predecessors
}

fn lane_live_at_join(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
    predecessors: &[usize],
    lane: TempLane,
) -> bool {
    routine.blocks.iter().enumerate().any(|(block_index, _)| {
        predecessors.get(block_index).copied().unwrap_or(0) > 1
            && liveness
                .live_in(block_index)
                .is_some_and(|live| lane_live(live, lane))
    })
}

fn live_across_cycle_edges(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
    keys: &BTreeSet<TempLane>,
) -> BTreeSet<TempLane> {
    let successors = routine
        .blocks
        .iter()
        .map(|block| block_successor_indices(routine, &block.terminator))
        .collect::<Vec<_>>();
    let mut live = BTreeSet::new();
    for (from, targets) in successors.iter().enumerate() {
        for target in targets {
            if !path_exists(&successors, *target, from) {
                continue;
            }
            for lane in keys {
                if liveness
                    .live_out(from)
                    .is_some_and(|set| lane_live(set, *lane))
                    && liveness
                        .live_in(*target)
                        .is_some_and(|set| lane_live(set, *lane))
                {
                    live.insert(*lane);
                }
            }
        }
    }
    live
}

fn path_exists(successors: &[Vec<usize>], start: usize, target: usize) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(block) = pending.pop() {
        if block == target {
            return true;
        }
        if !visited.insert(block) {
            continue;
        }
        pending.extend(successors.get(block).into_iter().flatten().copied());
    }
    false
}

fn live_across_barriers(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
    widths: &BTreeMap<MirTempId, MirWidth>,
    keys: &BTreeSet<TempLane>,
) -> (BTreeSet<TempLane>, BTreeSet<TempLane>, BTreeSet<TempLane>) {
    let mut call_live = BTreeSet::new();
    let mut machine_live = BTreeSet::new();
    let mut barrier_live = BTreeSet::new();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        let mut live = keys
            .iter()
            .copied()
            .filter(|lane| {
                liveness
                    .live_out(block_index)
                    .is_some_and(|set| lane_live(set, *lane))
            })
            .collect::<BTreeSet<_>>();
        let mut terminator_facts = BTreeMap::new();
        record_terminator_uses(
            &block.terminator,
            block_index,
            widths,
            &mut terminator_facts,
        );
        live.extend(terminator_facts.into_keys());

        for (op_index, op) in block.ops.iter().enumerate().rev() {
            let live_after = live.clone();
            let mut op_facts = BTreeMap::new();
            record_op_defs(op, block_index, op_index, &mut op_facts);
            record_op_uses(op, block_index, op_index, widths, &mut op_facts);
            for (lane, facts) in &op_facts {
                if !facts.defs.is_empty() {
                    live.remove(lane);
                }
            }
            for (lane, facts) in &op_facts {
                if !facts.uses.is_empty() {
                    live.insert(*lane);
                }
            }
            if matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. }) {
                call_live.extend(live.intersection(&live_after).copied());
            }
            if matches!(op, MirOp::MachineBlock { .. }) {
                machine_live.extend(live.intersection(&live_after).copied());
            }
            if matches!(op, MirOp::Barrier { .. }) {
                barrier_live.extend(live.intersection(&live_after).copied());
            }
        }
    }
    (call_live, machine_live, barrier_live)
}

fn lane_live(set: &MirTempLiveSet, lane: TempLane) -> bool {
    set.full_temp_live(lane.id) || set.exact_lane_live(lane.id, lane.byte)
}

fn accumulator_clobbered_between(
    routine: &MirRoutine,
    block_index: usize,
    def_op: usize,
    use_op: usize,
) -> bool {
    let Some(block) = routine.blocks.get(block_index) else {
        return true;
    };
    if use_op <= def_op {
        return true;
    }
    block.ops[def_op.saturating_add(1)..use_op]
        .iter()
        .any(op_may_clobber_accumulator_during_materialization)
}

fn op_may_clobber_accumulator_during_materialization(op: &MirOp) -> bool {
    if op_may_clobber_reg(op, MirReg::A) {
        return true;
    }
    if matches!(
        op,
        MirOp::Move {
            dst: MirDef::VTemp(_) | MirDef::VTempByte { .. },
            src: MirValue::Def(MirDef::Reg(MirReg::X | MirReg::Y)),
            width: MirWidth::Byte,
        }
    ) {
        return false;
    }
    if op_def(op).is_some_and(|def| matches!(def, MirDef::VTemp(_) | MirDef::VTempByte { .. })) {
        return true;
    }
    match op {
        MirOp::Compare {
            width: MirWidth::Byte,
            ..
        }
        | MirOp::StoreIndirect { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. } => true,
        MirOp::Store {
            src:
                MirValue::ConstU8(_)
                | MirValue::PointerCell(_)
                | MirValue::StorageAddrByte { .. }
                | MirValue::RoutineAddrByte { .. }
                | MirValue::Def(MirDef::VTemp(_) | MirDef::VTempByte { .. }),
            width: MirWidth::Byte,
            ..
        } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirBlockId, MirEffects, MirFrame, MirMem, MirRoutineAbi, MirTemp, RoutineId,
    };
    use crate::mir6502::materialize::temp_liveness::analyze_temp_liveness;

    fn routine(blocks: Vec<MirBlock>, temps: u32) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "HomeCensus".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: (0..temps).map(|id| MirTemp { id: MirTempId(id) }).collect(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: MirBlockId(id),
            label: format!("b{id}"),
            ops,
            terminator,
        }
    }

    fn temp(id: u32) -> MirDef {
        MirDef::VTemp(MirTempId(id))
    }

    fn temp_value(id: u32) -> MirValue {
        MirValue::Def(temp(id))
    }

    fn store_temp(id: u32) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x4000)),
            src: temp_value(id),
            width: MirWidth::Byte,
        }
    }

    fn scan(routine: &MirRoutine) -> HomeDemandCensus {
        let liveness = analyze_temp_liveness(routine);
        let census = scan_home_demand_census(routine, &liveness);
        assert_eq!(
            census.temp_lanes,
            census.same_block_a_candidates
                + census.retained_unused_lanes
                + census.retained_non_single_def_lanes
                + census.retained_multi_use_lanes
                + census.retained_terminator_lanes
                + census.retained_coupled_lanes
                + census.retained_call_live_lanes
                + census.retained_machine_live_lanes
                + census.retained_barrier_live_lanes
                + census.retained_backedge_live_lanes
                + census.retained_join_live_lanes
                + census.retained_cross_block_lanes
                + census.retained_non_accumulator_lanes
                + census.retained_clobber_lanes
                + census.retained_unsupported_consumer_lanes,
            "candidate and retained-home reasons must partition temp lanes"
        );
        census
    }

    #[test]
    fn census_finds_same_block_single_use_accumulator_candidate() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.temp_lanes, 1);
        assert_eq!(census.definitions, 1);
        assert_eq!(census.uses, 1);
        assert_eq!(census.single_use_lanes, 1);
        assert_eq!(census.same_block_lanes, 1);
        assert_eq!(census.natural_a_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 1);
        assert_eq!(census.gross_absolute_code_bytes, 6);
        assert_eq!(census.gross_storage_bytes, 1);
    }

    #[test]
    fn census_records_aggregate_reporting_counters() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );
        let liveness = analyze_temp_liveness(&routine);
        let mut stats = MirPeepholeStats::default();

        record_home_demand_census(&routine, &liveness, &mut stats);

        let counts = stats.aggregate_counts();
        assert_eq!(counts.get("home-demand-temp-lanes"), Some(&1));
        assert_eq!(counts.get("home-demand-same-block-a-candidates"), Some(&1));
        assert_eq!(
            counts.get("home-demand-gross-absolute-code-bytes"),
            Some(&6)
        );
    }

    #[test]
    fn census_reports_accumulator_clobber_before_single_use() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::LoadImm {
                        dst: MirDef::Reg(MirReg::A),
                        value: 9,
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.blocked_clobber_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
    }

    #[test]
    fn census_treats_implicit_temp_materialization_as_accumulator_clobber() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::LoadIndirect {
                        consumer: crate::mir6502::ir::MirAddressConsumer::IndirectIndexedY(
                            crate::mir6502::ir::MirPointerPair::Fixed {
                                lo: crate::mir6502::ir::MirFixedZpSlot(0xAC),
                            },
                        ),
                        dst: temp(1),
                        offset: 0,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            2,
        );

        let census = scan(&routine);

        assert_eq!(census.blocked_clobber_lanes, 1);
        assert_eq!(census.retained_unused_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
    }

    #[test]
    fn census_tracks_values_live_across_calls_machine_blocks_and_barriers() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::Call {
                        target: MirCallTarget::Builtin {
                            name: "noop".to_string(),
                            address: Some(0x4000),
                        },
                        abi: crate::mir6502::ir::MirCallAbi {
                            params: Vec::new(),
                            result: None,
                            clobbers: Default::default(),
                            preserves: Default::default(),
                        },
                        args: Vec::new(),
                        result: None,
                        effects: MirEffects::default(),
                    },
                    MirOp::MachineBlock {
                        id: crate::mir6502::ir::MirMachineBlockId(0),
                        effects: MirEffects::default(),
                    },
                    MirOp::Barrier {
                        effects: MirEffects::default(),
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.call_live_lanes, 1);
        assert_eq!(census.machine_live_lanes, 1);
        assert_eq!(census.barrier_live_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
    }

    #[test]
    fn census_tracks_join_backedge_and_terminator_liveness() {
        let routine = routine(
            vec![
                block(
                    0,
                    vec![MirOp::LoadImm {
                        dst: temp(0),
                        value: 1,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Branch {
                        cond: MirCond::BoolValue(temp_value(0)),
                        then_block: MirBlockId(1),
                        else_block: MirBlockId(2),
                    },
                ),
                block(1, Vec::new(), MirTerminator::Jump(MirBlockId(3))),
                block(2, Vec::new(), MirTerminator::Jump(MirBlockId(3))),
                block(3, vec![store_temp(0)], MirTerminator::Jump(MirBlockId(3))),
            ],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.cross_block_lanes, 1);
        assert_eq!(census.terminator_lanes, 1);
        assert_eq!(census.join_live_lanes, 1);
        assert_eq!(census.backedge_live_lanes, 1);
    }

    #[test]
    fn census_tracks_word_coupling_and_non_accumulator_producers() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 0x1234,
                        width: MirWidth::Word,
                    },
                    MirOp::Move {
                        dst: temp(1),
                        src: MirValue::Def(MirDef::Reg(MirReg::X)),
                        width: MirWidth::Byte,
                    },
                    store_temp(1),
                ],
                MirTerminator::Return,
            )],
            2,
        );

        let census = scan(&routine);

        assert_eq!(census.temp_lanes, 3);
        assert_eq!(census.coupled_lanes, 2);
        assert_eq!(census.natural_x_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
    }

    #[test]
    fn census_reports_unsupported_accumulator_consumer() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::Binary {
                        op: crate::mir6502::ir::MirBinaryOp::And,
                        dst: MirDef::Reg(MirReg::A),
                        left: MirValue::ConstU8(0xff),
                        right: temp_value(0),
                        width: MirWidth::Byte,
                        carry_in: None,
                        carry_out: MirCarryOut::Ignore,
                    },
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.unsupported_consumer_lanes, 1);
        assert_eq!(census.coupled_lanes, 0);
    }
}
