//! Bounded entry-to-terminal-call consumption. Discovery owns its input bodies;
//! expanded verification reconstructs the call identity instead of replaying an
//! instruction index from before materialization.
use super::*;
use crate::mir6502::analysis::{
    known_callees::MirCalleeEntryFact,
    prehome::PreHomeAnalysisSnapshot,
    sites::{MirProgramPoint, MirRoutineGeneration, MirSite},
    use_def::MirDefSite,
};
use crate::mir6502::rewrite::context::{MirProof, PreHomeRewriteContext};

use crate::mir6502::analysis::argument_staging::MAX_SETUP_OPS;

#[derive(Clone, Debug)]
pub(super) struct WrapperFacts {
    source: Arc<MirRoutine>,
    callee: MirCalleeEntryFact,
    #[allow(dead_code)] // Retained generation/site witness, exercised by proof tests.
    pub call: MirProgramPoint,
    #[allow(dead_code)]
    pub definitions: Vec<MirDefSite>,
    pub outgoing: Vec<MirParamLane>,
}
impl WrapperFacts {
    pub fn is_current(&self, program: &MirProgram) -> bool {
        program.routines.iter().any(|r| r == self.source.as_ref())
            && self.callee.is_current(program, MirCalleeEntryStage::PlannedCapture)
    }
    pub fn contract(&self) -> MirParamHandoff {
        MirParamHandoff {
            target: self.callee.source.id,
            argument_bytes: self.outgoing.len() as u16,
        }
    }
}

#[cfg(test)]
pub(super) fn analyze(program: &MirProgram, routine: &MirRoutine) -> Result<WrapperFacts, &'static str> {
    analyze_with_address_control(program, routine, &ObjectAddressControl::new(program))
}

fn analyze_with_address_control(program: &MirProgram, routine: &MirRoutine,
    addresses: &ObjectAddressControl<'_>) -> Result<WrapperFacts, &'static str> {
    if routine.abi != MirRoutineAbi::Action
        || routine.frame.parameter_homes.is_some()
        || routine.frame.params.is_empty()
        || routine.frame.params.len() > 4
    {
        return Err("not-an-unbound-action-wrapper");
    }
    let incoming: u16 = routine
        .frame
        .params
        .iter()
        .map(|s| s.abi_byte_width().unwrap_or(5))
        .sum();
    if incoming > 4
        || routine.frame.params.iter().any(|s| {
            s.storage != MirStorageClass::Scalar
                || s.offset != 0
                || s.init.is_some()
                || !matches!(
                    (s.scalar_width, s.storage_size),
                    (Some(MirWidth::Byte), 1) | (Some(MirWidth::Word), 2)
                )
                || !matches!(s.base, MirStorageBase::Param(id) if addresses.compiler_controls_address(routine.id, &MirMem::Param { id, offset: 0 }))
        })
    {
        return Err("unsupported-or-exposed-parameters");
    }
    let (block, index, target, outgoing) = terminal_call(routine)?;
    if index > MAX_SETUP_OPS {
        return Err("setup-budget");
    }
    for op in &routine.blocks[0].ops[..index] {
        match op {
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Param { .. }),
                ..
            }
            | MirOp::Load {
                src: MirAddr::Deref { .. } | MirAddr::PointerCell { .. },
                width: MirWidth::Byte,
                ..
            }
            | MirOp::LoadImm { .. }
            | MirOp::Move { .. }
            | MirOp::Extend { .. }
            | MirOp::Truncate { .. }
            | MirOp::Binary {
                op:
                    MirBinaryOp::Add
                    | MirBinaryOp::Sub
                    | MirBinaryOp::And
                    | MirBinaryOp::Or
                    | MirBinaryOp::Xor,
                ..
            } => {}
            _ => return Err("unsupported-setup-or-parameter-mutation"),
        }
        let effects = super::super::analysis::effects::classify_op(op);
        if effects.memory.opaque || effects.memory.indirect_writes {
            return Err("opaque-setup");
        }
    }
    let callee = MirCalleeEntryFact::analyze(program, target, MirCalleeEntryStage::PlannedCapture).ok_or("unknown-callee-entry")?;
    inputs_supplied(&callee, &outgoing)?;
    let snapshot = PreHomeAnalysisSnapshot::new(routine, MirRoutineGeneration::initial())
        .map_err(|_| "open-wrapper-cfg")?;
    let context = PreHomeRewriteContext::new(&snapshot);
    let mut definitions = BTreeSet::new();
    for temp in &routine.temps {
        for usage in snapshot.use_def().uses_of_temp(temp.id) {
            let lanes: BTreeSet<_> = snapshot
                .use_def()
                .definitions_of_temp(temp.id)
                .filter(|d| usage.requirement.requires(d.lane))
                .map(|d| d.lane)
                .collect();
            if lanes.is_empty() {
                return Err("missing-argument-definition");
            }
            for lane in lanes {
                let MirProof::Proven(definition) = context.unique_reaching_definition(*usage, lane)
                else {
                    return Err("ambiguous-argument-definition");
                };
                definitions.insert(definition);
            }
        }
    }
    // A full call argument has an explicit width even when a malformed producer
    // defined only one lane. Never infer the required width from definitions.
    let MirOp::Call { args, .. } = &routine.blocks[0].ops[index] else {
        unreachable!()
    };
    for arg in args {
        if let MirValue::Def(MirDef::VTemp(temp)) = arg.value {
            let uses = context.uses_at(
                temp,
                MirSite::Op {
                    block,
                    op_index: index,
                },
            );
            for byte in 0..super::super::abi::action_arg_width_bytes(arg.width) as u8 {
                let lane = crate::mir6502::analysis::use_def::MirTempLane { temp, byte };
                let Some(usage) = uses.iter().find(|u| u.requirement.requires(lane)) else {
                    return Err("incomplete-argument-definition");
                };
                if !context.unique_reaching_definition(*usage, lane).is_proven() {
                    return Err("incomplete-argument-definition");
                }
            }
        }
    }
    Ok(WrapperFacts {
        source: Arc::new(routine.clone()),
        callee,
        call: context.point(MirSite::Op {
            block,
            op_index: index,
        }),
        definitions: definitions.into_iter().collect(),
        outgoing,
    })
}

