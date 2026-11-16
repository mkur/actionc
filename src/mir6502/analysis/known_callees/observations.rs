//! Explicit assembly storage references, independent of complete callee effects.
//! Unknown pointees do not expose private storage. Encoded addresses still do.
use super::*;
use crate::mir6502::analysis::effects::MirMemoryRange;

/// Resolve only explicit fixed backing, without consulting an emitted layout or
/// replacing the caller's logical identity. Relocation addends are already in
/// the memory offset; each alias contributes its own offset exactly once.
pub(in crate::mir6502) fn fixed_storage_address(
    program: &MirProgram,
    memory: &MirMem,
) -> Option<u16> {
    let (mut id, mut offset) = match *memory {
        MirMem::Absolute(address) => return Some(address),
        MirMem::FixedZeroPage(slot) => return Some(u16::from(slot.0)),
        MirMem::Global { id, offset } => (id, offset),
        _ => return None,
    };
    // A valid chain visits at most every global once. Cycles, missing targets
    // and overflow cannot produce a physical-address proof.
    for _ in 0..program.globals.len() {
        let global = program.globals.iter().find(|g| g.id == id)?;
        match global.backing {
            MirGlobalBacking::Absolute(address) => return address.checked_add(offset),
            MirGlobalBacking::Alias { target, offset: alias_offset } => {
                offset = offset.checked_add(alias_offset)?;
                id = target;
            }
            MirGlobalBacking::Ordinary { .. } => return None,
        }
    }
    None
}

