//! The ordered direct Action entry capture shared by materialization and entry
//! analysis. This builder does not decide whether an entry needs compiler capture
//! or whether its argument frame requires SArgs.
use crate::mir6502::ir::{
    MirAddr, MirArgHome, MirDef, MirMem, MirOp, MirStorageBase, MirValue, MirWidth,
};
use crate::nir::ParamId;

pub(in crate::mir6502) fn direct_param_capture(
    routine: &crate::mir6502::ir::MirRoutine,
) -> Vec<MirOp> {
    let mut bytes = Vec::new();
    let mut offset = 0u16;
    for param in &routine.frame.params {
        let (id, needs_capture) = match param.base {
            MirStorageBase::Param(id) => (id, routine.frame.parameter_has_memory(id)),
            MirStorageBase::ParamAbiOnly(id) => (id, false),
            _ => continue,
        };
        let start = offset;
        let width = param.abi_byte_width().unwrap_or(0);
        if needs_capture {
            for byte in 0..width {
                if !routine.frame.parameter_byte_has_memory(id, byte) {
                    continue;
                }
                let incoming = super::action_arg_home(start + byte, MirWidth::Byte);
                let same_slot = matches!((routine.frame.parameter_lane(id, byte), &incoming),
                    (Some(crate::mir6502::ir::MirParamLane::ZeroPage(home)), MirArgHome::FixedZeroPage(source))
                    if home == *source);
                if !same_slot {
                    bytes.push(store_param_byte_from_abi_home(id, byte, incoming));
                }
            }
        }
        offset = offset.saturating_add(width);
    }
    if bytes.len() == 2
        && (routine.frame.parameter_homes.is_none()
            || bytes.iter().all(|op| {
                matches!(
                    op,
                    MirOp::Store {
                        src: MirValue::Def(MirDef::Reg(_)),
                        ..
                    }
                )
            }))
    {
        bytes.swap(0, 1);
    }
    bytes
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
