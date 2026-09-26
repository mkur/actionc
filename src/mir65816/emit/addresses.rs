//! Exact-width selection for structured native addresses.
use super::*;

#[cfg(test)]
#[path = "address_tests.rs"]
mod tests;

/// Direct symbolic places already use checked relocation addends in the
/// fallback. Indirect symbolic values only admit zero here: folding their
/// modular displacement requires a separate object-extent proof.
fn direct_symbol(address: &Mir65816Address) -> Option<(Target, u32)> {
    if address.index.is_some() {
        return None;
    }
    let id = match &address.base {
        Mir65816AddressBase::Static(NirStorageId::Global(id))
        | Mir65816AddressBase::External(Mir65816ExternalAddress::Global(id)) => {
            Mir65816DataId::Global(*id)
        }
        Mir65816AddressBase::Indirect(value) if address.displacement.get() == 0 => match value {
            Mir65816Value::GlobalAddress(id, bytes) if bytes.get() == 3 => {
                Mir65816DataId::Global(*id)
            }
            Mir65816Value::StaticAddress(id, bytes) if bytes.get() == 3 => {
                Mir65816DataId::Static(*id)
            }
            _ => return None,
        },
        _ => return None,
    };
    Some((Target::Data(id), address.displacement.get()))
}

impl Builder<'_> {
    pub(super) fn symbol_address(
        &mut self,
        dest: TempId,
        address: &Mir65816Address,
    ) -> Result<bool, String> {
        let Some((target, addend)) = direct_symbol(address) else {
            return Ok(false);
        };
        self.write_symbol_address(dest, target, addend)
    }
    fn write_symbol_address(
        &mut self,
        dest: TempId,
        target: Target,
        addend: u32,
    ) -> Result<bool, String> {
        let home = self.temp(dest)?;
        if home.slot().width != 3 {
            return Err("symbol address requires a complete three-byte home".into());
        }
        let Location::Stack(slot) = home else {
            return Ok(false);
        };
        // Check the entire destination before committing any instruction/fixup.
        abi::stack::access_displacement(
            ByteOffset::new(slot.offset.into()),
            ByteSize::new(3),
            ByteSize::new(self.code.delta()),
        )
        .map_err(|e| e.to_string())?;
        self.code.barrier();
        self.code.a8();
        for byte in 0..3 {
            self.code
                .reference(ReferenceOp::LdaByte, target, addend, Some(byte));
            self.store_memory(home.into(), byte.into())?;
        }
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug)]
struct Symbol {
    target: Mir65816DataId,
    addend: u32,
}

#[derive(Clone)]
enum IndexedBase {
    Symbol(Symbol),
    Captured(Mir65816Value),
}

struct Indexed {
    base: IndexedBase,
    index: Slot,
    displacement: u16,
    stride: u16,
    bytes: u8,
}