pub(super) fn inputs_supplied(
    callee: &MirCalleeEntryFact,
    outgoing: &[MirParamLane],
) -> Result<(), &'static str> {
    register_inputs_supplied(callee, outgoing)?;
    if extra_fixed_inputs(callee, outgoing).next().is_some() {
        return Err("extra-entry-fixed-byte");
    }
    Ok(())
}

pub(super) fn extra_fixed_inputs<'a>(callee: &'a MirCalleeEntryFact, outgoing: &'a [MirParamLane])
    -> impl Iterator<Item = u8> + 'a {
    callee.inputs.fixed.iter().copied().filter(|b|
        !outgoing.contains(&MirParamLane::ZeroPage(MirFixedZpSlot(*b))))
}

pub(super) fn register_inputs_supplied(callee: &MirCalleeEntryFact, outgoing: &[MirParamLane])
    -> Result<(), &'static str> {
    let inputs = &callee.inputs;
    if inputs.flags.any() || inputs.registers.sp {
        return Err("extra-entry-flags-or-stack");
    }
    for (reg, read) in [
        (MirReg::A, inputs.registers.a),
        (MirReg::X, inputs.registers.x),
        (MirReg::Y, inputs.registers.y),
    ] {
        if read && !outgoing.contains(&MirParamLane::Reg(reg)) {
            return Err("extra-entry-register");
        }
    }
    Ok(())
}

fn terminal_call(
    routine: &MirRoutine,
) -> Result<(MirBlockId, usize, RoutineId, Vec<MirParamLane>), &'static str> {
    if routine
        .scalar_signature
        .as_ref()
        .is_some_and(|s| !s.result.is_empty())
    {
        return Err("nonvoid-wrapper");
    }
    let [block] = routine.blocks.as_slice() else {
        return Err("wrapper-control-flow");
    };
    if block.terminator != MirTerminator::Return {
        return Err("nonvoid-terminal-boundary");
    }
    let (index, op) = block
        .ops
        .iter()
        .enumerate()
        .last()
        .ok_or("no-terminal-call")?;
    let MirOp::Call {
        target: MirCallTarget::Routine(target),
        args,
        result: None,
        additional_results,
        abi,
        ..
    } = op
    else {
        return Err("nonterminal-or-indirect-call");
    };
    if !additional_results.is_empty() || abi.result.is_some() || !abi.additional_results.is_empty()
    {
        return Err("call-result-forwarding");
    }
    if block.ops[..index].iter().any(|op| {
        matches!(
            op,
            MirOp::Call { .. }
                | MirOp::RuntimeHelper { .. }
                | MirOp::SArgsCapture { .. }
                | MirOp::MachineBlock { .. }
                | MirOp::Barrier { .. }
        )
    }) {
        return Err("earlier-barrier");
    }
    let mut outgoing = Vec::new();
    for arg in args {
        let lanes = super::super::abi::physical_lanes(&arg.home, arg.width)
            .ok_or("unsupported-argument-home")?;
        let expected = super::super::abi::physical_lanes(
            &super::super::abi::action_arg_home(outgoing.len() as u16, arg.width),
            arg.width,
        )
        .ok_or("unsupported-abi")?;
        if lanes != expected {
            return Err("noncanonical-arguments");
        }
        outgoing.extend(lanes);
    }
    if outgoing.len() > 5 {
        return Err("outgoing-window");
    }
    Ok((block.id, index, *target, outgoing))
}

