use std::collections::{BTreeMap, BTreeSet};

use crate::codegen::{AddressingMode, decode_6502_opcode};

use super::ir::{
    MirArgHome, MirGlobalBacking, MirMachineAtom, MirMachineByteSelector, MirMachineItem, MirOp,
    MirProgram, MirReg, MirRoutine, MirTerminator, RoutineId,
};

const SHADOW_BASE: u8 = 0xA0;
const SHADOW_LANES: u8 = 3;
const ALL_SHADOW_LANES: u8 = (1 << SHADOW_LANES) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MachineByte {
    Known(u8),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MachineValue {
    Known(u16),
    UnknownWord,
    UnknownWidth,
}

pub(super) fn elide_unobserved_shadow_args(
    program: &mut MirProgram,
    public_routines: &BTreeSet<RoutineId>,
) {
    let demanded = public_routines
        .iter()
        .map(|id| {
            let demand = program
                .routines
                .iter()
                .find(|routine| routine.id == *id)
                .and_then(|routine| machine_entry_shadow_demand(routine, program))
                .unwrap_or(ALL_SHADOW_LANES);
            (*id, demand)
        })
        .collect::<BTreeMap<_, _>>();

    for routine in &mut program.routines {
        for block in &mut routine.blocks {
            for op in &mut block.ops {
                let MirOp::Call {
                    target, abi, args, ..
                } = op
                else {
                    continue;
                };
                let super::ir::MirCallTarget::Routine(callee) = target else {
                    continue;
                };
                let Some(&demand) = demanded.get(callee) else {
                    continue;
                };
                if demand == ALL_SHADOW_LANES {
                    continue;
                }
                let Some(primary_start) = args
                    .iter()
                    .position(|arg| arg_home_uses_register_a(&arg.home))
                else {
                    continue;
                };

                let mut index = 0usize;
                args.retain(|arg| {
                    let keep = index >= primary_start || shadow_lane_mask(&arg.home) & demand != 0;
                    index += 1;
                    keep
                });
                abi.params = args.iter().map(|arg| arg.home.clone()).collect();
            }
        }
    }
}

fn machine_entry_shadow_demand(routine: &MirRoutine, program: &MirProgram) -> Option<u8> {
    let [block] = routine.blocks.as_slice() else {
        return None;
    };
    if !matches!(
        block.terminator,
        MirTerminator::Return | MirTerminator::Unreachable
    ) {
        return None;
    }
    let [MirOp::MachineBlock { id, .. }] = block.ops.as_slice() else {
        return None;
    };
    let machine_block = program
        .machine_blocks
        .iter()
        .find(|block| block.id == *id)?;
    let bytes = flatten_machine_items(&machine_block.items, routine, program)?;
    analyze_machine_entry(&bytes)
}

fn analyze_machine_entry(bytes: &[MachineByte]) -> Option<u8> {
    let mut cursor = 0usize;
    let mut demanded = 0u8;
    let mut overwritten = 0u8;

    while cursor < bytes.len() {
        let MachineByte::Known(opcode) = bytes[cursor] else {
            return Some(demanded | (ALL_SHADOW_LANES & !overwritten));
        };
        let Some((mnemonic, mode, len)) = decode_6502_opcode(opcode) else {
            return Some(demanded | (ALL_SHADOW_LANES & !overwritten));
        };
        let end = cursor.checked_add(len)?;
        if end > bytes.len() {
            return Some(demanded | (ALL_SHADOW_LANES & !overwritten));
        }
        let operand = &bytes[cursor + 1..end];
        let (reads, writes) = machine_memory_access(mnemonic, mode, operand);
        demanded |= reads & !overwritten;
        overwritten |= writes;

        if overwritten == ALL_SHADOW_LANES || demanded == ALL_SHADOW_LANES {
            return Some(demanded);
        }
        if matches!(mnemonic, "RTS" | "RTI") {
            return Some(demanded);
        }
        if is_entry_proof_barrier(mnemonic, mode) {
            return Some(demanded | (ALL_SHADOW_LANES & !overwritten));
        }
        cursor = end;
    }

    Some(demanded | (ALL_SHADOW_LANES & !overwritten))
}

fn machine_memory_access(
    mnemonic: &str,
    mode: AddressingMode,
    operand: &[MachineByte],
) -> (u8, u8) {
    let reads_memory = matches!(
        mnemonic,
        "ORA"
            | "ASL"
            | "BIT"
            | "AND"
            | "ROL"
            | "EOR"
            | "LSR"
            | "ADC"
            | "ROR"
            | "LDA"
            | "LDX"
            | "LDY"
            | "CMP"
            | "CPX"
            | "CPY"
            | "DEC"
            | "SBC"
            | "INC"
    );
    let writes_memory = matches!(
        mnemonic,
        "ASL" | "ROL" | "LSR" | "ROR" | "STA" | "STX" | "STY" | "DEC" | "INC"
    );
    if !reads_memory && !writes_memory {
        return (0, 0);
    }

    match mode {
        AddressingMode::ZeroPage | AddressingMode::Absolute => {
            let Some(address) = known_operand(operand) else {
                return (reads_memory.then_some(ALL_SHADOW_LANES).unwrap_or(0), 0);
            };
            let lanes = direct_shadow_lane(address);
            (
                if reads_memory { lanes } else { 0 },
                if writes_memory { lanes } else { 0 },
            )
        }
        AddressingMode::ZeroPageX | AddressingMode::ZeroPageY => {
            (if reads_memory { ALL_SHADOW_LANES } else { 0 }, 0)
        }
        AddressingMode::AbsoluteX | AddressingMode::AbsoluteY => {
            let lanes = known_operand(operand)
                .map(indexed_absolute_shadow_lanes)
                .unwrap_or(ALL_SHADOW_LANES);
            (if reads_memory { lanes } else { 0 }, 0)
        }
        AddressingMode::IndexedIndirectX => (ALL_SHADOW_LANES, 0),
        AddressingMode::IndirectIndexedY => {
            let pointer_lanes = known_operand(operand)
                .map(indirect_y_pointer_shadow_lanes)
                .unwrap_or(ALL_SHADOW_LANES);
            (
                if reads_memory {
                    ALL_SHADOW_LANES
                } else {
                    pointer_lanes
                },
                0,
            )
        }
        AddressingMode::Implied
        | AddressingMode::Accumulator
        | AddressingMode::Immediate
        | AddressingMode::Indirect
        | AddressingMode::Relative => (0, 0),
    }
}

fn is_entry_proof_barrier(mnemonic: &str, mode: AddressingMode) -> bool {
    matches!(mnemonic, "BRK" | "JSR" | "JMP") || mode == AddressingMode::Relative
}

fn known_operand(bytes: &[MachineByte]) -> Option<u16> {
    match bytes {
        [MachineByte::Known(value)] => Some(u16::from(*value)),
        [MachineByte::Known(lo), MachineByte::Known(hi)] => Some(u16::from_le_bytes([*lo, *hi])),
        _ => None,
    }
}

fn direct_shadow_lane(address: u16) -> u8 {
    let Ok(address) = u8::try_from(address) else {
        return 0;
    };
    if (SHADOW_BASE..SHADOW_BASE + SHADOW_LANES).contains(&address) {
        1 << (address - SHADOW_BASE)
    } else {
        0
    }
}

fn indexed_absolute_shadow_lanes(base: u16) -> u8 {
    (0..SHADOW_LANES).fold(0, |lanes, lane| {
        let address = u16::from(SHADOW_BASE + lane);
        if address.wrapping_sub(base) <= u16::from(u8::MAX) {
            lanes | (1 << lane)
        } else {
            lanes
        }
    })
}

fn indirect_y_pointer_shadow_lanes(base: u16) -> u8 {
    let base = base as u8;
    direct_shadow_lane(u16::from(base)) | direct_shadow_lane(u16::from(base.wrapping_add(1)))
}

fn flatten_machine_items(
    items: &[MirMachineItem],
    routine: &MirRoutine,
    program: &MirProgram,
) -> Option<Vec<MachineByte>> {
    let mut bytes = Vec::new();
    for item in items {
        match item {
            MirMachineItem::Byte(value) => bytes.push(MachineByte::Known(*value)),
            MirMachineItem::Word(value) => {
                bytes.extend(value.to_le_bytes().into_iter().map(MachineByte::Known))
            }
            MirMachineItem::Name(name) => {
                flatten_machine_value(machine_symbol_value(name, routine, program)?, &mut bytes)?
            }
            MirMachineItem::AddressExpr {
                selector,
                atom,
                offset,
                ..
            } => flatten_address_expr(*selector, atom, *offset, routine, program, &mut bytes)?,
            MirMachineItem::AddressByte { high, name } => {
                let value = machine_symbol_value(name, routine, program)?;
                bytes.push(match value {
                    MachineValue::Known(address) => MachineByte::Known(if *high {
                        (address >> 8) as u8
                    } else {
                        address as u8
                    }),
                    MachineValue::UnknownWord | MachineValue::UnknownWidth => MachineByte::Unknown,
                });
            }
            MirMachineItem::StringLiteral(_) | MirMachineItem::CharLiteral(_) => return None,
        }
    }
    Some(bytes)
}

fn flatten_address_expr(
    selector: Option<MirMachineByteSelector>,
    atom: &MirMachineAtom,
    offset: i32,
    routine: &MirRoutine,
    program: &MirProgram,
    bytes: &mut Vec<MachineByte>,
) -> Option<()> {
    let value = match atom {
        MirMachineAtom::Number(value) => MachineValue::Known(value.wrapping_add(offset as u16)),
        MirMachineAtom::Name(name) => {
            apply_machine_offset(machine_symbol_value(name, routine, program)?, offset)
        }
        MirMachineAtom::Current => MachineValue::UnknownWord,
    };
    match (selector, value) {
        (Some(MirMachineByteSelector::Low), MachineValue::Known(value)) => {
            bytes.push(MachineByte::Known(value as u8));
        }
        (Some(MirMachineByteSelector::High), MachineValue::Known(value)) => {
            bytes.push(MachineByte::Known((value >> 8) as u8));
        }
        (Some(_), MachineValue::UnknownWord | MachineValue::UnknownWidth) => {
            bytes.push(MachineByte::Unknown)
        }
        (None, value) => flatten_machine_value(value, bytes)?,
    }
    Some(())
}

fn apply_machine_offset(value: MachineValue, offset: i32) -> MachineValue {
    match value {
        MachineValue::Known(value) => MachineValue::Known(value.wrapping_add(offset as u16)),
        MachineValue::UnknownWord => MachineValue::UnknownWord,
        MachineValue::UnknownWidth => MachineValue::UnknownWidth,
    }
}

fn flatten_machine_value(value: MachineValue, bytes: &mut Vec<MachineByte>) -> Option<()> {
    match value {
        MachineValue::Known(value) if value <= u16::from(u8::MAX) => {
            bytes.push(MachineByte::Known(value as u8));
        }
        MachineValue::Known(value) => {
            bytes.extend(value.to_le_bytes().into_iter().map(MachineByte::Known))
        }
        MachineValue::UnknownWord => bytes.extend([MachineByte::Unknown, MachineByte::Unknown]),
        MachineValue::UnknownWidth => return None,
    }
    Some(())
}

fn machine_symbol_value(
    name: &str,
    routine: &MirRoutine,
    program: &MirProgram,
) -> Option<MachineValue> {
    if routine
        .frame
        .params
        .iter()
        .chain(&routine.frame.locals)
        .filter_map(|slot| slot.name.as_deref())
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
    {
        return Some(MachineValue::UnknownWidth);
    }
    if let Some(global) = program
        .globals
        .iter()
        .find(|global| global.name.eq_ignore_ascii_case(name))
    {
        return global_machine_value(global.id, program, 0);
    }
    if program
        .statics
        .iter()
        .any(|static_data| static_data.name.eq_ignore_ascii_case(name))
        || program
            .routines
            .iter()
            .any(|candidate| candidate.name.eq_ignore_ascii_case(name))
    {
        return Some(MachineValue::UnknownWord);
    }
    None
}

fn global_machine_value(
    id: crate::nir::SymbolId,
    program: &MirProgram,
    depth: usize,
) -> Option<MachineValue> {
    if depth > program.globals.len() {
        return None;
    }
    let global = program.globals.iter().find(|global| global.id == id)?;
    match global.backing {
        MirGlobalBacking::Absolute(address) => Some(MachineValue::Known(address)),
        MirGlobalBacking::Alias { target, offset } => {
            global_machine_value(target, program, depth.saturating_add(1))
                .map(|value| apply_machine_offset(value, i32::from(offset)))
        }
        MirGlobalBacking::Ordinary { .. } => Some(MachineValue::UnknownWord),
    }
}

fn arg_home_uses_register_a(home: &MirArgHome) -> bool {
    match home {
        MirArgHome::Reg(MirReg::A) => true,
        MirArgHome::RegisterPair { lo, hi } => *lo == MirReg::A || *hi == MirReg::A,
        MirArgHome::BytePair { lo, hi } => {
            arg_home_uses_register_a(lo) || arg_home_uses_register_a(hi)
        }
        MirArgHome::Reg(_)
        | MirArgHome::ZeroPage(_)
        | MirArgHome::FixedZeroPage(_)
        | MirArgHome::Absolute(_)
        | MirArgHome::StackFrame { .. } => false,
    }
}

fn shadow_lane_mask(home: &MirArgHome) -> u8 {
    match home {
        MirArgHome::BytePair { lo, hi } => shadow_lane_mask(lo) | shadow_lane_mask(hi),
        MirArgHome::FixedZeroPage(slot) => direct_shadow_lane(u16::from(slot.0)),
        MirArgHome::Reg(_)
        | MirArgHome::RegisterPair { .. }
        | MirArgHome::ZeroPage(_)
        | MirArgHome::Absolute(_)
        | MirArgHome::StackFrame { .. } => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_stores_discharge_shadow_demand() {
        let bytes = [
            MachineByte::Known(0x85),
            MachineByte::Known(0xA0),
            MachineByte::Known(0x86),
            MachineByte::Known(0xA1),
            MachineByte::Known(0x84),
            MachineByte::Known(0xA2),
            MachineByte::Known(0x60),
        ];

        assert_eq!(analyze_machine_entry(&bytes), Some(0));
    }

    #[test]
    fn entry_read_keeps_observed_lane() {
        let bytes = [
            MachineByte::Known(0x85),
            MachineByte::Known(0xA0),
            MachineByte::Known(0xA5),
            MachineByte::Known(0xA1),
            MachineByte::Known(0x60),
        ];

        assert_eq!(analyze_machine_entry(&bytes), Some(0b010));
    }

    #[test]
    fn call_keeps_every_lane_not_overwritten_by_entry_prefix() {
        let bytes = [
            MachineByte::Known(0x85),
            MachineByte::Known(0xA0),
            MachineByte::Known(0x20),
            MachineByte::Unknown,
            MachineByte::Unknown,
        ];

        assert_eq!(analyze_machine_entry(&bytes), Some(0b110));
    }
}