/// The encoded base retains identity independently of its signed relocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct StorageAddress {
    pub base: MirMem,
    pub addend: i32,
    pub requires_zero_page: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mir6502) enum ObservationExtent {
    Bytes(u16),
    FullZeroPage,
    /// Low and high fetches, with the NMOS same-page wrap (also used in ZP).
    PointerFetch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct StorageObservation {
    pub address: StorageAddress,
    pub extent: ObservationExtent,
}

#[derive(Default, Debug)]
pub(in crate::mir6502) struct MachineStorageObservations {
    pub reads: Vec<StorageObservation>,
    pub writes: Vec<StorageObservation>,
}

/// Invalid fixed backing must not masquerade as a pending ordinary address.
fn valid_backing(program: &MirProgram, memory: &MirMem) -> bool {
    let MirMem::Global { mut id, mut offset } = *memory else { return true; };
    for _ in 0..program.globals.len() {
        let Some(global) = program.globals.iter().find(|g| g.id == id) else { return false; };
        match global.backing {
            MirGlobalBacking::Ordinary { .. } => return true,
            MirGlobalBacking::Absolute(address) => return address.checked_add(offset).is_some(),
            MirGlobalBacking::Alias { target, offset: more } => {
                let Some(next) = offset.checked_add(more) else { return false; };
                id = target;
                offset = next;
            }
        }
    }
    false
}

impl StorageAddress {
    fn at(&self, base: u16) -> Option<u16> {
        let address = u16::try_from(i32::from(base).checked_add(self.addend)?).ok()?;
        (!self.requires_zero_page || address < 256).then_some(address)
    }

    pub fn fixed(&self, program: &MirProgram) -> Option<u16> {
        self.at(fixed_storage_address(program, &self.base)?)
    }
}

/// This is an explicit exposure inventory, never a complete memory-effect summary.
/// None means invalid/opaque operands; a typed unresolved address remains pending.
pub(in crate::mir6502) fn machine_storage_observations(
    machine: &MirMachineBlock,
    routine: &MirRoutine,
    program: &MirProgram,
) -> Option<MachineStorageObservations> {
    use crate::asm6502::InlineAsmRelocationKind as Kind;
    use crate::mir6502::ir::MirInlineAsmTarget as Target;
    let mut addresses = BTreeMap::new();
    let bytes = flatten_machine_items_with(&machine.items, |item, remaining, at| {
        let MirMachineItem::Relocation { kind, target, addend, requires_zero_page, .. } = item else {
            return expand_machine_operand(item, remaining, routine, program);
        };
        let base = match target {
            Target::Memory(memory) => {
                if !machine_memory_within_object(memory, routine, program) || !valid_backing(program, memory) {
                    return None;
                }
                Some(memory.clone())
            }
            Target::Absolute(address) => Some(absolute_mem(*address)),
            Target::Routine(_) | Target::InlineOffset(_) => None,
        };
        let width = if *kind == Kind::Absolute16 { 2 } else { 1 };
        let full = matches!(kind, Kind::Absolute16 | Kind::Byte8)
            || (*kind == Kind::Low8 && *requires_zero_page);
        let address = base.map(|base| StorageAddress { base, addend: *addend,
            requires_zero_page: *requires_zero_page || *kind == Kind::Byte8 });
        let value = if let Some(address) = &address {
            if let Some(base) = fixed_storage_address(program, &address.base) {
                address.at(base)?;
                if full { Some(address.fixed(program)?) }
                else { resolved_absolute_address(&address.base).and_then(|base| address.at(base)) }
            } else { None }
        } else { None };
        if full && let Some(address) = address { addresses.insert(at, address); }
        Some((0..width).map(|i| MachineByte {
            value: value.map(|v| (v >> if *kind == Kind::High8 { 8 } else { i * 8 }) as u8),
            memory: None,
        }).collect())
    })?;
    let mut result = MachineStorageObservations::default();
    let mut index = 0;
    while index < bytes.len() {
        let (mnemonic, mode, len) = crate::codegen::decode_6502_opcode(bytes[index].value?)?;
        let operands = bytes.get(index + 1..index + len)?;
        let stores = matches!(mnemonic, "STA" | "STX" | "STY");
        let writes = stores || matches!(mnemonic, "ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC");
        let control = matches!(mnemonic, "JMP" | "JSR");
        let extent = match mode {
            AddressingMode::ZeroPage | AddressingMode::Absolute if !control => Some(ObservationExtent::Bytes(1)),
            AddressingMode::AbsoluteX | AddressingMode::AbsoluteY => Some(ObservationExtent::Bytes(256)),
            AddressingMode::ZeroPageX | AddressingMode::ZeroPageY
            | AddressingMode::IndexedIndirectX => Some(ObservationExtent::FullZeroPage),
            AddressingMode::IndirectIndexedY | AddressingMode::Indirect => Some(ObservationExtent::PointerFetch),
            _ => None,
        };
        if let Some(extent) = extent {
            let mut address = addresses.get(&(index + 1)).cloned().or_else(|| {
                Some(StorageAddress { base: machine_operand_mem(operands)?, addend: 0, requires_zero_page: false })
            })?;
            address.requires_zero_page |= matches!(mode, AddressingMode::ZeroPage | AddressingMode::ZeroPageX
                | AddressingMode::ZeroPageY | AddressingMode::IndirectIndexedY | AddressingMode::IndexedIndirectX);
            if !valid_backing(program, &address.base) { return None; }
            if let Some(base) = fixed_storage_address(program, &address.base) { address.at(base)?; }
            let observation = StorageObservation { address, extent };
            let indirect = matches!(mode, AddressingMode::IndirectIndexedY | AddressingMode::IndexedIndirectX | AddressingMode::Indirect);
            if !stores || indirect { result.reads.push(observation.clone()); }
            if writes && !indirect { result.writes.push(observation); }
        }
        index += len;
    }
    Some(result)
}

impl StorageObservation {
    /// Resolve this explicit operand against its executable's authoritative map.
    /// None without a map is pending; None with a map is a failed obligation.
    pub fn resolve(
        &self,
        program: &MirProgram,
        owner: RoutineId,
        memory: Option<&crate::mir6502::analysis::record_bases::MirPhysicalMemory>,
    ) -> Option<Vec<MirMemoryRange>> {
        let base = fixed_storage_address(program, &self.address.base)
            .or_else(|| memory?.assembly_address(owner, &self.address.base))?;
        let address = self.address.at(base)?;
        let ranges = match self.extent {
            ObservationExtent::Bytes(bytes) => vec![(address, bytes)],
            ObservationExtent::FullZeroPage => vec![(0, 256)],
            ObservationExtent::PointerFetch => vec![(address, 1),
                ((address & 0xff00) | u16::from((address as u8).wrapping_add(1)), 1)],
        };
        Some(ranges.into_iter().map(|(address, bytes)| MirMemoryRange {
            base: absolute_mem(address), bytes,
        }).collect())
    }
}
