//! Whole-routine forwarding proofs, selected before any invocation storage.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Plan {
    owner: RoutineId,
    target: RoutineId,
    block: BlockId,
    call: usize,
    parameters: Vec<ParamId>,
    arguments: Vec<Mir65816AbiHome>,
    result: Option<Mir65816AbiHome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Parameter(ParamId),
    Result,
}

fn origin(value: &Mir65816Value, values: &BTreeMap<TempId, Origin>) -> Option<Origin> {
    match value {
        Mir65816Value::Param(id) => Some(Origin::Parameter(*id)),
        Mir65816Value::Temp(id, _) => values.get(id).copied(),
        _ => None,
    }
}

fn ordinary(r: &Mir65816Routine) -> bool {
    !r.entry.external
        && r.helper.is_none()
        && r.entry.placement == crate::nir::NirRoutinePlacement::Relocatable
        && !r.blocks.is_empty()
        && r.epilogue.return_form == Mir65816ReturnForm::FarRtl
}

fn candidate(
    r: &Mir65816Routine,
    routines: &BTreeMap<RoutineId, &Mir65816Routine>,
) -> Option<Plan> {
    // F1 admits one-argument functions; other forwarding shapes retain their
    // ordinary call until the next executable slice.
    if !ordinary(r)
        || r.frame.parameters.len() != 1
        || r.result_home.is_none()
        || !r.frame.objects.is_empty()
        || r.frame.extent.get() != 0
        || r.frame.parameters.iter().any(|p| p.frame_object.is_some())
        || !r.prologue.parameter_copies.is_empty()
    {
        return None;
    }
    let [block] = r.blocks.as_slice() else {
        return None;
    };
    if !block.params.is_empty() {
        return None;
    }
    let Mir65816Terminator::Return {
        value: returned, ..
    } = &block.terminator
    else {
        return None;
    };
    let types: BTreeMap<_, _> = r.temps.iter().map(|(id, ty)| (*id, ty)).collect();
    let mut values = BTreeMap::new();
    let mut used = BTreeSet::new();
    let mut selected = None;
    for (index, op) in block.ops.iter().enumerate() {
        match op {
            Mir65816Op::Load {
                dest,
                width,
                address,
                volatile: false,
            } if selected.is_none() => {
                let Mir65816AddressBase::Parameter(id) = address.base else {
                    return None;
                };
                let p = r.frame.parameters.iter().find(|p| p.param == id)?;
                if address.index.is_some()
                    || address.displacement.get() != 0
                    || !matches!(p.incoming, Mir65816AbiHome::StackArgument { size, .. } if size == *width)
                    || values.insert(*dest, Origin::Parameter(id)).is_some()
                {
                    return None;
                }
            }
            Mir65816Op::Cast {
                dest,
                from,
                to,
                kind,
                value,
                ..
            } if from == to => {
                let Mir65816Value::Temp(input, width) = value else {
                    return None;
                };
                let source = types.get(input)?;
                let output = types.get(dest)?;
                let identity = source.kind == output.kind && source.pointer == output.pointer;
                let pointer = *kind == NirCastKind::Pointer
                    && source.pointer
                    && output.pointer
                    && from.get() == 3;
                if width != from
                    || source.width != Some(*from)
                    || output.width != Some(*to)
                    || !(pointer || (*kind == NirCastKind::Integer && identity))
                {
                    return None;
                }
                let value = origin(value, &values)?;
                if selected.is_some() && value != Origin::Result {
                    return None;
                }
                used.insert(*input);
                if values.insert(*dest, value).is_some() {
                    return None;
                }
            }
            Mir65816Op::Call {
                target: Mir65816CallTarget::Direct(id),
                args,
                result,
                plan,
                ..
            } if selected.is_none() => {
                let target = RoutineId(*id);
                let callee = *routines.get(&target)?;
                if !ordinary(callee)
                    || callee.convention != r.convention
                    || callee.result_home != r.result_home
                    || plan.result != r.result_home
                    || plan.arguments
                        != r.frame
                            .parameters
                            .iter()
                            .map(|p| p.incoming)
                            .collect::<Vec<_>>()
                    || plan.arguments
                        != callee
                            .frame
                            .parameters
                            .iter()
                            .map(|p| p.incoming)
                            .collect::<Vec<_>>()
                    || plan.outgoing_bytes != r.frame.incoming_extent
                    || plan.outgoing_bytes != callee.frame.incoming_extent
                    || plan.mode_before != r.prologue.required_mode
                    || plan.mode_before != callee.prologue.required_mode
                    || plan.mode_after != r.epilogue.restored_mode
                    || args.len() != r.frame.parameters.len()
                    || effects::CallContract::from_plan(plan, abi::FarTransfer::Jsl).is_err()
                {
                    return None;
                }
                for (arg, p) in args.iter().zip(&r.frame.parameters) {
                    if origin(arg, &values) != Some(Origin::Parameter(p.param)) {
                        return None;
                    }
                    if let Mir65816Value::Temp(id, _) = arg {
                        used.insert(*id);
                    }
                }
                if let Some((id, _)) = result {
                    if values.insert(*id, Origin::Result).is_some() {
                        return None;
                    }
                }
                selected = Some(Plan {
                    owner: r.id,
                    target,
                    block: block.id,
                    call: index,
                    parameters: r.frame.parameters.iter().map(|p| p.param).collect(),
                    arguments: plan.arguments.clone(),
                    result: plan.result,
                });
            }
            _ => return None,
        }
    }
    let plan = selected?;
    match (plan.result, returned) {
        (Some(_), Some(value)) if origin(value, &values) == Some(Origin::Result) => {
            if let Mir65816Value::Temp(id, _) = value {
                used.insert(*id);
            }
        }
        (None, None) => (),
        _ => return None,
    }
    // No unexplained definitions or extra reads disappear with the wrapper.
    if values.len() != r.temps.len() || values.keys().any(|id| !used.contains(id)) {
        return None;
    }
    Some(plan)
}

