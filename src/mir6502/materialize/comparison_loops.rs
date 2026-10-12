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
    let mut ops = vec![];
    for (i, op) in body.ops.iter().enumerate() {
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
    reserve_used_fixed_zero_page_slots(&mut result);
    MirCfg::from_routine(&result).ok()?;
    if !crate::mir6502::analysis::live_y::invalid_uses(&result).is_empty() {
        return None;
    }
    Some(result)
}

fn select(r: &mut super::super::ir::MirRoutine) -> usize {
    let mut generation = MirRoutineGeneration::initial();
    let mut count = 0;
    loop {
        let Ok(snapshot) = PreHomeAnalysisSnapshot::new(r, generation) else {
            break;
        };
        let context = PreHomeRewriteContext::new(&snapshot);
        let accepted = r
            .blocks
            .iter()
            .filter_map(|b| context.comparison_loop(r, b.id).ok())
            .find_map(|f| candidate(r, &context, &f));
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
    if !scratch_private(&original, origin) {
        return fallback(original, &off, report);
    }
    let mut selected = original.clone();
    let mut candidates = BTreeSet::new();
    for r in &mut selected.routines {
        if select(r) > 0 {
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
        assert!(candidate(r, &c, &stale).is_none());
        let result = candidate(r, &c, &f).expect("proven region");
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
        assert_eq!(select(r), 1);
        assert_eq!(select(r), 0);
    }
}
