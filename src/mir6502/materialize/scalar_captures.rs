//! Bounded scalar capture forwarding and independently proved home deletion.
//! Purpose metadata selects candidates; it never establishes memory privacy.
use super::{address_values::Cost, temp_widths};
use crate::mir6502::{
    Mir6502Config, MirDiagnostic,
    analysis::{
        effects::classify_op,
        private_cells,
        use_def::{MirTempLane, MirTempUseDefIndex},
    },
    ir::*,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Selection {
    forwarded: BTreeMap<RoutineId, usize>,
    removed: BTreeMap<RoutineId, Vec<MirMem>>,
    kept: BTreeMap<RoutineId, Vec<MirMem>>,
}

fn enabled(slot: &MirStorageSlot, config: &Mir6502Config) -> bool {
    if !matches!(slot.storage_size, 1 | 2) || slot.scalar_width.is_none() {
        return false;
    }
    match slot.scalar_purpose {
        MirScalarPurpose::ArgumentCapture => config.enable_argument_capture_forwarding,
        MirScalarPurpose::PatternBinding => false,
        MirScalarPurpose::Storage => false,
    }
}

pub(in crate::mir6502) fn materialize_costed(
    original: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let mut off = config.clone();
    off.enable_argument_capture_forwarding = false;
    let fallback = |p, reporting| {
        crate::mir6502::materialize_resolved_program_without_scalar_costing(
            p, &off, origin, runtime, reporting,
        )
    };
    if !original
        .routines
        .iter()
        .any(|r| r.frame.locals.iter().any(|s| enabled(s, config)))
    {
        return fallback(original, report);
    }
    let mut selected = original.clone();
    let mut selection = select(&mut selected, config);
    let mut affected = selection
        .forwarded
        .keys()
        .chain(selection.removed.keys())
        .chain(selection.kept.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    if affected.is_empty() {
        return fallback(original, report);
    }
    let attempted = affected.clone();
    crate::mir6502::verify_program(&selected, MirPhase::PreMaterialization)?;
    let baseline = fallback(original.clone(), false)?;
    let mut proposed = fallback(selected.clone(), false).ok();
    // One growing routine must not hide an independently profitable routine.
    // Roll back rejected routines once, then remeasure the complete layout.
    if let Some((old, new)) = Cost::measure(&baseline, origin)
        .zip(proposed.as_ref().and_then(|p| Cost::measure(p, origin)))
    {
        let rejected = affected
            .iter()
            .copied()
            .filter(|&id| !new.does_not_grow(&old, id))
            .collect::<BTreeSet<_>>();
        if !rejected.is_empty() {
            for r in &mut selected.routines {
                if rejected.contains(&r.id) {
                    *r = original
                        .routines
                        .iter()
                        .find(|old| old.id == r.id)
                        .unwrap()
                        .clone();
                }
            }
            affected.retain(|id| !rejected.contains(id));
            selection.forwarded.retain(|id, _| !rejected.contains(id));
            selection.removed.retain(|id, _| !rejected.contains(id));
            selection.kept.retain(|id, _| !rejected.contains(id));
            proposed = if affected.is_empty() {
                None
            } else {
                fallback(selected.clone(), false).ok()
            };
        }
    }
    let accepted = proposed.as_ref().is_some_and(|proposed| {
        Cost::measure(&baseline, origin)
            .zip(Cost::measure(proposed, origin))
            .is_some_and(|(old, new)| {
                new.bytes < old.bytes && affected.iter().all(|&id| new.does_not_grow(&old, id))
            })
            && occupied(proposed).is_subset(&occupied(&baseline))
            && private_cells::physical_cells_unaliased(&baseline, origin, &selection.removed)
            && private_cells::physical_cells_unaliased(&baseline, origin, &selection.kept)
            && private_cells::physical_cells_unaliased(proposed, origin, &selection.kept)
    });
    if report {
        let mut stats = super::MirPeepholeStats::default();
        for &id in &attempted {
            if accepted && affected.contains(&id) {
                stats.record_many(
                    id,
                    "scalar-capture-load-forwarded",
                    selection.forwarded.get(&id).copied().unwrap_or(0),
                );
                stats.record_many(
                    id,
                    "scalar-capture-stored-bytes-removed",
                    selection.kept.get(&id).map_or(0, Vec::len),
                );
                stats.record_many(
                    id,
                    "scalar-capture-home-bytes-removed",
                    selection.removed.get(&id).map_or(0, Vec::len),
                );
            } else {
                stats.record(id, "scalar-capture-declined-final-cost-or-alias");
            }
        }
        super::maybe_report_peepholes(&original, &stats, config);
    }
    if report && super::stats::reporting_enabled(config) {
        fallback(if accepted { selected } else { original }, true)
    } else {
        Ok(if accepted {
            proposed.unwrap()
        } else {
            baseline
        })
    }
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

fn captured(
    value: &MirValue,
    width: MirWidth,
    defs: &MirTempUseDefIndex,
    widths: &BTreeMap<MirTempId, MirWidth>,
) -> bool {
    match value {
        MirValue::ConstU8(_) | MirValue::ConstU16(_) => true,
        MirValue::Def(MirDef::VTemp(id)) => {
            widths.get(id) == Some(&width)
                && (0..if width == MirWidth::Byte { 1 } else { 2 }).all(|byte| {
                    defs.unique_definition(MirTempLane { temp: *id, byte })
                        .is_some()
                })
        }
        MirValue::Def(MirDef::VTempByte { id, byte }) if width == MirWidth::Byte => defs
            .unique_definition(MirTempLane {
                temp: *id,
                byte: *byte,
            })
            .is_some(),
        MirValue::Word { lo, hi } if width == MirWidth::Word => {
            captured(lo, MirWidth::Byte, defs, widths) && captured(hi, MirWidth::Byte, defs, widths)
        }
        // Physical registers and memory operands are live reads, not captures.
        _ => false,
    }
}

fn select(program: &mut MirProgram, config: &Mir6502Config) -> Selection {
    let mut selection = Selection::default();
    for routine in &mut program.routines {
        let slots = routine.frame.locals.iter().filter_map(|s| {
            let MirStorageBase::Local(id) = s.base else { return None; };
            let width = s.scalar_width?;
            (enabled(s, config) && s.offset == 0 && s.init.is_none()
                && s.storage_size == if width == MirWidth::Byte { 1 } else { 2 }
                && !routine.frame.locals.iter().any(|other| matches!(other.base, MirStorageBase::LocalAlias { target, .. } if target == id)))
                .then_some((id, width))
        }).collect::<BTreeMap<_, _>>();
        if slots.is_empty() {
            continue;
        }
        let defs = MirTempUseDefIndex::from_routine(routine);
        let widths = temp_widths::collect_routine_temp_widths(routine);
        for block in &mut routine.blocks {
            let mut values = BTreeMap::<crate::nir::LocalId, (MirValue, usize)>::new();
            for (index, op) in block.ops.iter_mut().enumerate() {
                // Bound extended live ranges even when a large block is pure.
                values.retain(|_, (_, start)| index - *start <= 32);
                if let MirOp::Load {
                    dst: MirDef::VTemp(dst),
                    src: MirAddr::Direct(MirMem::Local { id, offset: 0 }),
                    width,
                } = op
                {
                    if slots.get(id) == Some(width) {
                        if let Some((value, _)) = values.get(id) {
                            *op = MirOp::Move {
                                dst: MirDef::VTemp(*dst),
                                src: value.clone(),
                                width: *width,
                            };
                            *selection.forwarded.entry(routine.id).or_default() += 1;
                            continue;
                        }
                    }
                }
                let effect = classify_op(op);
                // Stop at unknown effects, absolute/hardware memory, calls and
                // pointer accesses. Ordinary distinct frame cells are disjoint;
                // alias views are excluded below as well as from candidates.
                if matches!(
                    op,
                    MirOp::Call { .. }
                        | MirOp::RuntimeHelper { .. }
                        | MirOp::MachineBlock { .. }
                        | MirOp::Barrier { .. }
                ) || effect.memory.opaque
                    || effect.memory.has_unknown_effects
                    || effect.memory.indirect_reads
                    || effect.memory.indirect_writes
                    || effect.memory.may_write_any
                    || effect.memory.structured_reads != MirMemoryEffect::None
                    || effect.memory.structured_writes != MirMemoryEffect::None
                    || effect
                        .memory
                        .direct_reads
                        .iter()
                        .chain(&effect.memory.direct_writes)
                        .any(|r| match r.base {
                            MirMem::Local { id, .. } => !routine
                                .frame
                                .locals
                                .iter()
                                .any(|s| s.base == MirStorageBase::Local(id)),
                            MirMem::Param { id, .. } => !routine
                                .frame
                                .params
                                .iter()
                                .any(|s| s.base == MirStorageBase::Param(id)),
                            _ => true,
                        })
                {
                    values.clear();
                    continue;
                }
                for range in &effect.memory.direct_writes {
                    if let MirMem::Local { id, .. } = range.base {
                        values.remove(&id);
                    }
                }
                if let MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Local { id, offset: 0 }),
                    src,
                    width,
                } = op
                {
                    if slots.get(id) == Some(width) && captured(src, *width, &defs, &widths) {
                        values.insert(*id, (src.clone(), index));
                    }
                }
            }
        }
    }
    // Rebuild observation facts after forwarding. Removing a store is a
    // separate proof, including paths after the last direct value use.
    let only_named_reads = private_cells::has_only_named_reads(program);
    for index in 0..program.routines.len() {
        let routine = &program.routines[index];
        let removable = routine
            .frame
            .locals
            .iter()
            .filter(|s| enabled(s, config))
            .filter_map(|s| match s.base {
                MirStorageBase::Local(id)
                    if private_cells::scalar_home_unobserved(program, routine, id) =>
                {
                    Some((id, s.storage_size))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let kept = routine
            .frame
            .locals
            .iter()
            .filter(|s| enabled(s, config))
            .filter_map(|s| match s.base {
                MirStorageBase::Local(id)
                    if !removable.contains_key(&id)
                        && only_named_reads
                        && private_cells::home_has_only_captured_stores(routine, id) =>
                {
                    Some((id, s.storage_size))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        if removable.is_empty() && kept.is_empty() {
            continue;
        }
        let routine = &mut program.routines[index];
        for block in &mut routine.blocks {
            block.ops.retain(|op| !matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, .. }), .. } if removable.contains_key(id) || kept.contains_key(id)));
        }
        routine.frame.locals.retain(
            |s| !matches!(s.base, MirStorageBase::Local(id) if removable.contains_key(&id)),
        );
        if !kept.is_empty() {
            selection.kept.insert(
                routine.id,
                kept.into_iter()
                    .flat_map(|(id, bytes)| {
                        (0..bytes).map(move |offset| MirMem::Local { id, offset })
                    })
                    .collect(),
            );
        }
        if !removable.is_empty() {
            selection.removed.insert(
                routine.id,
                removable
                    .into_iter()
                    .flat_map(|(id, bytes)| {
                        (0..bytes).map(move |offset| MirMem::Local { id, offset })
                    })
                    .collect(),
            );
        }
    }
    selection
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lower(source: &str, optimized: bool) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let raw = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let nir = if optimized {
            crate::nir::optimize_program(&raw).unwrap()
        } else {
            raw
        };
        crate::mir6502::lower_program(&nir).unwrap()
    }
    const ARGUMENTS: &str = "TYPE Data=[CARD value]\nData result\nCARD number\nData FUNC Make(CARD n BYTE b)\nData item\nitem.value=n+b\nRETURN(item)\nPROC Main()\nresult=Make(number,42)\nRETURN\n";

    #[test]
    fn scalar_captures_forward_arguments_but_preserve_observable_stores() {
        for optimized in [false, true] {
            let mut program = lower(ARGUMENTS, optimized);
            let selected = select(&mut program, &Mir6502Config::default());
            assert!(optimized || selected.forwarded.values().sum::<usize>() >= 1);
            assert!(
                selected.removed.is_empty(),
                "result address exposure retains the allocation"
            );
            assert!(
                !selected.kept.is_empty(),
                "named reads prove argument stores dead"
            );
            crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
            assert!(
                select(&mut program, &Mir6502Config::default())
                    .forwarded
                    .is_empty()
            );
        }
    }

    #[test]
    fn scalar_captures_final_cost_never_grows() {
        for source in [ARGUMENTS] {
            for optimized in [false, true] {
                let program = lower(source, optimized);
                for runtime in [
                    crate::runtime::Runtime::ActionCart,
                    crate::runtime::Runtime::Standalone,
                ] {
                    let mut off = Mir6502Config::default();
                    off.enable_argument_capture_forwarding = false;
                                    let old = crate::mir6502::materialize_program_with_origin_and_runtime(
                        program.clone(),
                        &off,
                        0x3000,
                        runtime,
                    )
                    .unwrap();
                    let new = crate::mir6502::materialize_program_with_origin_and_runtime(
                        program.clone(),
                        &Mir6502Config::default(),
                        0x3000,
                        runtime,
                    )
                    .unwrap();
                    let a = Cost::measure(&old, 0x3000).unwrap();
                    let b = Cost::measure(&new, 0x3000).unwrap();
                    assert!(b.bytes <= a.bytes);
                    if source == ARGUMENTS || !optimized {
                        assert!(b.bytes < a.bytes);
                    } else {
                        assert_eq!(old, new, "already promoted NIR needs no additional rewrite");
                    }
                }
            }
        }
    }

    #[test]
    fn scalar_capture_observation_census_rejects_hardware_and_pointer_effects() {
        assert!(private_cells::has_only_named_reads(&lower(
            ARGUMENTS, false
        )));
        for reason in [
            "barrier",
            "absolute read",
            "pointer read",
            "pointer write",
            "volatile copy",
            "absolute global",
        ] {
            let mut p = lower(ARGUMENTS, false);
            if reason == "absolute global" {
                p.globals
                    .iter_mut()
                    .find(|g| g.name == "number")
                    .unwrap()
                    .backing = MirGlobalBacking::Absolute(0xD20A);
            } else if reason == "volatile copy" {
                let copy = p
                    .routines
                    .iter_mut()
                    .flat_map(|r| &mut r.blocks)
                    .flat_map(|b| &mut b.ops)
                    .find(|op| matches!(op, MirOp::CopyBytes { .. }))
                    .unwrap();
                let MirOp::CopyBytes {
                    destination_volatile,
                    ..
                } = copy
                else {
                    unreachable!()
                };
                *destination_volatile = true;
            } else {
                let op = match reason {
                    "barrier" => MirOp::Barrier {
                        effects: MirEffects::default(),
                    },
                    "absolute read" => MirOp::Load {
                        dst: MirDef::VTemp(MirTempId(900)),
                        src: MirAddr::Direct(MirMem::Absolute(0xD20A)),
                        width: MirWidth::Byte,
                    },
                    "pointer read" => MirOp::Load {
                        dst: MirDef::VTemp(MirTempId(900)),
                        src: MirAddr::Deref {
                            ptr: MirValue::ConstU16(0x900),
                            offset: 0,
                        },
                        width: MirWidth::Byte,
                    },
                    "pointer write" => MirOp::Store {
                        dst: MirAddr::Deref {
                            ptr: MirValue::ConstU16(0x900),
                            offset: 0,
                        },
                        src: MirValue::ConstU8(0),
                        width: MirWidth::Byte,
                    },
                    _ => unreachable!(),
                };
                p.routines[0].blocks[0].ops.insert(0, op);
            }
            assert!(!private_cells::has_only_named_reads(&p), "{reason}");
        }
    }
}
