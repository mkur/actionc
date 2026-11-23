//! Public Action interfaces retain their selected provider identity. Assembly
//! contracts are reviewed here; ordinary native wrappers use existing analysis.
use super::*;
use super::invocation::InvocationSummary;
use super::buffers::{BufferAccess, BufferExtent};

use crate::callee6502::sys_review::SysReview;
use crate::callee6502::sys_cio::CioTransfer;
use crate::callee6502::sys_decimal::DecimalRoutine;
use crate::callee6502::sys_strings::{StringArguments, StringRoutine};
use crate::mir6502::analysis::machine_values::MirMachineValueAvailability;
use crate::mir6502::analysis::sites::MirSite;

pub(in crate::mir6502) fn bind(
    program: &MirProgram, interface: &MirRoutine, target: MirRuntimeHelperTarget,
    sys_review: Option<SysReview>,
) -> Arc<MirLibraryBinding> {
    let kind = match crate::runtime_bindings::binding_key(&interface.name).as_str() {
        "SYS.SCOMPARE" => Some(MirLibraryRoutine::SCompare),
        "SYS.SCOPY" => Some(MirLibraryRoutine::SCopy),
        "SYS.ZERO" => Some(MirLibraryRoutine::Zero),
        "SYS.SETBLOCK" => Some(MirLibraryRoutine::SetBlock),
        "SYS.MOVEBLOCK" => Some(MirLibraryRoutine::MoveBlock),
        _ => None,
    };
    let backward_branch = sys_review.and_then(SysReview::linked_backward_branch);
    Arc::new(MirLibraryBinding {
        interface: Arc::new(interface.clone()), kind, sys_review,
        implementation: MirRuntimeImplementation::capture(program, &target, None, backward_branch),
    })
}

fn current_binding<'a>(program: &MirProgram, op: &'a MirOp) -> Option<&'a MirLibraryBinding> {
    let MirOp::Call { target, abi, .. } = op else { return None; };
    let binding = abi.library.as_ref()?;
    let implementation = binding.implementation.as_ref()?;
    let current = match target {
        MirCallTarget::Routine(id) => MirRuntimeHelperTarget::Routine(*id),
        MirCallTarget::Runtime { address: Some(a), .. } => MirRuntimeHelperTarget::KnownAbsolute(*a),
        _ => return None,
    };
    if implementation.helper.is_some() || !implementation.is_current(program, &current) { return None; }
    Some(binding)
}

pub(in crate::mir6502) fn graphics_cio_boundary(program: &MirProgram, op: &MirOp) -> Option<&'static str> {
    let binding = current_binding(program, op)?;
    let SysReview::Graphics(graphics) = binding.sys_review? else { return None; };
    graphics.cio_boundary()
}

pub(in crate::mir6502) fn reviewed_cio_transfer(program: &MirProgram, op: &MirOp) -> Option<CioTransfer> {
    let binding = current_binding(program, op)?;
    let SysReview::Cio(routine) = binding.sys_review? else { return None; };
    let MirOp::Call { abi, args, .. } = op else { return None; };
    if !abi.params.iter().eq(args.iter().map(|arg| &arg.home)) { return None; }
    crate::mir6502::abi::canonical_call_lanes(&binding.interface, args).ok()?;
    crate::mir6502::abi::validate_direct_call_result(&binding.interface, op).ok()?;
    let runtime = match binding.implementation.as_ref()?.target {
        MirRuntimeHelperTarget::KnownAbsolute(_) => crate::runtime::Runtime::ActionCart,
        MirRuntimeHelperTarget::Routine(_) => crate::runtime::Runtime::Standalone,
        MirRuntimeHelperTarget::Deferred => return None,
    };
    Some(routine.transfer(runtime))
}

pub(in crate::mir6502) fn library_invocation(program: &MirProgram, op: &MirOp) -> Option<InvocationSummary> {
    library_invocation_with_values(program, op, None, None)
}

