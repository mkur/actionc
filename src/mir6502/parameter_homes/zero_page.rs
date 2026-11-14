//! Deterministic, whole-parameter ZP placement. The frame reservation is shared
//! with existing temporary allocation; final expansion checks the lifetime.
use super::*;

pub(super) fn bindings(program: &MirProgram, routine: &MirRoutine) -> Option<Vec<MirParamBinding>> {
    // A single read-only byte already has the optimal legacy forwarding path.
    if routine.frame.params.len() == 1 && routine.frame.params[0].storage_size == 1 {
        return None;
    }
    let mut used = [false; 256];
    for byte in source_zero_page(program, routine) {
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
            // ABI-window bytes precede the virtual pool. Do not use pointer
            // scratch until its consumers have been qualified in the next slice.
            let choices = (0xa0..=0xa3u8).chain(0xe0..=0xef);
            let start = choices
                .filter(|start| {
                    u16::from(*start) + width <= if *start < 0xe0 { 0xa4 } else { 0xf0 }
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
