//! Inspect already selected instructions before committing the whole batch.
//! Retained and expanded sites belong to different CFGs and have separate liveness.
use super::*;
use crate::mir6502::analysis::effects::{MirHomeByte, classify_op, writes_ordinary_pointee};

pub(super) struct ExpandedConflicts<'a> {
    logical: logical::LogicalHomes<'a>,
    sites: Vec<(MirSite, Result<BTreeSet<u8>, &'static str>)>,
    entry: BTreeSet<u8>,
}

impl<'a> ExpandedConflicts<'a> {
    pub fn new(
        routine: &'a MirRoutine,
        facts: &mut ResidenceCalls<'_>,
        pool_writes: impl Fn(&MirOp, MirKnownMemoryWrites) -> Result<BTreeSet<u8>, &'static str>,
    ) -> Result<Self, &'static str> {
        let logical = logical::LogicalHomes::new(routine).ok_or("missing expanded lifetime")?;
        let mut sites = Vec::new();
        let mut entry = BTreeSet::new();
        for block in &routine.blocks {
            for (op_index, op) in block.ops.iter().enumerate() {
                let site = MirSite::Op {
                    block: block.id,
                    op_index,
                };
                let mut bytes = BTreeSet::new();
                if matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. }) {
                    entry.extend(facts.inputs_at(routine, site, op)?);
                    let writes = facts.home_writes_at(routine, site, op)
                        .and_then(|writes| pool_writes(op, writes));
                    sites.push((site, writes));
                    continue;
                }
                if matches!(op, MirOp::SArgsCapture { .. }) {
                    let capture = facts
                        .sargs_capture(routine, op)
                        .ok_or("missing SArgs contract")?;
                    entry.extend(
                        capture
                            .scratch
                            .iter()
                            .filter_map(|m| logical::physical_slot(m, 0).map(|s| s.0)),
                    );
                    continue;
                }
                let effects = classify_op(op);
                if effects.memory.opaque
                    || (effects.memory.indirect_writes && !writes_ordinary_pointee(op))
                {
                    return Err("unsupported expanded memory effects");
                }
                for range in effects
                    .memory
                    .direct_writes
                    .iter()
                    .chain(&effects.memory.direct_reads)
                {
                    bytes.extend(
                        (0..range.bytes).filter_map(|i| {
                            logical::physical_slot(&range.base, i).map(|slot| slot.0)
                        }),
                    );
                }
                for home in effects
                    .homes
                    .writes
                    .iter()
                    .chain(&effects.addresses.pair_writes)
                {
                    if let MirHomeByte::FixedZeroPage(slot) = home {
                        bytes.insert(slot.0);
                    }
                }
                if !bytes.is_empty() {
                    sites.push((site, Ok(bytes)));
                }
            }
        }
        Ok(Self {
            logical,
            sites,
            entry,
        })
    }

    pub fn exclude(
        &self,
        object: &MirMem,
        width: u16,
        forbidden: &mut BTreeSet<u8>,
    ) -> Result<(), (MirSite, &'static str)> {
        forbidden.extend(&self.entry);
        for (site, writes) in &self.sites {
            if (0..width).any(|offset| {
                let lane = match *object {
                    MirMem::Param { id, .. } => MirMem::Param { id, offset },
                    MirMem::Local { id, .. } => MirMem::Local { id, offset },
                    _ => return true,
                };
                self.logical.live_after(&lane, *site) != Some(false)
            }) {
                forbidden.extend(writes.as_ref().map_err(|reason| (*site, *reason))?);
            }
        }
        Ok(())
    }
}
