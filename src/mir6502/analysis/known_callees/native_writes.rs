//! Normal-return writes through native pointers. Only residence can use the
//! distinction between an ordinary pointee and the physical bytes of a caller.
use super::*;
use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirDef, MirParamLane, MirPointerPair, MirValue, MirWidth,
};

/// A selected indirect instruction either sets Y from its operand or uses the
/// live byte in Y. Keep the latter's full 256 possible offsets when necessary.
fn destination(op: &MirOp) -> Option<(MirPointerPair, u16, u16, bool)> {
    let (consumer, offset, width) = match op {
        MirOp::StoreIndirect {
            consumer, offset, ..
        } => (*consumer, *offset, 1),
        MirOp::CopyIndirectWord {
            destination,
            destination_offset,
            ..
        }
        | MirOp::CopyDirectWordToIndirect {
            destination,
            destination_offset,
            ..
        } => (*destination, *destination_offset, 2),
        MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY { zp },
            width,
            ..
        } => {
            return Some((
                MirPointerPair::Fixed { lo: *zp },
                0,
                crate::mir6502::abi::action_arg_width_bytes(*width),
                true,
            ));
        }
        MirOp::Store {
            dst: MirAddr::IndirectIndexedY { zp },
            width,
            ..
        } => {
            return Some((
                MirPointerPair::Virtual(*zp),
                0,
                crate::mir6502::abi::action_arg_width_bytes(*width),
                true,
            ));
        }
        _ => return None,
    };
    match consumer {
        MirAddressConsumer::IndirectIndexedY(pair) => Some((pair, offset, width, false)),
        MirAddressConsumer::LiveIndirectIndexedY(pair) if offset == 0 => {
            Some((pair, offset, width, true))
        }
        _ => None,
    }
}

