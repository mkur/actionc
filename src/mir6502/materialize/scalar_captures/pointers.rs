//! An independent, store-only transaction using bounded pointer footprints.
//! Allocations survive; an unsuccessful proposal keeps prior scalar savings.
use super::*;
use crate::mir6502::analysis::bounded_pointers::{self, Object, Summary};

pub(super) fn improve(
    selected: &mut MirProgram,
    emitted: &mut MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
) -> super::super::MirPeepholeStats {
    let mut stats = super::super::MirPeepholeStats::default();
    let mut forwarded = selected.clone();
    let counts = forward_loads(&mut forwarded, config);
    let mut proposal = selected.clone();
    let mut candidates = BTreeMap::<RoutineId, Vec<MirMem>>::new();
    for (index, r) in forwarded.routines.iter().enumerate() {
        let ids: BTreeMap<_, _> = r
            .frame
            .locals
            .iter()
            .filter_map(|s| {
                let MirStorageBase::Local(id) = s.base else {
                    return None;
                };
                (enabled(s, config)
                    && s.init.is_none()
                    && s.offset == 0
                    && private_cells::home_has_only_captured_stores(r, id))
                .then_some((id, s.storage_size))
            })
            .collect();
        if ids.is_empty() {
            continue;
        }
        // Prove one routine against the accepted callees. Rejected tentative
        // rewrites of another routine must not become assumptions here.
        let mut trial = selected.clone();
        trial.routines[index] = r.clone();
        for b in &mut trial.routines[index].blocks {
            b.ops.retain(|op| !matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, .. }), .. } if ids.contains_key(id)));
        }
        let cells: Vec<_> = ids
            .into_iter()
            .flat_map(|(id, n)| (0..n).map(move |offset| MirMem::Local { id, offset }))
            .collect();
        let Some(summary) = bounded_pointers::summarize(&trial, &trial.routines[index]) else {
            continue;
        };
        if !disjoint(&summary, r.id, &cells) {
            continue;
        }
        proposal.routines[index] = trial.routines[index].clone();
        candidates.insert(r.id, cells);
    }
    if candidates.is_empty() {
        return stats;
    }
    // Recheck the combined proposal as well; the facts belong to this exact
    // program, including the complete bodies of any changed callees.
    let summaries: Option<BTreeMap<_, _>> = candidates
        .iter()
        .map(|(&id, cells)| {
            let r = proposal.routines.iter().find(|r| r.id == id)?;
            let summary = bounded_pointers::summarize(&proposal, r)?;
            disjoint(&summary, id, cells).then_some((id, summary))
        })
        .collect();
    let materialized = summaries.as_ref().and_then(|_| {
        crate::mir6502::verify_program(&proposal, MirPhase::PreMaterialization).ok()?;
        crate::mir6502::materialize_resolved_program_without_scalar_costing(
            proposal.clone(),
            config,
            origin,
            runtime,
            false,
        )
        .ok()
    });
    let accepted = materialized.as_ref().is_some_and(|new| {
        Cost::measure(emitted, origin)
            .zip(Cost::measure(new, origin))
            .is_some_and(|(old, new)| {
                new.bytes < old.bytes && candidates.keys().all(|&id| new.does_not_grow(&old, id))
            })
            && occupied(new).is_subset(&occupied(emitted))
            && physical(emitted, origin, &candidates, summaries.as_ref().unwrap())
            && physical(new, origin, &candidates, summaries.as_ref().unwrap())
    });
    for (&id, cells) in &candidates {
        if accepted {
            stats.record_many(
                id,
                "scalar-capture-load-forwarded",
                counts.get(&id).copied().unwrap_or(0),
            );
        }
        stats.record_many(
            id,
            if accepted {
                "scalar-capture-pointer-stored-bytes-removed"
            } else {
                "scalar-capture-pointer-declined-cost-or-alias"
            },
            cells.len(),
        );
    }
    if accepted {
        *selected = proposal;
        *emitted = materialized.unwrap();
    }
    stats
}

