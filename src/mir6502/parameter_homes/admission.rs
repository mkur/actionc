//! Discovery permissions, separate from the candidate's expanded residence proof.
use super::*;
use crate::mir6502::analysis::{effects::{classify_op, writes_ordinary_pointee}, sites::MirSite};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reason {
    Abi, BoundFrame, NoObjects, CaptureExtent, ParameterStorage,
    PointerArithmetic, CallsDisabled, Effects, Operation, HelperContract,
}

impl Reason {
    pub fn description(self) -> &'static str {
        match self {
            Self::Abi => "unsupported source-home ABI",
            Self::BoundFrame => "parameter homes already selected",
            Self::NoObjects => "no candidate source objects",
            Self::CaptureExtent => "unsupported incoming capture extent",
            Self::ParameterStorage => "unsupported parameter storage",
            Self::PointerArithmetic => "unqualified pointer and variable word arithmetic",
            Self::CallsDisabled => "invocation requires cross-call discovery",
            Self::Effects => "unsupported operation effects",
            Self::Operation => "unqualified operation expansion",
            Self::HelperContract => "unsupported or stale helper invocation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Rejection {
    pub reason: Reason,
    pub site: Option<MirSite>,
}

impl From<Reason> for Rejection {
    fn from(reason: Reason) -> Self { Self { reason, site: None } }
}

/// Preserve the closed expansion set and report one deterministic blocker.
pub(super) fn check(addresses: &ObjectAddressControl<'_>, routine: &MirRoutine,
    calls: bool, mixed: bool, selected: Option<&MirProgram>) -> Result<(), Rejection> {
    if routine.abi != MirRoutineAbi::Action { return Err(Reason::Abi.into()); }
    if routine.frame.parameter_homes.is_some() { return Err(Reason::BoundFrame.into()); }
    if routine.frame.params.is_empty() && (!calls || routine.frame.locals.is_empty()) {
        return Err(Reason::NoObjects.into());
    }
    let bytes = routine.frame.abi_parameter_bytes().ok_or(Reason::CaptureExtent)?;
    if bytes > crate::action_abi::MAX_DIRECT_PARAM_CAPTURE_BYTES
        && (usize::from(bytes) > SARGS_HOME_POOL.len() || !selected.is_some_and(|p| {
            p.routines.iter().find(|r| r.id == routine.id).is_some_and(|r|
                r.blocks.first().and_then(|b| b.ops.first())
                    .and_then(|op| crate::mir6502::runtime_contracts::sargs_capture(p, r, op))
                    .is_some_and(|capture| capture.inputs.len() == usize::from(bytes)))
        })) {
        return Err(Reason::CaptureExtent.into());
    }
    if !mixed && !routine.frame.params.iter().all(|p| eligible_parameter(addresses, routine, p)) {
        return Err(Reason::ParameterStorage.into());
    }
    // Pointer-dependent word operations have separate staging recipes. A pair
    // of direct word loads feeding a direct arithmetic store qualifies on its own.
    if has_pointer_reads(routine) {
        for block in &routine.blocks {
            for (op_index, op) in block.ops.iter().enumerate() {
                if matches!(op, MirOp::Binary { left, right, width: MirWidth::Word, .. }
                    if !matches!(left, MirValue::ConstU8(_) | MirValue::ConstU16(_))
                    && !matches!(right, MirValue::ConstU8(_) | MirValue::ConstU16(_)))
                    && !crate::mir6502::materialize::direct_word_store_supports_source_homes(&block.ops, op_index)
                {
                    let site = MirSite::Op { block: block.id, op_index };
                    return Err(Rejection { reason: Reason::PointerArithmetic, site: Some(site) });
                }
            }
        }
    }
    let ops = routine.blocks.iter().flat_map(|b| b.ops.iter().enumerate()
        .map(move |(op_index, op)| (MirSite::Op { block: b.id, op_index }, op)));
    for (site, op) in ops {
        let reject = |reason| Rejection { reason, site: Some(site) };
        if matches!(op, MirOp::Call { .. }) {
            if !calls { return Err(reject(Reason::CallsDisabled)); }
            continue;
        }
        if matches!(op, MirOp::RuntimeHelper { .. }) {
            if !calls { return Err(reject(Reason::CallsDisabled)); }
            if selected.and_then(|p| crate::mir6502::runtime_contracts::helper_invocation(p,op)).is_none() {
                return Err(reject(Reason::HelperContract));
            }
            continue;
        }
        let effects = classify_op(op);
        if effects.memory.opaque || (effects.memory.indirect_writes && !writes_ordinary_pointee(op)) {
            return Err(reject(Reason::Effects));
        }
        let supported = matches!(op,
            MirOp::LoadImm { .. } | MirOp::Move { .. } | MirOp::Extend { .. }
                | MirOp::Truncate { .. } | MirOp::Compare { .. } | MirOp::UpdateMem { .. }
                | MirOp::Binary { op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::And
                    | MirBinaryOp::Or | MirBinaryOp::Xor, .. })
            || (mixed && matches!(op, MirOp::LeaAddr { target: MirMem::Param { .. } | MirMem::Local { .. }, .. }))
            || matches!(op, MirOp::LeaAddr { target: MirMem::Global { .. } | MirMem::Static { .. }, width: MirWidth::Word, .. })
            || crate::mir6502::materialize::word_shift_supports_source_homes(op)
            || matches!(op, MirOp::Binary { op: MirBinaryOp::Lsh | MirBinaryOp::Rsh, width: MirWidth::Byte,
                right: MirValue::ConstU8(_) | MirValue::ConstU16(0..=255), .. })
            || matches!(op, MirOp::Unary { op: MirUnaryOp::Neg, dst: MirDef::VTemp(_), .. }
                | MirOp::Unary { op: MirUnaryOp::BitNot | MirUnaryOp::SignMask, width: MirWidth::Byte, .. })
            || matches!(op, MirOp::PackedRealCopy {
                source: MirAddr::Direct(_), destination: MirAddr::Direct(_), .. })
            || matches!(op, MirOp::Load { src, width: MirWidth::Byte | MirWidth::Word, .. }
                | MirOp::Store { dst: src, width: MirWidth::Byte | MirWidth::Word, .. } if native_address(src));
        if !supported { return Err(reject(Reason::Operation)); }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod helper_tests;
#[cfg(test)]
mod address_tests;

#[cfg(test)]
mod shift_tests;
