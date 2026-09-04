use std::collections::BTreeSet;

use crate::mir6502::analysis::counted_loops::{
    MirDynamicWordCountedLoop, analyze_dynamic_word_counted_loops,
};
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirCompareOp, MirCond, MirCondDest, MirDef, MirEdge, MirMem,
    MirOp, MirPointerPair, MirRoutine, MirTemp, MirTempId, MirTerminator, MirUpdateOp, MirValue,
    MirWidth, MirZpSlot,
};

use super::memory::op_may_write_mem;
use super::temp_uses::{op_uses_temp, terminator_uses_temp};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DynamicWordIndexLoopStats {
    pub candidates: usize,
    pub selected: usize,
    pub blocked_final_index: usize,
    pub blocked_bound_invariance: usize,
    pub blocked_index_use: usize,
    pub blocked_alias: usize,
    pub blocked_shape: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DynamicWordIndexLoopPlan {
    counted: MirDynamicWordCountedLoop,
    base: MirValue,
    base_load_site: Option<MirSite>,
    indexed_load_sites: BTreeSet<MirSite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DynamicWordIndexLoopBlock {
    FinalIndex,
    BoundInvariance,
    IndexUse,
    Alias,
    Shape,
}

/// Select the generic runtime-pointer loop as one transactional pre-home CFG
/// rewrite. Rotation, index strength reduction, and cursor/countdown creation
/// are applied together so a partially selected loop cannot leave a more
/// expensive intermediate form for later materialization.
pub(super) fn select_dynamic_word_index_loops(
    routine: &mut MirRoutine,
) -> DynamicWordIndexLoopStats {
    let mut stats = DynamicWordIndexLoopStats::default();
    let mut observed_headers = BTreeSet::new();
    loop {
        let candidates = analyze_dynamic_word_counted_loops(routine);
        let mut selected = false;
        for counted in candidates {
            if observed_headers.insert(counted.header) {
                stats.candidates += 1;
            }
            match build_plan(routine, counted) {
                Ok(plan) => {
                    if apply_plan(routine, plan) {
                        stats.selected += 1;
                        selected = true;
                        break;
                    }
                    stats.blocked_shape += 1;
                }
                Err(DynamicWordIndexLoopBlock::FinalIndex) => stats.blocked_final_index += 1,
                Err(DynamicWordIndexLoopBlock::BoundInvariance) => {
                    stats.blocked_bound_invariance += 1
                }
                Err(DynamicWordIndexLoopBlock::IndexUse) => stats.blocked_index_use += 1,
                Err(DynamicWordIndexLoopBlock::Alias) => stats.blocked_alias += 1,
                Err(DynamicWordIndexLoopBlock::Shape) => stats.blocked_shape += 1,
            }
        }
        if !selected {
            break;
        }
    }
    stats
}

fn build_plan(
    routine: &MirRoutine,
    counted: MirDynamicWordCountedLoop,
) -> Result<DynamicWordIndexLoopPlan, DynamicWordIndexLoopBlock> {
    if counted.final_value_observable {
        return Err(DynamicWordIndexLoopBlock::FinalIndex);
    }
    let bound_load =
        op_at_site(routine, counted.bound_load_site).ok_or(DynamicWordIndexLoopBlock::Shape)?;
    if !matches!(
        bound_load,
        MirOp::Load {
            dst: MirDef::VTemp(temp),
            src: MirAddr::Direct(mem),
            width: MirWidth::Word,
        } if *temp == counted.bound && mem == &counted.bound_mem
    ) {
        return Err(DynamicWordIndexLoopBlock::Shape);
    }
    if !mem_allows_invariant_hoist(&counted.bound_mem) {
        return Err(DynamicWordIndexLoopBlock::BoundInvariance);
    }
    if loop_invalidates_mem(routine, &counted, &counted.bound_mem) {
        return Err(DynamicWordIndexLoopBlock::BoundInvariance);
    }

    let mut indexed_load_sites = BTreeSet::new();
    let mut base = None;
    for block in &routine.blocks {
        for (op_index, op) in block.ops.iter().enumerate() {
            if !op_uses_temp(op, counted.induction) {
                continue;
            }
            let site = MirSite::Op {
                block: block.id,
                op_index,
            };
            if site == counted.compare_site || site == counted.update_site {
                continue;
            }
            let MirOp::Load {
                src:
                    MirAddr::ComputedIndex {
                        base: candidate_base,
                        index: MirValue::Def(MirDef::VTemp(index)),
                        elem_size: 1,
                        offset: 0,
                    },
                width: MirWidth::Byte,
                ..
            } = op
            else {
                return Err(DynamicWordIndexLoopBlock::IndexUse);
            };
            if *index != counted.induction || !counted.loop_nodes.contains(&block.id) {
                return Err(DynamicWordIndexLoopBlock::IndexUse);
            }
            if base.as_ref().is_some_and(|base| base != candidate_base) {
                return Err(DynamicWordIndexLoopBlock::IndexUse);
            }
            base = Some(candidate_base.clone());
            indexed_load_sites.insert(site);
        }
        if terminator_uses_temp(&block.terminator, counted.induction) {
            return Err(DynamicWordIndexLoopBlock::IndexUse);
        }
    }
    let base = base.ok_or(DynamicWordIndexLoopBlock::Shape)?;

    let (base_load_site, base_mem) = match &base {
        MirValue::Def(MirDef::VTemp(temp)) => {
            let (site, mem) =
                unique_direct_word_load(routine, *temp).ok_or(DynamicWordIndexLoopBlock::Shape)?;
            if !counted.loop_nodes.contains(&site.block())
                || !temp_is_used_only_at_sites(routine, *temp, &indexed_load_sites)
            {
                return Err(DynamicWordIndexLoopBlock::Shape);
            }
            (Some(site), Some(mem))
        }
        value if !value_contains_temp(value) => (None, pointer_value_mem(value)),
        _ => return Err(DynamicWordIndexLoopBlock::Shape),
    };
    if let Some(base_mem) = &base_mem {
        if !mem_allows_invariant_hoist(base_mem)
            || loop_invalidates_mem(routine, &counted, base_mem)
        {
            return Err(DynamicWordIndexLoopBlock::Alias);
        }
    }
    if loop_has_unsupported_effects(routine, &counted) {
        return Err(DynamicWordIndexLoopBlock::Alias);
    }

    Ok(DynamicWordIndexLoopPlan {
        counted,
        base,
        base_load_site,
        indexed_load_sites,
    })
}

fn mem_allows_invariant_hoist(mem: &MirMem) -> bool {
    matches!(
        mem,
        MirMem::Static { .. }
            | MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
    )
}

fn apply_plan(routine: &mut MirRoutine, plan: DynamicWordIndexLoopPlan) -> bool {
    let Some(bound_load) = op_at_site(routine, plan.counted.bound_load_site).cloned() else {
        return false;
    };
    if block_by_id(routine, plan.counted.preheader).is_none()
        || block_by_id(routine, plan.counted.latch).is_none()
    {
        return false;
    }
    let Some((cursor, cursor_high, remaining, remaining_high)) = allocate_virtual_zp_pairs(routine)
    else {
        return false;
    };
    let Some(latch_condition) = allocate_temp(routine) else {
        return false;
    };
    debug_assert_eq!(cursor_high.0, cursor.0 + 1);
    debug_assert_eq!(remaining_high.0, remaining.0 + 1);

    let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(cursor));
    let remaining_mem = MirMem::ZeroPage(remaining);
    let cursor_mem = MirMem::ZeroPage(cursor);
    let base_load = plan
        .base_load_site
        .and_then(|site| op_at_site(routine, site).cloned());

    for block in &mut routine.blocks {
        let block_id = block.id;
        let mut ops = Vec::with_capacity(block.ops.len() + 4);
        for (op_index, op) in std::mem::take(&mut block.ops).into_iter().enumerate() {
            let site = MirSite::Op {
                block: block_id,
                op_index,
            };
            if site == plan.counted.initial_site
                || site == plan.counted.bound_load_site
                || site == plan.counted.update_site
                || plan.base_load_site == Some(site)
            {
                continue;
            }
            if site == plan.counted.compare_site {
                let MirOp::Compare { dst, .. } = op else {
                    return false;
                };
                ops.push(MirOp::Compare {
                    dst,
                    op: MirCompareOp::Ne,
                    left: MirValue::Word {
                        lo: Box::new(MirValue::PointerCell(remaining_mem.clone())),
                        hi: Box::new(MirValue::PointerCell(MirMem::ZeroPage(remaining_high))),
                    },
                    right: MirValue::ConstU16(0),
                    width: MirWidth::Word,
                    signed: false,
                });
                continue;
            }
            if plan.indexed_load_sites.contains(&site) {
                let MirOp::Load { dst, .. } = op else {
                    return false;
                };
                ops.push(MirOp::LoadIndirect {
                    consumer,
                    dst,
                    offset: 0,
                });
                continue;
            }
            ops.push(op);
        }
        block.ops = ops;
    }

    let Some(preheader) = block_by_id_mut(routine, plan.counted.preheader) else {
        return false;
    };
    preheader.ops.push(bound_load);
    preheader.ops.push(MirOp::Store {
        dst: MirAddr::Direct(remaining_mem.clone()),
        src: MirValue::Def(MirDef::VTempByte {
            id: plan.counted.bound,
            byte: 0,
        }),
        width: MirWidth::Byte,
    });
    preheader.ops.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::ZeroPage(remaining_high)),
        src: MirValue::Def(MirDef::VTempByte {
            id: plan.counted.bound,
            byte: 1,
        }),
        width: MirWidth::Byte,
    });
    if let Some(base_load) = base_load {
        preheader.ops.push(base_load);
    }
    preheader.ops.push(MirOp::MaterializeAddress {
        consumer,
        value: plan.base,
    });

    let Some(latch) = block_by_id_mut(routine, plan.counted.latch) else {
        return false;
    };
    latch.ops.push(MirOp::UpdateMem {
        op: MirUpdateOp::Inc,
        mem: cursor_mem,
        width: MirWidth::Word,
    });
    latch.ops.push(MirOp::UpdateMem {
        op: MirUpdateOp::Dec,
        mem: remaining_mem.clone(),
        width: MirWidth::Word,
    });
    latch.ops.push(MirOp::Compare {
        dst: MirCondDest::Temp(latch_condition),
        op: MirCompareOp::Ne,
        left: MirValue::Word {
            lo: Box::new(MirValue::PointerCell(remaining_mem)),
            hi: Box::new(MirValue::PointerCell(MirMem::ZeroPage(remaining_high))),
        },
        right: MirValue::ConstU16(0),
        width: MirWidth::Word,
        signed: false,
    });
    latch.terminator = MirTerminator::Branch {
        cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(latch_condition))),
        then_edge: MirEdge::plain(plan.counted.body),
        else_edge: MirEdge::plain(plan.counted.exit),
    };

    routine
        .temps
        .retain(|temp| temp.id != plan.counted.induction);
    true
}