pub(super) fn terminal_site(routine: &MirRoutine) -> Option<MirSite> {
    let contract = routine.frame.parameter_handoff.as_ref()?;
    let (block, index, target, outgoing) = terminal_call(routine).ok()?;
    (target == contract.target && outgoing.len() == usize::from(contract.argument_bytes)).then_some(
        MirSite::Op {
            block,
            op_index: index,
        },
    )
}

pub(super) fn verify(program: &MirProgram, routine: &MirRoutine, stage: MirCalleeEntryStage) -> Vec<MirDiagnostic> {
    let Some(contract) = &routine.frame.parameter_handoff else {
        return vec![];
    };
    let result = (|| {
        if routine.frame.parameter_homes.is_none() {
            return Err("handoff requires parameter bindings");
        }
        let (_, _, target, outgoing) = terminal_call(routine)?;
        if target != contract.target || outgoing.len() != usize::from(contract.argument_bytes) {
            return Err("stale terminal handoff contract");
        }
        let callee = MirCalleeEntryFact::analyze(program, target, stage).ok_or("unknown-callee-entry")?;
        inputs_supplied(&callee, &outgoing)
    })();
    result
        .err()
        .map(|reason| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("invalid parameter handoff: {reason}"),
            )]
        })
        .unwrap_or_default()
}

/// One deterministic batch, retaining every already-selected leaf binding.
pub(super) fn propose(input: &MirProgram, selected: &MirProgram) -> Option<MirProgram> {
    let addresses = ObjectAddressControl::new(input);
    let mut proposal = None;
    'routines: for (index, routine) in input.routines.iter().enumerate() {
        // Already-elided homes have no storage saving to justify a new capture.
        let Some(old) = selected.routines.iter().find(|r| r.id == routine.id) else {
            continue;
        };
        if !old.frame.params.iter().any(|p| {
            matches!(p.base, MirStorageBase::Param(id)
            if old.frame.parameter_has_memory(id)
                && matches!(old.frame.parameter_home(id), None | Some(MirParamHome::Inline)))
        }) {
            continue;
        }
        let Ok(facts) = analyze_with_address_control(input, routine, &addresses) else {
            continue;
        };
        if !facts.is_current(input) {
            continue;
        }
        let Some(pointer_params) = pointer_parameters(routine) else {
            continue;
        };
        let owned = source_zero_page(input, routine);
        let mut offset = 0u8;
        let mut bindings = Vec::new();
        let mut reserved = Vec::new();
        for slot in &routine.frame.params {
            let MirStorageBase::Param(param) = slot.base else {
                unreachable!()
            };
            let start = 0xa0 + offset;
            if absent_parameter(routine, param) {
                bindings.push(MirParamBinding {
                    param,
                    home: MirParamHome::Absent,
                });
                offset += slot.storage_size as u8;
                continue;
            }
            // Retaining the fourth input in A3 requires an identity outgoing
            // lane. A permutation would overwrite it before register reloads;
            // leave that cycle to the original staging strategy.
            if u16::from(offset) + slot.storage_size == 4
                && !forwards_to_same_home(routine, param, offset as u16, slot.scalar_width.unwrap())
            {
                continue 'routines;
            }
            for byte in start..start + slot.storage_size as u8 {
                reserved.push(MirFixedZpSlot(byte));
            }
            let mut lanes = (0..slot.storage_size)
                .map(|i| MirParamLane::ZeroPage(MirFixedZpSlot(start + i as u8)))
                .collect::<Vec<_>>();
            if !pointer_params.contains(&param)
                && direct_register_forward(
                    routine,
                    param,
                    offset as u16,
                    slot.scalar_width.unwrap(),
                )
            {
                for (lane, incoming) in lanes.iter_mut().zip(
                    super::super::abi::physical_lanes(
                        &super::super::abi::action_arg_home(
                            offset as u16,
                            slot.scalar_width.unwrap(),
                        ),
                        slot.scalar_width.unwrap(),
                    )
                    .unwrap(),
                ) {
                    if incoming == MirParamLane::Reg(MirReg::X)
                        || (incoming == MirParamLane::Reg(MirReg::Y) && pointer_params.is_empty())
                    {
                        *lane = incoming;
                    }
                }
            }
            bindings.push(MirParamBinding {
                param,
                home: MirParamHome::Lanes(lanes),
            });
            offset += slot.storage_size as u8;
        }
        reserved.retain(|slot| {
            bindings.iter().any(|b| {
                matches!(&b.home,
            MirParamHome::Lanes(lanes) if lanes.contains(&MirParamLane::ZeroPage(*slot)))
            })
        });
        if reserved.iter().any(|s| owned.contains(&s.0)) {
            continue;
        }
        let result = proposal.get_or_insert_with(|| input.clone());
        let r = &mut result.routines[index];
        for slot in reserved {
            r.frame.reserve_named_lane(slot);
        }
        r.frame.parameter_homes = Some(bindings);
        r.frame.parameter_handoff = Some(facts.contract());
    }
    proposal
}

