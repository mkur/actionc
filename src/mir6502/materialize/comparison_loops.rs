//! Bounded, transactional selection followed by one final batch cost gate.
use super::*;
use crate::mir6502::analysis::{
    comparison_loops::ComparisonLoop,
    prehome::PreHomeAnalysisSnapshot,
    sites::{MirRoutineGeneration, MirSite},
};
use crate::mir6502::rewrite::context::PreHomeRewriteContext;

fn consumer(lo: u8, live: bool) -> MirAddressConsumer {
    let p = MirPointerPair::Fixed {
        lo: MirFixedZpSlot(lo),
    };
    if live {
        MirAddressConsumer::LiveIndirectIndexedY(p)
    } else {
        MirAddressConsumer::IndirectIndexedY(p)
    }
}
fn next_byte(mem: &MirMem) -> Option<MirMem> {
    match mem {
        MirMem::Local { id, offset } => Some(MirMem::Local {
            id: *id,
            offset: offset.checked_add(1)?,
        }),
        MirMem::Param { id, offset } => Some(MirMem::Param {
            id: *id,
            offset: offset.checked_add(1)?,
        }),
        _ => None,
    }
}
fn candidate(
    r: &super::super::ir::MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    f: &ComparisonLoop,
    fuse: bool,
    carry_y: bool,
) -> Option<super::super::ir::MirRoutine> {
    if f.generation != c.generation() {
        return None;
    }
    // Reconstruct the proof independently when applying, including every edge
    // and complete use chain. A stale or edited source never partially applies.
    if c.comparison_loop(r, f.header).ok().as_ref() != Some(f) {
        return None;
    }
    for id in [f.body, f.header] {
        let point = c.point(MirSite::Terminator { block: id });
        if !c.register_dead_after(MirReg::Y, point).is_proven()
            || [consumer(0xAA, false), consumer(0xAC, false)]
                .iter()
                .any(|&p| !c.pointer_pair_dead_after(p, point).is_proven())
        {
            return None;
        }
    }
    if [f.header, f.body].iter().any(|&block| {
        !c.flags_dead_after(
            crate::mir6502::analysis::effects::MirFlagSet::all(),
            MirSite::Terminator { block },
        )
    }) {
        return None;
    }
    let fusion = fuse
        .then(|| super::comparison_fusion::prove(r, c, f))
        .flatten();
    let carry_y = carry_y && super::comparison_index::prove(r, c, f);
    let initial_store = r.blocks.iter().find(|b| b.id == f.preheader)?.ops.len() - 1;
    let mut result = r.clone();
    let pre = result.blocks.iter_mut().find(|b| b.id == f.preheader)?;
    for (operand, lo) in [(&f.left, 0xAA), (&f.right, 0xAC)] {
        pre.ops.push(MirOp::MaterializeIndexedAddress {
            consumer: consumer(lo, false),
            base: MirValue::Word {
                lo: Box::new(MirValue::PointerCell(operand.source.clone())),
                hi: Box::new(MirValue::PointerCell(next_byte(&operand.source)?)),
            },
            index: MirValue::ConstU16(operand.displacement),
            scale: 1,
        });
    }
    let body = result.blocks.iter_mut().find(|b| b.id == f.body)?;
    let removed = f
        .left
        .construction
        .union(&f.right.construction)
        .copied()
        .collect::<BTreeSet<_>>();
    let fused_removed = if fusion.is_some() {
        BTreeSet::from([
            f.left.load,
            f.left.load + 1,
            f.right.load,
            f.right.load + 1,
            body.ops.len() - 3,
            body.ops.len() - 2,
            body.ops.len() - 1,
        ])
    } else {
        BTreeSet::new()
    };
    let mut ops = vec![];
    for (i, op) in body.ops.iter().enumerate() {
        if fusion.is_some() && i == f.left.load {
            ops.push(MirOp::Move {
                dst: MirDef::Reg(MirReg::Y),
                src: MirValue::PointerCell(f.induction.clone()),
                width: MirWidth::Byte,
            });
            ops.push(MirOp::CompareIndirectBytes {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Ne,
                left: consumer(0xAA, true),
                right: consumer(0xAC, true),
                offset: 0,
                signed: false,
            });
        }
        if fused_removed.contains(&i) {
            continue;
        }
        if removed.contains(&i) {
            continue;
        }
        let lo = if i == f.left.load {
            Some(0xAA)
        } else if i == f.right.load {
            Some(0xAC)
        } else {
            None
        };
        if let Some(lo) = lo {
            if i == f.left.load {
                ops.push(MirOp::Move {
                    dst: MirDef::Reg(MirReg::Y),
                    src: MirValue::PointerCell(f.induction.clone()),
                    width: MirWidth::Byte,
                });
            }
            let MirOp::Load { dst, .. } = op else {
                return None;
            };
            ops.push(MirOp::LoadIndirect {
                consumer: consumer(lo, true),
                dst: dst.clone(),
                offset: 0,
            });
        } else {
            ops.push(op.clone())
        }
    }
    body.ops = ops;
    if let Some(ordering) = fusion {
        if let MirTerminator::Branch { cond, .. } = &mut body.terminator {
            *cond = MirCond::FlagTest(MirFlagTest::ZClear)
        }
        if let Some(op) = ordering {
            super::comparison_fusion::replace_ordering(&mut result, f, op);
        }
    }
    if carry_y {
        super::comparison_index::apply(&mut result, f, initial_store);
    }
    reserve_used_fixed_zero_page_slots(&mut result);
    MirCfg::from_routine(&result).ok()?;
    if !crate::mir6502::analysis::live_y::invalid_uses(&result).is_empty() {
        return None;
    }
    Some(result)
}

