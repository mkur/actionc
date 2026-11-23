//! Compose a reviewed native buffer bound with the selected MIR scratch.
//! The bound comes from the embedded Action! source. Physical writes come
//! from the current selection, including the actual helper bindings.

use super::*;
use crate::callee6502::{BufferLength, sys_review::SysReview, sys_wide_decimal::WideDecimal};
use crate::mir6502::abi::{
    action_arg_home, canonical_call_lanes, direct_parameter_frame_matches,
    validate_direct_call_result,
};
use crate::mir6502::analysis::{
    effects::classify_op,
    known_callees::{MirKnownMemoryWrites, fixed_storage_address, record_physical_writes},
    machine_values::MirMachineValueAvailability,
    sites::MirSite,
    use_def::local_word_temp_definition,
};

pub(super) fn binding(op: &MirOp) -> Option<(WideDecimal, &MirLibraryBinding, RoutineId)> {
    let MirOp::Call {
        target: MirCallTarget::Routine(target),
        abi,
        args,
        ..
    } = op
    else {
        return None;
    };
    let binding = abi.library.as_deref()?;
    let SysReview::WideDecimal(review) = binding.sys_review? else {
        return None;
    };
    if binding.implementation.as_ref()?.target != MirRuntimeHelperTarget::Routine(*target)
        || !abi.params.iter().eq(args.iter().map(|arg| &arg.home))
        || canonical_call_lanes(&binding.interface, args).is_err()
        || validate_direct_call_result(&binding.interface, op).is_err()
    {
        return None;
    }
    Some((review, binding, *target))
}

pub(super) fn buffer_at(
    program: &MirProgram,
    routine: &MirRoutine,
    op: &MirOp,
    values: Option<(&MirMachineValueAvailability, MirSite)>,
) -> Option<(u16, u16, bool)> {
    let (review, ..) = binding(op)?;
    let buffer = review.buffer();
    let BufferLength::Fixed(bytes) = buffer.length else {
        return None;
    };
    // The formatter's STRING follows the LONG. The parser takes only STRING.
    let offset = if review.parses() { 0 } else { 4 };
    let address = buffers::argument_word(program, op, offset, values).or_else(|| {
        let (_, site) = values?;
        let MirOp::Call { args, .. } = op else {
            return None;
        };
        let home = action_arg_home(offset, MirWidth::Word);
        let arg = args
            .iter()
            .find(|arg| arg.width == MirWidth::Word && arg.home == home)?;
        let MirValue::Def(MirDef::VTemp(temp)) = arg.value else {
            return None;
        };
        // A retained address-of is still a complete word. Once expanded,
        // only the actual staged byte values can authorize the call.
        let (
            _,
            MirOp::LeaAddr {
                target,
                width: MirWidth::Word,
                ..
            },
        ) = local_word_temp_definition(routine, site, temp)?
        else {
            return None;
        };
        fixed_storage_address(program, target)
    })?;
    Some((address, bytes, buffer.write))
}

pub(super) fn invocation_at(
    program: &MirProgram,
    routine: &MirRoutine,
    op: &MirOp,
    values: (&MirMachineValueAvailability, MirSite),
    origin: u16,
) -> Option<invocation::InvocationSummary> {
    let (review, binding, target) = binding(op)?;
    let (address, bytes, output) = buffer_at(program, routine, op, Some(values))?;
    let last = address.checked_add(bytes - 1)?;
    // Ordinary RAM below the program excludes captured parameters, spills,
    // local arrays, stack, hardware, and fixed zero-page address carriers.
    if address < 0x0600 || last >= origin || last >= 0xd000 {
        return None;
    }
    let implementation = binding.implementation.as_ref()?;
    let mut writes = MirKnownMemoryWrites::Exact(Vec::new());
    record_selected_writes(
        program,
        implementation,
        target,
        &mut BTreeSet::new(),
        &mut writes,
    )?;
    let MirKnownMemoryWrites::Exact(mut writes) = writes else {
        return None;
    };
    if writes
        .iter()
        .filter_map(|mem| fixed_storage_address(program, mem))
        .any(|byte| (address..=last).contains(&byte))
    {
        return None;
    }
    if output {
        writes.extend((address..=last).map(MirMem::Absolute));
    }
    let slots = review.facts::<()>().defines_on_return;
    let selected = program
        .routines
        .iter()
        .find(|routine| routine.id == target)?;
    if !slots.is_empty() && !defines_on_all_returns(selected, &slots) {
        return None;
    }
    let MirOp::Call { abi, .. } = op else {
        return None;
    };
    Some(invocation::InvocationSummary {
        sys_review: Some(SysReview::WideDecimal(review)),
        inputs: abi.params.clone(),
        writes,
        definitions: slots
            .into_iter()
            .map(|slot| MirMem::FixedZeroPage(MirFixedZpSlot(slot)))
            .collect(),
        register_definitions: MirRegisterSet::default(),
        buffers: Vec::new(),
    })
}