fn fixed_pair(routine: &MirRoutine, pair: MirPointerPair) -> Option<u8> {
    match pair {
        MirPointerPair::Fixed { lo } => Some(lo.0),
        MirPointerPair::Virtual(slot) => routine
            .frame
            .zero_page_allocations
            .iter()
            .find(|a| a.slot == slot && a.size >= 2)
            .map(|a| a.start.0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CountWidth {
    Byte { zero_means_256: bool },
    Word,
}

/// A pending write is relative to the incoming ABI stream. The physical
/// carrier matters for a multibyte store which rereads its pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::mir6502) struct FormalWrite {
    argument: u16,
    offset: u16,
    bytes: u16,
    carrier: Option<u8>,
    excluded_bases: ExcludedBases,
    // Count and implementation scratch travel with a forwarded copy destination.
    count: Option<(u16, CountWidth)>,
    reviewed_copy: Option<RoutineId>,
}

impl FormalWrite {
    pub(super) fn from_reviewed(
        buffer: &crate::mir6502::runtime_contracts::buffers::BufferAccess,
        owner: RoutineId,
    ) -> Option<Self> {
        use crate::mir6502::runtime_contracts::buffers::BufferExtent;
        if !buffer.write { return None; }
        let (bytes, count) = match buffer.extent {
            BufferExtent::Fixed(bytes) => (bytes, None),
            BufferExtent::ByteCount { argument_byte, zero_means_256 } =>
                (crate::callee6502::buffer_length(None, zero_means_256), Some((argument_byte, CountWidth::Byte { zero_means_256 }))),
            BufferExtent::Count { argument_byte } => (u16::MAX, Some((argument_byte, CountWidth::Word))),
            _ => return None,
        };
        Some(Self { argument: buffer.pointer_byte, offset: 0, bytes, carrier: None, excluded_bases: Default::default(),
            count, reviewed_copy: (bytes > 1).then_some(owner) })
    }

    fn at_call(mut self, program: &MirProgram, routine: &MirRoutine, call: &MirOp,
        known: &MirKnownCalleeSummaries, values: &MirMachineValueAvailability, site: MirSite,
    ) -> Self {
        let Some((offset, width)) = self.count.take() else { return self; };
        let MirOp::Call { args, .. } = call else { return self; };
        let staged = args.iter().all(|a| a.width == MirWidth::Byte);
        let state = staged.then_some((values, site));
        use crate::mir6502::runtime_contracts::buffers::{argument_byte_value, argument_word};
        let count = match width {
            CountWidth::Word => argument_word(program, call, offset, state),
            CountWidth::Byte { zero_means_256 } => argument_byte_value(program, call, offset, MirWidth::Byte, state)
                .map(|n| crate::callee6502::buffer_length(Some(n), zero_means_256)),
        };
        if let Some(bytes) = count {
            self.bytes = bytes;
        } else if staged {
            let input = |i: u16| {
                let lanes = crate::mir6502::abi::argument_lanes(args.len().try_into().ok()?)?;
                let lane = *lanes.get(usize::from(i))?;
                match origin(TracePurpose::Preservation, program, routine, known, site, lane.into())? {
                    Origin::Input(n) => Some(n),
                    _ => None,
                }
            };
            if let Some(low) = input(offset)
                && (width != CountWidth::Word || offset.checked_add(1).and_then(input) == low.checked_add(1)) {
                self.count = Some((low, width));
            }
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Lane {
    Reg(MirReg),
    Cell(MirMem),
}

impl From<MirParamLane> for Lane {
    fn from(lane: MirParamLane) -> Self {
        match lane {
            MirParamLane::Reg(r) => Self::Reg(r),
            MirParamLane::ZeroPage(z) => Self::Cell(MirMem::FixedZeroPage(z)),
        }
    }
}

enum Origin {
    Constant(u8),
    DataCell(MirMem),
    DataByte { preparation: MirSite, offset: u16 },
    Input(u16),
    Address(MirMem, u8),
}

fn physical_cell(
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    m: &MirMem,
) -> Option<MirMem> {
    let m = resolve_routine_private_zero_page(routine, m.clone())?;
    Some(
        match known
            .fixed_storage
            .address(&m)
            .and_then(|a| u8::try_from(a).ok())
        {
            Some(byte) => MirMem::FixedZeroPage(MirFixedZpSlot(byte)),
            None => m,
        },
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TracePurpose {
    Preservation,
    NumericObservation,
}

/// Identify the prepared address behind a byte load. Two independently staged
/// bytes form a program-data word only when both use the same preparation.
fn loaded_byte(
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    block: &crate::mir6502::ir::MirBlock,
    before: usize,
    consumer: MirAddressConsumer,
    offset: u16,
) -> Option<Origin> {
    if !matches!(
        consumer,
        MirAddressConsumer::ScaledIndirectIndexedY(_) | MirAddressConsumer::IndirectIndexedY(_)
    ) {
        return None;
    }
    let lo = fixed_pair(routine, consumer.pointer_pair())?;
    for (index, op) in block.ops[..before].iter().enumerate().rev() {
        match op {
            MirOp::MaterializeIndexedAddress {
                consumer: prepared, ..
            }
            | MirOp::MaterializeAddress {
                consumer: prepared, ..
            } if *prepared == consumer => {
                return Some(Origin::DataByte {
                    preparation: MirSite::Op {
                        block: block.id,
                        op_index: index,
                    },
                    offset,
                });
            }
            // The selector defines these offsets relative to the prepared base,
            // including the Y adjustments emitted between scaled word lanes.
            MirOp::LoadIndirect {
                consumer: other,
                dst: MirDef::Reg(MirReg::A | MirReg::X),
                ..
            } if *other == consumer => continue,
            _ => {}
        }
        if matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. })
            || (consumer.preserves_index_in_y()
                && classify_op(op).may_clobber_reg_compat(MirReg::Y))
        {
            return None;
        }
        let mut writes = MirKnownMemoryWrites::exact();
        summarize_op_writes(op, routine, known, &mut writes);
        if !writes.preserves_fixed_pair(MirFixedZpSlot(lo), &known.fixed_storage) {
            return None;
        }
    }
    None
}

/// Follow byte copies back to entry or a relocation. At a join, a loop, or
/// any other definition the incoming identity is unavailable.
fn origin(
    purpose: TracePurpose,
    program: &MirProgram,
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    site: MirSite,
    mut lane: Lane,
) -> Option<Origin> {
    let cfg = MirCfg::from_routine(routine).ok()?;
    let MirSite::Op {
        mut block,
        mut op_index,
    } = site
    else {
        return None;
    };
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(block) {
            return None;
        }
        let b = &routine.blocks[cfg.block_index(block)?];
        for (index, op) in b.ops.get(..op_index)?.iter().enumerate().rev() {
            let value = match (&lane, op) {
                (
                    Lane::Reg(r),
                    MirOp::LoadIndirect {
                        dst: MirDef::Reg(dst),
                        consumer,
                        offset,
                    },
                ) if r == dst => return loaded_byte(routine, known, b, index, *consumer, *offset),
                (
                    Lane::Cell(cell),
                    MirOp::Store {
                        dst: MirAddr::Direct(dst),
                        src,
                        width: MirWidth::Byte,
                    },
                ) if physical_cell(routine, known, dst).as_ref() == Some(cell) => Some(src),
                (
                    Lane::Reg(r),
                    MirOp::Move {
                        dst: MirDef::Reg(dst),
                        src,
                        width: MirWidth::Byte,
                    },
                ) if r == dst => Some(src),
                (
                    Lane::Reg(r),
                    MirOp::Load {
                        dst: MirDef::Reg(dst),
                        src: MirAddr::Direct(src),
                        width: MirWidth::Byte,
                    },
                ) if r == dst => {
                    lane = Lane::Cell(physical_cell(routine, known, src)?);
                    continue;
                }
                _ => None,
            };
            if let Some(value) = value {
                match value {
                    MirValue::Def(MirDef::Reg(r)) => lane = Lane::Reg(*r),
                    MirValue::PointerCell(mem) => {
                        lane = Lane::Cell(physical_cell(routine, known, mem)?)
                    }
                    MirValue::ConstU8(n) => return Some(Origin::Constant(*n)),
                    MirValue::StorageAddrByte { mem, byte } => {
                        return Some(Origin::Address(mem.clone(), *byte));
                    }
                    _ => return None,
                }
                continue;
            }
            // SArgs copies the input stream to the selected parameter cells.
            // Follow only the matched contract's ordered input/definition pair.
            if let (Lane::Cell(cell), MirOp::SArgsCapture { .. }) = (&lane, op) {
                let capture =
                    crate::mir6502::runtime_contracts::sargs_capture(program, routine, op)?;
                let index = capture.definitions.iter().position(|m| m == cell)?;
                let lanes = crate::mir6502::abi::physical_lanes(
                    capture.inputs.get(index)?,
                    MirWidth::Byte,
                )?;
                let [input] = lanes.as_slice() else {
                    return None;
                };
                lane = (*input).into();
                continue;
            }
            match &lane {
                Lane::Reg(r) if classify_op(op).may_clobber_reg_compat(*r) => return None,
                Lane::Cell(cell) => {
                    let mut writes = MirKnownMemoryWrites::exact();
                    if let MirOp::Call { target, .. } = op {
                        // The inventory records possible observations even if the
                        // intervening call could also have changed this byte.
                        if purpose == TracePurpose::NumericObservation {
                            continue;
                        }
                        known.for_target(target)?;
                    }
                    summarize_op_writes(op, routine, known, &mut writes);
                    if writes.may_write_mem(cell, &known.fixed_storage) {
                        return None;
                    }
                }
                _ => {}
            }
        }
        let preds = cfg.predecessors(block);
        if Some(block) == cfg.entry() && preds.is_empty() {
            break;
        }
        if preds.len() != 1 {
            return None;
        }
        block = *preds.first()?;
        let pred = &routine.blocks[cfg.block_index(block)?];
        let effects = super::super::effects::classify_terminator(&pred.terminator);
        if matches!(lane, Lane::Reg(_))
            && effects.machine.possible_register_writes() != MirRegisterSet::default()
        {
            return None;
        }
        op_index = pred.ops.len();
    }
    if let Lane::Cell(mem @ (MirMem::Global { .. } | MirMem::Static { .. })) = lane {
        return Some(Origin::DataCell(mem));
    }
    let params = routine.frame.direct_parameter_abi.as_ref()?;
    let lanes = crate::mir6502::abi::argument_lanes(crate::mir6502::abi::parameter_bytes(params)?)?;
    Some(Origin::Input(
        lanes.into_iter().position(|l| Lane::from(l) == lane)? as u16,
    ))
}

enum Base {
    Numeric(u16),
    Formal(u16),
    Object,
}

fn paired_origin(
    purpose: TracePurpose,
    program: &MirProgram,
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    site: MirSite,
    lanes: [Lane; 2],
    offset: u16,
    bytes: u16,
) -> Option<Base> {
    let [l, h] = lanes;
    match (
        origin(purpose, program, routine, known, site, l)?,
        origin(purpose, program, routine, known, site, h)?,
    ) {
        (Origin::Constant(lo), Origin::Constant(hi)) => {
            Some(Base::Numeric(u16::from_le_bytes([lo, hi])))
        }
        (Origin::DataCell(lo), Origin::DataCell(hi)) if offset_mem(&lo, 1) == hi && lo != hi => {
            Some(Base::Object)
        }
        (
            Origin::DataByte {
                preparation: l,
                offset: lo,
            },
            Origin::DataByte {
                preparation: h,
                offset: hi,
            },
        ) if l == h && lo.checked_add(1) == Some(hi) => Some(Base::Object),
        (Origin::Input(l), Origin::Input(h)) if h == l + 1 => {
            let mut start = 0;
            for param in routine.frame.direct_parameter_abi.as_ref()? {
                if start == l && param.width().bytes() == 2 {
                    return Some(Base::Formal(l));
                }
                start += param.width().bytes();
            }
            None
        }
        (Origin::Address(m, 0), Origin::Address(other, 1)) if m == other => {
            address_base(program, routine, &m, offset, bytes)
        }
        _ => None,
    }
}

fn address_base(
    program: &MirProgram,
    routine: &MirRoutine,
    m: &MirMem,
    offset: u16,
    bytes: u16,
) -> Option<Base> {
    if let Some(base) = fixed_storage_address(program, m) {
        return Some(Base::Numeric(base));
    }
    let pointer = super::super::bounded_pointers::Pointer::named(program, routine, m)?;
    pointer.add(offset)?.covers(bytes).then_some(Base::Object)
}

/// Before ABI staging a whole pointer operand is authoritative. In particular,
/// a retained @local is a relocation defined by LEA, independent of A and X.
fn retained_base(
    program: &MirProgram,
    routine: &MirRoutine,
    site: MirSite,
    value: &MirValue,
    write: FormalWrite,
) -> Option<Base> {
    use super::super::use_def::{MirTempLane, MirTempUseDefIndex};
    let mem = match value {
        MirValue::ConstU8(n) => return Some(Base::Numeric(u16::from(*n))),
        MirValue::ConstU16(n) => return Some(Base::Numeric(*n)),
        MirValue::GlobalAddr(id) => MirMem::Global { id: *id, offset: 0 },
        MirValue::StaticAddr(id) => MirMem::Static { id: *id, offset: 0 },
        MirValue::Def(MirDef::VTemp(temp)) => {
            let defs = MirTempUseDefIndex::from_routine(routine);
            let definition = |byte| {
                Some(
                    defs.unique_definition(MirTempLane { temp: *temp, byte })?
                        .site,
                )
            };
            let MirSite::Op { block, op_index } = definition(0)? else {
                return None;
            };
            if definition(0) != definition(1)
                || !matches!(site, MirSite::Op {block:b,op_index:i} if b==block && i>op_index)
            {
                return None;
            }
            let op = routine
                .blocks
                .iter()
                .find(|b| b.id == block)?
                .ops
                .get(op_index)?;
            match op {
                MirOp::LeaAddr {
                    target,
                    width: MirWidth::Word,
                    ..
                } => target.clone(),
                MirOp::Load {
                    src: MirAddr::Direct(MirMem::Param { id, offset: 0 }),
                    width: MirWidth::Word,
                    ..
                } => {
                    // Retained parameter storage denotes its incoming value only
                    // before any reachable write. This bounded case starts in
                    // the entry block and stops at the first possible alias.
                    let cfg = MirCfg::from_routine(routine).ok()?;
                    if cfg.entry() != Some(block) || !cfg.predecessors(block).is_empty() {
                        return None;
                    }
                    let b = &routine.blocks[cfg.block_index(block)?];

                    if b.ops[..op_index].iter().any(|op| {
                        let e = classify_op(op);
                        e.memory.may_write_any
                            || e.memory.opaque
                            || matches!(op, MirOp::Call { .. })
                            || e.memory
                                .direct_writes
                                .iter()
                                .any(|r| matches!(r.base,MirMem::Param{id:other,..} if other==*id))
                    }) {
                        return None;
                    }
                    let index = routine
                        .frame
                        .params
                        .iter()
                        .position(|p| p.base == MirStorageBase::Param(*id))?;
                    let params = routine.frame.direct_parameter_abi.as_ref()?;
                    if params.get(index)?.width().bytes() != 2 {
                        return None;
                    }
                    return Some(Base::Formal(crate::mir6502::abi::parameter_bytes(
                        params.get(..index)?,
                    )?));
                }
                MirOp::Load {
                    src:
                        MirAddr::Direct(MirMem::Global { .. } | MirMem::Static { .. })
                        | MirAddr::ComputedIndex { .. }
                        | MirAddr::PointerIndex { .. }
                        | MirAddr::PointerCell { .. }
                        | MirAddr::Deref { .. },
                    width: MirWidth::Word,
                    ..
                } => return Some(Base::Object),
                _ => return None,
            }
        }
        _ => return None,
    };
    address_base(program, routine, &mem, write.offset, write.bytes)
}

fn record_base(
    program: &MirProgram,
    base: Base,
    write: FormalWrite,
    numeric: &mut Vec<(u16, u16)>,
    pending: &mut BTreeSet<FormalWrite>,
) -> Option<()> {
    match base {
        Base::Numeric(base) => {
            if write.excluded_bases.contains(base) { return None; }
            let start = base.wrapping_add(write.offset);
            if write.bytes > 1 && let Some(owner) = write.reviewed_copy {
                let review = program.routines.iter().find(|r| r.id == owner)?.frame.callee_review.as_ref()?;
                for range in review.facts.reads.as_ref()?.iter().chain(review.facts.writes.as_ref()?) {
                    let base = match range.global {
                        Some(id) => fixed_storage_address(program, &MirMem::Global { id, offset: 0 })?,
                        None => 0,
                    };
                    let first = base.checked_add(range.start)?;
                    let last = base.checked_add(range.end)?;
                    if (first..=last).contains(&start) || first.wrapping_sub(start) < write.bytes {
                        return None;
                    }
                }
            }
            if write.bytes > 1
                && let Some(carrier) = write.carrier
                && [carrier, carrier.wrapping_add(1)]
                    .into_iter()
                    .any(|s| u16::from(s).wrapping_sub(start) < write.bytes)
            {
                return None;
            }
            if !numeric.contains(&(start, write.bytes)) {
                numeric.push((start, write.bytes));
            }
        }
        Base::Formal(argument) => {
            pending.insert(FormalWrite { argument, ..write });
        }
        Base::Object if write.excluded_bases != ExcludedBases::default() => return None,
        Base::Object => {}
    }
    Some(())
}

impl MirKnownCalleeSummaries {
    pub(in crate::mir6502) fn writes_at_site(
        &self,
        program: &MirProgram,
        routine: &MirRoutine,
        call: &MirOp,
        values: &MirMachineValueAvailability,
        site: MirSite,
    ) -> MirKnownMemoryWrites {
        self.project_formals(program, routine, call, values, site)
            .unwrap_or(MirKnownMemoryWrites::Unknown)
    }

    fn project_formals(
        &self,
        program: &MirProgram,
        routine: &MirRoutine,
        call: &MirOp,
        values: &MirMachineValueAvailability,
        site: MirSite,
    ) -> Option<MirKnownMemoryWrites> {
        let MirOp::Call {
            target: MirCallTarget::Routine(id),
            ..
        } = call
        else {
            return None;
        };
        let summary = self.get(*id)?;
        let mut writes = if let Some(facts) = summary.reviewed_facts()
            && facts.buffers.iter().any(|buffer| buffer.write)
        {
            let callee = program.routines.iter().find(|r| r.id == *id)?;
            super::reviewed_buffers::residence_writes(callee, facts, call)?
        } else {
            summary.writes().clone()
        };
        let MirKnownMemoryWrites::Pointee {
            formals,
            numeric,
            ordinary,
            ..
        } = &mut writes
        else {
            return Some(writes);
        };
        for write in std::mem::take(formals) {
            let write = write.at_call(program, routine, call, self, values, site);
            if write.bytes == 0 { continue; }
            let base = self.call_base(
                TracePurpose::Preservation,
                program,
                routine,
                call,
                values,
                site,
                write,
            )?;
            *ordinary |= matches!(base, Base::Object);
            record_base(program, base, write, numeric, formals)?;
        }
        Some(writes)
    }
    fn call_base(
        &self,
        purpose: TracePurpose,
        program: &MirProgram,
        routine: &MirRoutine,
        call: &MirOp,
        values: &MirMachineValueAvailability,
        site: MirSite,
        write: FormalWrite,
    ) -> Option<Base> {
        let MirOp::Call {
            target: MirCallTarget::Routine(id),
            args,
            ..
        } = call
        else {
            return None;
        };
        let callee = program.routines.iter().find(|r| r.id == *id)?;
        let lanes = crate::mir6502::abi::canonical_call_lanes(callee, args).ok()?;
        crate::mir6502::abi::validate_direct_call_result(callee, call).ok()?;

        let n = usize::from(write.argument);
        let mut start = 0;
        let arg = args.iter().find(|arg| {
            let found = start == write.argument;
            start += crate::mir6502::abi::action_arg_width_bytes(arg.width);
            found
        })?;
        Some(if arg.width == MirWidth::Word {
            retained_base(program, routine, site, &arg.value, write)?
        } else {
            match crate::mir6502::runtime_contracts::buffers::argument_word(
                program,
                call,
                write.argument,
                Some((values, site)),
            ) {
                Some(base) => Base::Numeric(base),
                None => paired_origin(
                    purpose,
                    program,
                    routine,
                    self,
                    site,
                    [(*lanes.get(n)?).into(), (*lanes.get(n + 1)?).into()],
                    write.offset,
                    write.bytes,
                )?,
            }
        })
    }
}

/// A reviewed producer initialized both bytes of a source-owned pointer.
/// Intervening calls must preserve the pair. A scratch pair of unknown origin
/// cannot borrow the source-pointer exception.
fn reviewed_pointer(
    program: &MirProgram,
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    site: MirSite,
    lo: u8,
) -> bool {
    let Some(hi) = lo.checked_add(1) else {
        return false;
    };
    if !program.globals.iter().any(|g| {
        g.storage_size == 2
            && known.fixed_storage.address(&MirMem::Global {
                id: g.id,
                offset: 0,
            }) == Some(u16::from(lo))
    }) {
        return false;
    }
    let MirSite::Op { block, op_index } = site else {
        return false;
    };
    let Some(block) = routine.blocks.iter().find(|b| b.id == block) else {
        return false;
    };
    for op in block.ops[..op_index].iter().rev() {
        if let MirOp::Call { target, .. } = op {
            let Some(summary) = known.for_target(target) else {
                return false;
            };
            if summary.reviewed_facts().is_some_and(|f| {
                f.defines_on_return.contains(&lo) && f.defines_on_return.contains(&hi)
            }) {
                return true;
            }
            if !summary
                .writes()
                .preserves_fixed_pair(MirFixedZpSlot(lo), &known.fixed_storage)
            {
                return false;
            }
        } else {
            let mut writes = MirKnownMemoryWrites::exact();
            summarize_op_writes(op, routine, known, &mut writes);
            if !writes.preserves_fixed_pair(MirFixedZpSlot(lo), &known.fixed_storage) {
                return false;
            }
        }
    }
    false
}

fn store_base(
    purpose: TracePurpose,
    program: &MirProgram,
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    values: &MirMachineValueAvailability,
    site: MirSite,
    op: &MirOp,
    formal: impl FnOnce() -> Option<EntryByte>,
) -> Option<(Base, FormalWrite)> {
    if let MirOp::Store { dst: addr, width, .. } = op {
        let indexed = match addr {
            MirAddr::AbsoluteIndexedX { base } => Some((base, MirReg::X)),
            MirAddr::AbsoluteIndexedY { base } => Some((base, MirReg::Y)),
            _ => None,
        };
        if let Some((mem, index)) = indexed {
            let index = values.register_at(site, index).ok()?.and_then(|v| match v {
                MirMachineValue::ConstU8(n) => Some(u16::from(n)), _ => None,
            });
            let base = if let Some(a) = fixed_storage_address(program, mem) { Base::Numeric(a) }
                else if super::super::bounded_pointers::Pointer::named(program, routine, mem).is_some() {
                    // The final-layout observation also checks indexed wrap into page zero.
                    Base::Object
                } else { return None };
            return Some((base, FormalWrite { argument: 0, offset: index.unwrap_or(0),
                bytes: crate::mir6502::abi::action_arg_width_bytes(*width) + if index.is_none() { 255 } else { 0 },
                carrier: None, excluded_bases: Default::default(), count: None, reviewed_copy: None }));
        }
    }
    let (pair, mut offset, mut bytes, indexed) = destination(op)?;
    let lo = fixed_pair(routine, pair)?;
    let hi = lo.wrapping_add(1);
    if indexed {
        if let Some(MirMachineValue::ConstU8(y)) = values.register_at(site, MirReg::Y).ok()? {
            offset = offset.wrapping_add(u16::from(y));
            if offset.checked_add(bytes)? > 256 {
                return None;
            }
        } else {
            bytes = bytes.checked_add(255)?;
        }
    }
    let byte = |slot| {
        values
            .fixed_zero_page_value_at(site, MirFixedZpSlot(slot))
            .ok()
            .flatten()
    };
    // Selected sequences load an eight-bit Y. Crossing its boundary in a
    // word store would require two separate intervals.
    if !indexed && offset.checked_add(bytes)? > 256 {
        return None;
    }
    let mut excluded_bases = ExcludedBases::default();
    let base = match (byte(lo), byte(hi)) {
        (Some(MirMachineValue::ConstU8(l)), Some(MirMachineValue::ConstU8(h))) => {
            Base::Numeric(u16::from_le_bytes([l, h]))
        }
        _ if reviewed_pointer(program, routine, known, site, lo) => Base::Object,
        _ => paired_origin(
            purpose,
            program,
            routine,
            known,
            site,
            [lo, hi].map(|s| Lane::Cell(MirMem::FixedZeroPage(MirFixedZpSlot(s)))),
            offset,
            bytes,
        ).or_else(|| {
            let fact = formal()?;
            excluded_bases = fact.excluded;
            Some(Base::Formal(fact.index))
        })?,
    };
    Some((
        base,
        FormalWrite {
            argument: 0,
            offset,
            bytes,
            carrier: Some(lo),
            excluded_bases,
            count: None, reviewed_copy: None,
        },
    ))
}

pub(super) fn store(
    program: &MirProgram,
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    values: &MirMachineValueAvailability,
    site: MirSite,
    op: &MirOp,
    formal: impl FnOnce() -> Option<EntryByte>,
) -> Option<MirKnownMemoryWrites> {
    let (base, write) = store_base(
        TracePurpose::Preservation,
        program,
        routine,
        known,
        values,
        site,
        op,
        formal,
    )?;
    let mut numeric = Vec::new();
    let mut formals = BTreeSet::new();
    record_base(program, base, write, &mut numeric, &mut formals)?;
    let effects = classify_op(op);
    if effects.memory.opaque || !effects.logical.temp_defs.is_empty() {
        return None;
    }
    let mut writes = MirKnownMemoryWrites::Pointee {
        fixed: Vec::new(),
        numeric,
        formals,
        ordinary: true,
        caller_dependent: false,
        opaque_preserves: None,
    };
    record_physical_writes(routine, effects, &mut writes);
    Some(writes)
}

mod formal_origins;
pub(super) use formal_origins::formal_origins;
use formal_origins::{EntryByte, ExcludedBases};
mod observations;
pub(in crate::mir6502) use observations::native_numeric_observations;

#[cfg(test)]
mod tests;
