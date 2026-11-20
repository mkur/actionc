//! Deterministic, whole-parameter ZP placement. The frame reservation is shared
//! with existing temporary allocation; final expansion checks the lifetime.
use super::*;
mod contenders;
pub(super) use contenders::allocate;

pub(super) fn bindings(program: &MirProgram, routine: &MirRoutine, addresses: &ObjectAddressControl<'_>,
    forbidden: &BTreeSet<u8>) -> Option<Vec<MirParamBinding>> {
    // A single read-only byte already has the optimal legacy forwarding path.
    if routine.frame.params.len() == 1 && routine.frame.params[0].storage_size == 1 {
        return None;
    }
    bindings_avoiding(program, routine, &|_, _| Some(forbidden.clone()), addresses)
}

pub(super) fn bindings_avoiding(
    program: &MirProgram,
    routine: &MirRoutine,
    forbidden: &impl Fn(&MirMem, u16) -> Option<BTreeSet<u8>>,
    addresses: &ObjectAddressControl<'_>,
) -> Option<Vec<MirParamBinding>> {
    let mut used = used_bytes(program, routine, &BTreeSet::new());
    let total = routine.frame.abi_parameter_bytes()?;
    if total > crate::action_abi::MAX_DIRECT_PARAM_CAPTURE_BYTES {
        return sargs_bindings(routine, used, forbidden, addresses);
    }
    let mut offset = 0;
    let mut result = Vec::new();
    for slot in &routine.frame.params {
        let MirStorageBase::Param(id) = slot.base else {
            return None;
        };
        let width = slot.storage_size;
        let home = if !eligible_parameter(addresses, routine, slot) {
            MirParamHome::Inline
        } else if absent_parameter(routine, id) {
            MirParamHome::Absent
        } else {
            // Reuse an incoming fixed lane first, then ABI-window storage and
            // the normal virtual pool. Persistent pointer consumers can also
            // use AA/AB or AE/AF. AC/AD remains the copy-and-rebase scratch pair.
            let incoming = incoming_lane(offset, width);
            let start = forbidden(&MirMem::Param { id, offset: 0 }, width)
                .and_then(|forbidden| pick(routine, &excluding(used, &forbidden), width, incoming, total, offset));
            if let Some(start) = start {
                for lane in 0..width {
                    used[start as usize + lane as usize] = true;
                }
                MirParamHome::ZeroPage(MirFixedZpSlot(start))
            } else {
                MirParamHome::Inline
            }
        };
        offset += width;
        result.push(MirParamBinding { param: id, home });
    }
    result
        .iter()
        .any(|b| matches!(b.home, MirParamHome::ZeroPage(_)))
        .then_some(result)
}

/// SArgs also writes unused parameters, so every byte needs a home.
fn sargs_bindings(
    routine: &MirRoutine,
    used: [bool; 256],
    forbidden: &impl Fn(&MirMem, u16) -> Option<BTreeSet<u8>>,
    addresses: &ObjectAddressControl<'_>,
) -> Option<Vec<MirParamBinding>> {
    let width = routine.frame.abi_parameter_bytes()?;
    let starts = SARGS_HOME_POOL.clone().filter(|start|
        u16::from(*start) + width <= u16::from(*SARGS_HOME_POOL.end()) + 1);
    let mut slots = Vec::new();
    for slot in &routine.frame.params {
        let MirStorageBase::Param(id) = slot.base else { return None };
        if !eligible_parameter(addresses, routine, slot) { return None; }
        slots.push((id, slot.storage_size, forbidden(&MirMem::Param { id, offset: 0 }, slot.storage_size)?));
    }
    for start in starts {
        let mut cursor = start;
        let mut bindings = Vec::new();
        for (param, width, forbidden) in &slots {
            let end = cursor + u8::try_from(*width).ok()?;
            if (cursor..end).any(|byte| used[usize::from(byte)] || forbidden.contains(&byte)) { break; }
            bindings.push(MirParamBinding { param: *param, home: MirParamHome::ZeroPage(MirFixedZpSlot(cursor)) });
            cursor = end;
        }
        if bindings.len() == slots.len() { return Some(bindings); }
    }
    None
}

fn excluding(mut used: [bool; 256], forbidden: &BTreeSet<u8>) -> [bool; 256] {
    for byte in forbidden { used[usize::from(*byte)] = true; }
    used
}