fn record_selected_writes(
    program: &MirProgram,
    implementation: &MirRuntimeImplementation,
    target: RoutineId,
    visited: &mut BTreeSet<RoutineId>,
    writes: &mut MirKnownMemoryWrites,
) -> Option<()> {
    if !visited.insert(target) {
        return Some(());
    }
    let selected = program
        .routines
        .iter()
        .find(|routine| routine.id == target)?;
    let planned = implementation
        .routines
        .iter()
        .find(|routine| routine.id == target)?;
    if selected.name != planned.name
        || selected.frame.direct_parameter_abi != planned.frame.direct_parameter_abi
        || selected.direct_result_abi != planned.direct_result_abi
        || !direct_parameter_frame_matches(selected)
        || selected.blocks.is_empty()
    {
        return None;
    }
    if matches!(
        selected.name.rsplit("::").next()?,
        "SysLongError" | "CartLongError"
    ) {
        // The independently matched source uses an opaque terminal shim. If
        // Error returns, the shim stops in its own loop. Its machine bytes
        // must remain current and its observations stay outside a returning
        // caller's preservation footprint.
        let [block] = selected.blocks.as_slice() else {
            return None;
        };
        let [MirOp::MachineBlock { id, .. }] = block.ops.as_slice() else {
            return None;
        };
        let witness = implementation
            .machines
            .iter()
            .find(|machine| machine.id == *id)?;
        let current = program
            .machine_blocks
            .iter()
            .find(|machine| machine.id == *id)?;
        return (selected.blocks == planned.blocks && witness == current).then_some(());
    }
    let direct_targets: BTreeSet<_> = planned
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            MirOp::Call {
                target: MirCallTarget::Routine(id),
                ..
            } => Some(*id),
            _ => None,
        })
        .collect();
    let mut retained_targets = BTreeSet::new();
    for op in selected.blocks.iter().flat_map(|block| &block.ops) {
        match op {
            MirOp::Call {
                target: MirCallTarget::Routine(id),
                args,
                ..
            } if direct_targets.contains(id) => {
                let callee = program.routines.iter().find(|routine| routine.id == *id)?;
                canonical_call_lanes(callee, args).ok()?;
                validate_direct_call_result(callee, op).ok()?;
                retained_targets.insert(*id);
                record_selected_writes(program, implementation, *id, visited, writes)?;
            }
            MirOp::RuntimeHelper { .. } | MirOp::SArgsCapture { .. } => {
                let summary = invocation::invocation(program, selected, op)?;
                for mem in summary.writes {
                    writes.record(mem);
                }
            }
            MirOp::Call { .. } | MirOp::MachineBlock { .. } | MirOp::Barrier { .. } => return None,
            _ => {
                let effects = classify_op(op);
                // The source review covers the string access and private
                // indexed stores. The selected body must still use an ordinary
                // indirect recipe and expose every additional physical write.
                let bounded_store = matches!(
                    op,
                    MirOp::StoreIndirect { .. }
                        | MirOp::Store {
                            dst: MirAddr::FixedIndirectIndexedY { .. }
                                | MirAddr::IndirectIndexedY { .. },
                            ..
                        }
                        | MirOp::Store {
                            dst: MirAddr::AbsoluteIndexedX {
                                base: MirMem::Local { .. }
                            } | MirAddr::AbsoluteIndexedY {
                                base: MirMem::Local { .. }
                            },
                            ..
                        }
                );
                if effects.memory.opaque
                    || !effects.logical.temp_defs.is_empty()
                    || (effects.memory.indirect_writes || effects.memory.may_write_any)
                        && !bounded_store
                    || effects.homes.unknown_writes && !bounded_store
                {
                    return None;
                }
                record_physical_writes(selected, effects, writes);
            }
        }
    }
    if retained_targets != direct_targets {
        return None;
    }
    Some(())
}

fn defines_on_all_returns(routine: &MirRoutine, slots: &BTreeSet<u8>) -> bool {
    let mut returns = routine
        .blocks
        .iter()
        .filter(|block| matches!(block.terminator, MirTerminator::Return));
    let defines_result = |block: &MirBlock| {
        let mut remaining = slots.clone();
        // Current decimal parsers write their four result lanes in their
        // return block, after all calls. A different selected shape is open.
        for op in block.ops.iter().rev() {
            if matches!(
                op,
                MirOp::Call { .. } | MirOp::RuntimeHelper { .. } | MirOp::SArgsCapture { .. }
            ) {
                return false;
            }
            if let MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(slot)),
                width: MirWidth::Byte,
                ..
            } = op
            {
                remaining.remove(&slot.0);
            }
            if remaining.is_empty() {
                return true;
            }
        }
        false
    };
    returns.next().is_some_and(defines_result) && returns.all(defines_result)
}