fn loop_invalidates_mem(
    routine: &MirRoutine,
    counted: &MirDynamicWordCountedLoop,
    mem: &MirMem,
) -> bool {
    counted.loop_nodes.iter().any(|block_id| {
        block_by_id(routine, *block_id)
            .is_none_or(|block| block.ops.iter().any(|op| op_may_write_mem(op, mem)))
    })
}

fn loop_has_unsupported_effects(routine: &MirRoutine, counted: &MirDynamicWordCountedLoop) -> bool {
    counted.loop_nodes.iter().any(|block_id| {
        block_by_id(routine, *block_id).is_none_or(|block| {
            block.ops.iter().any(|op| {
                if matches!(
                    op,
                    MirOp::Call { .. }
                        | MirOp::RuntimeHelper { .. }
                        | MirOp::Barrier { .. }
                        | MirOp::MachineBlock { .. }
                        | MirOp::StoreIndirect { .. }
                        | MirOp::CopyBytes { .. }
                        | MirOp::PackedRealCopy { .. }
                        | MirOp::IndirectByteCompound { .. }
                        | MirOp::IndirectWordCompound { .. }
                ) {
                    return true;
                }
                let effects = classify_op(op);
                effects.memory.opaque
                    || effects.memory.indirect_writes
                    || effects.memory.may_write_any
            })
        })
    })
}

