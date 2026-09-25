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
}

fn indexed_address(
    routine: &Mir65816Routine,
    frame: &AllocatedFrame,
    address: &Mir65816Address,
    known: &BTreeMap<TempId, Symbol>,
    data: &[Mir65816Data],
) -> Result<Option<Indexed>, String> {
    let Some(index) = &address.index else {
        return Ok(None);
    };
    if index.stride != ByteSize::ONE || address.displacement.get() != 0 {
        return Ok(None);
    }
    let Mir65816Value::Temp(id, width) = index.value else {
        return Ok(None);
    };
    if width.get() != 2
        || !routine.temps.iter().any(|(temp, ty)| {
            *temp == id
                && ty.width == Some(width)
                && ty.kind.integer().is_some_and(|i| i.bits == 16 && !i.signed)
        })
    {
        return Ok(None);
    }
    let Some(index) = checked_stack_home(frame, id, 2)? else {
        return Ok(None);
    };
    let mut unindexed = address.clone();
    unindexed.index = None;
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
    Ok(Some(Indexed { base, index }))
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
                    && width.get() == 1
                    && checked_stack_home(frame, *dest, 1)?.is_some()
                    && let Some(indexed) = indexed_address(routine, frame, address, &known, data)?
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
                    && width.get() == 1
                    && captured_byte(frame, value)?
                    && let Some(indexed) = indexed_address(routine, frame, address, &known, data)?
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
        if let Some(indexed) = self.indexed.get(&(block, index)) {
            b.code.barrier();
            match &indexed.base {
                IndexedBase::Symbol(symbol) => b.address_to_pointer(Memory::Symbol(
                    Target::Data(symbol.target),
                    symbol.addend,
                ))?,
                IndexedBase::Captured(value) => b.pointer_value(value, PTR)?,
            }
            b.code.a16();
            b.load_memory(Memory::Stack(indexed.index.offset.into()), 0)?;
            b.code.op(Implied::Tay);
            b.code.a8();
            match op {
                Mir65816Op::Load { dest, .. } => {
                    b.code.byte(ByteOp::LdaIndirectY, PTR);
                    b.save_byte(*dest, 0)?;
                }
                Mir65816Op::Store { value, .. } => {
                    // Only an immediate or captured stack byte was admitted.
                    // Loading it cannot change Y or the prepared PTR bytes.
                    b.value_byte(value, 0)?;
                    b.code.byte(ByteOp::StaIndirectY, PTR);
                }
                _ => return Err("indexed BYTE access lost its operation".into()),
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
                    b.save_byte(*dest, 0)?;
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
        b.operation(op)
    }
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
