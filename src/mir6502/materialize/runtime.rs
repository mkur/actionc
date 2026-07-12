use super::layout::MaterializeLayout;
use super::values::{split_def, split_value_with_temp_widths};
use crate::mir6502::ir::{
    MirAddr, MirArgHome, MirBinaryOp, MirCallAbi, MirDef, MirEffects, MirFixedZpSlot, MirMem,
    MirMemoryEffect, MirOp, MirProgram, MirReg, MirRegisterSet, MirResultHome, MirRuntimeHelper,
    MirRuntimeHelperDecl, MirRuntimeHelperTarget, MirTempId, MirValue, MirWidth,
};
use std::collections::BTreeMap;

pub(super) fn ensure_helper_decl(program: &mut MirProgram, helper: MirRuntimeHelper) {
    if program
        .runtime_helpers
        .iter()
        .any(|decl| decl.helper == helper)
    {
        return;
    }
    program.runtime_helpers.push(MirRuntimeHelperDecl {
        target: helper_target(&helper),
        helper,
        abi: helper_abi(),
        effects: helper_effects(),
    });
}

fn helper_target(helper: &MirRuntimeHelper) -> MirRuntimeHelperTarget {
    use crate::codegen::runtime_helper;

    let address = match helper {
        MirRuntimeHelper::Mul => runtime_helper::CARTRIDGE_MUL,
        MirRuntimeHelper::Div => runtime_helper::CARTRIDGE_DIV,
        MirRuntimeHelper::Mod => runtime_helper::CARTRIDGE_MOD,
        MirRuntimeHelper::Lsh => runtime_helper::CARTRIDGE_LSH,
        MirRuntimeHelper::Rsh => runtime_helper::CARTRIDGE_RSH,
        MirRuntimeHelper::SArgs => runtime_helper::CARTRIDGE_SARGS,
    };
    MirRuntimeHelperTarget::KnownAbsolute(address.address())
}

pub(in crate::mir6502) fn helper_abi() -> MirCallAbi {
    MirCallAbi {
        params: vec![
            MirArgHome::StackFrame { base: 0, offset: 0 },
            MirArgHome::StackFrame { base: 0, offset: 2 },
        ],
        result: Some(MirResultHome::ReturnSlot { offset: 0 }),
        clobbers: MirRegisterSet {
            a: true,
            x: true,
            y: true,
            flags: true,
            sp: false,
        },
        preserves: MirRegisterSet::default(),
    }
}

pub(in crate::mir6502) fn helper_effects() -> MirEffects {
    MirEffects {
        memory_reads: MirMemoryEffect::Unknown,
        memory_writes: MirMemoryEffect::Unknown,
        clobbers: helper_abi().clobbers,
        preserves: MirRegisterSet::default(),
        stack_depth_delta: None,
        may_call_os: false,
        opaque: true,
    }
}

pub(super) fn helper_for_binary(op: MirBinaryOp, width: MirWidth) -> Option<MirRuntimeHelper> {
    match (op, width) {
        (MirBinaryOp::Mul, _) => Some(MirRuntimeHelper::Mul),
        (MirBinaryOp::Div, _) => Some(MirRuntimeHelper::Div),
        (MirBinaryOp::Mod, _) => Some(MirRuntimeHelper::Mod),
        (MirBinaryOp::Lsh | MirBinaryOp::Rsh, MirWidth::Word) => match op {
            MirBinaryOp::Lsh => Some(MirRuntimeHelper::Lsh),
            MirBinaryOp::Rsh => Some(MirRuntimeHelper::Rsh),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn materialize_runtime_helper_binary(
    helper: MirRuntimeHelper,
    dst: Option<MirDef>,
    left: MirValue,
    right: MirValue,
    operand_width: MirWidth,
    result_width: MirWidth,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    let (left_lo, left_hi) = split_value_for_width(left, operand_width, layout, temp_widths);
    let (right_lo, right_hi) = split_value_for_width(right, operand_width, layout, temp_widths);

    match helper {
        MirRuntimeHelper::Mul | MirRuntimeHelper::Div | MirRuntimeHelper::Mod => {
            materialize_helper_arg_to_mem(
                right_lo,
                MirMem::FixedZeroPage(MirFixedZpSlot(0x84)),
                out,
            );
            materialize_helper_arg_to_mem(
                right_hi,
                MirMem::FixedZeroPage(MirFixedZpSlot(0x85)),
                out,
            );
        }
        MirRuntimeHelper::Lsh | MirRuntimeHelper::Rsh => {
            materialize_helper_arg_to_mem(
                right_lo,
                MirMem::FixedZeroPage(MirFixedZpSlot(0x84)),
                out,
            );
        }
        MirRuntimeHelper::SArgs => {}
    }

    materialize_helper_arg_to_reg(left_lo, MirReg::A, out);
    materialize_helper_arg_to_reg(left_hi, MirReg::X, out);
    out.push(MirOp::RuntimeHelper {
        helper,
        args: Vec::new(),
        result: None,
        effects: helper_effects(),
    });
    if let Some(dst) = dst {
        materialize_runtime_helper_result(dst, result_width, out);
    }
}

fn materialize_helper_arg_to_reg(value: MirValue, reg: MirReg, out: &mut Vec<MirOp>) {
    match value {
        MirValue::PointerCell(mem) => out.push(MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }),
        value => out.push(MirOp::Move {
            dst: MirDef::Reg(reg),
            src: value,
            width: MirWidth::Byte,
        }),
    }
}

fn materialize_helper_arg_to_mem(value: MirValue, dst: MirMem, out: &mut Vec<MirOp>) {
    let src = match value {
        MirValue::PointerCell(mem) => {
            out.push(MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            });
            MirValue::Def(MirDef::Reg(MirReg::A))
        }
        value => value,
    };
    out.push(MirOp::Store {
        dst: MirAddr::Direct(dst),
        src,
        width: MirWidth::Byte,
    });
}

fn split_value_for_width(
    value: MirValue,
    width: MirWidth,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
) -> (MirValue, MirValue) {
    match width {
        MirWidth::Byte => (value, MirValue::ConstU8(0)),
        MirWidth::Word => split_value_with_temp_widths(value, layout, temp_widths),
    }
}

pub(super) fn runtime_helper_result_width(
    helper: &MirRuntimeHelper,
    width: MirWidth,
    dst: &MirDef,
) -> MirWidth {
    match (helper, width) {
        (MirRuntimeHelper::Mul, MirWidth::Byte) if split_def(dst.clone()).is_some() => {
            MirWidth::Word
        }
        _ => width,
    }
}

fn materialize_runtime_helper_result(dst: MirDef, width: MirWidth, out: &mut Vec<MirOp>) {
    match width {
        MirWidth::Byte => out.push(MirOp::Move {
            dst,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }),
        MirWidth::Word => {
            if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                out.push(MirOp::Move {
                    dst: lo_dst,
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                });
                out.push(MirOp::Move {
                    dst: hi_dst,
                    src: MirValue::Def(MirDef::Reg(MirReg::X)),
                    width: MirWidth::Byte,
                });
            } else {
                out.push(MirOp::Move {
                    dst,
                    src: MirValue::Word {
                        lo: Box::new(MirValue::Def(MirDef::Reg(MirReg::A))),
                        hi: Box::new(MirValue::Def(MirDef::Reg(MirReg::X))),
                    },
                    width: MirWidth::Word,
                });
            }
        }
    }
}
