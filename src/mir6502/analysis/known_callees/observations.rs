//! Explicit assembly storage references, independent of complete callee effects.
//! Unknown pointees do not expose private storage. Encoded addresses still do.
use super::*;
use crate::mir6502::analysis::effects::MirMemoryRange;

#[derive(Default)]
pub(in crate::mir6502) struct MachineStorageObservations {
    pub reads: Vec<MirMemoryRange>,
    pub writes: Vec<MirMemoryRange>,
}

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

/// This is an exposure inventory, never a complete read/write summary. Failure
/// means an explicit operand could not be decoded, not merely an unknown pointee.
pub(in crate::mir6502) fn machine_storage_observations(
    machine: &MirMachineBlock,
    routine: &MirRoutine,
    program: &MirProgram,
) -> Option<MachineStorageObservations> {
    let bytes = flatten_machine_items(&machine.items, routine, program)?;
    let mut result = MachineStorageObservations::default();
    let mut index = 0;
    while index < bytes.len() {
        let (mnemonic, mode, len) = crate::codegen::decode_6502_opcode(bytes[index].value?)?;
        let operands = bytes.get(index + 1..index + len)?;
        let stores = matches!(mnemonic, "STA" | "STX" | "STY");
        let writes = stores || matches!(mnemonic, "ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC");
        let control = matches!(mnemonic, "JMP" | "JSR");
        if matches!(mode, AddressingMode::ZeroPage | AddressingMode::ZeroPageX
            | AddressingMode::ZeroPageY | AddressingMode::IndirectIndexedY
            | AddressingMode::IndexedIndirectX)
            && fixed_storage_address(program, &machine_operand_mem(operands)?)
                .is_some_and(|address| address > 255)
        {
            return None;
        }
        let direct = match mode {
            AddressingMode::ZeroPage | AddressingMode::Absolute if !control => {
                Some(MirMemoryRange {
                    base: machine_operand_mem(operands)?,
                    bytes: 1,
                })
            }
            AddressingMode::AbsoluteX | AddressingMode::AbsoluteY => {
                // The ISA permits the full byte index; no inferred index bound.
                Some(MirMemoryRange {
                    base: machine_operand_mem(operands)?,
                    bytes: 256,
                })
            }
            AddressingMode::ZeroPageX | AddressingMode::ZeroPageY => {
                // Indexed zero-page addressing wraps within the page.
                machine_operand_mem(operands)?;
                Some(MirMemoryRange {
                    base: absolute_mem(0),
                    bytes: 256,
                })
            }
            AddressingMode::IndirectIndexedY | AddressingMode::Indirect => {
                let lo = machine_operand_mem(operands)?;
                let hi = if let Some(address) = fixed_storage_address(program, &lo) {
                    // Both ZP pointer fetches and NMOS JMP indirect wrap the
                    // high-byte fetch within the pointer's original page.
                    absolute_mem((address & 0xff00) | u16::from((address as u8).wrapping_add(1)))
                } else {
                    // A symbolic pointer's page boundary is unresolved here.
                    // Its two physical fetches need final placement evidence.
                    return None;
                };
                result
                    .reads
                    .extend([lo, hi].map(|base| MirMemoryRange { base, bytes: 1 }));
                None
            }
            AddressingMode::IndexedIndirectX => {
                machine_operand_mem(operands)?;
                result.reads.push(MirMemoryRange {
                    base: absolute_mem(0),
                    bytes: 256,
                });
                None
            }
            _ => None,
        };
        if let Some(access) = direct {
            if !stores {
                result.reads.push(access.clone());
            }
            if writes {
                result.writes.push(access);
            }
        }
        index += len;
    }
    Some(result)
}
