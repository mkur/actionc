use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirCallTarget, MirFixedZpSlot, MirGlobalBacking, MirMem, MirOp,
    MirPointerPair, MirProgram, MirRoutine, MirValue, MirZpAllocation,
};

pub(super) fn reserve_pointer_scratch_slots(program: &mut MirProgram) {
    for routine in &mut program.routines {
        if !routine_uses_deref(routine) {
            continue;
        }

        if !routine
            .frame
            .fixed_zero_page
            .contains(&MirFixedZpSlot(super::POINTER_SCRATCH_LO))
        {
            routine
                .frame
                .fixed_zero_page
                .push(MirFixedZpSlot(super::POINTER_SCRATCH_LO));
        }
        if !routine
            .frame
            .fixed_zero_page
            .contains(&MirFixedZpSlot(super::POINTER_SCRATCH_HI))
        {
            routine
                .frame
                .fixed_zero_page
                .push(MirFixedZpSlot(super::POINTER_SCRATCH_HI));
        }
        if routine_uses_address_advance(routine)
            && !routine
                .frame
                .fixed_zero_page
                .contains(&MirFixedZpSlot(super::POINTER_INDEX_SCRATCH_LO))
        {
            routine
                .frame
                .fixed_zero_page
                .push(MirFixedZpSlot(super::POINTER_INDEX_SCRATCH_LO));
        }
        if routine_uses_address_advance(routine)
            && !routine
                .frame
                .fixed_zero_page
                .contains(&MirFixedZpSlot(super::POINTER_INDEX_SCRATCH_HI))
        {
            routine
                .frame
                .fixed_zero_page
                .push(MirFixedZpSlot(super::POINTER_INDEX_SCRATCH_HI));
        }
    }
}

fn routine_uses_deref(routine: &MirRoutine) -> bool {
    routine.blocks.iter().any(|block| {
        block.ops.iter().any(|op| match op {
            MirOp::Load { src, .. } | MirOp::Store { dst: src, .. } => addr_contains_deref(src),
            MirOp::LoadIndirect { .. }
            | MirOp::StoreIndirect { .. }
            | MirOp::CopyIndirectWord { .. }
            | MirOp::OffsetPointerByIndirectByte { .. }
            | MirOp::IndirectByteCompound { .. } => true,
            MirOp::MaterializeAddress { .. }
            | MirOp::MaterializeIndexedAddress { .. }
            | MirOp::AdvanceAddress { .. } => true,
            MirOp::LoadImm { .. }
            | MirOp::Move { .. }
            | MirOp::Extend { .. }
            | MirOp::Truncate { .. }
            | MirOp::Unary { .. }
            | MirOp::Binary { .. }
            | MirOp::Compare { .. }
            | MirOp::CompareIndirectBytes { .. }
            | MirOp::Call { .. }
            | MirOp::RuntimeHelper { .. }
            | MirOp::Barrier { .. }
            | MirOp::LeaAddr { .. }
            | MirOp::UpdateMem { .. }
            | MirOp::UpdateIndexedMem { .. }
            | MirOp::AddByteToWordMem { .. }
            | MirOp::SubByteFromWordMem { .. }
            | MirOp::MachineBlock { .. } => false,
        })
    })
}

fn routine_uses_address_advance(routine: &MirRoutine) -> bool {
    routine.blocks.iter().any(|block| {
        block
            .ops
            .iter()
            .any(|op| matches!(op, MirOp::AdvanceAddress { .. }))
    })
}

fn addr_contains_deref(addr: &MirAddr) -> bool {
    matches!(addr, MirAddr::Deref { .. })
}

pub(super) fn reserve_used_fixed_zero_page_slots(routine: &mut MirRoutine) {
    let mut slots = routine.frame.fixed_zero_page.clone();
    for block in &routine.blocks {
        for op in &block.ops {
            collect_op_fixed_zero_page(op, &mut slots);
        }
    }
    routine.frame.fixed_zero_page = slots;
}

