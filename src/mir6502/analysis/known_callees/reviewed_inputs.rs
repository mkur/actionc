//! Identity of a reviewed direct-call entry. The review is an assertion about
//! caller obligations. It does not modify the instruction-level input summary.
use super::*;
use crate::asm6502::InlineAsmRelocationKind;
use crate::mir6502::ir::{MirDirectParamAbi, MirDirectResultAbi, MirInlineAsmTarget};

pub(in crate::mir6502) fn abi_inputs_review_fingerprint(
    program: &MirProgram,
    routine: &MirRoutine,
) -> Option<u64> {
    let [block] = routine.blocks.as_slice() else {
        return None;
    };
    let [MirOp::MachineBlock { id, effects }] = block.ops.as_slice() else {
        return None;
    };
    if block.terminator != MirTerminator::Unreachable
        || effects.opaque
        || effects.may_call_os
        || routine.effects.opaque
        || routine.effects.may_call_os
        || !crate::mir6502::abi::direct_parameter_frame_matches(routine)
    {
        return None;
    }
    let machine = program.machine_blocks.iter().find(|m| m.id == *id)?;
    use crate::callee6502::{BodyItem, ENTRY_VERSION, fingerprint};
    let params = routine.frame.direct_parameter_abi.as_ref()?.iter().map(|abi| match abi {
        MirDirectParamAbi::Scalar(w) => w.bytes() as u8,
        MirDirectParamAbi::Pointer => b'P',
    }).collect::<Vec<_>>();
    let result = match routine.direct_result_abi? {
        MirDirectResultAbi::Void => 0,
        MirDirectResultAbi::Scalar(w) => w.bytes() as u8,
        MirDirectResultAbi::Pointer => b'P',
    };
    let mut items = Vec::with_capacity(machine.items.len());
    for item in &machine.items {
        match item {
            MirMachineItem::Byte(b) => items.push(BodyItem::Byte(*b)),
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Low8 | InlineAsmRelocationKind::Byte8,
                target: MirInlineAsmTarget::Absolute(address),
                addend: 0,
                requires_zero_page: true,
                ..
            } => items.push(BodyItem::Byte(u8::try_from(*address).ok()?)),
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Absolute16,
                target: MirInlineAsmTarget::Absolute(address),
                addend: 0,
                requires_zero_page: false,
                ..
            } => {
                for b in address.to_le_bytes() {
                    items.push(BodyItem::Byte(b));
                }
            }
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Absolute16,
                target: MirInlineAsmTarget::Memory(MirMem::Global { id, offset: 0 }),
                addend: 0,
                requires_zero_page: false,
                ..
            } => {
                let global = program.globals.iter().find(|g| g.id == *id)?;
                if !matches!(global.backing, MirGlobalBacking::Ordinary { .. }) {
                    return None;
                }
                items.push(BodyItem::Global { name: &global.name, extent: global.storage_size });
            }
            _ => return None,
        }
    }
    fingerprint(&params, result, &items, ENTRY_VERSION)
}
