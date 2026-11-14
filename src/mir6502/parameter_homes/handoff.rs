//! Bounded entry-to-terminal-call consumption. Discovery owns its input bodies;
//! expanded verification reconstructs the call identity instead of replaying an
//! instruction index from before materialization.
#![allow(dead_code)] // Discovery is enabled only after the productive selection slice.
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
    pub call: MirProgramPoint,
    pub definitions: Vec<MirDefSite>,
    pub outgoing: Vec<MirParamLane>,
}
impl WrapperFacts {
    pub fn is_current(&self, program: &MirProgram) -> bool {
        program.routines.iter().any(|r| r == self.source.as_ref())
            && self.callee.is_current(program)
    }
    pub fn contract(&self) -> MirParamHandoff {
        MirParamHandoff {
            target: self.callee.source.id,
            argument_bytes: self.outgoing.len() as u16,
        }
    }
}

pub(super) fn analyze(
    program: &MirProgram,
    routine: &MirRoutine,
) -> Result<WrapperFacts, &'static str> {
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
                || !matches!(s.base, MirStorageBase::Param(id) if !exposed(program, routine, id))
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
    let callee = MirCalleeEntryFact::analyze(program, target).ok_or("unknown-callee-entry")?;
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

fn inputs_supplied(
    callee: &MirCalleeEntryFact,
    outgoing: &[MirParamLane],
) -> Result<(), &'static str> {
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
    if inputs
        .fixed
        .iter()
        .any(|b| !outgoing.contains(&MirParamLane::ZeroPage(MirFixedZpSlot(*b))))
    {
        return Err("extra-entry-fixed-byte");
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

pub(super) fn verify(program: &MirProgram, routine: &MirRoutine) -> Vec<MirDiagnostic> {
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
        let callee = MirCalleeEntryFact::analyze(program, target).ok_or("unknown-callee-entry")?;
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
