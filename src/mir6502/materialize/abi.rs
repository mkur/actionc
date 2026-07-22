use super::helper_effects;
use crate::mir6502::ir::{
    MirAddr, MirArgHome, MirBlock, MirCallTarget, MirCond, MirDef, MirEffects, MirFixedZpSlot,
    MirMachineBlock, MirMachineBlockId, MirMachineItem, MirMem, MirOp, MirRuntimeHelper,
    MirStorageBase, MirTerminator, MirValue, MirWidth,
};
use crate::nir::ParamId;

pub(super) fn prepend_action_abi_param_prologue(
    routine: &mut crate::mir6502::ir::MirRoutine,
    machine_blocks: &mut Vec<MirMachineBlock>,
    helpers: &mut Vec<MirRuntimeHelper>,
) {
    if routine.frame.params.is_empty() {
        return;
    }
    if routine_contains_machine_block(routine) && !routine_references_param_storage(routine) {
        return;
    }
    let arg_bytes = routine
        .frame
        .params
        .iter()
        .map(|param| width_bytes(param.width))
        .sum::<u16>();
    let prologue = if arg_bytes >= 3 {
        action_abi_sargs_param_prologue(routine, arg_bytes, machine_blocks, helpers)
    } else {
        action_abi_direct_param_prologue(routine)
    };
    if prologue.is_empty() {
        return;
    }
    let Some(entry) = routine.blocks.first_mut() else {
        return;
    };
    let mut ops = prologue;
    ops.extend(entry.ops.clone());
    entry.ops = ops;
}

fn routine_references_param_storage(routine: &crate::mir6502::ir::MirRoutine) -> bool {
    routine.blocks.iter().any(block_references_param_storage)
}

fn routine_contains_machine_block(routine: &crate::mir6502::ir::MirRoutine) -> bool {
    routine
        .blocks
        .iter()
        .flat_map(|block| block.ops.iter())
        .any(|op| matches!(op, MirOp::MachineBlock { .. }))
}

fn block_references_param_storage(block: &MirBlock) -> bool {
    block.ops.iter().any(op_references_param_storage)
        || terminator_references_param_storage(&block.terminator)
}

fn op_references_param_storage(op: &MirOp) -> bool {
    match op {
        MirOp::LoadImm { .. } | MirOp::Barrier { .. } | MirOp::MachineBlock { .. } => false,
        MirOp::Load { src, .. } => addr_references_param_storage(src),
        MirOp::Store { dst, src, .. } => {
            addr_references_param_storage(dst) || value_references_param_storage(src)
        }
        MirOp::UpdateMem { mem, .. } | MirOp::UpdateIndexedMem { base: mem, .. } => {
            mem_references_param_storage(mem)
        }
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            mem_references_param_storage(mem) || value_references_param_storage(value)
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => value_references_param_storage(src),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            value_references_param_storage(base) || value_references_param_storage(index)
        }
        MirOp::LeaAddr { target, .. } => mem_references_param_storage(target),
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            value_references_param_storage(left) || value_references_param_storage(right)
        }
        MirOp::Call { target, args, .. } => {
            call_target_references_param_storage(target)
                || args
                    .iter()
                    .any(|arg| value_references_param_storage(&arg.value))
        }
        MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. } => false,
    }
}

fn terminator_references_param_storage(terminator: &MirTerminator) -> bool {
    match terminator {
        MirTerminator::Branch { cond, .. } => cond_references_param_storage(cond),
        MirTerminator::Jump(_)
        | MirTerminator::Return
        | MirTerminator::Exit
        | MirTerminator::Unreachable => false,
    }
}

fn cond_references_param_storage(cond: &MirCond) -> bool {
    match cond {
        MirCond::BoolValue(value) => value_references_param_storage(value),
        MirCond::Deferred
        | MirCond::FlagTest(_)
        | MirCond::AnyFlagTest(_)
        | MirCond::FusedCompare { .. } => false,
    }
}

fn call_target_references_param_storage(target: &MirCallTarget) -> bool {
    match target {
        MirCallTarget::Indirect { target, .. } => value_references_param_storage(target),
        MirCallTarget::Routine(_)
        | MirCallTarget::Builtin { .. }
        | MirCallTarget::Runtime { .. } => false,
    }
}

fn addr_references_param_storage(addr: &MirAddr) -> bool {
    match addr {
        MirAddr::Direct(mem)
        | MirAddr::AbsoluteIndexedX { base: mem }
        | MirAddr::AbsoluteIndexedY { base: mem }
        | MirAddr::PointerCell { ptr: mem, .. } => mem_references_param_storage(mem),
        MirAddr::ComputedIndex { base, index, .. } => {
            value_references_param_storage(base) || value_references_param_storage(index)
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            mem_references_param_storage(ptr) || value_references_param_storage(index)
        }
        MirAddr::Deref { ptr, .. } => value_references_param_storage(ptr),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => false,
    }
}

