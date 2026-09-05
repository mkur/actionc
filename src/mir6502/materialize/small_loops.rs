use std::collections::{BTreeMap, BTreeSet};

use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::{classify_op, classify_terminator};
use crate::mir6502::ir::{
    MirAddr, MirBinaryOp, MirBlock, MirBlockId, MirCarryIn, MirCarryOut, MirCompareOp, MirCond,
    MirCondDest, MirDef, MirEdge, MirFlagTest, MirMem, MirOp, MirRoutine, MirTemp, MirTempId,
    MirTerminator, MirValue, MirWidth,
};

use super::layout::MaterializeLayout;
use super::memory::op_may_write_mem;

const MAX_UNROLLED_BODY_GROWTH_UNITS: usize = 128;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SmallLoopUnrollStats {
    pub candidates: usize,
    pub selected: usize,
    pub blocked_growth: usize,
    pub blocked_effects: usize,
    pub blocked_observable_induction: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SmallLoopUnrollPlan {
    preheader: MirBlockId,
    header: MirBlockId,
    body: MirBlockId,
    latch: MirBlockId,
    exit: MirBlockId,
    induction: MirMem,
    initial_op: usize,
    body_nodes: BTreeSet<MirBlockId>,
    trip_count: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmallLoopBlock {
    Growth,
    Effects,
    ObservableInduction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HighBitShiftXorPlan {
    test: MirBlockId,
    shifted_xor: MirBlockId,
    shifted_only: MirBlockId,
    value: MirMem,
    xor_rhs: MirValue,
}

/// Select the native 6502 carry produced by a one-bit left shift for the
/// canonical `(value AND $80) # 0` shift/XOR diamond.
///
/// The source cell must be ordinary compiler-controlled memory because this
/// rewrite coalesces three reads into one.  Both original arms already finish
/// with the carry, N, and Z state produced by the same shift (and optional
/// XOR), so the selected form preserves all outgoing machine state as well as
/// the stored value.
pub(super) fn select_high_bit_shift_xor_diamonds(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
) -> usize {
    let mut selected = 0;
    while let Some(plan) = discover_high_bit_shift_xor_diamond(routine, layout) {
        if !apply_high_bit_shift_xor_plan(routine, &plan) {
            break;
        }
        selected += 1;
    }
    selected
}

/// Remove the intermediate backing-store writes between consecutive selected
/// shift/XOR diamonds once register-value cleanup has removed the matching
/// reload.  The two incoming arms must be the next block's only predecessors,
/// so A is the complete value carrier on every path and the memory cell cannot
/// be observed between the removed store and the next shift.
pub(super) fn coalesce_chained_shift_xor_stores(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
) -> usize {
    let mut selected = 0;
    loop {
        let Ok(cfg) = MirCfg::from_routine(routine) else {
            break;
        };
        let mut plan = None;
        for test in &routine.blocks {
            let MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::CSet),
                then_edge,
                else_edge,
            } = &test.terminator
            else {
                continue;
            };
            let Some((then_mem, then_target)) =
                terminal_store_a_and_jump(routine, then_edge.target)
            else {
                continue;
            };
            let Some((else_mem, else_target)) =
                terminal_store_a_and_jump(routine, else_edge.target)
            else {
                continue;
            };
            if then_mem != else_mem
                || then_target != else_target
                || !mem_allows_dead_store_removal(layout, &then_mem)
                || cfg.predecessors(then_target)
                    != &BTreeSet::from([then_edge.target, else_edge.target])
            {
                continue;
            }
            let Some(next) = block_by_id(routine, then_target) else {
                continue;
            };
            if !matches!(
                next.ops.first(),
                Some(MirOp::Binary {
                    op: MirBinaryOp::Lsh,
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    right: MirValue::ConstU8(1) | MirValue::ConstU16(1),
                    width: MirWidth::Byte,
                    carry_in: None,
                    carry_out: MirCarryOut::Produce,
                })
            ) {
                continue;
            }
            plan = Some((then_edge.target, else_edge.target));
            break;
        }
        let Some((then_block, else_block)) = plan else {
            break;
        };
        for block_id in [then_block, else_block] {
            let Some(block) = routine.blocks.iter_mut().find(|block| block.id == block_id) else {
                return selected;
            };
            block.ops.pop();
        }
        selected += 1;
    }
    selected
}

/// Remove stores feeding a selected accumulator shift when every predecessor
/// writes the same A value and every forward path overwrites that backing cell
/// before reading it or leaving the routine.  This covers the first step of an
/// unrolled recurrence, whose input is commonly produced by the surrounding
/// loop body rather than by another shift/XOR diamond.
pub(super) fn coalesce_selected_shift_entry_stores(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
) -> usize {
    let mut selected = 0;
    loop {
        let Ok(cfg) = MirCfg::from_routine(routine) else {
            break;
        };
        let mut plan = None;
        for block in &routine.blocks {
            if !matches!(
                block.ops.first(),
                Some(MirOp::Binary {
                    op: MirBinaryOp::Lsh,
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    right: MirValue::ConstU8(1) | MirValue::ConstU16(1),
                    width: MirWidth::Byte,
                    carry_in: None,
                    carry_out: MirCarryOut::Produce,
                })
            ) {
                continue;
            }
            let predecessors = cfg.predecessors(block.id);
            let Some(first_predecessor) = predecessors.first() else {
                continue;
            };
            let Some(first_block) = block_by_id(routine, *first_predecessor) else {
                continue;
            };
            let Some(first_mem) = terminal_store_a(first_block) else {
                continue;
            };
            if !mem_allows_dead_store_removal(layout, &first_mem)
                || !predecessors.iter().all(|predecessor| {
                    terminal_store_a(block_by_id(routine, *predecessor).expect("CFG block exists"))
                        == Some(first_mem.clone())
                })
                || !mem_overwritten_before_read_on_all_paths(routine, &cfg, block.id, &first_mem)
            {
                continue;
            }
            plan = Some(predecessors.iter().copied().collect::<Vec<_>>());
            break;
        }
        let Some(predecessors) = plan else {
            break;
        };
        for predecessor in predecessors {
            let Some(block) = routine
                .blocks
                .iter_mut()
                .find(|block| block.id == predecessor)
            else {
                return selected;
            };
            block.ops.pop();
        }
        selected += 1;
    }
    selected
}

fn terminal_store_a(block: &MirBlock) -> Option<MirMem> {
    match block.ops.last() {
        Some(MirOp::Store {
            dst: MirAddr::Direct(mem),
            src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
            width: MirWidth::Byte,
        }) => Some(mem.clone()),
        _ => None,
    }
}

fn mem_overwritten_before_read_on_all_paths(
    routine: &MirRoutine,
    cfg: &MirCfg,
    start: MirBlockId,
    mem: &MirMem,
) -> bool {
    fn visit(
        routine: &MirRoutine,
        cfg: &MirCfg,
        block_id: MirBlockId,
        mem: &MirMem,
        state: &mut BTreeMap<MirBlockId, u8>,
    ) -> bool {
        match state.get(&block_id) {
            Some(2) => return true,
            Some(1 | 3) => return false,
            _ => {}
        }
        state.insert(block_id, 1);
        let Some(block) = block_by_id(routine, block_id) else {
            state.insert(block_id, 3);
            return false;
        };
        for op in &block.ops {
            let effects = classify_op(op);
            if effects.memory.reads(mem) {
                state.insert(block_id, 3);
                return false;
            }
            if effects.memory.definitely_writes(mem) {
                state.insert(block_id, 2);
                return true;
            }
        }
        let term = classify_terminator(&block.terminator);
        if term.memory.reads(mem) {
            state.insert(block_id, 3);
            return false;
        }
        let successors = cfg.successors(block_id);
        let safe = !successors.is_empty()
            && successors
                .iter()
                .all(|successor| visit(routine, cfg, *successor, mem, state));
        state.insert(block_id, if safe { 2 } else { 3 });
        safe
    }

    visit(routine, cfg, start, mem, &mut BTreeMap::new())
}

/// Merge the identical terminal store from the two arms of a selected
/// shift/XOR diamond into their unique common successor.
pub(super) fn hoist_common_shift_xor_stores(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
) -> usize {
    let mut selected = 0;
    loop {
        let Ok(cfg) = MirCfg::from_routine(routine) else {
            break;
        };
        let mut plan = None;
        for test in &routine.blocks {
            let MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::CSet),
                then_edge,
                else_edge,
            } = &test.terminator
            else {
                continue;
            };
            if !matches!(
                test.ops.last(),
                Some(MirOp::Binary {
                    op: MirBinaryOp::Lsh,
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    right: MirValue::ConstU8(1) | MirValue::ConstU16(1),
                    width: MirWidth::Byte,
                    carry_in: None,
                    carry_out: MirCarryOut::Produce,
                })
            ) {
                continue;
            }
            let Some((then_mem, then_target)) =
                terminal_store_a_and_jump(routine, then_edge.target)
            else {
                continue;
            };
            let Some((else_mem, else_target)) =
                terminal_store_a_and_jump(routine, else_edge.target)
            else {
                continue;
            };
            if then_mem != else_mem
                || then_target != else_target
                || !mem_allows_dead_store_removal(layout, &then_mem)
                || cfg.predecessors(then_target)
                    != &BTreeSet::from([then_edge.target, else_edge.target])
                || block_by_id(routine, then_target).is_none_or(|block| !block.params.is_empty())
            {
                continue;
            }
            plan = Some((then_edge.target, else_edge.target, then_target, then_mem));
            break;
        }
        let Some((then_block, else_block, join, mem)) = plan else {
            break;
        };
        for block_id in [then_block, else_block] {
            let Some(block) = routine.blocks.iter_mut().find(|block| block.id == block_id) else {
                return selected;
            };
            block.ops.pop();
        }
        let Some(join) = routine.blocks.iter_mut().find(|block| block.id == join) else {
            return selected;
        };
        join.ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                width: MirWidth::Byte,
            },
        );
        selected += 1;
    }
    selected
}