fn unique_direct_word_load(routine: &MirRoutine, temp: MirTempId) -> Option<(MirSite, MirMem)> {
    let mut definitions = routine.blocks.iter().flat_map(|block| {
        block
            .ops
            .iter()
            .enumerate()
            .filter_map(move |(op_index, op)| {
                if !classify_op(op)
                    .logical
                    .temp_defs
                    .iter()
                    .any(|access| access.temp() == temp)
                {
                    return None;
                }
                Some((
                    MirSite::Op {
                        block: block.id,
                        op_index,
                    },
                    match op {
                        MirOp::Load {
                            dst: MirDef::VTemp(dst),
                            src: MirAddr::Direct(mem),
                            width: MirWidth::Word,
                        } if *dst == temp => Some(mem.clone()),
                        _ => None,
                    },
                ))
            })
    });
    let (site, mem) = definitions.next()?;
    if definitions.next().is_some() {
        return None;
    }
    Some((site, mem?))
}

fn temp_is_used_only_at_sites(
    routine: &MirRoutine,
    temp: MirTempId,
    allowed: &BTreeSet<MirSite>,
) -> bool {
    routine.blocks.iter().all(|block| {
        block.ops.iter().enumerate().all(|(op_index, op)| {
            !op_uses_temp(op, temp)
                || allowed.contains(&MirSite::Op {
                    block: block.id,
                    op_index,
                })
        }) && !terminator_uses_temp(&block.terminator, temp)
    })
}

