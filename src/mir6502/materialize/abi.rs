use super::helper_effects;
use super::stats::MirPeepholeStats;
use crate::mir6502::analysis::effects::{MirMemoryRange, classify_op, classify_terminator};
use crate::mir6502::ir::{
    MirAddr, MirArgHome, MirBlock, MirCallTarget, MirCond, MirDef, MirEffects, MirFixedZpSlot,
    MirMachineBlock, MirMachineBlockId, MirMachineItem, MirMem, MirMemoryEffect,
    MirMemoryRegionKind, MirOp, MirRoutineAbi, MirRuntimeHelper, MirStorageBase, MirTerminator,
    MirValue, MirWidth,
};
use crate::nir::ParamId;
use std::collections::BTreeSet;

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

pub(super) fn elide_write_only_param_homes(
    routine: &mut crate::mir6502::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) {
    if routine.abi != MirRoutineAbi::Action || routine_contains_machine_block(routine) {
        return;
    }
    let arg_bytes = routine
        .frame
        .params
        .iter()
        .map(|param| width_bytes(param.width))
        .sum::<u16>();
    if arg_bytes == 0 || arg_bytes > 2 {
        return;
    }

    let expected_prologue = action_abi_direct_param_prologue(routine);
    let Some(entry) = routine.blocks.first() else {
        return;
    };
    if expected_prologue.is_empty() || !entry.ops.starts_with(&expected_prologue) {
        return;
    }
    let candidates = routine
        .frame
        .params
        .iter()
        .filter_map(|slot| {
            let MirStorageBase::Param(id) = slot.base else {
                return None;
            };
            let capture_count = expected_prologue
                .iter()
                .filter(|op| store_targets_param(op, id))
                .count();
            let store_count = routine_param_removable_store_count(routine, id)?;
            (capture_count == usize::from(width_bytes(slot.width))).then_some((
                id,
                width_bytes(slot.width),
                slot.name.clone(),
                store_count,
                capture_count,
            ))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return;
    }
    let candidate_ids = candidates
        .iter()
        .map(|(id, _, _, _, _)| *id)
        .collect::<BTreeSet<_>>();

    for block in &mut routine.blocks {
        block.ops = block
            .ops
            .drain(..)
            .filter(|op| !store_targets_any_param(op, &candidate_ids))
            .collect();
    }
    for slot in &mut routine.frame.params {
        let MirStorageBase::Param(id) = slot.base else {
            continue;
        };
        if candidate_ids.contains(&id) {
            slot.base = MirStorageBase::ParamAbiOnly(id);
        }
    }
    for (id, bytes, name, store_count, capture_count) in candidates {
        peephole_stats.record_many(
            routine.id,
            "write-only-param-home-elided",
            usize::from(bytes),
        );
        peephole_stats.record_site(
            routine.id,
            "write-only-param-home-elided",
            format!(
                "param=p{} name={} bytes={} stores={} kind={}",
                id.0,
                name.as_deref().unwrap_or("<unnamed>"),
                bytes,
                store_count,
                if store_count == capture_count {
                    "capture-only"
                } else {
                    "write-only"
                }
            ),
        );
    }
}

fn routine_param_removable_store_count(
    routine: &crate::mir6502::ir::MirRoutine,
    id: ParamId,
) -> Option<usize> {
    let mut store_count = 0usize;
    for block in &routine.blocks {
        for op in &block.ops {
            if removable_param_store(op, id) {
                store_count = store_count.saturating_add(1);
            } else if op_effects_reference_param(op, id) {
                return None;
            }
        }
        if terminator_effects_reference_param(&block.terminator, id) {
            return None;
        }
    }
    Some(store_count)
}

fn removable_param_store(op: &MirOp, id: ParamId) -> bool {
    if !store_targets_param(op, id) {
        return false;
    }
    let effects = classify_op(op);
    !effects.memory.opaque
        && !effects
            .memory
            .direct_references
            .iter()
            .any(|mem| mem_references_param(mem, id))
        && !structured_effect_references_param(&effects.memory.structured_reads, id)
        && !structured_effect_references_param(&effects.memory.structured_writes, id)
}

fn op_effects_reference_param(op: &MirOp, id: ParamId) -> bool {
    let effects = classify_op(op);
    effects.memory.opaque
        || effects
            .memory
            .direct_references
            .iter()
            .any(|mem| mem_references_param(mem, id))
        || effects
            .memory
            .direct_reads
            .iter()
            .any(|range| range_references_param(range, id))
        || effects
            .memory
            .direct_writes
            .iter()
            .any(|range| range_references_param(range, id))
        || structured_effect_references_param(&effects.memory.structured_reads, id)
        || structured_effect_references_param(&effects.memory.structured_writes, id)
}

fn terminator_effects_reference_param(terminator: &MirTerminator, id: ParamId) -> bool {
    let effects = classify_terminator(terminator);
    effects.memory.opaque
        || effects
            .memory
            .direct_references
            .iter()
            .any(|mem| mem_references_param(mem, id))
        || effects
            .memory
            .direct_reads
            .iter()
            .any(|range| range_references_param(range, id))
        || effects
            .memory
            .direct_writes
            .iter()
            .any(|range| range_references_param(range, id))
        || structured_effect_references_param(&effects.memory.structured_reads, id)
        || structured_effect_references_param(&effects.memory.structured_writes, id)
}

fn range_references_param(range: &MirMemoryRange, id: ParamId) -> bool {
    mem_references_param(&range.base, id)
}

fn mem_references_param(mem: &MirMem, id: ParamId) -> bool {
    matches!(mem, MirMem::Param { id: candidate, .. } if *candidate == id)
}

fn structured_effect_references_param(effect: &MirMemoryEffect, id: ParamId) -> bool {
    match effect {
        MirMemoryEffect::None => false,
        MirMemoryEffect::Regions(regions) => regions.iter().any(
            |region| matches!(region.kind, MirMemoryRegionKind::Param(candidate) if candidate == id),
        ),
        MirMemoryEffect::Unknown | MirMemoryEffect::All => true,
    }
}

fn store_targets_param(op: &MirOp, id: ParamId) -> bool {
    matches!(
        op,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Param { id: candidate, .. }),
            ..
        } if *candidate == id
    )
}

fn store_targets_any_param(op: &MirOp, ids: &BTreeSet<ParamId>) -> bool {
    matches!(
        op,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Param { id, .. }),
            ..
        } if ids.contains(id)
    )
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
        MirOp::OffsetPointerByIndirectByte { dst, .. } => mem_references_param_storage(dst),
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
        MirOp::CopyDirectWordToIndirect { source, .. } => mem_references_param_storage(source),
        MirOp::AbsoluteWordSubToIndirect { rhs, .. } => mem_references_param_storage(rhs),
        MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. } => false,
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
        MirValue::PointerCell(mem) | MirValue::StorageAddrByte { mem, .. } => {
            mem_references_param_storage(mem)
        }
        MirValue::Word { lo, hi } => {
            value_references_param_storage(lo) || value_references_param_storage(hi)
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => false,
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
        let (id, needs_capture) = match param.base {
            MirStorageBase::Param(id) => (id, true),
            MirStorageBase::ParamAbiOnly(id) => (id, false),
            _ => continue,
        };
        let start = offset;
        match param.width {
            MirWidth::Byte => {
                if needs_capture {
                    bytes.push(store_param_byte_from_abi_home(
                        id,
                        0,
                        action_abi_byte_home(start),
                    ));
                }
                offset = offset.saturating_add(1);
            }
            MirWidth::Word => {
                if needs_capture {
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
                }
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