fn used_bytes(program: &MirProgram, routine: &MirRoutine, forbidden: &BTreeSet<u8>) -> [bool; 256] {
    let mut used = [false; 256];
    for byte in source_zero_page(program, routine)
        .into_iter()
        .chain(forbidden.iter().copied())
    {
        used[byte as usize] = true;
    }
    for byte in persistent::slots(program) { used[usize::from(byte)] = true; }
    for slot in &routine.frame.fixed_zero_page {
        // Result homes are subject to the explicit last-use handoff check.
        if !matches!(slot.0, 0xa0 | 0xa1) {
            used[slot.0 as usize] = true;
        }
    }
    for allocation in &routine.frame.zero_page_allocations {
        for lane in 0..allocation.size {
            used[allocation.start.0.wrapping_add(lane) as usize] = true;
        }
    }
    if let Some(bytes) = routine.frame.abi_parameter_bytes()
        && bytes > crate::action_abi::MAX_DIRECT_PARAM_CAPTURE_BYTES {
        used[0xa0..(0xa0 + usize::from(bytes)).min(256)].fill(true);
    }
    for (_, slot) in routine.frame.resident_bytes() {
        used[usize::from(slot.0)] = true;
    }
    used
}

fn incoming_lane(offset: u16, width: u16) -> Option<u8> {
    (0..width).find_map(|lane| match super::super::abi::action_arg_home(offset + lane, MirWidth::Byte) {
        MirArgHome::FixedZeroPage(slot) => slot.0.checked_sub(lane as u8),
        _ => None,
    })
}

/// Universe of bytes the source-home picker can assign for this ABI.
pub(super) fn pool_bytes() -> impl Iterator<Item = u8> {
    (0xa0..=0xa3).chain(0xaa..=0xab).chain(0xae..=0xaf).chain(SARGS_HOME_POOL)
}

fn pick(
    routine: &MirRoutine,
    used: &[bool; 256],
    width: u16,
    incoming: Option<u8>,
    total: u16,
    offset: u16,
) -> Option<u8> {
    incoming
        .into_iter()
        .chain(0xa0..=0xa3u8)
        .chain(0xe0..=0xef)
        .chain(
            [0xaau8, 0xae]
                .into_iter()
                .filter(|_| width == 2 && has_pointer_reads(routine)),
        )
        .filter(|start| {
            u16::from(*start) + width
                <= match *start {
                    0xa0..=0xa3 => 0xa4,
                    0xaa => 0xac,
                    0xae => 0xb0,
                    _ => 0xf0,
                }
        })
        .find(|start| {
            (0..width).all(|lane| {
                let byte = u16::from(*start) + lane;
                !used[byte as usize] && !(total == 4 && byte == 0xa3 && offset + lane != 3)
            })
        })
}

pub(super) fn local_bindings(
    program: &MirProgram,
    routine: &MirRoutine,
    forbidden: &impl Fn(&MirMem, u16) -> Option<BTreeSet<u8>>,
    addresses: &ObjectAddressControl<'_>,
) -> Option<Vec<MirLocalBinding>> {
    let entry = super::locals::entry_dependencies(routine)?;
    let mut used = used_bytes(program, routine, &BTreeSet::new());
    let total: u16 = routine.frame.params.iter().map(|s| s.storage_size).sum();
    let bindings: Vec<_> = routine
        .frame
        .locals
        .iter()
        .filter_map(|slot| {
            let MirStorageBase::Local(local) = slot.base else {
                return None;
            };
            let start = (super::locals::eligible(addresses, routine, slot)
                && !entry.contains(&local))
            .then(|| forbidden(&MirMem::Local { id: local, offset: 0 }, slot.storage_size))
            .flatten()
            .and_then(|forbidden| pick(routine, &excluding(used, &forbidden), slot.storage_size, None, total, total));
            let home = if let Some(start) = start {
                for lane in 0..slot.storage_size {
                    used[usize::from(start) + usize::from(lane)] = true;
                }
                MirLocalHome::ZeroPage(MirFixedZpSlot(start))
            } else {
                MirLocalHome::Inline
            };
            Some(MirLocalBinding { local, home })
        })
        .collect();
    bindings
        .iter()
        .any(|b| matches!(b.home, MirLocalHome::ZeroPage(_)))
        .then_some(bindings)
}