fn value_contains_temp(value: &MirValue) -> bool {
    match value {
        MirValue::Def(MirDef::VTemp(_) | MirDef::VTempByte { .. }) => true,
        MirValue::Word { lo, hi } => value_contains_temp(lo) || value_contains_temp(hi),
        _ => false,
    }
}

fn pointer_value_mem(value: &MirValue) -> Option<MirMem> {
    match value {
        MirValue::PointerCell(mem) => Some(mem.clone()),
        MirValue::Word { lo, hi } => match (lo.as_ref(), hi.as_ref()) {
            (MirValue::PointerCell(lo), MirValue::PointerCell(_)) => Some(lo.clone()),
            _ => None,
        },
        MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_) => None,
        _ => None,
    }
}

fn allocate_virtual_zp_pairs(
    routine: &mut MirRoutine,
) -> Option<(MirZpSlot, MirZpSlot, MirZpSlot, MirZpSlot)> {
    let next = routine
        .frame
        .virtual_zero_page
        .iter()
        .map(|slot| slot.0)
        .chain(
            routine
                .frame
                .zero_page_allocations
                .iter()
                .map(|allocation| allocation.slot.0),
        )
        .max()
        .map_or(Some(0), |slot| slot.checked_add(1))?;
    let cursor = MirZpSlot(next);
    let cursor_high = MirZpSlot(next.checked_add(1)?);
    let remaining = MirZpSlot(next.checked_add(2)?);
    let remaining_high = MirZpSlot(next.checked_add(3)?);
    // Virtual slots are allocated in declaration order. Retaining explicit
    // high-lane reservations keeps both word homes contiguous.
    routine
        .frame
        .virtual_zero_page
        .extend([cursor, cursor_high, remaining, remaining_high]);
    Some((cursor, cursor_high, remaining, remaining_high))
}

fn allocate_temp(routine: &mut MirRoutine) -> Option<MirTempId> {
    let next = routine
        .temps
        .iter()
        .map(|temp| temp.id.0)
        .max()
        .map_or(Some(0), |id| id.checked_add(1))?;
    let id = MirTempId(next);
    routine.temps.push(MirTemp { id });
    Some(id)
}

fn op_at_site(routine: &MirRoutine, site: MirSite) -> Option<&MirOp> {
    let MirSite::Op { block, op_index } = site else {
        return None;
    };
    block_by_id(routine, block)?.ops.get(op_index)
}

fn block_by_id(
    routine: &MirRoutine,
    id: crate::mir6502::ir::MirBlockId,
) -> Option<&crate::mir6502::ir::MirBlock> {
    routine.blocks.iter().find(|block| block.id == id)
}

