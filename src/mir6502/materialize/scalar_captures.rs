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

mod observations;

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
    match slot.purpose {
        MirStoragePurpose::ArgumentCapture => config.enable_argument_capture_forwarding,
        MirStoragePurpose::PatternBinding => config.enable_pattern_binding_forwarding,
        MirStoragePurpose::Storage | MirStoragePurpose::AggregateCapture => false,
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
    off.enable_pattern_binding_forwarding = false;
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
        let emitted = fallback(original.clone(), false)?;
        return finish(original, emitted, config, origin, runtime, report);
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
    finish(
        if accepted { selected } else { original },
        if accepted {
            proposed.unwrap()
        } else {
            baseline
        },
        config,
        origin,
        runtime,
        report,
    )
}

fn finish(
    mut selected: MirProgram,
    mut emitted: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    if config.enable_scalar_capture_call_elision {
        let stats = observations::improve(&mut selected, &mut emitted, config, origin, runtime);
        if report {
            super::maybe_report_peepholes(&selected, &stats, config);
        }
    }
    if report && super::stats::reporting_enabled(config) {
        crate::mir6502::materialize_resolved_program_without_scalar_costing(
            selected, config, origin, runtime, true,
        )
    } else {
        Ok(emitted)
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
        MirValue::StaticAddr(_) | MirValue::GlobalAddr(_) | MirValue::RoutineAddr(_)
            if width == MirWidth::Word =>
        {
            true
        }
        MirValue::RoutineAddrByte { .. } | MirValue::StorageAddrByte { .. }
            if width == MirWidth::Byte =>
        {
            true
        }
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

fn forward_loads(program: &mut MirProgram, config: &Mir6502Config) -> BTreeMap<RoutineId, usize> {
    let mut forwarded = BTreeMap::new();
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
                            *forwarded.entry(routine.id).or_default() += 1;
                            continue;
                        }
                    }
                }
                // Reading ordinary RAM cannot change an already captured
                // value. Keep this load in place; only its effect on the
                // availability map changes. Stores and other global accesses
                // still go through the conservative barrier checks below.
                if ordinary_global_load(op, &program.globals) {
                    continue;
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
    forwarded
}

fn ordinary_global_load(op: &MirOp, globals: &[MirGlobal]) -> bool {
    let MirOp::Load {
        src: MirAddr::Direct(MirMem::Global { id, offset }),
        width,
        ..
    } = op
    else {
        return false;
    };
    let bytes = if *width == MirWidth::Byte { 1 } else { 2 };
    globals.iter().any(|g| {
        g.id == *id
            && matches!(g.backing, MirGlobalBacking::Ordinary { .. })
            && offset
                .checked_add(bytes)
                .is_some_and(|end| end <= g.storage_size)
    })
}