fn indexed_address(
    routine: &Mir65816Routine,
    frame: &AllocatedFrame,
    address: &Mir65816Address,
    known: &BTreeMap<TempId, Symbol>,
    data: &[Mir65816Data],
    bytes: u8,
) -> Result<Option<Indexed>, String> {
    let Some(index) = &address.index else {
        return Ok(None);
    };
    let stride = index.stride.get();
    if stride == 0 || !(1..=4).contains(&bytes) {
        return Ok(None);
    }
    let Mir65816Value::Temp(id, width) = index.value else {
        return Ok(None);
    };
    if !matches!(width.get(), 1 | 2)
        || !routine.temps.iter().any(|(temp, ty)| {
            *temp == id
                && ty.width == Some(width)
                && ty
                    .kind
                    .integer()
                    .is_some_and(|i| u32::from(i.bits) == width.get() * 8 && !i.signed)
        })
    {
        return Ok(None);
    }
    let displacement = address.displacement.get();
    let maximum = if width.get() == 1 { 255u64 } else { 65535 };
    if maximum * u64::from(stride) + u64::from(displacement) + u64::from(bytes) - 1 > 65535 {
        return Ok(None);
    }
    if !profitable_scale(stride) {
        return Ok(None);
    }
    let Some(index) = checked_stack_home(frame, id, width.get() as u8)? else {
        return Ok(None);
    };
    let mut unindexed = address.clone();
    unindexed.index = None;
    // The displacement belongs to dynamic Y, not to a checked relocation.
    unindexed.displacement = ByteOffset::ZERO;
    let base = if let Some(symbol) = resolve(&unindexed, known, data) {
        IndexedBase::Symbol(symbol)
    } else {
        let Mir65816AddressBase::Indirect(value) = &address.base else {
            return Ok(None);
        };
        match value {
            Mir65816Value::Temp(id, width) if width.get() == 3 => {
                if checked_stack_home(frame, *id, 3)?.is_none() {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        }
        IndexedBase::Captured(value.clone())
    };
    Ok(Some(Indexed {
        base,
        index,
        displacement: displacement as u16,
        stride: stride as u16,
        bytes,
    }))
}

// Compare a conservative upper bound for the new address overhead with only
// the old scale's mandatory bytes. Shared base/payload traffic cancels. The
// extra 20 budgets exact index read/zero extension (9), displacement (4), TAY
// (1), payload mode repair (2) and symbolic-vs-captured base preparation (4).
// The generic path additionally initializes INDEX and reloads Y per piece;
// neither saving is needed to authorize this selection.
fn profitable_scale(stride: u32) -> bool {
    if stride == 0 {
        return false;
    }
    if stride.is_power_of_two() {
        return true;
    }
    let shifts = 31 - stride.leading_zeros();
    let ones = stride.count_ones();
    let native_upper = 2 + shifts + 3 * (ones - 1) + 20;
    let generic_lower = 19 * ones + 6 * shifts;
    native_upper < generic_lower
}

fn symbolic_value(value: &Mir65816Value, known: &BTreeMap<TempId, Symbol>) -> Option<Symbol> {
    let target = match value {
        Mir65816Value::GlobalAddress(id, w) if w.get() == 3 => Mir65816DataId::Global(*id),
        Mir65816Value::StaticAddress(id, w) if w.get() == 3 => Mir65816DataId::Static(*id),
        Mir65816Value::Temp(id, w) if w.get() == 3 => return known.get(id).copied(),
        _ => return None,
    };
    Some(Symbol { target, addend: 0 })
}

fn resolve(
    address: &Mir65816Address,
    known: &BTreeMap<TempId, Symbol>,
    data: &[Mir65816Data],
) -> Option<Symbol> {
    let mut symbol = match &address.base {
        Mir65816AddressBase::Static(NirStorageId::Global(id))
        | Mir65816AddressBase::External(Mir65816ExternalAddress::Global(id)) => Symbol {
            target: Mir65816DataId::Global(*id),
            addend: 0,
        },
        Mir65816AddressBase::Indirect(value) => symbolic_value(value, known)?,
        _ => return None,
    };
    let index = if let Some(index) = &address.index {
        if index.stride != ByteSize::ONE {
            return None;
        }
        match index.value {
            Mir65816Value::U8(v) => u32::from(v),
            Mir65816Value::U16(v) => u32::from(v),
            Mir65816Value::U24(v) | Mir65816Value::U32(v) => v,
            _ => return None,
        }
    } else {
        0
    };
    symbol.addend = symbol
        .addend
        .checked_add(address.displacement.get())?
        .checked_add(index)?;
    let object = data.iter().find(|d| d.id == symbol.target)?;
    // Only an interior address of allocated storage is nonwrapping at every
    // valid placement. One-past at the top of the bus must keep modular runtime
    // arithmetic. Alias/absolute placement needs its own extent proof.
    if object.placement != Mir65816DataPlacement::Allocate || symbol.addend >= object.size.get() {
        return None;
    }
    Some(symbol)
}

#[derive(Default)]
pub(super) struct Plan {
    symbols: BTreeMap<(BlockId, usize), (TempId, Symbol)>,
    accesses: BTreeMap<(BlockId, usize), Symbol>,
    indexed: BTreeMap<(BlockId, usize), Indexed>,
    constants: BTreeMap<(BlockId, usize), u16>,
    omitted: BTreeSet<TempId>,
}

impl Plan {
    pub(super) fn new(
        routine: &Mir65816Routine,
        frame: &AllocatedFrame,
        data: &[Mir65816Data],
    ) -> Result<Self, String> {
        let mut plan = Self::default();
        let mut uses = liveness::input_counts(routine);
        for block in &routine.blocks {
            let mut known = BTreeMap::new();
            for (i, op) in block.ops.iter().enumerate() {
                if let Some(offset) = constant_index(frame, op)? {
                    // Keep the smaller direct-symbol BYTE path when available.
                    let address = match op {
                        Mir65816Op::Load { address, .. } | Mir65816Op::Store { address, .. } => {
                            address
                        }
                        _ => unreachable!(),
                    };
                    if resolve(address, &known, data).is_none() {
                        plan.constants.insert((block.id, i), offset);
                        continue;
                    }
                }
                if matches!(
                    op,
                    Mir65816Op::Call { .. }
                        | Mir65816Op::Load { volatile: true, .. }
                        | Mir65816Op::Store { volatile: true, .. }
                        | Mir65816Op::Copy {
                            source_volatile: true,
                            ..
                        }
                        | Mir65816Op::Copy {
                            destination_volatile: true,
                            ..
                        }
                ) {
                    known.clear();
                }
                if let Mir65816Op::Load {
                    dest,
                    address,
                    width,
                    volatile: false,
                } = op
                    && (1..=4).contains(&width.get())
                    && checked_stack_home(frame, *dest, width.get() as u8)?.is_some()
                    && let Some(indexed) =
                        indexed_address(routine, frame, address, &known, data, width.get() as u8)?
                {
                    if matches!(indexed.base, IndexedBase::Symbol(_)) {
                        remove_base_use(address, &mut uses)?;
                    }
                    plan.indexed.insert((block.id, i), indexed);
                    continue;
                }
                if let Mir65816Op::Store {
                    address,
                    value,
                    width,
                    volatile: false,
                } = op
                    && (1..=4).contains(&width.get())
                    && captured_payload(frame, value, width.get() as u8)?
                    && let Some(indexed) =
                        indexed_address(routine, frame, address, &known, data, width.get() as u8)?
                {
                    if matches!(indexed.base, IndexedBase::Symbol(_)) {
                        remove_base_use(address, &mut uses)?;
                    }
                    plan.indexed.insert((block.id, i), indexed);
                    continue;
                }
                let access = match op {
                    Mir65816Op::Load {
                        dest,
                        address,
                        width,
                        volatile: false,
                    } if width.get() == 1 => checked_stack_home(frame, *dest, 1)?.map(|_| address),
                    Mir65816Op::Store {
                        address,
                        value,
                        width,
                        volatile: false,
                    } if width.get() == 1 => captured_byte(frame, value)?.then_some(address),
                    _ => None,
                };
                if let Some(address) = access {
                    if let Some(symbol) = resolve(address, &known, data) {
                        plan.accesses.insert((block.id, i), symbol);
                        remove_base_use(address, &mut uses)?;
                    }
                    continue;
                }
                let Mir65816Op::AddressOf {
                    dest,
                    address,
                    width,
                } = op
                else {
                    continue;
                };
                if width.get() != 3 {
                    continue;
                }
                let Some(symbol) = resolve(address, &known, data) else {
                    continue;
                };
                let Some(Location::Stack(slot)) = frame.temps.get(dest) else {
                    continue;
                };
                if slot.width != 3 {
                    return Err("symbol address requires a complete three-byte home".into());
                }
                abi::stack::access_displacement(
                    ByteOffset::new(slot.offset.into()),
                    ByteSize::new(3),
                    ByteSize::ZERO,
                )
                .map_err(|e| e.to_string())?;
                known.insert(*dest, symbol);
                plan.symbols.insert((block.id, i), (*dest, symbol));
                // resolve has replaced this exact address operand. Index
                // operands are constants; no other temp occurrence is removed.
                remove_base_use(address, &mut uses)?;
            }
        }
        for (dest, _) in plan.symbols.values() {
            if uses.get(dest).copied().unwrap_or(0) == 0 {
                plan.omitted.insert(*dest);
            }
        }
        Ok(plan)
    }
    pub(super) fn emit(
        &self,
        b: &mut Builder<'_>,
        block: BlockId,
        index: usize,
        op: &Mir65816Op,
    ) -> Result<(), String> {
        self.emit_capture(b, block, index, op, true)
    }
    pub(super) fn emit_byte_load(
        &self,
        b: &mut Builder<'_>,
        block: BlockId,
        index: usize,
        op: &Mir65816Op,
    ) -> Result<(), String> {
        if !matches!(op, Mir65816Op::Load { width, volatile: false, .. } if *width == ByteSize::ONE)
        {
            return Err("BYTE consumer lost its exact nonvolatile load".into());
        }
        self.emit_capture(b, block, index, op, false)
    }
    fn emit_capture(
        &self,
        b: &mut Builder<'_>,
        block: BlockId,
        index: usize,
        op: &Mir65816Op,
        capture: bool,
    ) -> Result<(), String> {
        if let Some(&offset) = self.constants.get(&(block, index)) {
            let address = match op {
                Mir65816Op::Load { address, .. } | Mir65816Op::Store { address, .. } => address,
                _ => return Err("constant index lost its operation".into()),
            };
            let Mir65816AddressBase::Indirect(value) = &address.base else {
                return Err("constant index lost its captured base".into());
            };
            b.code.barrier();
            b.pointer_value(value, PTR)?;
            let memory = Memory::Pointer { slot: PTR, offset };
            match op {
                Mir65816Op::Load { dest, width, .. } => {
                    if capture {
                        b.transfer(memory, b.temp(*dest)?.into(), width.get() as u8, true)?;
                    } else {
                        b.code.a8();
                        b.load_memory(memory, 0)?;
                    }
                }
                Mir65816Op::Store { value, width, .. } => {
                    let bytes = width.get() as u8;
                    if !b.constant_store(memory, value, bytes, false)? {
                        if let Some(source) = b.value_memory(value)? {
                            b.transfer(source, memory, bytes, true)?;
                        } else {
                            b.code.a8();
                            b.value_byte(value, 0)?;
                            b.store_memory(memory, 0)?;
                        }
                    }
                }
                _ => unreachable!(),
            }
            return Ok(());
        }
        if let Some(indexed) = self.indexed.get(&(block, index)) {
            b.code.barrier();
            match &indexed.base {
                IndexedBase::Symbol(symbol) => b.address_to_pointer(Memory::Symbol(
                    Target::Data(symbol.target),
                    symbol.addend,
                ))?,
                IndexedBase::Captured(value) => b.pointer_value(value, PTR)?,
            }
            if indexed.index.width == 1 {
                b.code.a8();
                b.load_memory(Memory::Stack(indexed.index.offset.into()), 0)?;
                b.code.a16();
                b.code.word(WordOp::AndImm, 0xff); // Hidden B is not part of the index.
            } else {
                b.code.a16();
                b.load_memory(Memory::Stack(indexed.index.offset.into()), 0)?;
            }
            if indexed.stride.is_power_of_two() {
                for _ in 0..indexed.stride.trailing_zeros() {
                    b.code.op(Implied::AslA);
                }
            } else {
                // Prefix coefficients never exceed the checked final stride.
                // INDEX holds only the original zero-extended BYTE, never PTR.
                b.code.byte(ByteOp::StaDp, INDEX);
                for bit in (0..15 - indexed.stride.leading_zeros()).rev() {
                    b.code.op(Implied::AslA);
                    if indexed.stride & (1 << bit) != 0 {
                        b.code.op(Implied::Clc);
                        b.code.byte(ByteOp::AdcDp, INDEX);
                    }
                }
            }
            if indexed.displacement != 0 {
                b.code.op(Implied::Clc);
                b.code.word(WordOp::AdcImm, indexed.displacement);
            }
            b.code.op(Implied::Tay);
            // Match transfer(..., wide=true): full words and an exact odd byte.
            // Volatile accesses were excluded before constructing this plan.
            let mut byte = 0;
            while byte < indexed.bytes {
                let word = byte + 1 < indexed.bytes;
                if word {
                    b.code.a16();
                } else {
                    b.code.a8();
                }
                match op {
                    Mir65816Op::Load { dest, .. } => {
                        b.code.byte(ByteOp::LdaIndirectY, PTR);
                        if capture {
                            b.save_byte(*dest, byte)?;
                        }
                    }
                    Mir65816Op::Store { value, .. } => {
                        if word {
                            if let Some(bits) = payload_constant(value) {
                                b.code.word(WordOp::LdaImm, (bits >> (byte * 8)) as u16);
                            } else {
                                b.load_memory(
                                    b.value_memory(value)?.ok_or("missing indexed payload")?,
                                    byte.into(),
                                )?;
                            }
                        } else {
                            b.value_byte(value, byte)?;
                        }
                        b.code.byte(ByteOp::StaIndirectY, PTR);
                    }
                    _ => return Err("indexed access lost its operation".into()),
                }
                let step = if word { 2 } else { 1 };
                byte += step;
                if byte < indexed.bytes {
                    for _ in 0..step {
                        b.code.op(Implied::Iny);
                    }
                }
            }
            return Ok(());
        }
        if let Some(symbol) = self.accesses.get(&(block, index)) {
            b.code.barrier();
            b.code.a8();
            match op {
                Mir65816Op::Load { dest, .. } => {
                    b.code.reference(
                        ReferenceOp::LdaLong,
                        Target::Data(symbol.target),
                        symbol.addend,
                        None,
                    );
                    if capture {
                        b.save_byte(*dest, 0)?;
                    }
                }
                Mir65816Op::Store { value, .. } => {
                    b.value_byte(value, 0)?;
                    b.code.reference(
                        ReferenceOp::StaLong,
                        Target::Data(symbol.target),
                        symbol.addend,
                        None,
                    );
                }
                _ => return Err("direct BYTE access plan lost its operation".into()),
            }
            return Ok(());
        }
        if let Some(&(dest, symbol)) = self.symbols.get(&(block, index)) {
            if !self.omitted.contains(&dest) {
                if !b.write_symbol_address(dest, Target::Data(symbol.target), symbol.addend)? {
                    return Err("symbolic address plan lost its destination home".into());
                }
            }
            return Ok(());
        }
        if !capture {
            let Mir65816Op::Load { address, .. } = op else {
                unreachable!()
            };
            b.code.barrier();
            let memory = b.prepare_address(address)?;
            b.code.a8();
            b.load_memory(memory, 0)
        } else {
            b.operation(op)
        }
    }
}

fn constant_index(frame: &AllocatedFrame, op: &Mir65816Op) -> Result<Option<u16>, String> {
    let (address, bytes) = match op {
        Mir65816Op::Load {
            address,
            dest,
            width,
            volatile: false,
        } if (1..=4).contains(&width.get())
            && checked_stack_home(frame, *dest, width.get() as u8)?.is_some() =>
        {
            (address, width.get())
        }
        Mir65816Op::Store {
            address,
            value,
            width,
            volatile: false,
        } if (1..=4).contains(&width.get())
            && captured_payload(frame, value, width.get() as u8)? =>
        {
            (address, width.get())
        }
        _ => return Ok(None),
    };
    let Some(index) = &address.index else {
        return Ok(None);
    };
    let Mir65816AddressBase::Indirect(Mir65816Value::Temp(id, width)) = address.base else {
        return Ok(None);
    };
    if width.get() != 3 || checked_stack_home(frame, id, 3)?.is_none() {
        return Ok(None);
    }
    let value = match index.value {
        Mir65816Value::U8(v) => u64::from(v),
        Mir65816Value::U16(v) => u64::from(v),
        Mir65816Value::U24(v) | Mir65816Value::U32(v) => u64::from(v),
        _ => return Ok(None),
    };
    let stride = index.stride.get();
    if stride == 0 || stride >= 0x1000000 {
        return Ok(None);
    }
    let offset = value * u64::from(stride) + u64::from(address.displacement.get());
    Ok((offset + u64::from(bytes) - 1 <= 65535).then_some(offset as u16))
}

fn checked_stack_home(
    frame: &AllocatedFrame,
    id: TempId,
    bytes: u8,
) -> Result<Option<Slot>, String> {
    let home = frame
        .temps
        .get(&id)
        .ok_or("missing address-selection home")?;
    if home.slot().width != bytes {
        return Err("address-selection home width mismatch".into());
    }
    let Location::Stack(slot) = home else {
        return Ok(None);
    };
    abi::stack::access_displacement(
        ByteOffset::new(slot.offset.into()),
        ByteSize::new(bytes.into()),
        ByteSize::ZERO,
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(*slot))
}

fn remove_base_use(
    address: &Mir65816Address,
    uses: &mut BTreeMap<TempId, usize>,
) -> Result<(), String> {
    if let Mir65816AddressBase::Indirect(Mir65816Value::Temp(id, _)) = address.base {
        let count = uses.get_mut(&id).ok_or("missing symbolic base use")?;
        *count = count.checked_sub(1).ok_or("symbolic base use underflow")?;
    }
    Ok(())
}

fn captured_byte(frame: &AllocatedFrame, value: &Mir65816Value) -> Result<bool, String> {
    match value {
        Mir65816Value::U8(_) => Ok(true),
        Mir65816Value::Temp(id, width) if width.get() == 1 => {
            Ok(checked_stack_home(frame, *id, 1)?.is_some())
        }
        _ => Ok(false),
    }
}

fn payload_constant(value: &Mir65816Value) -> Option<u32> {
    match value {
        Mir65816Value::U8(v) => Some((*v).into()),
        Mir65816Value::U16(v) => Some((*v).into()),
        Mir65816Value::U24(v) | Mir65816Value::U32(v) => Some(*v),
        Mir65816Value::Null(_) => Some(0),
        _ => None,
    }
}

fn captured_payload(
    frame: &AllocatedFrame,
    value: &Mir65816Value,
    bytes: u8,
) -> Result<bool, String> {
    if payload_constant(value).is_some() {
        return Ok(true);
    }
    match value {
        Mir65816Value::Temp(id, width) if width.get() == u32::from(bytes) => {
            Ok(checked_stack_home(frame, *id, bytes)?.is_some())
        }
        _ => Ok(false),
    }
}
