//! Closed producer shapes and physical read-purity evidence for terminal DCE.
use super::*;

/// With no map this is only a logical preflight; acceptance always repeats the
/// same discovery with both baseline and final candidate physical placements.
pub(super) fn pure_register(
    op: &MirOp,
    routine: RoutineId,
    memory: Option<&MirPhysicalMemory>,
) -> Option<MirReg> {
    match op {
        MirOp::LoadImm {
            dst: MirDef::Reg(reg),
            width: MirWidth::Byte,
            ..
        } => Some(*reg),
        MirOp::Move {
            dst: MirDef::Reg(dst),
            src: MirValue::Def(MirDef::Reg(src)),
            width: MirWidth::Byte,
        } if dst != src && (*src == MirReg::A || *dst == MirReg::A) => Some(*dst),
        MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } if !matches!(mem, MirMem::Absolute(_) | MirMem::FixedZeroPage(_))
            && memory.is_none_or(|m| {
                m.ordinary(routine, mem)
                    && m.address(routine, mem)
                        .is_some_and(|a| m.execution_disjoint(a))
            }) =>
        {
            Some(*reg)
        }
        _ => None,
    }
}
