use super::*;

pub(super) fn forward_unique_word_load_address_consumers(
    routine: &mut super::super::ir::MirRoutine,
    layout: &MaterializeLayout,
    include_indexes: bool,
) -> usize {
    let liveness = analyze_temp_liveness(routine);
    let mut forwarded = 0usize;
    // Newly admitted index reads must be ordinary compiler-owned storage, not
    // absolute-backed source cells, and must be disjoint from zero-page setup.
    let ordinary = |mem: &MirMem| match mem {
        MirMem::Param { id, .. } => routine
            .frame
            .params
            .iter()
            .any(|s| s.base == MirStorageBase::Param(*id)),
        MirMem::Local { id, .. } => routine
            .frame
            .locals
            .iter()
            .any(|s| s.base == MirStorageBase::Local(*id)),
        _ => layout.mem_allows_deferred_direct_read(mem),
    };
    let index_sources = routine
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .filter_map(|op| {
            let MirOp::Load {
                src: MirAddr::Direct(mem),
                width: MirWidth::Word,
                ..
            } = op
            else {
                return None;
            };
            (ordinary(mem)
                && layout
                    .mem_address(routine.id, mem)
                    .is_some_and(|a| a >= 0x100 && a < 0xFFFF))
            .then_some(mem.clone())
        })
        .collect::<Vec<_>>();
    for (block_index, block) in routine.blocks.iter_mut().enumerate() {
        let live_out = liveness
            .live_out(block_index)
            .expect("block liveness exists");
        let (ops, count) = forward_unique_word_load_address_consumers_in_block(
            std::mem::take(&mut block.ops),
            &block.terminator,
            live_out,
            layout,
            include_indexes,
            &index_sources,
        );
        block.ops = ops;
        forwarded += count;
    }
    forwarded
}

fn forward_unique_word_load_address_consumers_in_block(
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
    live_out: &MirTempLiveSet,
    layout: &MaterializeLayout,
    include_indexes: bool,
    index_sources: &[MirMem],
) -> (Vec<MirOp>, usize) {
    let mut removals = BTreeSet::new();
    let mut replacements = BTreeMap::<usize, Vec<(MirTempId, MirValue)>>::new();

    for (producer_index, producer) in ops.iter().enumerate() {
        let MirOp::Load {
            dst: MirDef::VTemp(temp),
            src: MirAddr::Direct(source),
            width: MirWidth::Word,
        } = producer
        else {
            continue;
        };
        if temp_live_out(live_out, *temp) || terminator_uses_temp(terminator, *temp) {
            continue;
        }
        let uses = ops[producer_index + 1..]
            .iter()
            .enumerate()
            .filter_map(|(offset, op)| {
                op_uses_temp(op, *temp).then_some(producer_index + 1 + offset)
            })
            .collect::<Vec<_>>();
        let &[consumer_index] = uses.as_slice() else {
            continue;
        };
        let consumer = &ops[consumer_index];
        let direct_static_index_use = consumer_index == producer_index + 1
            && op_is_static_byte_word_index_consumer(consumer, *temp);
        if (!layout.mem_allows_deferred_direct_read(source) && !direct_static_index_use)
            || op_uses_temp_more_than_once(consumer, *temp)
            || !op_address_uses_temp(
                consumer,
                *temp,
                include_indexes && index_sources.contains(source),
            )
            || ops[producer_index + 1..consumer_index]
                .iter()
                .any(op_blocks_deferred_direct_read)
        {
            continue;
        }

        removals.insert(producer_index);
        replacements
            .entry(consumer_index)
            .or_default()
            .push((*temp, pointer_value_from_mem(source)));
    }

    let forwarded = removals.len();
    let mut out = Vec::with_capacity(ops.len().saturating_sub(forwarded));
    for (index, mut op) in ops.into_iter().enumerate() {
        if removals.contains(&index) {
            continue;
        }
        if let Some(items) = replacements.get(&index) {
            for (temp, replacement) in items {
                replace_op_address_temp(&mut op, *temp, replacement);
            }
        }
        out.push(op);
    }
    (out, forwarded)
}