fn value_references_param_storage(value: &MirValue) -> bool {
    match value {
        MirValue::PointerCell(mem) => mem_references_param_storage(mem),
        MirValue::Word { lo, hi } => {
            value_references_param_storage(lo) || value_references_param_storage(hi)
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => false,
    }
}

fn mem_references_param_storage(mem: &MirMem) -> bool {
    matches!(mem, MirMem::Param { .. })
}

fn action_abi_sargs_param_prologue(
    routine: &crate::mir6502::ir::MirRoutine,
    arg_bytes: u16,
    machine_blocks: &mut Vec<MirMachineBlock>,
    helpers: &mut Vec<MirRuntimeHelper>,
) -> Vec<MirOp> {
    let Some(frame_name) = routine
        .frame
        .params
        .iter()
        .find_map(|param| param.name.clone())
    else {
        return action_abi_direct_param_prologue(routine);
    };
    let Ok(byte_count_minus_one) = u8::try_from(arg_bytes.saturating_sub(1)) else {
        return action_abi_direct_param_prologue(routine);
    };
    helpers.push(MirRuntimeHelper::SArgs);
    let id = MirMachineBlockId(machine_blocks.len() as u32);
    machine_blocks.push(MirMachineBlock {
        id,
        items: vec![
            MirMachineItem::AddressByte {
                high: false,
                name: frame_name.clone(),
            },
            MirMachineItem::AddressByte {
                high: true,
                name: frame_name,
            },
            MirMachineItem::Byte(byte_count_minus_one),
        ],
    });
    vec![
        MirOp::RuntimeHelper {
            helper: MirRuntimeHelper::SArgs,
            args: Vec::new(),
            result: None,
            effects: helper_effects(),
        },
        MirOp::MachineBlock {
            id,
            effects: MirEffects::default(),
        },
    ]
}

fn action_abi_direct_param_prologue(routine: &crate::mir6502::ir::MirRoutine) -> Vec<MirOp> {
    let mut bytes = Vec::new();
    let mut offset = 0u16;
    for param in &routine.frame.params {
        let id = match param.base {
            MirStorageBase::Param(id) => id,
            _ => continue,
        };
        let start = offset;
        match param.width {
            MirWidth::Byte => {
                bytes.push(store_param_byte_from_abi_home(
                    id,
                    0,
                    action_abi_byte_home(start),
                ));
                offset = offset.saturating_add(1);
            }
            MirWidth::Word => {
                bytes.push(store_param_byte_from_abi_home(
                    id,
                    0,
                    action_abi_byte_home(start),
                ));
                bytes.push(store_param_byte_from_abi_home(
                    id,
                    1,
                    action_abi_byte_home(start.saturating_add(1)),
                ));
                offset = offset.saturating_add(2);
            }
        }
    }
    if bytes.len() == 2 {
        bytes.swap(0, 1);
    }
    bytes
}

fn action_abi_byte_home(offset: u16) -> MirArgHome {
    match offset {
        0 => MirArgHome::Reg(crate::mir6502::ir::MirReg::A),
        1 => MirArgHome::Reg(crate::mir6502::ir::MirReg::X),
        2 => MirArgHome::Reg(crate::mir6502::ir::MirReg::Y),
        _ => MirArgHome::FixedZeroPage(MirFixedZpSlot(
            u8::try_from(0x00A0u16.saturating_add(offset)).unwrap_or(u8::MAX),
        )),
    }
}

pub(super) fn width_bytes(width: MirWidth) -> u16 {
    match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    }
}

fn store_param_byte_from_abi_home(id: ParamId, param_offset: u16, home: MirArgHome) -> MirOp {
    let src = match home {
        MirArgHome::Reg(reg) => MirValue::Def(MirDef::Reg(reg)),
        MirArgHome::StackFrame { base, offset } => {
            MirValue::PointerCell(MirMem::Absolute(base.saturating_add(offset)))
        }
        MirArgHome::RegisterPair { .. }
        | MirArgHome::BytePair { .. }
        | MirArgHome::ZeroPage(_)
        | MirArgHome::Absolute(_) => {
            unreachable!("param byte home should be lowered to register or stack byte")
        }
        MirArgHome::FixedZeroPage(slot) => MirValue::PointerCell(MirMem::FixedZeroPage(slot)),
    };
    MirOp::Store {
        dst: MirAddr::Direct(MirMem::Param {
            id,
            offset: param_offset,
        }),
        src,
        width: MirWidth::Byte,
    }
}