pub(in crate::mir6502) fn library_invocation_at(
    program: &MirProgram, routine: &MirRoutine, site: MirSite, op: &MirOp, origin: Option<u16>,
) -> Option<InvocationSummary> {
    if let Some(invocation) = library_invocation(program, op) { return Some(invocation); }
    if reviewed_native_decimal(op) {
        let values = super::buffers::argument_values(program, routine)?;
        return super::native_decimal::invocation_at(program, routine, op, (&values, site), origin?);
    }
    if !requires_argument_proof(program, op) {
        return None;
    }
    let values = super::buffers::argument_values(program, routine)?;
    library_invocation_with_values(program, op, Some((&values, site)), origin)
}

pub(in crate::mir6502) fn requires_argument_proof(program: &MirProgram, op: &MirOp) -> bool {
    match current_binding(program, op).and_then(|binding| binding.sys_review) {
        Some(SysReview::Hardware(hw)) => hw.needs_argument_proof(),
        Some(SysReview::Memory(_) | SysReview::Strings(_) | SysReview::Decimal(_)) => true,
        _ => false,
    }
}

pub(in crate::mir6502) fn reviewed_native_decimal(op: &MirOp) -> bool {
    super::native_decimal::binding(op).is_some()
}

#[cfg(test)]
pub(in crate::mir6502) fn native_decimal_buffers(program: &MirProgram, routine: &MirRoutine, op: &MirOp,
    values: Option<(&MirMachineValueAvailability, MirSite)>,
) -> Vec<(u16, u16, bool)> {
    super::native_decimal::buffers_at(program, routine, op, values)
}

pub(in crate::mir6502) use super::native_decimal::observations_at as native_decimal_observations;

pub(in crate::mir6502) fn reviewed_memory_routine(
    program: &MirProgram, op: &MirOp,
) -> Option<crate::callee6502::sys_memory::MemoryRoutine> {
    let SysReview::Memory(routine) = current_binding(program, op)?.sys_review? else { return None; };
    Some(routine)
}

pub(in crate::mir6502) fn reviewed_string_routine(program: &MirProgram, op: &MirOp) -> Option<StringRoutine> {
    let SysReview::Strings(routine) = current_binding(program, op)?.sys_review? else { return None; };
    Some(routine)
}

pub(in crate::mir6502) fn reviewed_decimal_routine(program: &MirProgram, op: &MirOp) -> Option<DecimalRoutine> {
    let SysReview::Decimal(routine) = current_binding(program, op)?.sys_review? else { return None; };
    Some(routine)
}

pub(in crate::mir6502) fn decimal_buffer(program: &MirProgram, op: &MirOp, routine: DecimalRoutine,
    values: Option<(&MirMachineValueAvailability, MirSite)>,
) -> Option<u16> {
    super::buffers::argument_word(program, op, routine.buffer_byte_offset(), values)
}

fn string_argument(program: &MirProgram, op: &MirOp, values: Option<(&MirMachineValueAvailability, MirSite)>, index: usize) -> Option<u16> {
    // Before expansion the two pointer operands are still words. The byte
    // arguments name their future lanes, which have not been set up yet.
    let values = if matches!(op, MirOp::Call { args, .. } if args.iter().any(|arg| arg.width == MirWidth::Word)) {
        None
    } else {
        values
    };
    match index {
        0 => super::buffers::argument_word(program, op, 0, values),
        1 => super::buffers::argument_word(program, op, 2, values),
        2 | 3 => super::buffers::argument_byte_value(program, op, index as u16 + 2, MirWidth::Byte, values).map(u16::from),
        _ => None,
    }
}

pub(in crate::mir6502) fn string_arguments(program: &MirProgram, op: &MirOp, values: Option<(&MirMachineValueAvailability, MirSite)>) -> StringArguments {
    StringArguments::from_arguments(|index| string_argument(program, op, values, index))
}