fn op_is_static_byte_word_index_consumer(op: &MirOp, temp: MirTempId) -> bool {
    let (addr, width) = match op {
        MirOp::Load { src, width, .. } => (src, width),
        MirOp::Store { dst, width, .. } => (dst, width),
        _ => return false,
    };
    let MirAddr::ComputedIndex {
        base,
        index,
        elem_size: 1,
        offset: 0,
    } = addr
    else {
        return false;
    };
    *width == MirWidth::Byte && value_uses_specific_temp(index, temp) && static_address_value(base)
}

fn static_address_value(value: &MirValue) -> bool {
    match value {
        MirValue::ConstU16(_) | MirValue::StaticAddr(_) | MirValue::GlobalAddr(_) => true,
        MirValue::Word { lo, hi } => matches!(
            (lo.as_ref(), hi.as_ref()),
            (
                MirValue::StorageAddrByte { mem: lo_mem, byte: 0 },
                MirValue::StorageAddrByte { mem: hi_mem, byte: 1 }
            ) if lo_mem == hi_mem
        ),
        _ => false,
    }
}

fn temp_live_out(live_out: &MirTempLiveSet, temp: MirTempId) -> bool {
    live_out.full_temp_live(temp)
        || live_out.exact_lane_live(temp, 0)
        || live_out.exact_lane_live(temp, 1)
}

fn op_address_uses_temp(op: &MirOp, temp: MirTempId, include_indexes: bool) -> bool {
    match op {
        MirOp::Load { src, .. } => addr_uses_temp(src, temp),
        MirOp::Store { dst, .. } => addr_uses_temp(dst, temp),
        MirOp::MaterializeAddress { value, .. } => value_uses_specific_temp(value, temp),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            value_uses_specific_temp(base, temp)
                || (include_indexes && value_uses_specific_temp(index, temp))
        }
        MirOp::AdvanceAddress { index, .. } => {
            include_indexes && value_uses_specific_temp(index, temp)
        }
        _ => false,
    }
}

fn addr_uses_temp(addr: &MirAddr, temp: MirTempId) -> bool {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            value_uses_specific_temp(base, temp) || value_uses_specific_temp(index, temp)
        }
        MirAddr::Deref { ptr, .. } => value_uses_specific_temp(ptr, temp),
        MirAddr::PointerIndex { index, .. } => value_uses_specific_temp(index, temp),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::PointerCell { .. } => false,
    }
}

fn value_uses_specific_temp(value: &MirValue, temp: MirTempId) -> bool {
    let mut uses = 0;
    count_value_temp_uses(value, temp, &mut uses);
    uses > 0
}

fn replace_op_address_temp(op: &mut MirOp, temp: MirTempId, replacement: &MirValue) {
    match op {
        MirOp::Load { src, .. } => {
            *src = replace_temp_addr(src.clone(), temp, replacement);
        }
        MirOp::Store { dst, .. } => {
            *dst = replace_temp_addr(dst.clone(), temp, replacement);
        }
        MirOp::MaterializeAddress { value, .. } => {
            *value = replace_temp_value(value.clone(), temp, replacement);
        }
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            *base = replace_temp_value(base.clone(), temp, replacement);
            *index = replace_temp_value(index.clone(), temp, replacement);
        }
        MirOp::AdvanceAddress { index, .. } => {
            *index = replace_temp_value(index.clone(), temp, replacement)
        }
        _ => {}
    }
}

fn op_blocks_deferred_direct_read(op: &MirOp) -> bool {
    if op_may_have_unknown_memory_effects(op) {
        return true;
    }
    match op {
        MirOp::Store { .. }
        | MirOp::CopyBytes { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::OffsetPointerByIndirectByte { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyDirectWordToIndirect { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::AbsoluteWordSubToIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. } => true,
        MirOp::PackedRealCopy { .. } => true,
        MirOp::Call { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => true,
        MirOp::Load { .. }
        | MirOp::LoadImm { .. }
        | MirOp::Move { .. }
        | MirOp::UpdateReg { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::ComputeAddress { .. }
        | MirOp::Extend { .. }
        | MirOp::Truncate { .. }
        | MirOp::Unary { .. }
        | MirOp::Binary { .. }
        | MirOp::BinaryDirectIndexedByte { .. }
        | MirOp::Compare { .. }
        | MirOp::CompareDirectIndexedBytes { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::LoadIndirect { .. } => false,
    }
}
