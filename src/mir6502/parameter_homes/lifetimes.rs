//! Project logical parameter lifetimes onto their selected bytes. A physical
//! scratch/result write is a possible clobber, never a definition of the
//! parameter. This distinction prevents a clobber from killing its own proof.
use super::*;
use crate::mir6502::analysis::{
    cfg::MirCfg,
    effects::{MirHomeByte, MirMemoryEffects, classify_op, classify_terminator},
    home_liveness::{MirHomeBlockTransfers, MirHomeLiveness, MirHomeTransfer},
    sites::MirSite,
};
use std::collections::BTreeMap;

pub(in crate::mir6502) fn verify(routine: &MirRoutine) -> Vec<MirDiagnostic> {
    let Some(bindings) = &routine.frame.parameter_homes else {
        return vec![];
    };
    let mut lanes = BTreeMap::new();
    for binding in bindings {
        if let MirParamHome::ZeroPage(start) = binding.home {
            let Some(slot) = routine
                .frame
                .params
                .iter()
                .find(|p| p.base == MirStorageBase::Param(binding.param))
            else {
                continue;
            };
            for offset in 0..slot.storage_size {
                lanes.insert(
                    (binding.param, offset),
                    MirFixedZpSlot(start.0.wrapping_add(offset as u8)),
                );
            }
        }
    }
    if lanes.is_empty() {
        return vec![];
    }
    let owned: BTreeSet<_> = lanes
        .values()
        .copied()
        .map(MirHomeByte::FixedZeroPage)
        .collect();
    let physical = |mem: &MirMem, offset: u16| match mem {
        MirMem::FixedZeroPage(slot) => Some(MirFixedZpSlot(slot.0.wrapping_add(offset as u8))),
        MirMem::Absolute(address) if address.wrapping_add(offset) < 256 => {
            Some(MirFixedZpSlot(address.wrapping_add(offset) as u8))
        }
        _ => None,
    };
    let transfer = |memory: &MirMemoryEffects, extra_reads: &BTreeSet<MirHomeByte>| {
        let mut result = MirHomeTransfer::default();
        for range in &memory.direct_reads {
            for offset in 0..range.bytes {
                let slot = match range.base {
                    MirMem::Param { id, offset: base } => {
                        lanes.get(&(id, base.wrapping_add(offset))).copied()
                    }
                    _ => physical(&range.base, offset),
                };
                if let Some(slot) = slot {
                    let home = MirHomeByte::FixedZeroPage(slot);
                    if owned.contains(&home) {
                        result.reads.insert(home);
                    }
                }
            }
        }
        result.reads.extend(extra_reads.intersection(&owned));
        for range in &memory.direct_writes {
            if let MirMem::Param { id, offset: base } = range.base {
                for offset in 0..range.bytes {
                    if let Some(slot) = lanes.get(&(id, base.wrapping_add(offset))) {
                        result.writes.insert(MirHomeByte::FixedZeroPage(*slot));
                    }
                }
            }
        }
        result
    };
    let mut transfers = BTreeMap::new();
    let mut clobbers = Vec::new();
    for block in &routine.blocks {
        let mut ops = Vec::new();
        for (op_index, op) in block.ops.iter().enumerate() {
            if matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) {
                return vec![MirDiagnostic::routine(
                    &routine.name,
                    "parameter residence acquired an unsupported expanded barrier",
                )];
            }
            let e = classify_op(op);
            if e.memory.opaque || e.memory.indirect_writes {
                return vec![MirDiagnostic::routine(
                    &routine.name,
                    "parameter residence acquired unsupported expanded memory effects",
                )];
            }
            let reads = e
                .homes
                .reads
                .union(&e.addresses.pair_reads)
                .copied()
                .collect();
            ops.push(transfer(&e.memory, &reads));
            let mut writes: BTreeSet<_> = e
                .homes
                .writes
                .union(&e.addresses.pair_writes)
                .copied()
                .collect();
            for range in &e.memory.direct_writes {
                for offset in 0..range.bytes {
                    if let Some(slot) = physical(&range.base, offset) {
                        writes.insert(MirHomeByte::FixedZeroPage(slot));
                    }
                }
            }
            for home in writes.intersection(&owned) {
                clobbers.push((
                    MirSite::Op {
                        block: block.id,
                        op_index,
                    },
                    *home,
                ));
            }
        }
        let e = classify_terminator(&block.terminator);
        transfers.insert(
            block.id,
            MirHomeBlockTransfers {
                ops,
                terminator: transfer(&e.memory, &BTreeSet::new()),
            },
        );
    }
    let Ok(cfg) = MirCfg::from_routine(routine) else {
        return vec![MirDiagnostic::routine(
            &routine.name,
            "parameter residence requires a closed CFG",
        )];
    };
    let liveness = MirHomeLiveness::analyze_transfers(routine, &cfg, transfers);
    clobbers
        .into_iter()
        .filter_map(|(site, home)| {
            (!matches!(liveness.live_after(home, site), Ok(false))).then(|| {
                MirDiagnostic::routine(
                    &routine.name,
                    format!("physical write clobbers live parameter home {home:?} at {site:?}"),
                )
            })
        })
        .collect()
}