fn direct_register_forward(
    routine: &MirRoutine,
    param: crate::nir::ParamId,
    incoming: u16,
    width: MirWidth,
) -> bool {
    let ops = &routine.blocks[0].ops;
    // This closed expansion set preserves X. Indirect reads need Y, which
    // selection separately excludes from register residence.
    if ops[..ops.len() - 1].iter().any(|op| {
        !matches!(
            op,
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Param { .. }),
                ..
            } | MirOp::Load {
                src: MirAddr::Deref { .. } | MirAddr::PointerCell { .. },
                width: MirWidth::Byte,
                ..
            } | MirOp::LoadImm {
                width: MirWidth::Byte,
                ..
            } | MirOp::Binary {
                width: MirWidth::Byte,
                ..
            }
        )
    }) {
        return false;
    }
    forwards_to_same_home(routine, param, incoming, width)
}

fn forwards_to_same_home(
    routine: &MirRoutine,
    param: crate::nir::ParamId,
    incoming: u16,
    width: MirWidth,
) -> bool {
    let ops = &routine.blocks[0].ops;
    let loads: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            MirOp::Load {
                dst,
                src: MirAddr::Direct(MirMem::Param { id, offset: 0 }),
                width: w,
            } if *id == param && *w == width => Some(dst),
            _ => None,
        })
        .collect();
    let [dst] = loads.as_slice() else {
        return false;
    };
    let MirOp::Call { args, .. } = ops.last().unwrap() else {
        return false;
    };
    let home = super::super::abi::action_arg_home(incoming, width);
    args.iter()
        .filter(|a| a.value == MirValue::Def((*dst).clone()))
        .count()
        == 1
        && args
            .iter()
            .any(|a| a.value == MirValue::Def((*dst).clone()) && a.width == width && a.home == home)
}

/// Keep indirect consumers on a coherent parameter pair. Definition uniqueness
/// was proved by WrapperFacts; no arithmetic pointer reconstruction is added.
fn pointer_parameters(routine: &MirRoutine) -> Option<BTreeSet<crate::nir::ParamId>> {
    let ops = &routine.blocks[0].ops;
    let mut parameters = BTreeSet::new();
    for op in ops {
        let MirOp::Load {
            src,
            width: MirWidth::Byte,
            ..
        } = op
        else {
            continue;
        };
        match src {
            MirAddr::Deref {
                ptr: MirValue::Def(def),
                offset,
            } if (0..=255).contains(offset) => {
                let mut sources = ops.iter().filter_map(|op| match op {
                    MirOp::Load {
                        dst,
                        src: MirAddr::Direct(MirMem::Param { id, offset: 0 }),
                        width: MirWidth::Word,
                    } if dst == def => Some(*id),
                    _ => None,
                });
                let id = sources.next()?;
                if sources.next().is_some() {
                    return None;
                }
                parameters.insert(id);
            }
            MirAddr::PointerCell {
                ptr: MirMem::Param { id, offset: 0 },
                offset,
            } if (0..=255).contains(offset) => {
                parameters.insert(*id);
            }
            MirAddr::Deref { .. } | MirAddr::PointerCell { .. } => return None,
            _ => {}
        }
    }
    Some(parameters)
}
