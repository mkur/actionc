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
    let bytes = flatten_machine_items(&machine.items, routine, program)?;
    let starts = machine_instruction_starts(&bytes)?;
    let last = *starts.last()?;
    if bytes[last].value != Some(0x60) {
        return None;
    }
    for pc in starts.range(..last) {
        let (op, mode, _) = crate::codegen::decode_6502_opcode(bytes[*pc].value?)?;
        if mode == AddressingMode::Relative
            || matches!(
                op,
                "JSR"
                    | "JMP"
                    | "BRK"
                    | "RTI"
                    | "RTS"
                    | "PHA"
                    | "PLA"
                    | "PHP"
                    | "PLP"
                    | "TXS"
                    | "TSX"
            )
        {
            return None;
        }
    }

    // Versioned FNV-1a drift check. Each field is tagged or fixed-width.
    // The typed relocation resolves the object. Its spelling identifies the
    // reviewed source dependency across builds with different numeric IDs.
    let mut hash = 0xcbf29ce484222325u64;
    let mut add = |bytes: &[u8]| {
        for b in bytes {
            hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
    };
    add(b"actionc-6502-abi-inputs-v1");
    for param in routine.frame.direct_parameter_abi.as_ref()? {
        add(&[match param {
            MirDirectParamAbi::Scalar(w) => w.bytes() as u8,
            MirDirectParamAbi::Pointer => b'P',
        }]);
    }
    add(&[
        0,
        match routine.direct_result_abi? {
            MirDirectResultAbi::Void => 0,
            MirDirectResultAbi::Scalar(w) => w.bytes() as u8,
            MirDirectResultAbi::Pointer => b'P',
        },
    ]);
    for item in &machine.items {
        match item {
            MirMachineItem::Byte(b) => add(&[0, *b]),
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Low8 | InlineAsmRelocationKind::Byte8,
                target: MirInlineAsmTarget::Absolute(address),
                addend: 0,
                requires_zero_page: true,
                ..
            } => add(&[0, u8::try_from(*address).ok()?]),
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Absolute16,
                target: MirInlineAsmTarget::Absolute(address),
                addend: 0,
                requires_zero_page: false,
                ..
            } => {
                for b in address.to_le_bytes() {
                    add(&[0, b]);
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
                add(&[1]);
                add(&global.storage_size.to_le_bytes());
                add(&u32::try_from(global.name.len()).ok()?.to_le_bytes());
                add(global.name.as_bytes());
            }
            _ => return None,
        }
    }
    Some(hash)
}