fn library_invocation_with_values(
    program: &MirProgram, op: &MirOp, values: Option<(&MirMachineValueAvailability, MirSite)>,
    origin: Option<u16>,
) -> Option<InvocationSummary> {
    let binding = current_binding(program, op)?;
    let MirOp::Call { abi, args, .. } = op else { return None; };
    let interface = &binding.interface;
    if let Some(review) = binding.sys_review {
        use crate::ast::FundType;
        use crate::mir6502::ir::MirAbiIntegerWidth::{Byte, Word};
        let argument = |index| match review {
            SysReview::Strings(_) => string_argument(program, op, values, index),
            SysReview::Memory(_) => super::buffers::argument_word(program, op, 0, values),
            SysReview::Decimal(decimal) => decimal_buffer(program, op, decimal, values),
            _ => super::buffers::argument_byte_value(program, op, 0, MirWidth::Byte, values).map(u16::from),
        };
        let facts = review.facts::<()>(argument, origin);
        if !facts.abi_inputs_only { return None; }
        let integer_width = |ty| if matches!(ty, FundType::Card | FundType::Int) { Word } else { Byte };
        let scalar = |ty| MirDirectParamAbi::Scalar(integer_width(ty));
        let (parameters, result) = match review {
            SysReview::Graphics(graphics) =>
                (graphics.params().iter().copied().map(scalar).collect(), MirDirectResultAbi::Void),
            SysReview::Hardware(hardware) => (
                vec![MirDirectParamAbi::Scalar(Byte); hardware.byte_parameters()],
                if hardware.returns_byte() { MirDirectResultAbi::Scalar(Byte) } else { MirDirectResultAbi::Void },
            ),
            SysReview::Memory(memory) => (
                memory.parameters().iter().copied().map(scalar).collect(),
                memory.result().map_or(MirDirectResultAbi::Void, |ty| MirDirectResultAbi::Scalar(integer_width(ty))),
            ),
            SysReview::Strings(_) => (
                vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Pointer, MirDirectParamAbi::Scalar(Byte), MirDirectParamAbi::Scalar(Byte)],
                MirDirectResultAbi::Void,
            ),
            SysReview::Decimal(decimal) if decimal.parses() => (
                vec![MirDirectParamAbi::Pointer], MirDirectResultAbi::Scalar(integer_width(decimal.integer())),
            ),
            SysReview::Decimal(decimal) => (
                vec![scalar(decimal.integer()), MirDirectParamAbi::Pointer], MirDirectResultAbi::Void,
            ),
            SysReview::Cio(_) | SysReview::WideDecimal(_) | SysReview::RealDecimal(_) => return None,
        };
        if interface.frame.direct_parameter_abi.as_ref() != Some(&parameters)
            || interface.direct_result_abi != Some(result)
            || !abi.params.iter().eq(args.iter().map(|arg| &arg.home))
        {
            return None;
        }
        let lanes = crate::mir6502::abi::canonical_call_lanes(interface, args).ok()?;
        crate::mir6502::abi::validate_direct_call_result(interface, op).ok()?;
        let writes = if review.uses_precomputed_writes() {
            super::buffers::absolute_writes(facts.writes?)
        } else {
            Vec::new()
        };
        return Some(InvocationSummary {
            inputs: lanes.into_iter().map(|lane| match lane {
                MirParamLane::Reg(reg) => MirArgHome::Reg(reg),
                MirParamLane::ZeroPage(slot) => MirArgHome::FixedZeroPage(slot),
            }).collect(),
            definitions: facts.defines_on_return.into_iter().map(|byte| MirMem::FixedZeroPage(MirFixedZpSlot(byte))).collect(),
            register_definitions: MirRegisterSet::default(),
            writes,
            buffers: Vec::new(),
            sys_review: Some(review),
        });
    }
    let (parameters, result, writes) = match binding.kind? {
        MirLibraryRoutine::SCompare => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Pointer],
            MirDirectResultAbi::Scalar(crate::mir6502::ir::MirAbiIntegerWidth::Word),
            vec![0xa0,0xa1,0xa2,0xa4,0xa5,0xa6]),
        MirLibraryRoutine::SCopy => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Pointer],
            MirDirectResultAbi::Void, vec![0xa0,0xa1,0xa2]),
        MirLibraryRoutine::Zero => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Scalar(crate::mir6502::ir::MirAbiIntegerWidth::Word)],
            MirDirectResultAbi::Void, vec![0xa0,0xa1,0xa2,0xa3,0xa4]),
        MirLibraryRoutine::SetBlock => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Scalar(crate::mir6502::ir::MirAbiIntegerWidth::Word), MirDirectParamAbi::Scalar(crate::mir6502::ir::MirAbiIntegerWidth::Byte)],
            MirDirectResultAbi::Void, vec![0xa0,0xa1,0xa2,0xa3]),
        MirLibraryRoutine::MoveBlock => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Pointer, MirDirectParamAbi::Scalar(crate::mir6502::ir::MirAbiIntegerWidth::Word)],
            MirDirectResultAbi::Void, vec![0xa0,0xa1,0xa2,0xa3,0xa5]),
    };
    if interface.frame.direct_parameter_abi.as_ref() != Some(&parameters)
        || interface.direct_result_abi != Some(result)
        || abi.params != args.iter().map(|a| a.home.clone()).collect::<Vec<_>>() {
        return None;
    }
    let lanes = crate::mir6502::abi::canonical_call_lanes(interface, args).ok()?;
    crate::mir6502::abi::validate_direct_call_result(interface, op).ok()?;
    Some(InvocationSummary {
        sys_review: None,
        inputs: lanes.into_iter().map(|lane| match lane {
            MirParamLane::Reg(reg) => MirArgHome::Reg(reg),
            MirParamLane::ZeroPage(slot) => MirArgHome::FixedZeroPage(slot),
        }).collect(),
        definitions: crate::mir6502::abi::direct_result_slots(result).iter()
            .map(|s| MirMem::FixedZeroPage(MirFixedZpSlot(*s))).collect(),
        register_definitions: MirRegisterSet::default(),
        writes: writes.into_iter().map(|s| MirMem::FixedZeroPage(MirFixedZpSlot(s))).collect(),
        buffers: match binding.kind? {
            MirLibraryRoutine::SCompare => vec![
                BufferAccess { pointer_byte: 0, extent: BufferExtent::String { length_pointer_byte: 0 }, write: false },
                BufferAccess { pointer_byte: 2, extent: BufferExtent::String { length_pointer_byte: 2 }, write: false }],
            MirLibraryRoutine::SCopy => vec![
                BufferAccess { pointer_byte: 2, extent: BufferExtent::String { length_pointer_byte: 2 }, write: false },
                BufferAccess { pointer_byte: 0, extent: BufferExtent::String { length_pointer_byte: 2 }, write: true }],
            MirLibraryRoutine::Zero | MirLibraryRoutine::SetBlock => vec![
                BufferAccess { pointer_byte: 0, extent: BufferExtent::Count { argument_byte: 2 }, write: true }],
            MirLibraryRoutine::MoveBlock => vec![
                BufferAccess { pointer_byte: 2, extent: BufferExtent::Count { argument_byte: 4 }, write: false },
                BufferAccess { pointer_byte: 0, extent: BufferExtent::Count { argument_byte: 4 }, write: true }],
        },
    })
}

/// A conservative trigger for physical layout obligations, not a certificate.
pub(in crate::mir6502) fn may_write_buffer(op: &MirOp) -> bool {
    matches!(op, MirOp::Call { abi, .. } if abi.library.as_ref().is_some_and(|b|
        matches!(b.kind, Some(MirLibraryRoutine::SCopy | MirLibraryRoutine::Zero | MirLibraryRoutine::SetBlock | MirLibraryRoutine::MoveBlock))))
}

impl std::fmt::Debug for MirLibraryBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirLibraryBinding").field("interface", &self.interface.name)
            .field("id", &self.interface.id).field("parameters", &self.interface.frame.direct_parameter_abi)
            .field("result", &self.interface.direct_result_abi).field("kind", &self.kind)
            .field("sys_review", &self.sys_review)
            .field("implementation", &self.implementation).finish()
    }
}
