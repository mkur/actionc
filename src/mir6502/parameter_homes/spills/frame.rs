//! Narrow exception to the physical frame's non-overlap invariant.
use super::super::logical::ObjectByte;
use super::*;

pub(in crate::mir6502) fn shared_frame_bytes(r: &MirRoutine) -> BTreeSet<u8> {
    let candidates: BTreeSet<_> = r
        .frame
        .zero_page_allocations
        .iter()
        .filter(|a| {
            r.frame.spill_zp_slots.contains(&a.slot)
                && SARGS_HOME_POOL.contains(&a.start.0)
                && (r.frame.fixed_zero_page.contains(&a.start)
                    || r.frame.zero_page_allocations.iter().any(|other| {
                        other.slot != a.slot
                            && a.start
                                .0
                                .checked_sub(other.start.0)
                                .is_some_and(|offset| offset < other.size)
                    }))
        })
        .map(|a| a.start.0)
        .collect();
    if candidates.is_empty() {
        return candidates;
    }
    let Ok(cfg) = MirCfg::from_routine(r) else {
        return BTreeSet::new();
    };
    let Some(lives) = interference::FrameLives::new(r, &cfg) else {
        return BTreeSet::new();
    };
    let Some(entry) = cfg.entry().and_then(|id| lives.temps.live_in(id)) else {
        return BTreeSet::new();
    };
    candidates
        .into_iter()
        .filter(|&address| shared_frame_byte(r, address, &lives, entry))
        .collect()
}

fn shared_frame_byte(
    r: &MirRoutine,
    address: u8,
    lives: &interference::FrameLives,
    entry: &crate::mir6502::analysis::home_liveness::MirHomeLiveSet,
) -> bool {
    let fixed = MirFixedZpSlot(address);
    let mut named = Vec::new();
    for (mem, slot) in r.frame.resident_bytes() {
        if slot == fixed {
            if matches!(mem, MirMem::Local { id, .. }
                if matches!(r.frame.local_home(id), Some(MirLocalHome::PersistentZeroPage(_))))
            {
                return false;
            }
            let Some(byte) = ObjectByte::from_mem(&mem, 0) else {
                return false;
            };
            named.push(Byte::Source(byte));
        }
    }
    if r.frame.fixed_zero_page.contains(&fixed)
        && (named.is_empty() || !r.frame.named_zp_reservations.contains(&fixed))
    {
        return false;
    }
    let mut spills = Vec::new();
    for a in r.frame.zero_page_allocations.iter().filter(|a| {
        address
            .checked_sub(a.start.0)
            .is_some_and(|offset| offset < a.size)
    }) {
        if a.size != 1
            || !r.frame.spill_zp_slots.contains(&a.slot)
            || !byte_accesses_only(r, &MirMem::ZeroPage(a.slot))
        {
            return false;
        }
        let key = MirHomeByte::VirtualZeroPage {
            slot: a.slot,
            offset: 0,
        };
        let byte = Byte::Home(key);
        let Some(range) = lives.get(byte) else {
            return false;
        };
        if entry.contains(key)
            || lives
                .get(Byte::Home(MirHomeByte::FixedZeroPage(fixed)))
                .is_some_and(|physical| !range.occupied.is_disjoint(&physical.occupied))
            || !named
                .iter()
                .chain(&spills)
                .all(|&other| lives.disjoint(byte, other))
        {
            return false;
        }
        spills.push(byte);
    }
    !spills.is_empty() && spills.len() + named.len() >= 2
}
