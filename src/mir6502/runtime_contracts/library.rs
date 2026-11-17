//! Public Action interfaces retain their selected provider identity. Assembly
//! contracts are reviewed here; ordinary native wrappers use existing analysis.
use super::*;
use super::invocation::InvocationSummary;
use super::buffers::{BufferAccess, BufferExtent};

pub(in crate::mir6502) fn bind(program: &MirProgram, interface: &MirRoutine, target: MirRuntimeHelperTarget) -> Arc<MirLibraryBinding> {
    let kind = match crate::runtime_bindings::binding_key(&interface.name).as_str() {
        "SYS.SCOMPARE" => Some(MirLibraryRoutine::SCompare),
        "SYS.SCOPY" => Some(MirLibraryRoutine::SCopy),
        "SYS.ZERO" => Some(MirLibraryRoutine::Zero),
        "SYS.SETBLOCK" => Some(MirLibraryRoutine::SetBlock),
        "SYS.MOVEBLOCK" => Some(MirLibraryRoutine::MoveBlock),
        _ => None,
    };
    Arc::new(MirLibraryBinding {
        interface: Arc::new(interface.clone()), kind,
        implementation: MirRuntimeImplementation::capture(program, &target, None),
    })
}

pub(in crate::mir6502) fn library_invocation(program: &MirProgram, op: &MirOp) -> Option<InvocationSummary> {
    let MirOp::Call { target, abi, args, .. } = op else { return None; };
    let binding = abi.library.as_ref()?;
    let implementation = binding.implementation.as_ref()?;
    let current = match target {
        MirCallTarget::Routine(id) => MirRuntimeHelperTarget::Routine(*id),
        MirCallTarget::Runtime { address: Some(a), .. } => MirRuntimeHelperTarget::KnownAbsolute(*a),
        _ => return None,
    };
    if implementation.helper.is_some() || !implementation.is_current(program, &current) { return None; }
    let interface = &binding.interface;
    let (parameters, result, writes) = match binding.kind? {
        MirLibraryRoutine::SCompare => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Pointer],
            MirDirectResultAbi::Scalar(MirWidth::Word),
            vec![0xa0,0xa1,0xa2,0xa4,0xa5,0xa6]),
        MirLibraryRoutine::SCopy => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Pointer],
            MirDirectResultAbi::Void, vec![0xa0,0xa1,0xa2]),
        MirLibraryRoutine::Zero => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Scalar(MirWidth::Word)],
            MirDirectResultAbi::Void, vec![0xa0,0xa1,0xa2,0xa3,0xa4]),
        MirLibraryRoutine::SetBlock => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Scalar(MirWidth::Word), MirDirectParamAbi::Scalar(MirWidth::Byte)],
            MirDirectResultAbi::Void, vec![0xa0,0xa1,0xa2,0xa3]),
        MirLibraryRoutine::MoveBlock => (
            vec![MirDirectParamAbi::Pointer, MirDirectParamAbi::Pointer, MirDirectParamAbi::Scalar(MirWidth::Word)],
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
            .field("implementation", &self.implementation).finish()
    }
}