fn terminal_store_a_and_jump(
    routine: &MirRoutine,
    block: MirBlockId,
) -> Option<(MirMem, MirBlockId)> {
    let block = block_by_id(routine, block)?;
    let mem = terminal_store_a(block)?;
    let MirTerminator::Jump(edge) = &block.terminator else {
        return None;
    };
    (block.params.is_empty() && edge.args.is_empty()).then(|| (mem, edge.target))
}

fn mem_allows_dead_store_removal(layout: &MaterializeLayout, mem: &MirMem) -> bool {
    match mem {
        MirMem::Local { .. }
        | MirMem::Param { .. }
        | MirMem::Static { .. }
        | MirMem::Spill { .. }
        | MirMem::ZeroPage(_) => true,
        MirMem::Global { id, .. } => layout.global_allows_idempotent_store_removal(*id),
        MirMem::Absolute(_) | MirMem::FixedZeroPage(_) => false,
    }
}

fn discover_high_bit_shift_xor_diamond(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
) -> Option<HighBitShiftXorPlan> {
    let cfg = MirCfg::from_routine(routine).ok()?;
    for test in &routine.blocks {
        let MirTerminator::Branch {
            cond:
                MirCond::FusedCompare {
                    producer,
                    flag_test: MirFlagTest::ZClear,
                },
            then_edge,
            else_edge,
        } = &test.terminator
        else {
            continue;
        };
        if producer.block != test.id
            || !test.params.is_empty()
            || !then_edge.args.is_empty()
            || !else_edge.args.is_empty()
        {
            continue;
        }
        if cfg.predecessors(then_edge.target) != &BTreeSet::from([test.id])
            || cfg.predecessors(else_edge.target) != &BTreeSet::from([test.id])
        {
            continue;
        }
        let shifted_xor = block_by_id(routine, then_edge.target)?;
        let shifted_only = block_by_id(routine, else_edge.target)?;
        let Some((xor_value, xor_rhs, xor_join)) = shifted_xor_arm(shifted_xor) else {
            continue;
        };
        let Some((shift_value, shift_join)) = shifted_only_arm(shifted_only) else {
            continue;
        };
        if xor_value != shift_value
            || xor_join != shift_join
            || !layout.mem_allows_pure_read_reordering(&xor_value)
            || !high_bit_test_uses_value(routine, &cfg, test, producer.op_index, &xor_value)
        {
            continue;
        }
        return Some(HighBitShiftXorPlan {
            test: test.id,
            shifted_xor: shifted_xor.id,
            shifted_only: shifted_only.id,
            value: xor_value,
            xor_rhs,
        });
    }
    None
}

