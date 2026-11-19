//! Dedicated local homes keep their value even while their owner is inactive.
use super::*;
use crate::mir6502::analysis::{
    cfg::MirCfg,
    effects::{MirHomeByte, classify_op, classify_terminator},
};

fn seed(slot: &MirStorageSlot) -> Option<Vec<u8>> {
    let size = usize::from(slot.storage_size);
    let mut bytes = match &slot.init {
        None | Some(MirStorageInit::ZeroFill { .. }) => Vec::new(),
        Some(MirStorageInit::Bytes { image, .. }) if image.relocations.is_empty() => {
            image.bytes.clone()
        }
        _ => return None,
    };
    if bytes.len() > size || !(1..=2).contains(&size) {
        return None;
    }
    bytes.resize(size, 0);
    Some(bytes)
}

fn dedicated(r: &MirRoutine) -> impl Iterator<Item = (&MirStorageSlot, u8)> {
    r.frame.locals.iter().filter_map(|s| {
        let MirStorageBase::Local(id) = s.base else {
            return None;
        };
        match r.frame.local_home(id)? {
            MirLocalHome::PersistentZeroPage(slot) => Some((s, slot.0)),
            _ => None,
        }
    })
}

pub(in crate::mir6502) fn slots(program: &MirProgram) -> impl Iterator<Item = u8> + '_ {
    program
        .routines
        .iter()
        .flat_map(dedicated)
        .flat_map(|(s, start)| {
            (0..s.storage_size)
                .filter_map(move |i| u8::try_from(u16::from(start).checked_add(i)?).ok())
        })
}

pub(in crate::mir6502) fn initializers(
    program: &MirProgram,
) -> impl Iterator<Item = (u8, u8)> + '_ {
    program
        .routines
        .iter()
        .flat_map(dedicated)
        .flat_map(|(slot, start)| {
            seed(slot)
                .expect("verified persistent local initializer")
                .into_iter()
                .enumerate()
                .map(move |(i, value)| (start + i as u8, value))
        })
}

// The ordinary ABI offers no guarantee while execution is outside the image.
// A closed native program that remains in its entry loop has no such interval.
fn closed(program: &MirProgram) -> bool {
    let Some(main) = program
        .routines
        .iter()
        .find(|r| r.abi == MirRoutineAbi::ProgramEntry)
    else {
        return false;
    };
    if !main.frame.params.is_empty()
        || !MirCfg::from_routine(main).is_ok_and(|cfg| cfg.entry().is_some() && cfg.exits().is_empty())
    {
        return false;
    }
    program.routines.iter().all(|r| {
        matches!(r.abi, MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry)
            && r.blocks.iter().flat_map(|b| &b.ops).all(|op| match op {
                MirOp::Call {
                    target: MirCallTarget::Routine(id),
                    ..
                } => *id != main.id && program.routines.iter().any(|r| r.id == *id),
                MirOp::Call { .. }
                | MirOp::RuntimeHelper { .. }
                | MirOp::SArgsCapture { .. }
                | MirOp::MachineBlock { .. }
                | MirOp::Barrier { .. } => false,
                _ => {
                    let e = classify_op(op);
                    !e.memory.opaque
                        && !e.memory.indirect_reads
                        && !e.memory.indirect_writes
                        && !e.homes.unknown_reads
                        && !e.homes.unknown_writes
                }
            })
    })
}

// Uses the actual representation, including compiler scratch and pre-existing
// invocation homes in *every* routine. A private local's named writes are its own.
fn unavailable(program: &MirProgram) -> Option<BTreeSet<u8>> {
    let mut used = BTreeSet::new();
    for (start, size) in physical::numeric_observations(program, None).ok()? {
        used.extend(SARGS_HOME_POOL.filter(|&byte| u16::from(byte).wrapping_sub(start) < size));
    }
    for r in &program.routines {
        used.extend(source_zero_page(program, r));
        let own: BTreeSet<_> = dedicated(r)
            .flat_map(|(s, start)| (0..s.storage_size).map(move |i| start.wrapping_add(i as u8)))
            .collect();
        used.extend(
            r.frame
                .fixed_zero_page
                .iter()
                .map(|s| s.0)
                .filter(|s| !own.contains(s)),
        );
        for (m, slot) in r.frame.resident_bytes() {
            if !matches!(m, MirMem::Local { id, .. } if matches!(r.frame.local_home(id), Some(MirLocalHome::PersistentZeroPage(_))))
            {
                used.insert(slot.0);
            }
        }
        for allocation in &r.frame.zero_page_allocations {
            used.extend((0..allocation.size).map(|i| allocation.start.0.wrapping_add(i as u8)));
        }
        // Large captures and machine entries were excluded by the closed-program check.
        for block in &r.blocks {
            for op in &block.ops {
                if let MirOp::Call { args, .. } = op {
                    for argument in args {
                        used.extend(
                            super::super::abi::physical_lanes(&argument.home, argument.width)?
                                .into_iter()
                                .filter_map(|lane| match lane {
                                    MirParamLane::ZeroPage(slot) => Some(slot.0),
                                    _ => None,
                                }),
                        );
                    }
                }
            }
            let effects = block.ops.iter().map(|op| {
                let e = classify_op(op);
                (e.memory, e.homes, Some(e.addresses))
            });
            let end = classify_terminator(&block.terminator);
            for (memory, homes, addresses) in
                effects.chain(std::iter::once((end.memory, end.homes, None)))
            {
                for range in memory.direct_reads.iter().chain(&memory.direct_writes) {
                    used.extend(
                        (0..range.bytes)
                            .filter_map(|i| logical::physical_slot(&range.base, i).map(|s| s.0)),
                    );
                }
                let pairs = addresses
                    .iter()
                    .flat_map(|a| a.pair_reads.iter().chain(&a.pair_writes));
                for lane in homes.reads.iter().chain(&homes.writes).chain(pairs) {
                    if let MirHomeByte::FixedZeroPage(slot) = lane {
                        used.insert(slot.0);
                    }
                }
            }
        }
    }
    Some(used)
}