pub(super) fn plans(program: &Mir65816Program) -> BTreeMap<RoutineId, Plan> {
    let routines = program.routines.iter().map(|r| (r.id, r)).collect();
    let candidates: BTreeMap<_, _> = program
        .routines
        .iter()
        .filter_map(|r| candidate(r, &routines).map(|p| (r.id, p)))
        .collect();
    candidates
        .iter()
        .filter_map(|(&id, plan)| {
            let mut seen = BTreeSet::new();
            let mut at = id;
            while let Some(next) = candidates.get(&at) {
                if !seen.insert(at) {
                    return None;
                }
                at = next.target;
            }
            Some((id, plan.clone()))
        })
        .collect()
}

impl Plan {
    pub fn target(&self) -> RoutineId {
        self.target
    }
    pub fn arguments(&self) -> &[Mir65816AbiHome] {
        &self.arguments
    }

    pub fn emit(
        &self,
        program: &Mir65816Program,
        r: &Mir65816Routine,
        trace: bool,
    ) -> Result<MachineRoutine, String> {
        let frame = AllocatedFrame::forwarding(program, r, self)?;
        let mut emitter = tracked::TrackedEmitter65816::for_entry(r.prologue.required_mode);
        #[cfg(feature = "native65816-state-proof")]
        if trace {
            emitter.trace();
        }
        for index in 0..r.blocks[0].ops.len() {
            let start = emitter.position();
            emitter.begin_source(self.block, index);
            if index == self.call {
                emitter
                    .instruction(selected::Instruction::NativeForward(Box::new(self.clone())))?;
            }
            emitter.span(self.block, index, start);
        }
        let end = emitter.position();
        emitter.begin_source(self.block, r.blocks[0].ops.len());
        emitter.span(self.block, r.blocks[0].ops.len(), end);
        let direct = emitter.finish_selected(r.id, &frame, None)?;
        let code = replay::emit(
            direct
                .selected
                .as_ref()
                .ok_or("missing forwarding selection")?,
            trace,
        )?;
        Ok(MachineRoutine {
            id: r.id,
            frame,
            code,
        })
    }

    pub fn verify(&self, program: &Mir65816Program, r: &Mir65816Routine) -> Result<(), String> {
        if self.owner != r.id
            || program.routines.iter().find(|p| p.id == r.id) != Some(r)
            || plans(program).get(&r.id) != Some(self)
        {
            return Err("invalid forwarding-wrapper proof".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "forwarding_tests.rs"]
mod tests;
