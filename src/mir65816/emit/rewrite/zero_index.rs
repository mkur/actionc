//! Remove zero-index setup only when the continuation cannot observe it.
use super::super::{
    Code,
    analysis::{
        machine_liveness::{ConditionFlag, RegisterLane},
        sites::Node,
    },
    selected::{Action, ByteOp, Instruction, SelectedRoutine, WordOp},
    state::Width,
};
use super::{
    context::Context,
    driver::Driver,
    plan::{Delta, Plan, Rule},
};

pub(super) fn candidate(selected: &SelectedRoutine, start: usize) -> Result<Plan, String> {
    let records = selected
        .records()
        .get(start..start + 2)
        .ok_or("missing zero-index pair")?;
    let Action::Instruction {
        form: Instruction::Word(WordOp::LdyImm, 0),
        ..
    } = records[0].action
    else {
        return Err("not LDY zero".into());
    };
    let Action::Instruction {
        form: Instruction::Byte(op, slot),
        ..
    } = records[1].action
    else {
        return Err("not an indirect access".into());
    };
    let op = match op {
        ByteOp::LdaIndirectY => ByteOp::LdaIndirect,
        ByteOp::StaIndirectY => ByteOp::StaIndirect,
        _ => return Err("not an indexed indirect access".into()),
    };
    let mut delta = Delta::default();
    for record in records {
        let Action::Instruction { effects, .. } = &record.action else {
            unreachable!()
        };
        delta.registers.a |= effects.writes.a | effects.clobbers.a;
        delta.registers.x |= effects.writes.x | effects.clobbers.x;
        delta.registers.y |= effects.writes.y | effects.clobbers.y;
        delta.flags |= effects.flag_writes | effects.flag_clobbers;
    }
    Ok(Plan {
        rule: Rule::ZeroIndex,
        first: selected.site(Node(start))?,
        last: selected.site(Node(start + 1))?,
        original: records.to_vec(),
        replacement: vec![Instruction::Byte(op, slot)],
        removed_definitions: vec![],
        delta,
    })
}

pub(super) fn prove(context: &Context<'_>, plan: &Plan) -> Result<(), String> {
    let start = context.facts.validate(plan.first)?.0;
    let expected = candidate(context.selected, start)?;
    if plan.last != expected.last || plan.replacement != expected.replacement {
        return Err("zero-index replacement must preserve the complete indirect access".into());
    }
    let records = &expected.original;
    let env = records[0].before.env;
    if !env.native || env.index != Width::Word || records[1].before.env != env {
        return Err("zero-index requires stable native widths and environment".into());
    }
    // With native Y=0 both forms read the same three DP address bytes and
    // access exactly the same external bytes in the same order and width.
    // The driver independently checks event/CFG boundaries and replays effects.
    for lane in [RegisterLane::YLow, RegisterLane::YHigh] {
        if !context
            .register_dead_after(plan.last, lane)
            .into_result()
            .map_err(|b| b.reason)?
        {
            return Err("zero-index Y is live after access".into());
        }
    }
    if matches!(
        plan.replacement[0],
        Instruction::Byte(ByteOp::StaIndirect, _)
    ) {
        for flag in [ConditionFlag::N, ConditionFlag::Z] {
            if !context
                .flag_dead_after(plan.last, flag)
                .into_result()
                .map_err(|b| b.reason)?
            {
                return Err("zero-index store flags are live after access".into());
            }
        }
    }
    // Loads establish identical A/N/Z; stores need the dead N/Z proof above.
    Ok(())
}

pub(in crate::mir65816::emit) fn apply(mut code: Code, trace: bool) -> Result<Code, String> {
    #[cfg(feature = "native65816-state-proof")]
    let observations = code.rewrite_observations.clone();
    let count = code
        .selected
        .as_ref()
        .ok_or("missing zero-index selection")?
        .records()
        .len();
    let mut driver = Driver::new(count);
    let mut cursor = 0;
    while cursor + 1 < code.selected.as_ref().unwrap().records().len() {
        let plan = candidate(code.selected.as_ref().unwrap(), cursor);
        if let Ok(plan) = plan {
            // A failed proof leaves the original pair intact. Re-discover each
            // later site in the current generation, never reuse stale facts.
            let _ = driver.apply(&mut code, &plan, trace);
        }
        cursor += 1;
    }
    #[cfg(feature = "native65816-state-proof")]
    {
        code.rewrite_observations = observations;
    }
    Ok(code)
}
