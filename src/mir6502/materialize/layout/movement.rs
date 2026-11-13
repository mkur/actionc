//! Diagnostic comparison of a declaration-only spill-pruning proposal.
//! No result here grants movement permission or describes final physical layout.
use super::*;
use crate::mir6502::trace::homes::{self, Home};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const MAX_CHANGES: usize = 2048;

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Placement {
    address: u16,
    bytes: u16,
    scalar_width: Option<&'static str>,
    descriptor: bool,
}

impl MaterializeLayout {
    fn frame_placements(&self) -> BTreeMap<Home, Placement> {
        let mut homes = BTreeMap::new();
        for (owner, frame) in &self.routine_storage {
            let mut add = |mem, address, bytes, width: Option<MirWidth>, descriptor| {
                homes.insert(
                    homes::memory(*owner, &mem),
                    Placement {
                        address,
                        bytes,
                        scalar_width: width.map(|w| match w {
                            MirWidth::Byte => "byte",
                            MirWidth::Word => "word",
                        }),
                        descriptor,
                    },
                );
            };
            for &(id, address, bytes, width) in &frame.params {
                add(
                    MirMem::Param { id, offset: 0 },
                    address,
                    bytes,
                    width,
                    frame.descriptor_params.contains(&id),
                );
            }
            for &(id, address, bytes, width) in &frame.locals {
                add(
                    MirMem::Local { id, offset: 0 },
                    address,
                    bytes,
                    width,
                    frame.descriptor_locals.contains(&id),
                );
            }
            for &(id, address, bytes) in &frame.spills {
                add(MirMem::Spill { id, offset: 0 }, address, bytes, None, false);
            }
        }
        homes
    }

    pub(in crate::mir6502) fn spill_pruning_movement(
        &self,
        candidate: &Self,
        removed: &BTreeMap<RoutineId, BTreeSet<MirSpillId>>,
    ) -> Value {
        let old = self.frame_placements();
        let new = candidate.frame_placements();
        let keys: BTreeSet<_> = old.keys().chain(new.keys()).collect();
        let mut changes = Vec::new();
        let mut total = 0;
        let mut moved = 0;
        let mut erased = 0;
        let mut routines = BTreeSet::new();
        for home in keys {
            let before = old.get(home);
            let after = new.get(home);
            if before == after {
                continue;
            }
            total += 1;
            routines.insert(home.routine);
            let expected_removal = home.kind == "spill"
                && after.is_none()
                && home.routine.is_some_and(|r| {
                    removed
                        .get(&RoutineId(r))
                        .is_some_and(|ids| ids.contains(&MirSpillId(home.id)))
                });
            erased += usize::from(expected_removal);
            moved += usize::from(
                before
                    .zip(after)
                    .is_some_and(|(a, b)| a.address != b.address),
            );
            if changes.len() < MAX_CHANGES {
                changes.push(json!({"home":home,"before":before,"after":after,
                    "expected_removal":expected_removal}));
            }
        }
        let mut bounds = self.ordinary_bounds.clone();
        bounds.remove_spills(removed.values().map(|ids| ids.len()).sum());
        json!({
            "scope":"whole-program planning frame entries, including alias views; independent of routine filters",
            "status":if total == changes.len() { "complete" } else { "truncated" },
            "changes":changes,"total_changes":total,"omitted":total - changes.len(),
            "moved_survivors":moved,"expected_removals":erased,"affected_routines":routines,
            "non_frame_facts_preserved":self.origin == candidate.origin && self.globals == candidate.globals
                && self.statics == candidate.statics && bounds == candidate.ordinary_bounds,
            "interpretation":"Declaration-only proposed layout. No final physical addresses, predicate replay, selected-decision certificate or compaction permission."
        })
    }
}
