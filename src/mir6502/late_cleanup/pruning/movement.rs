//! One ordinary private byte, accessed only through symbolic byte loads/stores.
//! This is a bounded storage proof, not a replay of diagnostic census events.
use super::*;

pub(super) struct PrivateByteMove {
    owner: RoutineId,
    spill: MirSpillId,
    before: u16,
    after: u16,
}

impl PrivateByteMove {
    pub(super) fn identity(&self) -> (RoutineId, MirSpillId) {
        (self.owner, self.spill)
    }

    pub(super) fn discover(
        program: &MirProgram,
        references: &[&MirProgram],
        old: &MaterializeLayout,
        new: &MaterializeLayout,
        removed: &Removed,
    ) -> Option<Self> {
        let mut movement = None;
        for r in &program.routines {
            for &id in &r.frame.spills {
                if removed.get(&r.id).is_some_and(|ids| ids.contains(&id)) {
                    continue;
                }
                let home = MirMem::Spill { id, offset: 0 };
                let before = old.mem_address(r.id, &home)?;
                let after = new.mem_address(r.id, &home)?;
                if before != after {
                    // More than one moving byte needs a separate range proof.
                    if movement.is_some() || !ordinary_move(before, after) {
                        return None;
                    }
                    movement = Some(Self {
                        owner: r.id,
                        spill: id,
                        before,
                        after,
                    });
                }
            }
        }
        let movement = movement?;
        if !old.preserves_surviving_storage(new, removed, Some(movement.identity())) {
            return None;
        }
        std::iter::once(program)
            .chain(references.iter().copied())
            .all(|p| movement.byte_uses(p))
            .then_some(movement)
    }

    fn byte_uses(&self, program: &MirProgram) -> bool {
        let home = MirMem::Spill {
            id: self.spill,
            offset: 0,
        };
        if StorageOwnership::new(program).role(self.owner, &home) != StorageRole::Temporary {
            return false;
        }
        for r in &program.routines {
            // An unresolved machine name can hide an address-taking use.
            if unresolved_machine_storage(program, r) {
                return false;
            }
            for block in &r.blocks {
                let end = classify_terminator(&block.terminator);
                if r.id == self.owner && end.projected_spill_reads.contains(&self.spill) {
                    return false;
                }
                for op in &block.ops {
                    let e = classify_op(op);
                    // A wider/offset private access could cover another cell
                    // without naming it. This first slice admits no such views.
                    if e.memory.direct_reads.iter().chain(&e.memory.direct_writes).any(|range| {
                        matches!(range.base, MirMem::Spill { offset, .. } if offset != 0 || range.bytes != 1)
                    }) {
                        return false;
                    }
                    if r.id == self.owner
                        && (e.projected_spill_reads.contains(&self.spill)
                            || e.projected_spill_writes.contains(&self.spill)
                            || e.memory.direct_references.iter().any(
                                |m| matches!(m, MirMem::Spill { id, .. } if *id == self.spill),
                            ))
                    {
                        let byte_access = matches!(op,
                            MirOp::Load { dst: MirDef::Reg(_), src: MirAddr::Direct(mem), width: MirWidth::Byte } if *mem == home)
                            || matches!(op,
                            MirOp::Store { dst: MirAddr::Direct(mem), src: MirValue::Def(MirDef::Reg(_)), width: MirWidth::Byte } if *mem == home);
                        if !byte_access {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    pub(super) fn baseline_safe(
        &self,
        program: &MirProgram,
        references: &[&MirProgram],
        old: &Measurement,
    ) -> bool {
        let Some(address) = old
            .storage_layout
            .as_ref()
            .and_then(|p| p.spill_address(self.owner, self.spill))
        else {
            return false;
        };
        // Protect both planning coordinates as well as actual storage. Fixed
        // source/ABI aliases must not change any earlier separation decision.
        std::iter::once(program)
            .chain(references.iter().copied())
            .all(|p| unobserved_bytes(p, old, &[self.before, self.after, address]))
    }

    pub(super) fn candidate_safe(
        &self,
        program: &MirProgram,
        references: &[&MirProgram],
        old: &Measurement,
        new: &Measurement,
    ) -> bool {
        let Some((before, after)) = old
            .storage_layout
            .as_ref()
            .and_then(|p| p.spill_address(self.owner, self.spill))
            .zip(
                new.storage_layout
                    .as_ref()
                    .and_then(|p| p.spill_address(self.owner, self.spill)),
            )
        else {
            return false;
        };
        ordinary_move(before, after)
            && std::iter::once(program)
                .chain(references.iter().copied())
                .all(|p| unobserved_bytes(p, new, &[after]))
    }
}

fn ordinary_move(before: u16, after: u16) -> bool {
    (0x400..0x8000).contains(&before) && (0x400..before).contains(&after)
}