fn disjoint(summary: &Summary, owner: RoutineId, cells: &[MirMem]) -> bool {
    summary.reads.iter().chain(&summary.writes).all(|a| {
        !cells.iter().any(|mem| {
            matches!(mem, MirMem::Local { id, offset }
            if a.pointer.object == Object::Local(owner, *id)
                && offset.checked_sub(a.pointer.offset).is_some_and(|n| n < a.bytes))
        })
    })
}

fn physical(
    p: &MirProgram,
    origin: u16,
    candidates: &BTreeMap<RoutineId, Vec<MirMem>>,
    summaries: &BTreeMap<RoutineId, Summary>,
) -> bool {
    let mut private = candidates.clone();
    for s in summaries.values() {
        for &object in &s.private_homes {
            private
                .entry(object.owner())
                .or_default()
                .extend((0..2).map(|n| object.memory(n)));
        }
    }
    if !private_cells::physical_cells_unaliased(p, origin, &private) {
        return false;
    }
    let layout = super::super::MaterializeLayout::new(p, origin);
    if private.iter().any(|(&id, cells)| {
        cells.iter().any(|mem| {
            layout
                .mem_address(id, mem)
                .is_none_or(|a| (0xD000..0xD800).contains(&a))
        })
    }) {
        return false;
    }
    let addresses: Option<Vec<_>> = candidates
        .iter()
        .flat_map(|(&id, cells)| {
            let layout = &layout;
            cells
                .iter()
                .map(move |mem| layout.mem_address(id, mem).map(u32::from))
        })
        .collect();
    let Some(addresses) = addresses else {
        return false;
    };
    summaries.values().all(|s| {
        s.borrowed.iter().all(|(&object, &extent)| {
            let Some(start) = layout
                .mem_address(object.owner(), &object.memory(0))
                .map(u32::from)
            else {
                return false;
            };
            let end = start + u32::from(extent);
            // Named objects must still have non-wrapping physical RAM extents.
            // The Atari hardware window cannot supply ordinary object semantics.
            start >= 0x100
                && end <= 0x10000
                && !(start < 0xD800 && end > 0xD000)
                && addresses.iter().all(|&a| a < start || a >= end)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::lower;
    use super::*;

    pub(super) const SOURCE: &str = "TYPE Data=[BYTE n BYTE b]\nData result\nData FUNC Make(BYTE ARRAY p)\nData item\nitem.n=p(0)\nitem.b=p(1)\nRETURN(item)\nPROC Run()\nBYTE ARRAY local(4)\nlocal(0)=17\nlocal(1)=29\nresult=Make(local)\nRETURN\nPROC Main()\nRun()\nRETURN\n";

    #[test]
    fn bounded_reads_enable_costed_store_removal_with_retained_allocations() {
        for optimized in [false, true] {
            let p = lower(SOURCE, optimized);
            let mut config = Mir6502Config::default();
            config.enable_scalar_capture_pointer_elision = false;
            let old = crate::mir6502::materialize_program_with_origin(p.clone(), &config, 0x3000)
                .unwrap();
            config.enable_scalar_capture_pointer_elision = true;
            let new = crate::mir6502::materialize_program_with_origin(p, &config, 0x3000).unwrap();
            assert!(
                Cost::measure(&new, 0x3000).unwrap().bytes
                    <= Cost::measure(&old, 0x3000).unwrap().bytes,
                "optimized={optimized}"
            );
            assert!(occupied(&new).is_subset(&occupied(&old)));
            if optimized {
                assert!(
                    Cost::measure(&new, 0x3000).unwrap().bytes
                        < Cost::measure(&old, 0x3000).unwrap().bytes
                );
            } else {
                // This raw-NIR proposal saves code but needs an additional
                // scratch byte. The independent transaction must decline it.
                assert_eq!(old, new);
            }
            let old = old.routines.iter().find(|r| r.name == "Run").unwrap();
            let new = new.routines.iter().find(|r| r.name == "Run").unwrap();
            let old_slots: Vec<_> = old
                .frame
                .locals
                .iter()
                .map(|s| (&s.base, s.storage_size, s.purpose))
                .collect();
            let new_slots: Vec<_> = new
                .frame
                .locals
                .iter()
                .map(|s| (&s.base, s.storage_size, s.purpose))
                .collect();
            assert_eq!(old_slots, new_slots, "capture allocations remain");
        }
    }
}

#[cfg(test)]
mod negative_tests {
    use super::super::tests::lower;

    #[test]
    fn bounded_capture_physical_proof_rejects_hardware_placements() {
        let p = lower(tests::SOURCE, true);
        let mut off = Mir6502Config::default();
        off.enable_scalar_capture_pointer_elision = false;
        let old = crate::mir6502::materialize_program_with_origin(p, &off, 0x3000).unwrap();
        let r = old.routines.iter().find(|r| r.name == "Run").unwrap();
        let cells = BTreeMap::from([(
            r.id,
            vec![MirMem::Local {
                id: crate::nir::LocalId(3),
                offset: 0,
            }],
        )]);
        assert!(physical(&old, 0x3000, &cells, &BTreeMap::new()));
        assert!(!physical(&old, 0xD000, &cells, &BTreeMap::new()));
    }
    use super::*;

    #[test]
    fn bounded_capture_cleanup_preserves_unknown_or_escaping_observations() {
        for source in [
            tests::SOURCE.replace("item.b=p(1)", "item.b=p(4)"),
            tests::SOURCE
                .replace("Data result", "Data result\nCARD saved")
                .replace("item.n=p(0)", "saved=CARD(p)\nitem.n=p(0)"),
        ] {
            let p = lower(&source, true);
            let mut off = Mir6502Config::default();
            off.enable_scalar_capture_pointer_elision = false;
            let old =
                crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
            let new = crate::mir6502::materialize_program_with_origin(
                p,
                &Mir6502Config::default(),
                0x3000,
            )
            .unwrap();
            assert_eq!(old, new);
        }
    }

    #[test]
    fn bounded_capture_cleanup_checks_capture_and_pointer_home_aliases_in_both_layouts() {
        let p = lower(tests::SOURCE, true);
        let mut off = Mir6502Config::default();
        off.enable_scalar_capture_pointer_elision = false;
        let old = crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
        let new = crate::mir6502::materialize_program_with_origin(
            p.clone(),
            &Mir6502Config::default(),
            0x3000,
        )
        .unwrap();
        assert!(
            Cost::measure(&new, 0x3000).unwrap().bytes < Cost::measure(&old, 0x3000).unwrap().bytes
        );
        for layout_program in [&old, &new] {
            let layout = super::super::super::MaterializeLayout::new(layout_program, 0x3000);
            let run = layout_program
                .routines
                .iter()
                .find(|r| r.name == "Run")
                .unwrap();
            let make = layout_program
                .routines
                .iter()
                .find(|r| r.name == "Make")
                .unwrap();
            let addresses = [
                layout
                    .mem_address(
                        run.id,
                        &MirMem::Local {
                            id: crate::nir::LocalId(3),
                            offset: 0,
                        },
                    )
                    .unwrap(),
                layout
                    .mem_address(
                        run.id,
                        &MirMem::Local {
                            id: crate::nir::LocalId(1),
                            offset: 0,
                        },
                    )
                    .unwrap(),
                layout
                    .mem_address(
                        make.id,
                        &MirMem::Param {
                            id: crate::nir::ParamId(0),
                            offset: 0,
                        },
                    )
                    .unwrap(),
            ];
            for address in addresses {
                let mut aliased = p.clone();
                let mut alias = aliased.globals[0].clone();
                alias.id = crate::nir::SymbolId(900);
                alias.name = "observer".into();
                alias.backing = MirGlobalBacking::Absolute(address);
                alias.storage_size = 1;
                alias.width = Some(MirWidth::Byte);
                alias.init = None;
                aliased.globals.push(alias);
                let before =
                    crate::mir6502::materialize_program_with_origin(aliased.clone(), &off, 0x3000)
                        .unwrap();
                let after = crate::mir6502::materialize_program_with_origin(
                    aliased,
                    &Mir6502Config::default(),
                    0x3000,
                )
                .unwrap();
                assert_eq!(before, after, "alias at {address:04X}");
            }
        }
    }
}