fn select(program: &mut MirProgram, config: &Mir6502Config) -> Selection {
    let mut selection = Selection {
        forwarded: forward_loads(program, config),
        ..Selection::default()
    };
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
    pub(super) fn lower(source: &str, optimized: bool) -> MirProgram {
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
    const BINDING: &str = "TYPE Maybe=VARIANT [NONE SOME [CARD value]]\nCARD answer\nPROC Main()\nLET sample=Maybe.SOME(1234)\nCASE sample OF\nWHEN Maybe.SOME(value) THEN\nanswer=value\nWHEN Maybe.NONE THEN\nanswer=0\nESAC\nRETURN\n";

    fn global_read_arguments(optimized: bool) -> (MirProgram, RoutineId, crate::nir::LocalId) {
        let source = ARGUMENTS
            .replace("CARD number\n", "CARD number\nBYTE flag\n")
            .replace("Make(number,42)", "Make(CARD(@Main),flag)");
        let p = lower(&source, optimized);
        let r = p.routines.iter().find(|r| r.name == "Main").unwrap();
        let local = r
            .frame
            .locals
            .iter()
            .find_map(|s| match s.base {
                MirStorageBase::Local(id)
                    if s.purpose == MirStoragePurpose::ArgumentCapture
                        && s.scalar_width == Some(MirWidth::Word) =>
                {
                    Some(id)
                }
                _ => None,
            })
            .unwrap();
        let owner = r.id;
        (p, owner, local)
    }

    #[test]
    fn scalar_captures_forward_across_bounded_global_reads_without_moving_them() {
        for optimized in [false, true] {
            let (mut p, owner, local) = global_read_arguments(optimized);
            let r = p.routines.iter().find(|r| r.id == owner).unwrap();
            let before = r.blocks.clone();
            assert!(before.iter().flat_map(|b| &b.ops).any(|op| matches!(op,
                MirOp::Load { src: MirAddr::Direct(MirMem::Local { id, offset: 0 }), .. } if *id == local)));
            assert!(forward_loads(&mut p, &Mir6502Config::default()).contains_key(&owner));
            let r = p.routines.iter().find(|r| r.id == owner).unwrap();
            for (old, new) in before.iter().zip(&r.blocks) {
                assert_eq!(old.ops.len(), new.ops.len());
                for (old, new) in old.ops.iter().zip(&new.ops) {
                    if matches!(
                        old,
                        MirOp::Load {
                            src: MirAddr::Direct(MirMem::Global { .. }),
                            ..
                        }
                    ) {
                        assert_eq!(old, new, "global reads stay at their evaluation point");
                    }
                    if matches!(old, MirOp::Load { src: MirAddr::Direct(MirMem::Local { id, offset: 0 }), .. } if *id == local)
                    {
                        assert!(
                            matches!(new, MirOp::Move { .. }),
                            "captured address is forwarded"
                        );
                    }
                }
            }
            crate::mir6502::verify_program(&p, MirPhase::PreMaterialization).unwrap();
        }
    }

    #[test]
    fn scalar_captures_global_reads_keep_alias_bounds_and_write_barriers() {
        for reason in [
            "absolute",
            "alias",
            "missing",
            "out of bounds",
            "overflow",
            "word overrun",
            "pointer",
            "write",
            "redefinition",
        ] {
            let (mut p, owner, local) = global_read_arguments(true);
            let flag = p.globals.iter().find(|g| g.name == "flag").unwrap().id;
            let number = p.globals.iter().find(|g| g.name == "number").unwrap().id;
            let g = p.globals.iter_mut().find(|g| g.id == flag).unwrap();
            match reason {
                "absolute" => g.backing = MirGlobalBacking::Absolute(0xD20A),
                "alias" => {
                    g.backing = MirGlobalBacking::Alias {
                        target: number,
                        offset: 0,
                    }
                }
                _ => {}
            }
            let r = p.routines.iter_mut().find(|r| r.id == owner).unwrap();
            let b = r
                .blocks
                .iter_mut()
                .find(|b| {
                    b.ops.iter().any(|op| {
                        matches!(op,
                MirOp::Load { src: MirAddr::Direct(MirMem::Global { id, .. }), .. } if *id == flag)
                    })
                })
                .unwrap();
            let at = b
                .ops
                .iter()
                .position(|op| {
                    matches!(op,
                MirOp::Load { src: MirAddr::Direct(MirMem::Global { id, .. }), .. } if *id == flag)
                })
                .unwrap();
            let MirOp::Load { src, .. } = &mut b.ops[at] else {
                unreachable!()
            };
            match reason {
                "missing" => {
                    *src = MirAddr::Direct(MirMem::Global {
                        id: crate::nir::SymbolId(900),
                        offset: 0,
                    })
                }
                "out of bounds" => {
                    *src = MirAddr::Direct(MirMem::Global {
                        id: flag,
                        offset: 1,
                    })
                }
                "overflow" => {
                    *src = MirAddr::Direct(MirMem::Global {
                        id: flag,
                        offset: u16::MAX,
                    })
                }
                "pointer" => {
                    *src = MirAddr::Deref {
                        ptr: MirValue::ConstU16(0x9000),
                        offset: 0,
                    }
                }
                "write" => b.ops.insert(
                    at,
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Global {
                            id: flag,
                            offset: 0,
                        }),
                        src: MirValue::ConstU8(7),
                        width: MirWidth::Byte,
                    },
                ),
                "word overrun" => {
                    r.temps.push(MirTemp { id: MirTempId(900) });
                    b.ops.insert(
                        at,
                        MirOp::Load {
                            dst: MirDef::VTemp(MirTempId(900)),
                            src: MirAddr::Direct(MirMem::Global {
                                id: flag,
                                offset: 0,
                            }),
                            width: MirWidth::Word,
                        },
                    );
                }
                "redefinition" => {
                    let dst = b
                        .ops
                        .iter()
                        .find_map(|op| match op {
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Local { id, .. }),
                                src: MirValue::Def(dst),
                                ..
                            } if *id == local => Some(dst.clone()),
                            _ => None,
                        })
                        .unwrap();
                    b.ops.insert(
                        at,
                        MirOp::Load {
                            dst,
                            src: MirAddr::Direct(MirMem::Global {
                                id: number,
                                offset: 0,
                            }),
                            width: MirWidth::Word,
                        },
                    );
                }
                _ => {}
            }
            forward_loads(&mut p, &Mir6502Config::default());
            let r = p.routines.iter().find(|r| r.id == owner).unwrap();
            assert!(r.blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(op,
                MirOp::Load { src: MirAddr::Direct(MirMem::Local { id, offset: 0 }), .. } if *id == local)), "{reason}");
        }
    }

    #[test]
    fn scalar_captures_forward_immutable_address_operands() {
        let original = lower(ARGUMENTS, false);
        let owner = original
            .routines
            .iter()
            .find(|r| r.name == "Main")
            .unwrap()
            .id;
        let global = original
            .globals
            .iter()
            .find(|g| g.name == "number")
            .unwrap()
            .id;
        for address in [
            MirValue::RoutineAddr(owner),
            MirValue::GlobalAddr(global),
            MirValue::Word {
                lo: Box::new(MirValue::RoutineAddrByte { id: owner, byte: 0 }),
                hi: Box::new(MirValue::RoutineAddrByte { id: owner, byte: 1 }),
            },
        ] {
            let mut p = original.clone();
            let r = p.routines.iter_mut().find(|r| r.id == owner).unwrap();
            let local = r
                .frame
                .locals
                .iter()
                .find_map(|s| match s.base {
                    MirStorageBase::Local(id)
                        if s.purpose == MirStoragePurpose::ArgumentCapture
                            && s.scalar_width == Some(MirWidth::Word) =>
                    {
                        Some(id)
                    }
                    _ => None,
                })
                .unwrap();
            let store = r.blocks.iter_mut().flat_map(|b| &mut b.ops).find(|op|
                matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, offset: 0 }), .. } if *id == local)).unwrap();
            // Exercise the legal constant form directly: source casts can
            // introduce virtual definitions before the capture reaches MIR.
            let MirOp::Store { src, .. } = store else {
                unreachable!()
            };
            *src = address.clone();
            crate::mir6502::verify_program(&p, MirPhase::PreMaterialization).unwrap();
            let selected = select(&mut p, &Mir6502Config::default());
            assert!(!selected.forwarded.is_empty());
            assert!(
                p.routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .any(|op| matches!(op, MirOp::Move { src, .. } if *src == address)),
                "the forwarded value must retain its relocation identity"
            );
            crate::mir6502::verify_program(&p, MirPhase::PreMaterialization).unwrap();
            crate::mir6502::materialize_program_with_origin(p, &Mir6502Config::default(), 0x3200)
                .unwrap();
        }
    }

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
    fn scalar_captures_promote_matched_scalar_bindings() {
        for optimized in [false, true] {
            let mut program = lower(BINDING, optimized);
            let selected = select(&mut program, &Mir6502Config::default());
            assert!(
                optimized || !selected.removed.is_empty(),
                "unobserved binding home removed"
            );
            crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
        }
    }

    #[test]
    fn scalar_captures_final_cost_never_grows() {
        for source in [ARGUMENTS, BINDING] {
            for optimized in [false, true] {
                let program = lower(source, optimized);
                for runtime in [
                    crate::runtime::Runtime::ActionCart,
                    crate::runtime::Runtime::Standalone,
                ] {
                    let mut off = Mir6502Config::default();
                    off.enable_argument_capture_forwarding = false;
                    off.enable_pattern_binding_forwarding = false;
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
    fn scalar_captures_reject_barriers_aliases_partial_writes_and_changed_values() {
        for reason in [
            "barrier",
            "absolute read",
            "pointer write",
            "partial write",
            "redefinition",
            "alias",
            "long interval",
        ] {
            let mut p = lower(BINDING, false);
            let r = &mut p.routines[0];
            let slot = r
                .frame
                .locals
                .iter()
                .find(|s| s.purpose == MirStoragePurpose::PatternBinding)
                .unwrap()
                .clone();
            let MirStorageBase::Local(id) = slot.base else {
                unreachable!()
            };
            let b = r.blocks.iter_mut().find(|b| b.ops.iter().any(|op| matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id: target, .. }), .. } if *target == id))).unwrap();
            let at = b.ops.iter().position(|op| matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id: target, .. }), .. } if *target == id)).unwrap();
            let MirOp::Store { src, .. } = b.ops[at].clone() else {
                unreachable!()
            };
            match reason {
                "barrier" => b.ops.insert(
                    at + 1,
                    MirOp::Barrier {
                        effects: MirEffects::default(),
                    },
                ),
                "absolute read" => {
                    r.temps.push(MirTemp { id: MirTempId(900) });
                    b.ops.insert(
                        at + 1,
                        MirOp::Load {
                            dst: MirDef::VTemp(MirTempId(900)),
                            src: MirAddr::Direct(MirMem::Absolute(0xD20A)),
                            width: MirWidth::Byte,
                        },
                    );
                }
                "pointer write" => b.ops.insert(
                    at + 1,
                    MirOp::Store {
                        dst: MirAddr::Deref {
                            ptr: MirValue::ConstU16(0x900),
                            offset: 0,
                        },
                        src: MirValue::ConstU8(0),
                        width: MirWidth::Byte,
                    },
                ),
                "partial write" => b.ops.insert(
                    at + 1,
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Local { id, offset: 1 }),
                        src: MirValue::ConstU8(0),
                        width: MirWidth::Byte,
                    },
                ),
                "redefinition" => {
                    let MirValue::Def(dst) = src else {
                        unreachable!()
                    };
                    b.ops.insert(
                        at + 1,
                        MirOp::LoadImm {
                            dst,
                            value: 0,
                            width: MirWidth::Word,
                        },
                    );
                }
                "alias" => r.frame.locals.push(MirStorageSlot {
                    id: MirStorageId(900),
                    purpose: MirStoragePurpose::Storage,
                    base: MirStorageBase::LocalAlias {
                        id: crate::nir::LocalId(900),
                        target: id,
                    },
                    ..slot
                }),
                "long interval" => {
                    for n in 0..33 {
                        let temp = MirTempId(900 + n);
                        r.temps.push(MirTemp { id: temp });
                        b.ops.insert(
                            at + 1,
                            MirOp::LoadImm {
                                dst: MirDef::VTemp(temp),
                                value: 0,
                                width: MirWidth::Byte,
                            },
                        );
                    }
                }
                _ => unreachable!(),
            }
            let selected = select(&mut p, &Mir6502Config::default());
            assert!(
                selected.forwarded.is_empty()
                    && selected.removed.is_empty()
                    && selected.kept.is_empty(),
                "{reason}"
            );
        }
    }

    #[test]
    fn scalar_captures_preserve_exposed_and_successor_observed_binding_stores() {
        for exposure in ["successor", "address", "data"] {
            let mut p = lower(BINDING, false);
            let r = &mut p.routines[0];
            let slot = r
                .frame
                .locals
                .iter()
                .find(|s| s.purpose == MirStoragePurpose::PatternBinding)
                .unwrap();
            let MirStorageBase::Local(id) = slot.base else {
                unreachable!()
            };
            if exposure == "address" {
                r.temps.push(MirTemp { id: MirTempId(900) });
                r.blocks[0].ops.insert(
                    0,
                    MirOp::LeaAddr {
                        dst: MirDef::VTemp(MirTempId(900)),
                        target: MirMem::Local {
                            id: crate::nir::LocalId(0),
                            offset: 0,
                        },
                        width: MirWidth::Word,
                    },
                );
            } else if exposure == "data" {
                p.statics.push(MirStatic {
                    id: crate::nir::SymbolId(900),
                    name: "capture_address".into(),
                    ty: "CARD".into(),
                    display: String::new(),
                    alignment: 1,
                    mutable: false,
                    section: "rodata".into(),
                    image: MirDataImage {
                        bytes: vec![0; 2],
                        relocations: vec![MirDataRelocation {
                            offset: 0,
                            kind: MirDataRelocationKind::Word16,
                            target: MirDataRelocationTarget::Local { routine: r.id, id },
                            addend: 0,
                            span: crate::source::Span::new(0, 0),
                        }],
                    },
                });
            } else {
                let exit = r
                    .blocks
                    .iter_mut()
                    .find(|b| matches!(b.terminator, MirTerminator::Return))
                    .unwrap();
                exit.ops.push(MirOp::Barrier {
                    effects: MirEffects::default(),
                });
            }
            let selected = select(&mut p, &Mir6502Config::default());
            assert!(!selected.forwarded.is_empty());
            assert!(selected.removed.is_empty() && selected.kept.is_empty());
            assert!(p.routines[0].blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id: target, .. }), .. } if *target == id)));
        }
    }

    #[test]
    fn scalar_captures_final_layout_rejects_absolute_and_neighbor_aliases() {
        for neighbor in [false, true] {
            let mut p = lower(BINDING, false);
            let mut off = Mir6502Config::default();
            off.enable_argument_capture_forwarding = false;
            off.enable_pattern_binding_forwarding = false;
            let baseline =
                crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
            let id = p.routines[0]
                .frame
                .locals
                .iter()
                .find_map(|s| match s.base {
                    MirStorageBase::Local(id) if s.purpose == MirStoragePurpose::PatternBinding => {
                        Some(id)
                    }
                    _ => None,
                })
                .unwrap();
            let address = super::super::MaterializeLayout::new(&baseline, 0x3000)
                .mem_address(p.routines[0].id, &MirMem::Local { id, offset: 0 })
                .unwrap();
            if neighbor {
                let r = &p.routines[0];
                let other = r
                    .frame
                    .locals
                    .iter()
                    .find_map(|s| match s.base {
                        MirStorageBase::Local(other) if other != id => Some(other),
                        _ => None,
                    })
                    .unwrap();
                let base = super::super::MaterializeLayout::new(&baseline, 0x3000)
                    .mem_address(
                        r.id,
                        &MirMem::Local {
                            id: other,
                            offset: 0,
                        },
                    )
                    .unwrap();
                let slot = r
                    .frame
                    .locals
                    .iter()
                    .find(|s| s.base == MirStorageBase::Local(id))
                    .unwrap()
                    .clone();
                p.routines[0].frame.locals.push(MirStorageSlot {
                    id: MirStorageId(900),
                    purpose: MirStoragePurpose::Storage,
                    base: MirStorageBase::LocalAlias {
                        id: crate::nir::LocalId(900),
                        target: other,
                    },
                    offset: address.checked_sub(base).unwrap(),
                    ..slot
                });
            } else {
                let mut alias = p
                    .globals
                    .iter()
                    .find(|g| g.storage_size > 0)
                    .unwrap()
                    .clone();
                alias.id = crate::nir::SymbolId(900);
                alias.name = "absolute_observer".into();
                alias.backing = MirGlobalBacking::Absolute(address);
                alias.init = None;
                p.globals.push(alias);
            }
            let before =
                crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
            let after = crate::mir6502::materialize_program_with_origin(
                p,
                &Mir6502Config::default(),
                0x3000,
            )
            .unwrap();
            assert_eq!(
                before, after,
                "declining the candidate restores the entire original program"
            );
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

    #[test]
    fn scalar_capture_store_deletion_requires_every_written_byte_in_bounds() {
        for (offset, expected) in [(0, true), (1, false), (2, false), (u16::MAX, false)] {
            let mut p = lower(BINDING, false);
            let r = &mut p.routines[0];
            let id = r
                .frame
                .locals
                .iter()
                .find_map(|s| match s.base {
                    MirStorageBase::Local(id) if s.purpose == MirStoragePurpose::PatternBinding => {
                        Some(id)
                    }
                    _ => None,
                })
                .unwrap();
            let block = r.blocks.iter_mut().find(|b| b.ops.iter().any(|op| matches!(op,
                MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id: target, .. }), .. } if *target == id))).unwrap();
            let mut source = None;
            for op in &mut block.ops {
                match op {
                    MirOp::Store {
                        dst:
                            MirAddr::Direct(MirMem::Local {
                                id: target,
                                offset: at,
                            }),
                        src,
                        ..
                    } if *target == id => {
                        *at = offset;
                        source = Some(src.clone());
                    }
                    MirOp::Load {
                        dst,
                        src: MirAddr::Direct(MirMem::Local { id: target, .. }),
                        width,
                    } if *target == id => {
                        *op = MirOp::Move {
                            dst: dst.clone(),
                            src: source.clone().unwrap(),
                            width: *width,
                        };
                    }
                    _ => {}
                }
            }
            assert_eq!(
                private_cells::home_has_only_captured_stores(&p.routines[0], id),
                expected
            );
            assert_eq!(
                private_cells::scalar_home_unobserved(&p, &p.routines[0], id),
                expected
            );
        }
    }
}
