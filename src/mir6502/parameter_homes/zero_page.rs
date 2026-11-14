//! Deterministic, whole-parameter ZP placement. The frame reservation is shared
//! with existing temporary allocation; final expansion checks the lifetime.
use super::*;

pub(super) fn bindings(program: &MirProgram, routine: &MirRoutine) -> Option<Vec<MirParamBinding>> {
    // A single read-only byte already has the optimal legacy forwarding path.
    if routine.frame.params.len() == 1 && routine.frame.params[0].storage_size == 1 {
        return None;
    }
    bindings_avoiding(program, routine, &BTreeSet::new())
}

pub(super) fn bindings_avoiding(program: &MirProgram, routine: &MirRoutine, forbidden: &BTreeSet<u8>) -> Option<Vec<MirParamBinding>> {
    let mut used = [false; 256];
    for byte in source_zero_page(program, routine).into_iter().chain(forbidden.iter().copied()) {
        used[byte as usize] = true;
    }
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
    let total: u16 = routine.frame.params.iter().map(|p| p.storage_size).sum();
    let mut offset = 0;
    let mut result = Vec::new();
    for slot in &routine.frame.params {
        let MirStorageBase::Param(id) = slot.base else {
            return None;
        };
        let width = slot.storage_size;
        let home = if absent_parameter(routine, id) {
            MirParamHome::Absent
        } else {
            // Reuse an incoming fixed lane first, then ABI-window storage and
            // the normal virtual pool. Persistent pointer consumers can also
            // use AA/AB or AE/AF. AC/AD remains the copy-and-rebase scratch pair.
            let incoming = (0..width).find_map(|lane| {
                match super::super::abi::action_arg_home(offset + lane, MirWidth::Byte) {
                    MirArgHome::FixedZeroPage(slot) => slot.0.checked_sub(lane as u8),
                    _ => None,
                }
            });
            let scratch = [0xaau8, 0xae]
                .into_iter()
                .filter(|_| width == 2 && has_pointer_reads(routine));
            let choices = incoming
                .into_iter()
                .chain(0xa0..=0xa3u8)
                .chain(0xe0..=0xef)
                .chain(scratch);
            let start = choices
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
                        !used[byte as usize]
                    // Preserve every uncaptured fixed-slot input. The fourth
                    // byte may stay in $A3 only as its own working lane.
                    && !(total == 4 && byte == 0xa3 && offset + lane != 3)
                    })
                });
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
