//! Argument-relative accesses are distinct from fixed scratch. These are
//! possible accesses, never definitions, purity claims, or overlap guarantees.
use super::*;
use crate::mir6502::analysis::{
    known_callees::fixed_storage_address,
    machine_values::{MirMachineValue, MirMachineValueAvailability},
    sites::MirSite,
};

/// Reuse the current machine tracker, resolving fixed relocation constants in
/// a local analysis view. No executable operation or cached source value changes.
pub(in crate::mir6502) fn argument_values(program: &MirProgram, routine: &MirRoutine) -> Option<MirMachineValueAvailability> {
    let cfg = crate::mir6502::analysis::cfg::MirCfg::from_routine(routine).ok()?;
    let mut view = routine.clone();
    for op in view.blocks.iter_mut().flat_map(|b| &mut b.ops) {
        if let MirOp::Move { src, width: MirWidth::Byte, .. }
            | MirOp::Store { src, width: MirWidth::Byte, .. } = op
            && matches!(src, MirValue::StorageAddrByte { .. })
            && let Some(byte) = constant_byte(program, src, 0) {
            *src = MirValue::ConstU8(byte);
        }
    }
    Some(MirMachineValueAvailability::analyze(&view, &cfg))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum BufferExtent {
    Fixed(u16),
    /// Includes the length byte. The length is read through this argument.
    String { length_pointer_byte: u16 },
    Count { argument_byte: u16 },
    ByteCount { argument_byte: u16, zero_means_256: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) struct BufferAccess {
    pub pointer_byte: u16,
    pub extent: BufferExtent,
    pub write: bool,
}

impl BufferAccess {
    /// Unknown pointer/extent stays unknown. A numeric base retains a physical
    /// upper bound; it cannot borrow the ordinary private-object exclusion.
    pub fn project(&self, program: &MirProgram, call: &MirOp,
        values: Option<(&MirMachineValueAvailability, MirSite)>,
    ) -> (Option<u16>, u16) {
        let bytes = match self.extent {
            BufferExtent::Fixed(bytes) => bytes,
            BufferExtent::String { .. } => 256,
            BufferExtent::ByteCount { argument_byte, zero_means_256 } =>
                crate::callee6502::buffer_length(argument_byte_value(program, call, argument_byte, MirWidth::Byte, values), zero_means_256),
            BufferExtent::Count { argument_byte } =>
                argument_word(program, call, argument_byte, values).unwrap_or(u16::MAX),
        };
        (argument_word(program, call, self.pointer_byte, values), bytes)
    }
}

impl super::invocation::InvocationSummary {
    pub fn normal_writes(&self, program: &MirProgram, call: &MirOp,
        values: Option<(&MirMachineValueAvailability, MirSite)>,
    ) -> crate::mir6502::analysis::known_callees::MirKnownMemoryWrites {
        use crate::mir6502::analysis::known_callees::MirKnownMemoryWrites;
        let mut numeric = Vec::new();
        let mut has_pointee = false;
        for buffer in self.buffers.iter().filter(|b| b.write) {
            let (address, bytes) = buffer.project(program, call, values);
            if bytes == 0 { continue; }
            has_pointee = true;
            if let Some(address) = address {
                // An explicit destination overlapping implementation scratch
                // may overwrite its pointers/counts and escape the normal
                // buffer bound. Do not certify preservation for that case.
                if self.writes.iter().filter_map(|m| fixed_storage_address(program, m))
                    .any(|scratch| scratch.wrapping_sub(address) < bytes) {
                    return MirKnownMemoryWrites::Unknown;
                }
                numeric.push((address, bytes));
            }
        }
        if has_pointee {
            MirKnownMemoryWrites::Pointee { fixed: self.writes.clone(), numeric, formals: BTreeSet::new(), ordinary: false, caller_dependent: true, opaque_preserves: None }
        } else { MirKnownMemoryWrites::Exact(self.writes.clone()) }
    }
}

pub(in crate::mir6502) fn argument_word(program: &MirProgram, call: &MirOp, offset: u16,
    values: Option<(&MirMachineValueAvailability, MirSite)>,
) -> Option<u16> {
    Some(u16::from(argument_byte_value(program, call, offset, MirWidth::Word, values)?)
        | (u16::from(argument_byte_value(program, call, offset.checked_add(1)?, MirWidth::Word, values)?) << 8))
}

pub(in crate::mir6502) fn argument_byte_value(program: &MirProgram, call: &MirOp, offset: u16, argument_width: MirWidth,
    values: Option<(&MirMachineValueAvailability, MirSite)>,
) -> Option<u8> {
    let MirOp::Call { args, .. } = call else { return None; };
    let mut start = 0;
    for arg in args {
        let width = match arg.width { MirWidth::Byte => 1, MirWidth::Word => 2 };
        if offset < start + width {
            let byte = (offset - start) as u8;
            // A word represented by separate byte arguments has already been staged.
            // Read its physical lanes even if a diagnostic operand is a literal.
            if let Some((values, site)) = values {
                let current = match &arg.home {
                    MirArgHome::Reg(reg) if width == 1 && (argument_width == MirWidth::Word
                        || matches!(arg.value, MirValue::Def(MirDef::Reg(_)))) =>
                        Some(values.register_at(site, *reg).ok().flatten()),
                    MirArgHome::FixedZeroPage(slot) =>
                        Some(values.fixed_zero_page_value_at(site, MirFixedZpSlot(slot.0.checked_add(byte)?)).ok().flatten()),
                    _ => None,
                };
                if let Some(current) = current {
                    return known_byte(values, site, current);
                }
            }
            return constant_byte(program, &arg.value, byte);
        }
        start += width;
    }
    None
}

fn known_byte(values: &MirMachineValueAvailability, site: MirSite,
    mut value: Option<MirMachineValue>,
) -> Option<u8> {
    let mut seen = BTreeSet::new();
    loop {
        let memory = match value? {
            MirMachineValue::ConstU8(byte) => return Some(byte),
            MirMachineValue::DirectMem(memory) => memory,
            _ => return None,
        };
        let slot = match memory {
            MirMem::FixedZeroPage(slot) => slot,
            MirMem::Absolute(address) => MirFixedZpSlot(u8::try_from(address).ok()?),
            _ => return None,
        };
        if !seen.insert(slot) { return None; }
        // The tracker invalidates these equalities when either cell changes.
        // A shared constant store can therefore supply both count bytes.
        value = values.fixed_zero_page_value_at(site, slot).ok().flatten();
    }
}

fn constant_byte(program: &MirProgram, value: &MirValue, byte: u8) -> Option<u8> {
    let address = match value {
        MirValue::ConstU8(b) => u16::from(*b),
        MirValue::ConstU16(w) => *w,
        MirValue::GlobalAddr(id) => fixed_storage_address(program, &MirMem::Global { id: *id, offset: 0 })?,
        MirValue::StorageAddrByte { mem, byte: lane } =>
            return (byte == 0).then(|| fixed_storage_address(program, mem).map(|w| (w >> (8 * *lane)) as u8)).flatten(),
        MirValue::Word { lo, hi } => return constant_byte(program, if byte == 0 { lo } else { hi }, 0),
        _ => return None,
    };
    Some((address >> (8 * byte)) as u8)
}