fn select(r: &mut super::super::ir::MirRoutine, fuse: bool, carry_y: bool) -> usize {
    let mut generation = MirRoutineGeneration::initial();
    let mut count = 0;
    loop {
        if !crate::mir6502::analysis::comparison_loops::might_match(r) {
            break;
        }
        let Ok(snapshot) = PreHomeAnalysisSnapshot::new(r, generation) else {
            break;
        };
        let context = PreHomeRewriteContext::new(&snapshot);
        let accepted = r
            .blocks
            .iter()
            .filter_map(|b| context.comparison_loop(r, b.id).ok())
            .find_map(|f| candidate(r, &context, &f, fuse, carry_y));
        let Some(candidate) = accepted else { break };
        *r = candidate;
        generation = generation.next();
        count += 1;
    }
    count
}

fn scratch_private(p: &MirProgram, origin: u16) -> bool {
    if origin < 0x100 {
        return false;
    }
    let layout = MaterializeLayout::new(p, origin);
    let overlaps =
        |a: u16, size: u16| (0..size).any(|n| (0xAA..=0xAD).contains(&a.wrapping_add(n)));
    for g in &p.globals {
        if layout
            .mem_address(
                RoutineId(0),
                &MirMem::Global {
                    id: g.id,
                    offset: 0,
                },
            )
            .is_none_or(|a| overlaps(a, g.storage_size))
        {
            return false;
        }
    }
    for r in &p.routines {
        for slot in r.frame.locals.iter().chain(&r.frame.params) {
            let mem = match slot.base {
                MirStorageBase::Local(id) => MirMem::Local { id, offset: 0 },
                MirStorageBase::Param(id) => MirMem::Param { id, offset: 0 },
                MirStorageBase::Absolute(a) => {
                    if overlaps(a.wrapping_add(slot.offset), slot.storage_size) {
                        return false;
                    }
                    continue;
                }
                _ => continue,
            };
            if layout
                .mem_address(r.id, &mem)
                .is_none_or(|a| overlaps(a, slot.storage_size))
            {
                return false;
            }
        }
        // Absolute aliases of compiler scratch are source-owned observations.
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            let effect = crate::mir6502::analysis::effects::classify_op(op);
            for range in effect
                .memory
                .direct_reads
                .iter()
                .chain(&effect.memory.direct_writes)
            {
                if let MirMem::Absolute(a) = range.base {
                    if overlaps(a, range.bytes) {
                        return false;
                    }
                }
            }
        }
    }
    true
}
// Unexposed private captures cannot be reached by source pointers. Check
// separately named absolute aliases again after every final layout/rollback.
fn private_cells_unaliased(
    p: &MirProgram,
    origin: u16,
    cells: &BTreeMap<RoutineId, Vec<MirMem>>,
) -> bool {
    let layout = MaterializeLayout::new(p, origin);
    let mut addresses = vec![];
    for (&id, mems) in cells {
        for mem in mems {
            let Some(a) = layout.mem_address(id, mem) else {
                return false;
            };
            addresses.push(a);
        }
    }
    let overlaps = |a: u16, bytes: u16| addresses.iter().any(|&x| x.wrapping_sub(a) < bytes);
    for g in &p.globals {
        if g.storage_size > 0
            && layout
                .mem_address(
                    RoutineId(0),
                    &MirMem::Global {
                        id: g.id,
                        offset: 0,
                    },
                )
                .is_none_or(|a| overlaps(a, g.storage_size))
        {
            return false;
        }
    }
    for r in &p.routines {
        for slot in r.frame.locals.iter().chain(&r.frame.params) {
            if let MirStorageBase::Absolute(a) = slot.base {
                if overlaps(a.wrapping_add(slot.offset), slot.storage_size) {
                    return false;
                }
            }
        }
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            let effect = crate::mir6502::analysis::effects::classify_op(op);
            if effect
                .memory
                .direct_reads
                .iter()
                .chain(&effect.memory.direct_writes)
                .any(|range| matches!(range.base,MirMem::Absolute(a) if overlaps(a,range.bytes)))
            {
                return false;
            }
        }
    }
    true
}