fn high_bit_test_uses_value(
    routine: &MirRoutine,
    cfg: &MirCfg,
    test: &MirBlock,
    compare_index: usize,
    value: &MirMem,
) -> bool {
    if compare_index + 1 != test.ops.len() {
        return false;
    }
    let suffix_matches = |ops: &[MirOp]| {
        matches!(
            ops,
            [
                MirOp::Binary {
                    op: MirBinaryOp::And,
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    right: MirValue::ConstU8(0x80) | MirValue::ConstU16(0x80),
                    width: MirWidth::Byte,
                    carry_in: None,
                    carry_out: MirCarryOut::Ignore,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Ne,
                    left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    right: MirValue::ConstU8(0) | MirValue::ConstU16(0),
                    width: MirWidth::Byte,
                    signed: false,
                },
            ]
        )
    };
    match test.ops.as_slice() {
        [
            MirOp::Load {
                dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                src: MirAddr::Direct(load_mem),
                width: MirWidth::Byte,
            },
            tail @ ..,
        ] => load_mem == value && suffix_matches(tail),
        ops if suffix_matches(ops) => {
            accumulator_is_stored_value_on_entry(routine, cfg, test.id, value)
        }
        _ => false,
    }
}

fn accumulator_is_stored_value_on_entry(
    routine: &MirRoutine,
    cfg: &MirCfg,
    block: MirBlockId,
    value: &MirMem,
) -> bool {
    let predecessors = cfg.predecessors(block);
    !predecessors.is_empty()
        && predecessors.iter().all(|predecessor| {
            block_by_id(routine, *predecessor).is_some_and(|block| {
                matches!(
                    block.ops.last(),
                    Some(MirOp::Store {
                        dst: MirAddr::Direct(store_mem),
                        src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                        width: MirWidth::Byte,
                    }) if store_mem == value
                )
            })
        })
}

