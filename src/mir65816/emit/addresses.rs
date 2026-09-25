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
                if let Mir65816AddressBase::Indirect(Mir65816Value::Temp(id, _)) = address.base {
                    *uses.get_mut(&id).ok_or("missing symbolic base use")? -= 1;
                }
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