fn block_by_id_mut(
    routine: &mut MirRoutine,
    id: crate::mir6502::ir::MirBlockId,
) -> Option<&mut crate::mir6502::ir::MirBlock> {
    routine.blocks.iter_mut().find(|block| block.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::includes::load_program_with_expanded_source;
    use crate::nir;
    use crate::semantic::{SemanticOptions, analyze_with_options, ir};
    use std::path::Path;

    fn lowered_fixture() -> MirRoutine {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/mir6502/dynamic_word_pointer_loop.act");
        let loaded = load_program_with_expanded_source(&path).unwrap();
        let model = analyze_with_options(&loaded.program, SemanticOptions::modern()).unwrap();
        let semir = ir::lower_program(&loaded.program, &model);
        let nir = nir::optimize_program(&nir::lower_program(&semir)).unwrap();
        let mut mir = crate::mir6502::lower_program(&nir).unwrap();
        let mut routine = mir.routines.remove(0);
        super::super::block_args::lower_block_arguments(&mut routine).unwrap();
        routine
    }

    #[test]
    fn recognizes_runtime_word_bound_after_block_argument_lowering() {
        let routine = lowered_fixture();
        let loops = analyze_dynamic_word_counted_loops(&routine);

        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].header.0, 1);
        assert_eq!(loops[0].body.0, 2);
        assert!(!loops[0].final_value_observable);
        assert!(matches!(loops[0].bound_mem, MirMem::Param { .. }));
    }

    #[test]
    fn selects_cursor_and_remaining_count_for_runtime_pointer_loop() {
        let mut routine = lowered_fixture();
        let stats = select_dynamic_word_index_loops(&mut routine);

        assert_eq!(stats.candidates, 1);
        assert_eq!(stats.selected, 1);
        assert!(
            routine
                .blocks
                .iter()
                .flat_map(|block| &block.ops)
                .any(|op| { matches!(op, MirOp::MaterializeAddress { .. }) })
        );
        assert!(
            routine
                .blocks
                .iter()
                .flat_map(|block| &block.ops)
                .any(|op| { matches!(op, MirOp::LoadIndirect { .. }) })
        );
        assert!(
            !routine
                .blocks
                .iter()
                .flat_map(|block| &block.ops)
                .any(|op| {
                    matches!(
                        op,
                        MirOp::Load {
                            src: MirAddr::ComputedIndex { .. },
                            ..
                        }
                    )
                })
        );
        assert!(
            !routine
                .blocks
                .iter()
                .flat_map(|block| &block.ops)
                .any(|op| {
                    matches!(
                        op,
                        MirOp::Compare {
                            op: MirCompareOp::Lt,
                            width: MirWidth::Word,
                            ..
                        }
                    )
                })
        );
    }

    #[test]
    fn rejects_an_observable_final_index() {
        let mut routine = lowered_fixture();
        let counted = analyze_dynamic_word_counted_loops(&routine).remove(0);
        let exit = routine
            .blocks
            .iter_mut()
            .find(|block| block.id == counted.exit)
            .unwrap();
        exit.ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x0600)),
                src: MirValue::Def(MirDef::VTemp(counted.induction)),
                width: MirWidth::Word,
            },
        );

        let stats = select_dynamic_word_index_loops(&mut routine);
        assert_eq!(stats.selected, 0);
        assert_eq!(stats.blocked_final_index, 1);
    }

    #[test]
    fn rejects_a_loop_write_to_the_runtime_bound() {
        let mut routine = lowered_fixture();
        let counted = analyze_dynamic_word_counted_loops(&routine).remove(0);
        let body = routine
            .blocks
            .iter_mut()
            .find(|block| block.id == counted.body)
            .unwrap();
        body.ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(counted.bound_mem),
                src: MirValue::ConstU16(0),
                width: MirWidth::Word,
            },
        );

        let stats = select_dynamic_word_index_loops(&mut routine);
        assert_eq!(stats.selected, 0);
        assert_eq!(stats.blocked_bound_invariance, 1);
    }
}