fn occupied(p: &MirProgram) -> BTreeSet<u8> {
    p.routines
        .iter()
        .flat_map(|r| {
            r.frame.fixed_zero_page.iter().map(|s| s.0).chain(
                r.frame
                    .zero_page_allocations
                    .iter()
                    .flat_map(|a| (0..a.size).map(move |n| a.start.0.wrapping_add(n))),
            )
        })
        .collect()
}

pub(in crate::mir6502) fn materialize_costed(
    original: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let mut off = config.clone();
    off.enable_comparison_loop_pointers = false;
    let fallback = |p, c, report| {
        crate::mir6502::materialize_resolved_program_without_comparison_costing(
            p, c, origin, runtime, report,
        )
    };
    if !original
        .routines
        .iter()
        .any(crate::mir6502::analysis::comparison_loops::might_match)
    {
        return fallback(original, &off, report);
    }
    if !scratch_private(&original, origin) {
        return fallback(original, &off, report);
    }
    let mut private_cells = BTreeMap::new();
    if config.enable_comparison_loop_fusion || config.enable_comparison_loop_y {
        for r in original
            .routines
            .iter()
            .filter(|r| crate::mir6502::analysis::comparison_loops::might_match(r))
        {
            if let Ok(snapshot) = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()) {
                let c = PreHomeRewriteContext::new(&snapshot);
                let cells = r
                    .blocks
                    .iter()
                    .filter_map(|b| c.comparison_loop(r, b.id).ok())
                    .flat_map(|f| [f.left.capture, f.right.capture, f.induction])
                    .collect::<Vec<_>>();
                if !cells.is_empty() {
                    private_cells.insert(r.id, cells);
                }
            }
        }
    }
    let mut selected = original.clone();
    let mut candidates = BTreeSet::new();
    for r in &mut selected.routines {
        if select(
            r,
            config.enable_comparison_loop_fusion,
            config.enable_comparison_loop_y,
        ) > 0
        {
            candidates.insert(r.id);
        }
    }
    if candidates.is_empty() {
        return fallback(original, &off, report);
    }
    crate::mir6502::verify::verify_program(&selected, MirPhase::PreMaterialization)?;
    // Baseline, batch and at most one partial-batch repair. Existing argument
    // and address gates remain coordinated inside these bounded trials.
    let baseline = fallback(original.clone(), &off, false)?;
    let Ok(mut proposed) = fallback(selected.clone(), &off, false) else {
        return fallback(original, &off, report);
    };
    let (Some(old), Some(new)) = (
        address_values::Cost::measure(&baseline, origin),
        address_values::Cost::measure(&proposed, origin),
    ) else {
        return fallback(original, &off, report);
    };
    let mut accepted = candidates
        .iter()
        .copied()
        .filter(|&id| new.does_not_grow(&old, id))
        .collect::<BTreeSet<_>>();
    if accepted.is_empty() {
        return if report && stats::reporting_enabled(config) {
            fallback(original, &off, true)
        } else {
            Ok(baseline)
        };
    }
    if accepted != candidates {
        for r in &mut selected.routines {
            if candidates.contains(&r.id) && !accepted.contains(&r.id) {
                *r = original
                    .routines
                    .iter()
                    .find(|o| o.id == r.id)
                    .unwrap()
                    .clone();
            }
        }
        let Ok(repaired) = fallback(selected.clone(), &off, false) else {
            return fallback(original, &off, report);
        };
        proposed = repaired;
    }
    // AA..AF is the existing fixed compiler pointer window; only virtual
    // allocations outside it must retain their old occupied set.
    let mut old_occupied = occupied(&baseline);
    old_occupied.extend(0xAA..=0xAF);
    if address_values::Cost::measure(&proposed, origin).is_none_or(|cost| {
        cost.bytes > old.bytes || accepted.iter().any(|&id| !cost.does_not_grow(&old, id))
    }) || !occupied(&proposed).is_subset(&old_occupied)
        || !scratch_private(&proposed, origin)
        || !private_cells_unaliased(&proposed, origin, &private_cells)
    {
        accepted.clear();
    }
    if report {
        let mut stats = MirPeepholeStats::default();
        for &id in &candidates {
            stats.record(
                id,
                if accepted.contains(&id) {
                    "comparison-loop-applied"
                } else {
                    "comparison-loop-declined-final-cost"
                },
            );
        }
        maybe_report_peepholes(&original, &stats, config);
    }
    if report && stats::reporting_enabled(config) {
        fallback(
            if accepted.is_empty() {
                original
            } else {
                selected
            },
            &off,
            true,
        )
    } else {
        Ok(if accepted.is_empty() {
            baseline
        } else {
            proposed
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::comparison_loops::tests::{facts, program};
    #[test]
    fn comparison_selection_is_atomic_and_keeps_captures() {
        let mut p = program();
        let r = &mut p.routines[1];
        let f = facts(r).remove(0);
        let snapshot = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        let c = PreHomeRewriteContext::new(&snapshot);
        let mut stale = f.clone();
        stale.generation = stale.generation.next();
        assert!(candidate(r, &c, &stale, false, false).is_none());
        let result = candidate(r, &c, &f, false, false).expect("proven region");
        assert_eq!(
            result
                .blocks
                .iter()
                .find(|b| b.id == f.body)
                .unwrap()
                .ops
                .iter()
                .filter(|o| matches!(o, MirOp::Store { .. }))
                .count(),
            2
        );
        assert!(crate::mir6502::analysis::live_y::invalid_uses(&result).is_empty());
        let mut broken = result.clone();
        broken
            .blocks
            .iter_mut()
            .find(|b| b.id == f.preheader)
            .unwrap()
            .ops
            .retain(|o| !matches!(o, MirOp::MaterializeIndexedAddress { .. }));
        assert!(!crate::mir6502::analysis::live_y::invalid_uses(&broken).is_empty());
        assert_eq!(select(r, false, false), 1);
        assert_eq!(select(r, false, false), 0);
    }
    #[test]
    fn comparison_fusion_keeps_observed_values_and_reuses_only_exact_ordering() {
        let p = program();
        for (r, count) in p.routines.iter().zip([2, 1, 0, 1, 0]) {
            let snapshot =
                PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
            let c = PreHomeRewriteContext::new(&snapshot);
            assert_eq!(
                facts(r)
                    .iter()
                    .filter(|f| super::super::comparison_fusion::prove(r, &c, f).is_some())
                    .count(),
                count,
                "{}",
                r.name
            );
        }
        let r = &p.routines[1];
        let f = facts(r).remove(0);
        let snapshot = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        let c = PreHomeRewriteContext::new(&snapshot);
        let result = candidate(r, &c, &f, true, false).unwrap();
        let body = result.blocks.iter().find(|b| b.id == f.body).unwrap();
        assert_eq!(body.ops.len(), 2);
        let effect = crate::mir6502::analysis::effects::classify_op(&body.ops[1]);
        assert!(effect.machine.register_reads.y);
        assert!(!effect.machine.register_writes.y);
        assert_eq!(effect.addresses.pair_reads.len(), 4);
        let mut broken = result.clone();
        broken
            .blocks
            .iter_mut()
            .find(|b| b.id == f.body)
            .unwrap()
            .ops
            .insert(
                0,
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAA))),
                    src: MirValue::ConstU8(0),
                    width: MirWidth::Byte,
                },
            );
        assert!(!crate::mir6502::analysis::live_y::invalid_uses(&broken).is_empty());
        let mut changed = r.clone();
        changed
            .blocks
            .iter_mut()
            .find(|b| b.id == f.complete)
            .unwrap()
            .ops
            .insert(
                0,
                MirOp::LeaAddr {
                    dst: MirDef::Reg(MirReg::A),
                    target: f.left.capture.clone(),
                    width: MirWidth::Word,
                },
            );
        let snapshot =
            PreHomeAnalysisSnapshot::new(&changed, MirRoutineGeneration::initial()).unwrap();
        let c = PreHomeRewriteContext::new(&snapshot);
        assert!(super::super::comparison_fusion::prove(&changed, &c, &f).is_none());
    }
    #[test]
    fn comparison_y_carrier_preserves_observed_induction_and_rejects_extra_temp_users() {
        let p = program();
        for (r, count) in p.routines.iter().zip([2, 1, 0, 1, 0]) {
            let snapshot =
                PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
            let c = PreHomeRewriteContext::new(&snapshot);
            assert_eq!(
                facts(r)
                    .iter()
                    .filter(|f| super::super::comparison_index::prove(r, &c, f))
                    .count(),
                count,
                "{}",
                r.name
            );
        }
        let r = &p.routines[1];
        let f = facts(r).remove(0);
        let snapshot = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        let c = PreHomeRewriteContext::new(&snapshot);
        let result = candidate(r, &c, &f, true, true).unwrap();
        assert!(crate::mir6502::analysis::live_y::invalid_uses(&result).is_empty());
        assert!(matches!(
            result
                .blocks
                .iter()
                .find(|b| b.id == f.latch)
                .unwrap()
                .ops
                .as_slice(),
            [
                MirOp::UpdateReg { reg: MirReg::Y, .. },
                MirOp::Compare { .. }
            ]
        ));
        let mut broken = result.clone();
        broken
            .blocks
            .iter_mut()
            .find(|b| b.id == f.preheader)
            .unwrap()
            .ops
            .retain(|o| {
                !matches!(
                    o,
                    MirOp::LoadImm {
                        dst: MirDef::Reg(MirReg::Y),
                        ..
                    }
                )
            });
        assert!(!crate::mir6502::analysis::live_y::invalid_uses(&broken).is_empty());
        let mut changed = r.clone();
        let header = changed.blocks.iter().find(|b| b.id == f.header).unwrap();
        let MirOp::Load { dst, .. } = &header.ops[0] else {
            panic!()
        };
        let dst = dst.clone();
        changed
            .blocks
            .iter_mut()
            .find(|b| b.id == f.complete)
            .unwrap()
            .ops
            .insert(
                0,
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
                    src: MirValue::Def(dst),
                    width: MirWidth::Byte,
                },
            );
        let snapshot =
            PreHomeAnalysisSnapshot::new(&changed, MirRoutineGeneration::initial()).unwrap();
        let c = PreHomeRewriteContext::new(&snapshot);
        assert!(!super::super::comparison_index::prove(&changed, &c, &f));
    }
    #[test]
    fn comparison_private_cells_reject_alias_metadata_and_malformed_live_consumers() {
        let mut p = program();
        let r = &p.routines[1];
        let f = facts(r).remove(0);
        let snapshot = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        let c = PreHomeRewriteContext::new(&snapshot);
        let selected = candidate(r, &c, &f, true, true).unwrap();
        let mut aliased = r.clone();
        let MirMem::Local { id, .. } = f.left.capture else {
            panic!()
        };
        let mut alias = aliased
            .frame
            .locals
            .iter()
            .find(|s| s.base == MirStorageBase::Local(id))
            .unwrap()
            .clone();
        alias.id = super::super::super::ir::MirStorageId(1000);
        alias.base = MirStorageBase::LocalAlias {
            id: crate::nir::LocalId(1000),
            target: id,
        };
        aliased.frame.locals.push(alias);
        let snapshot =
            PreHomeAnalysisSnapshot::new(&aliased, MirRoutineGeneration::initial()).unwrap();
        let c = PreHomeRewriteContext::new(&snapshot);
        assert!(super::super::comparison_fusion::prove(&aliased, &c, &f).is_none());
        let mut exposed_frame = r.clone();
        exposed_frame
            .blocks
            .iter_mut()
            .find(|b| b.id == f.complete)
            .unwrap()
            .ops
            .insert(
                0,
                MirOp::LeaAddr {
                    dst: MirDef::Reg(MirReg::A),
                    target: f.induction.clone(),
                    width: MirWidth::Word,
                },
            );
        let snapshot =
            PreHomeAnalysisSnapshot::new(&exposed_frame, MirRoutineGeneration::initial()).unwrap();
        let c = PreHomeRewriteContext::new(&snapshot);
        assert!(super::super::comparison_fusion::prove(&exposed_frame, &c, &f).is_none());
        let mut self_copy = selected.clone();
        for op in &mut self_copy
            .blocks
            .iter_mut()
            .find(|b| b.id == f.preheader)
            .unwrap()
            .ops
        {
            if matches!(
                op,
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::Y),
                    ..
                }
            ) {
                *op = MirOp::Move {
                    dst: MirDef::Reg(MirReg::Y),
                    src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                    width: MirWidth::Byte,
                };
            }
        }
        assert!(!crate::mir6502::analysis::live_y::invalid_uses(&self_copy).is_empty());
        p.routines[1] = selected;
        crate::mir6502::verify::verify_program(&p, MirPhase::PreMaterialization).unwrap();
        for malformed in 0..4 {
            let mut broken = p.clone();
            let op = broken.routines[1]
                .blocks
                .iter_mut()
                .flat_map(|b| &mut b.ops)
                .find(|o| matches!(o, MirOp::CompareIndirectBytes { .. }))
                .unwrap();
            let MirOp::CompareIndirectBytes {
                left,
                right,
                offset,
                ..
            } = op
            else {
                unreachable!()
            };
            match malformed {
                0 => *offset = 1,
                1 => *right = consumer(0xAC, false),
                2 => *right = *left,
                _ => *right = consumer(255, true),
            }
            assert!(
                crate::mir6502::verify::verify_program(&broken, MirPhase::PreMaterialization)
                    .is_err()
            );
        }
    }
}