pub(super) fn propose(
    input: &MirProgram,
    selected: &MirProgram,
    addresses: &ObjectAddressControl<'_>,
    proposal: &mut Option<MirProgram>,
    trace: &super::super::trace::Trace,
) {
    if !closed(selected) || !closed(input) {
        return;
    }
    let current = proposal.as_ref().unwrap_or(input);
    let Some(mut used) = unavailable(current) else {
        return;
    };
    let Some(physical) = unavailable(selected) else {
        return;
    };
    used.extend(physical);
    used.extend(slots(current));
    let mut additions = Vec::new();
    for r in &current.routines {
        if r.frame.locals.is_empty() {
            continue;
        }
        let Some(entry) = locals::entry_dependencies(r) else {
            continue;
        };
        for s in &r.frame.locals {
            let MirStorageBase::Local(id) = s.base else {
                continue;
            };
            if !r.frame.inline_local(id)
                || !locals::ordinary(addresses, r, s)
                || seed(s).is_none()
                || (s.init.is_none() && !entry.contains(&id))
            {
                continue;
            }
            let width = s.storage_size as u8;
            if let Some(start) = SARGS_HOME_POOL.rev().find(|&start| {
                start + width - 1 <= *SARGS_HOME_POOL.end()
                    && (start..start + width).all(|byte| !used.contains(&byte))
            }) {
                used.extend(start..start + width);
                additions.push((r.id, id, start, width));
            }
        }
    }
    if additions.is_empty() {
        return;
    }
    let p = proposal.get_or_insert_with(|| input.clone());
    for (owner, id, start, width) in additions {
        let r = p.routines.iter_mut().find(|r| r.id == owner).unwrap();
        let bindings = r.frame.local_homes.get_or_insert_with(|| {
            r.frame
                .locals
                .iter()
                .filter_map(|s| match s.base {
                    MirStorageBase::Local(local) => Some(MirLocalBinding {
                        local,
                        home: MirLocalHome::Inline,
                    }),
                    _ => None,
                })
                .collect()
        });
        bindings.iter_mut().find(|b| b.local == id).unwrap().home =
            MirLocalHome::PersistentZeroPage(MirFixedZpSlot(start));
        r.frame
            .fixed_zero_page
            .extend((start..start + width).map(MirFixedZpSlot));
        trace.event(
            "persistent-source-home",
            || serde_json::json!({"routine":r.name,"local":id.0,"start":start,"width":width}),
        );
    }
}

pub(in crate::mir6502) fn verify(program: &MirProgram) -> Vec<MirDiagnostic> {
    if !program
        .routines
        .iter()
        .any(|r| dedicated(r).next().is_some())
    {
        return Vec::new();
    }
    let Some(mut used) = closed(program).then(|| unavailable(program)).flatten() else {
        return vec![MirDiagnostic::routine(
            "persistent homes",
            "no whole-program preservation guarantee",
        )];
    };
    for r in &program.routines {
        for (s, start) in dedicated(r) {
            if seed(s).is_none()
                || !(0..s.storage_size).all(|i| {
                    u8::try_from(u16::from(start) + i)
                        .is_ok_and(|byte| SARGS_HOME_POOL.contains(&byte) && used.insert(byte))
                })
            {
                return vec![MirDiagnostic::routine(
                    &r.name,
                    "persistent home has conflicting physical use or unsupported initial contents",
                )];
            }
        }
    }
    Vec::new()
}