fn shifted_xor_arm(block: &MirBlock) -> Option<(MirMem, MirValue, MirBlockId)> {
    let [
        MirOp::Load {
            dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
            src: MirAddr::Direct(load_mem),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: MirBinaryOp::Lsh,
            dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
            left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
            right: MirValue::ConstU8(1) | MirValue::ConstU16(1),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Binary {
            op: MirBinaryOp::Xor,
            dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
            left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
            right: xor_rhs,
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Store {
            dst: MirAddr::Direct(store_mem),
            src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
            width: MirWidth::Byte,
        },
    ] = block.ops.as_slice()
    else {
        return None;
    };
    let MirTerminator::Jump(edge) = &block.terminator else {
        return None;
    };
    (block.params.is_empty()
        && edge.args.is_empty()
        && load_mem == store_mem
        && matches!(xor_rhs, MirValue::ConstU8(_) | MirValue::ConstU16(_)))
    .then(|| (load_mem.clone(), xor_rhs.clone(), edge.target))
}

fn shifted_only_arm(block: &MirBlock) -> Option<(MirMem, MirBlockId)> {
    let [
        MirOp::Load {
            dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
            src: MirAddr::Direct(load_mem),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: MirBinaryOp::Lsh,
            dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
            left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
            right: MirValue::ConstU8(1) | MirValue::ConstU16(1),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Store {
            dst: MirAddr::Direct(store_mem),
            src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
            width: MirWidth::Byte,
        },
    ] = block.ops.as_slice()
    else {
        return None;
    };
    let MirTerminator::Jump(edge) = &block.terminator else {
        return None;
    };
    (block.params.is_empty() && edge.args.is_empty() && load_mem == store_mem)
        .then(|| (load_mem.clone(), edge.target))
}

fn apply_high_bit_shift_xor_plan(routine: &mut MirRoutine, plan: &HighBitShiftXorPlan) -> bool {
    let Some(test) = routine
        .blocks
        .iter_mut()
        .find(|block| block.id == plan.test)
    else {
        return false;
    };
    let MirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &test.terminator
    else {
        return false;
    };
    let then_edge = then_edge.clone();
    let else_edge = else_edge.clone();
    test.ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
            src: MirAddr::Direct(plan.value.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: MirBinaryOp::Lsh,
            dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
            left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Produce,
        },
    ];
    test.terminator = MirTerminator::Branch {
        cond: MirCond::FlagTest(MirFlagTest::CSet),
        then_edge,
        else_edge,
    };

    let Some(shifted_xor) = routine
        .blocks
        .iter_mut()
        .find(|block| block.id == plan.shifted_xor)
    else {
        return false;
    };
    shifted_xor.ops = vec![
        MirOp::Binary {
            op: MirBinaryOp::Xor,
            dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
            left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
            right: plan.xor_rhs.clone(),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Store {
            dst: MirAddr::Direct(plan.value.clone()),
            src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
            width: MirWidth::Byte,
        },
    ];

    let Some(shifted_only) = routine
        .blocks
        .iter_mut()
        .find(|block| block.id == plan.shifted_only)
    else {
        return false;
    };
    shifted_only.ops = vec![MirOp::Store {
        dst: MirAddr::Direct(plan.value.clone()),
        src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
        width: MirWidth::Byte,
    }];
    true
}

pub(super) fn unroll_small_counted_loops(
    routine: &mut MirRoutine,
    trip_limit: u8,
) -> SmallLoopUnrollStats {
    let mut stats = SmallLoopUnrollStats::default();
    let mut observed = BTreeSet::new();
    loop {
        let Some(candidate) = discover_candidate(routine, trip_limit) else {
            break;
        };
        if observed.insert(candidate.header) {
            stats.candidates += 1;
        }
        match prove_candidate(routine, candidate) {
            Ok(plan) => {
                if !apply_plan(routine, &plan) {
                    stats.blocked_effects += 1;
                    break;
                }
                stats.selected += 1;
            }
            Err(SmallLoopBlock::Growth) => {
                stats.blocked_growth += 1;
                break;
            }
            Err(SmallLoopBlock::Effects) => {
                stats.blocked_effects += 1;
                break;
            }
            Err(SmallLoopBlock::ObservableInduction) => {
                stats.blocked_observable_induction += 1;
                break;
            }
        }
    }
    stats
}

fn discover_candidate(routine: &MirRoutine, trip_limit: u8) -> Option<SmallLoopUnrollPlan> {
    let cfg = MirCfg::from_routine(routine).ok()?;
    for header in &routine.blocks {
        let MirTerminator::Branch {
            cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(condition))),
            then_edge,
            else_edge,
        } = &header.terminator
        else {
            continue;
        };
        if !then_edge.args.is_empty() || !else_edge.args.is_empty() {
            continue;
        }
        let Some((induction, trip_count)) = header_trip_count(header, *condition) else {
            continue;
        };
        if !(2..=trip_limit.min(8)).contains(&trip_count) {
            continue;
        }
        let predecessors = cfg.predecessors(header.id);
        if predecessors.len() != 2 {
            continue;
        }
        let mut preheader = None;
        let mut latch = None;
        let mut initial_op = None;
        for predecessor in predecessors {
            let block = block_by_id(routine, *predecessor)?;
            if let Some(op_index) = zero_initial(block, header.id, &induction) {
                preheader = Some(block.id);
                initial_op = Some(op_index);
            } else if unit_increment_latch(block, header.id, &induction) {
                latch = Some(block.id);
            }
        }
        let (Some(preheader), Some(latch), Some(initial_op)) = (preheader, latch, initial_op)
        else {
            continue;
        };
        let loop_nodes = natural_loop(&cfg, header.id, latch)?;
        let body_nodes = loop_nodes
            .iter()
            .copied()
            .filter(|block| *block != header.id && *block != latch)
            .collect::<BTreeSet<_>>();
        if body_nodes.is_empty()
            || !body_nodes.contains(&then_edge.target)
            || loop_nodes.contains(&preheader)
            || loop_nodes.contains(&else_edge.target)
            || cfg.predecessors(else_edge.target) != &BTreeSet::from([header.id])
        {
            continue;
        }
        if !body_region_is_closed(&cfg, &body_nodes, header.id, then_edge.target, latch) {
            continue;
        }
        return Some(SmallLoopUnrollPlan {
            preheader,
            header: header.id,
            body: then_edge.target,
            latch,
            exit: else_edge.target,
            induction,
            initial_op,
            body_nodes,
            trip_count,
        });
    }
    None
}

fn prove_candidate(
    routine: &MirRoutine,
    plan: SmallLoopUnrollPlan,
) -> Result<SmallLoopUnrollPlan, SmallLoopBlock> {
    let cfg = MirCfg::from_routine(routine).map_err(|_| SmallLoopBlock::Effects)?;
    if mem_may_be_read_from(routine, &cfg, plan.exit, &plan.induction) {
        return Err(SmallLoopBlock::ObservableInduction);
    }
    let mut growth_units = 0usize;
    for block_id in &plan.body_nodes {
        let block = block_by_id(routine, *block_id).ok_or(SmallLoopBlock::Effects)?;
        growth_units = growth_units.saturating_add(block.ops.len() + 1);
        if !block.params.is_empty()
            || terminator_edges(&block.terminator).any(|edge| !edge.args.is_empty())
            || block.ops.iter().any(|op| {
                let effects = classify_op(op);
                op_may_write_mem(op, &plan.induction)
                    || effects.memory.reads(&plan.induction)
                    || unsupported_unrolled_effect(op)
            })
        {
            return Err(SmallLoopBlock::Effects);
        }
    }
    if growth_units.saturating_mul(usize::from(plan.trip_count.saturating_sub(1)))
        > MAX_UNROLLED_BODY_GROWTH_UNITS
    {
        return Err(SmallLoopBlock::Growth);
    }
    Ok(plan)
}

fn apply_plan(routine: &mut MirRoutine, plan: &SmallLoopUnrollPlan) -> bool {
    let mut next_block = routine
        .blocks
        .iter()
        .map(|block| block.id.0)
        .max()
        .and_then(|id| id.checked_add(1))
        .unwrap_or(0);
    let body_order = routine
        .blocks
        .iter()
        .filter(|block| plan.body_nodes.contains(&block.id))
        .map(|block| block.id)
        .collect::<Vec<_>>();
    if body_order.is_empty() {
        return false;
    }

    let mut maps = Vec::with_capacity(usize::from(plan.trip_count));
    maps.push(
        body_order
            .iter()
            .copied()
            .map(|block| (block, block))
            .collect::<BTreeMap<_, _>>(),
    );
    for _ in 1..plan.trip_count {
        let mut map = BTreeMap::new();
        for block in &body_order {
            let id = MirBlockId(next_block);
            let Some(next) = next_block.checked_add(1) else {
                return false;
            };
            next_block = next;
            map.insert(*block, id);
        }
        maps.push(map);
    }

    let originals = plan
        .body_nodes
        .iter()
        .filter_map(|id| block_by_id(routine, *id).cloned().map(|block| (*id, block)))
        .collect::<BTreeMap<_, _>>();
    if originals.len() != plan.body_nodes.len() {
        return false;
    }
    let defined_temps = originals
        .values()
        .flat_map(|block| &block.ops)
        .flat_map(|op| classify_op(op).logical.temp_defs)
        .map(|access| access.temp())
        .collect::<BTreeSet<_>>();
    let mut next_temp = routine
        .temps
        .iter()
        .map(|temp| temp.id.0)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut temp_maps = vec![BTreeMap::new()];
    for _ in 1..plan.trip_count {
        let mut map = BTreeMap::new();
        for old in &defined_temps {
            let new = MirTempId(next_temp);
            let Some(next) = next_temp.checked_add(1) else {
                return false;
            };
            next_temp = next;
            routine.temps.push(MirTemp { id: new });
            map.insert(*old, new);
        }
        temp_maps.push(map);
    }
    let mut unrolled = Vec::new();
    for iteration in 0..usize::from(plan.trip_count) {
        for original_id in &body_order {
            let mut block = originals
                .get(original_id)
                .cloned()
                .expect("collected body block");
            block.id = maps[iteration][original_id];
            if iteration != 0 {
                block.label = format!("{}.unroll{iteration}", block.label);
                remap_block_temps(&mut block, &temp_maps[iteration]);
            }
            if !remap_body_terminator(
                &mut block.terminator,
                &maps[iteration],
                maps.get(iteration + 1),
                plan.body,
                plan.latch,
                plan.exit,
            ) {
                return false;
            }
            unrolled.push(block);
        }
    }

    let Some(preheader) = routine
        .blocks
        .iter_mut()
        .find(|block| block.id == plan.preheader)
    else {
        return false;
    };
    if preheader.ops.get(plan.initial_op).is_none() {
        return false;
    }
    preheader.ops.remove(plan.initial_op);
    preheader.terminator = MirTerminator::Jump(MirEdge::plain(plan.body));

    routine.blocks.retain(|block| {
        block.id != plan.header && block.id != plan.latch && !plan.body_nodes.contains(&block.id)
    });
    routine.blocks.extend(unrolled);
    true
}

pub(super) fn remap_block_temps(block: &mut MirBlock, replacements: &BTreeMap<MirTempId, MirTempId>) {
    // Both cloning clients allocate above every old ID. Sequential visits are
    // therefore a simultaneous substitution: no new ID can be renamed again.
    debug_assert!(replacements.values().all(|id| !replacements.contains_key(id)));
    for (old, new) in replacements {
        for param in &mut block.params {
            if param.dest == *old {
                param.dest = *new;
            }
        }
        for op in &mut block.ops {
            remap_op_temp(op, *old, *new);
        }
        if let MirTerminator::Branch {
            cond: MirCond::BoolValue(value),
            ..
        } = &mut block.terminator
        {
            remap_value_temp(value, *old, *new);
        }
        for edge in terminator_edges_mut(&mut block.terminator) {
            for arg in &mut edge.args {
                remap_value_temp(&mut arg.value, *old, *new);
            }
        }
    }
}

fn remap_op_temp(op: &mut MirOp, old: MirTempId, new: MirTempId) {
    match op {
        MirOp::Load { dst, src, .. } => {
            remap_def_temp(dst, old, new);
            remap_addr_temp(src, old, new);
        }
        MirOp::Store { dst, src, .. } => {
            remap_addr_temp(dst, old, new);
            remap_value_temp(src, old, new);
        }
        MirOp::CopyBytes {
            source,
            destination,
            ..
        }
        | MirOp::PackedRealCopy {
            source,
            destination,
            ..
        } => {
            remap_addr_temp(source, old, new);
            remap_addr_temp(destination, old, new);
        }
        MirOp::Move { dst, src, .. }
        | MirOp::Extend { dst, src, .. }
        | MirOp::Truncate { dst, src, .. }
        | MirOp::Unary { dst, src, .. } => {
            remap_def_temp(dst, old, new);
            remap_value_temp(src, old, new);
        }
        MirOp::Binary {
            dst, left, right, ..
        } => {
            remap_def_temp(dst, old, new);
            remap_value_temp(left, old, new);
            remap_value_temp(right, old, new);
        }
        MirOp::AddByteToWordMem { value, .. }
        | MirOp::SubByteFromWordMem { value, .. }
        | MirOp::MaterializeAddress { value, .. }
        | MirOp::AdvanceAddress { index: value, .. }
        | MirOp::StoreIndirect { src: value, .. } => remap_value_temp(value, old, new),
        MirOp::Compare {
            dst, left, right, ..
        } => {
            remap_cond_dest_temp(dst, old, new);
            remap_value_temp(left, old, new);
            remap_value_temp(right, old, new);
        }
        MirOp::CompareIndirectBytes { dst, .. }
        | MirOp::CompareDirectIndexedBytes { dst, .. }
        | MirOp::CompareIndirectWords { dst, .. } => remap_cond_dest_temp(dst, old, new),
        MirOp::Call {
            target,
            args,
            result,
            ..
        } => {
            if let crate::mir6502::ir::MirCallTarget::Indirect { target, .. } = target {
                remap_value_temp(target, old, new);
            }
            for arg in args {
                remap_value_temp(&mut arg.value, old, new);
            }
            if let Some(result) = result {
                remap_def_temp(&mut result.dst, old, new);
            }
        }
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            remap_value_temp(base, old, new);
            remap_value_temp(index, old, new);
        }
        MirOp::LoadIndirect { dst, .. } => remap_def_temp(dst, old, new),
        MirOp::LoadImm { dst, .. } | MirOp::LeaAddr { dst, .. } => remap_def_temp(dst, old, new),
        MirOp::UpdateMem { .. }
        | MirOp::UpdateReg { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::BinaryDirectIndexedByte { .. }
        | MirOp::OffsetPointerByIndirectByte { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyDirectWordToIndirect { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::AbsoluteWordSubToIndirect { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn remap_def_temp(def: &mut MirDef, old: MirTempId, new: MirTempId) {
    match def {
        MirDef::VTemp(id) | MirDef::VTempByte { id, .. } if *id == old => *id = new,
        MirDef::VTemp(_) | MirDef::VTempByte { .. } | MirDef::Reg(_) => {}
    }
}

fn remap_cond_dest_temp(dst: &mut MirCondDest, old: MirTempId, new: MirTempId) {
    if matches!(dst, MirCondDest::Temp(id) if *id == old) {
        *dst = MirCondDest::Temp(new);
    }
}

fn remap_value_temp(value: &mut MirValue, old: MirTempId, new: MirTempId) {
    match value {
        MirValue::Def(MirDef::VTemp(id) | MirDef::VTempByte { id, .. }) if *id == old => {
            *id = new;
        }
        MirValue::Word { lo, hi } => {
            remap_value_temp(lo, old, new);
            remap_value_temp(hi, old, new);
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. }
        | MirValue::PointerCell(_) => {}
    }
}

fn remap_addr_temp(addr: &mut MirAddr, old: MirTempId, new: MirTempId) {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            remap_value_temp(base, old, new);
            remap_value_temp(index, old, new);
        }
        MirAddr::PointerIndex { index, .. } => remap_value_temp(index, old, new),
        MirAddr::Deref { ptr, .. } => remap_value_temp(ptr, old, new),
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

fn header_trip_count(header: &MirBlock, condition: MirTempId) -> Option<(MirMem, u8)> {
    let [
        MirOp::Load {
            dst: MirDef::VTemp(loaded),
            src: MirAddr::Direct(induction),
            width: MirWidth::Byte,
        },
        MirOp::Compare {
            dst: MirCondDest::Temp(dst),
            op,
            left: MirValue::Def(MirDef::VTemp(value)),
            right,
            width: MirWidth::Byte,
            signed: false,
        },
    ] = header.ops.as_slice()
    else {
        return None;
    };
    if dst != &condition || loaded != value {
        return None;
    }
    let bound = match right {
        MirValue::ConstU8(value) => *value,
        MirValue::ConstU16(value) => u8::try_from(*value).ok()?,
        _ => return None,
    };
    let trip_count = match op {
        MirCompareOp::Lt => bound,
        MirCompareOp::Le => bound.checked_add(1)?,
        _ => return None,
    };
    Some((induction.clone(), trip_count))
}

fn zero_initial(block: &MirBlock, header: MirBlockId, induction: &MirMem) -> Option<usize> {
    let MirTerminator::Jump(edge) = &block.terminator else {
        return None;
    };
    if edge.target != header || !edge.args.is_empty() {
        return None;
    }
    block
        .ops
        .iter()
        .enumerate()
        .rev()
        .find_map(|(op_index, op)| match op {
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                src: MirValue::ConstU8(0) | MirValue::ConstU16(0),
                width: MirWidth::Byte,
            } if mem == induction => Some(op_index),
            _ => None,
        })
}

fn unit_increment_latch(block: &MirBlock, header: MirBlockId, induction: &MirMem) -> bool {
    let MirTerminator::Jump(edge) = &block.terminator else {
        return false;
    };
    if edge.target != header || !edge.args.is_empty() || block.ops.len() != 3 {
        return false;
    }
    let ops = &block.ops[block.ops.len() - 3..];
    matches!(
        ops,
        [
            MirOp::Load {
                dst: MirDef::VTemp(loaded),
                src: MirAddr::Direct(load_mem),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::VTemp(next),
                left: MirValue::Def(MirDef::VTemp(left)),
                right: MirValue::ConstU8(1) | MirValue::ConstU16(1),
                width: MirWidth::Byte,
                carry_in: None | Some(MirCarryIn::Clear),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_mem),
                src: MirValue::Def(MirDef::VTemp(stored)),
                width: MirWidth::Byte,
            },
        ] if load_mem == induction && store_mem == induction && loaded == left && next == stored
    )
}

fn natural_loop(
    cfg: &MirCfg,
    header: MirBlockId,
    latch: MirBlockId,
) -> Option<BTreeSet<MirBlockId>> {
    let mut nodes = BTreeSet::from([header, latch]);
    let mut pending = vec![latch];
    while let Some(block) = pending.pop() {
        for predecessor in cfg.predecessors(block) {
            if *predecessor != header && nodes.insert(*predecessor) {
                pending.push(*predecessor);
            }
        }
    }
    cfg.successors(latch).contains(&header).then_some(nodes)
}

fn body_region_is_closed(
    cfg: &MirCfg,
    nodes: &BTreeSet<MirBlockId>,
    header: MirBlockId,
    body: MirBlockId,
    latch: MirBlockId,
) -> bool {
    nodes.iter().all(|block| {
        cfg.predecessors(*block).iter().all(|predecessor| {
            nodes.contains(predecessor) || (*block == body && *predecessor == header)
        }) && cfg
            .successors(*block)
            .iter()
            .all(|successor| nodes.contains(successor) || *successor == latch)
    })
}

fn remap_body_terminator(
    terminator: &mut MirTerminator,
    current: &BTreeMap<MirBlockId, MirBlockId>,
    next: Option<&BTreeMap<MirBlockId, MirBlockId>>,
    body: MirBlockId,
    latch: MirBlockId,
    exit: MirBlockId,
) -> bool {
    for edge in terminator_edges_mut(terminator) {
        if let Some(target) = current.get(&edge.target) {
            edge.target = *target;
        } else if edge.target == latch {
            edge.target = next.map_or(exit, |map| map[&body]);
        } else {
            return false;
        }
    }
    true
}

fn unsupported_unrolled_effect(op: &MirOp) -> bool {
    if matches!(
        op,
        MirOp::Call { .. }
            | MirOp::RuntimeHelper { .. }
            | MirOp::Barrier { .. }
            | MirOp::MachineBlock { .. }
            | MirOp::CopyBytes { .. }
            | MirOp::PackedRealCopy { .. }
    ) {
        return true;
    }
    let effects = classify_op(op);
    effects.memory.opaque || effects.memory.indirect_writes || effects.memory.may_write_any
}

fn mem_may_be_read_from(
    routine: &MirRoutine,
    cfg: &MirCfg,
    start: MirBlockId,
    mem: &MirMem,
) -> bool {
    let mut pending = vec![start];
    let mut reachable = BTreeSet::new();
    while let Some(block_id) = pending.pop() {
        if !reachable.insert(block_id) {
            continue;
        }
        let Some(block) = block_by_id(routine, block_id) else {
            return true;
        };
        let mut overwritten = false;
        for op in &block.ops {
            let effects = classify_op(op);
            if effects.memory.reads(mem) {
                return true;
            }
            if effects.memory.definitely_writes(mem) {
                overwritten = true;
                break;
            }
        }
        if !overwritten {
            let term = classify_terminator(&block.terminator);
            if term.memory.reads(mem) {
                return true;
            }
            pending.extend(cfg.successors(block_id).iter().copied());
        }
    }
    false
}

fn terminator_edges(terminator: &MirTerminator) -> impl Iterator<Item = &MirEdge> {
    let mut edges = Vec::new();
    match terminator {
        MirTerminator::Jump(edge) => edges.push(edge),
        MirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => {
            edges.push(then_edge);
            edges.push(else_edge);
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
    }
    edges.into_iter()
}

fn terminator_edges_mut(terminator: &mut MirTerminator) -> Vec<&mut MirEdge> {
    match terminator {
        MirTerminator::Jump(edge) => vec![edge],
        MirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => Vec::new(),
    }
}

fn block_by_id(routine: &MirRoutine, id: MirBlockId) -> Option<&MirBlock> {
    routine.blocks.iter().find(|block| block.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir;
    use crate::semantic::{SemanticOptions, analyze_with_options, ir};

    fn lowered_crc() -> MirRoutine {
        let source = "
            BYTE result
            BYTE FUNC ShiftEight(BYTE value)
              BYTE bit
              FOR bit=0 TO 7 DO
                IF (value AND $80)#0 THEN
                  value=(value LSH 1) XOR $1D
                ELSE
                  value=value LSH 1
                FI
              OD
            RETURN(value)
            PROC Main()
              result=ShiftEight(1)
            RETURN
        ";
        lowered_routine(source, "ShiftEight")
    }

    fn lowered_routine(source: &str, name: &str) -> MirRoutine {
        let tokens = crate::lexer::tokenize(source).unwrap();
        let program = crate::parser::parse(&tokens).unwrap();
        let model = analyze_with_options(&program, SemanticOptions::modern()).unwrap();
        let semir = ir::lower_program(&program, &model);
        let nir = nir::optimize_program(&nir::lower_program(&semir)).unwrap();
        let mir = crate::mir6502::lower_program(&nir).unwrap();
        let mut routine = mir
            .routines
            .into_iter()
            .find(|routine| routine.name == name)
            .unwrap();
        super::super::block_args::lower_block_arguments(&mut routine).unwrap();
        routine
    }

    #[test]
    fn unrolls_the_generic_exact_eight_iteration_inner_loop() {
        let mut routine = lowered_crc();
        let before = routine.blocks.len();
        let stats = unroll_small_counted_loops(&mut routine, 8);

        assert_eq!(stats.selected, 1);
        assert!(routine.blocks.len() > before);
        assert!(
            !routine
                .blocks
                .iter()
                .flat_map(|block| &block.ops)
                .any(|op| {
                    matches!(
                        op,
                        MirOp::Compare {
                            op: MirCompareOp::Le,
                            right: MirValue::ConstU8(7),
                            ..
                        }
                    )
                })
        );
        let mut definitions = BTreeSet::new();
        for access in routine
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .flat_map(|op| classify_op(op).logical.temp_defs)
        {
            assert!(
                definitions.insert(access),
                "unrolled iterations must have distinct temporary definitions: {access:?}"
            );
        }
    }

    #[test]
    fn rejects_a_trip_count_above_the_configured_limit() {
        let mut routine = lowered_crc();
        let stats = unroll_small_counted_loops(&mut routine, 7);

        assert_eq!(stats.selected, 0);
    }

    #[test]
    fn rejects_a_loop_with_an_opaque_barrier_in_its_body() {
        let mut routine = lowered_crc();
        let candidate = discover_candidate(&routine, 8).expect("canonical small loop");
        let body = *candidate.body_nodes.first().expect("loop body");
        block_by_id_mut(&mut routine, body).ops.insert(
            0,
            MirOp::Barrier {
                effects: Default::default(),
            },
        );
        let stats = unroll_small_counted_loops(&mut routine, 8);

        assert_eq!(stats.selected, 0);
        assert_eq!(stats.blocked_effects, 1);
    }

    #[test]
    fn rejects_a_loop_whose_final_induction_value_is_observable() {
        let mut routine = lowered_crc();
        let candidate = discover_candidate(&routine, 8).expect("canonical small loop");
        block_by_id_mut(&mut routine, candidate.exit).ops.insert(
            0,
            MirOp::Load {
                dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                src: MirAddr::Direct(candidate.induction),
                width: MirWidth::Byte,
            },
        );
        let stats = unroll_small_counted_loops(&mut routine, 8);

        assert_eq!(stats.selected, 0);
        assert_eq!(stats.blocked_observable_induction, 1);
    }

    fn block_by_id_mut(routine: &mut MirRoutine, id: MirBlockId) -> &mut MirBlock {
        routine
            .blocks
            .iter_mut()
            .find(|block| block.id == id)
            .expect("MIR block exists")
    }
}