fn collect_op_fixed_zero_page(op: &MirOp, slots: &mut Vec<MirFixedZpSlot>) {
    match op {
        MirOp::Load {
            src: MirAddr::Direct(mem),
            ..
        } => collect_mem_fixed_zero_page(mem, slots),
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            ..
        } => collect_mem_fixed_zero_page(mem, slots),
        MirOp::UpdateMem { mem, .. } | MirOp::UpdateIndexedMem { base: mem, .. } => {
            collect_mem_fixed_zero_page(mem, slots)
        }
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            collect_mem_fixed_zero_page(mem, slots);
            collect_value_fixed_zero_page(value, slots);
        }
        MirOp::OffsetPointerByIndirectByte { dst, source, .. } => {
            collect_mem_fixed_zero_page(dst, slots);
            collect_consumer_fixed_zero_page(*source, slots);
        }
        MirOp::MaterializeAddress { consumer, value } => {
            collect_consumer_fixed_zero_page(*consumer, slots);
            collect_value_fixed_zero_page(value, slots);
        }
        MirOp::AdvanceAddress {
            consumer, index, ..
        } => {
            collect_consumer_fixed_zero_page(*consumer, slots);
            collect_value_fixed_zero_page(index, slots);
            collect_fixed_zero_page_slot(MirFixedZpSlot(super::POINTER_SCRATCH_LO), slots);
            collect_fixed_zero_page_slot(MirFixedZpSlot(super::POINTER_SCRATCH_HI), slots);
            collect_fixed_zero_page_slot(MirFixedZpSlot(super::POINTER_INDEX_SCRATCH_LO), slots);
            collect_fixed_zero_page_slot(MirFixedZpSlot(super::POINTER_INDEX_SCRATCH_HI), slots);
        }
        MirOp::LoadIndirect { consumer, .. } => collect_consumer_fixed_zero_page(*consumer, slots),
        MirOp::StoreIndirect { consumer, src, .. } => {
            collect_consumer_fixed_zero_page(*consumer, slots);
            collect_value_fixed_zero_page(src, slots);
        }
        MirOp::CopyIndirectWord {
            source,
            destination,
            ..
        } => {
            collect_consumer_fixed_zero_page(*source, slots);
            collect_consumer_fixed_zero_page(*destination, slots);
        }
        MirOp::IndirectByteCompound { target, source, .. } => {
            collect_consumer_fixed_zero_page(*target, slots);
            collect_consumer_fixed_zero_page(*source, slots);
        }
        MirOp::CompareIndirectBytes { left, right, .. } => {
            collect_consumer_fixed_zero_page(*left, slots);
            collect_consumer_fixed_zero_page(*right, slots);
        }
        MirOp::Move { src, .. } => collect_value_fixed_zero_page(src, slots),
        MirOp::Call { target, args, .. } => {
            collect_call_target_fixed_zero_page(target, slots);
            for arg in args {
                collect_value_fixed_zero_page(&arg.value, slots);
            }
        }
        MirOp::Compare { left, right, .. } | MirOp::Binary { left, right, .. } => {
            collect_value_fixed_zero_page(left, slots);
            collect_value_fixed_zero_page(right, slots);
        }
        _ => {}
    }
}

fn collect_consumer_fixed_zero_page(consumer: MirAddressConsumer, slots: &mut Vec<MirFixedZpSlot>) {
    match consumer.pointer_pair() {
        MirPointerPair::Fixed { lo } => {
            collect_fixed_zero_page_slot(lo, slots);
            collect_fixed_zero_page_slot(MirFixedZpSlot(lo.0.saturating_add(1)), slots);
        }
        MirPointerPair::Virtual(_) => {}
    }
}

fn collect_call_target_fixed_zero_page(target: &MirCallTarget, slots: &mut Vec<MirFixedZpSlot>) {
    if let MirCallTarget::Indirect { target, .. } = target {
        collect_value_fixed_zero_page(target, slots);
    }
}

fn collect_value_fixed_zero_page(value: &MirValue, slots: &mut Vec<MirFixedZpSlot>) {
    match value {
        MirValue::PointerCell(mem) => collect_mem_fixed_zero_page(mem, slots),
        MirValue::Word { lo, hi } => {
            collect_value_fixed_zero_page(lo, slots);
            collect_value_fixed_zero_page(hi, slots);
        }
        _ => {}
    }
}

fn collect_mem_fixed_zero_page(mem: &MirMem, slots: &mut Vec<MirFixedZpSlot>) {
    if let MirMem::FixedZeroPage(slot) = mem {
        collect_fixed_zero_page_slot(*slot, slots);
    }
}

fn collect_fixed_zero_page_slot(slot: MirFixedZpSlot, slots: &mut Vec<MirFixedZpSlot>) {
    if !slots.contains(&slot) {
        slots.push(slot);
    }
}

pub(super) fn allocate_zero_page_slots(program: &mut MirProgram) {
    const SCRATCH_START: u8 = 0xE0;
    const SCRATCH_END: u8 = 0xEF;
    let source_zero_page = source_zero_page_slots(program);

    for routine in &mut program.routines {
        let mut used = [false; 256];
        for fixed in &source_zero_page {
            used[fixed.0 as usize] = true;
        }
        for fixed in &routine.frame.fixed_zero_page {
            used[fixed.0 as usize] = true;
        }
        for allocation in &routine.frame.zero_page_allocations {
            mark_zp_range(&mut used, allocation.start.0, allocation.size);
        }

        for slot in &routine.frame.virtual_zero_page {
            if routine
                .frame
                .zero_page_allocations
                .iter()
                .any(|allocation| allocation.slot == *slot)
            {
                continue;
            }
            let Some(start) = find_zp_range(&used, SCRATCH_START, SCRATCH_END, 1) else {
                continue;
            };
            mark_zp_range(&mut used, start, 1);
            routine.frame.zero_page_allocations.push(MirZpAllocation {
                slot: *slot,
                start: MirFixedZpSlot(start),
                size: 1,
            });
        }
    }
}

pub(super) fn source_zero_page_slots(program: &MirProgram) -> Vec<MirFixedZpSlot> {
    let mut slots = Vec::new();
    for global in &program.globals {
        let MirGlobalBacking::Absolute(address) = global.backing else {
            continue;
        };
        if address >= 0x0100 {
            continue;
        }
        for offset in 0..global.storage_size {
            let slot = MirFixedZpSlot(address.wrapping_add(offset) as u8);
            if !slots.contains(&slot) {
                slots.push(slot);
            }
        }
    }
    slots
}

pub(super) fn mark_zp_range(used: &mut [bool; 256], start: u8, size: u8) {
    for offset in 0..size {
        used[start.wrapping_add(offset) as usize] = true;
    }
}

pub(super) fn find_zp_range(used: &[bool; 256], start: u8, end: u8, size: u8) -> Option<u8> {
    if size == 0 {
        return None;
    }
    let last = end.checked_sub(size - 1)?;
    (start..=last)
        .find(|candidate| (0..size).all(|offset| !used[candidate.wrapping_add(offset) as usize]))
}
