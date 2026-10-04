//! Shorten bounded A16 stack arithmetic only when C/V are dead.
use super::super::{
    Code,
    analysis::{machine_liveness::ConditionFlag, sites::Node},
    selected::{Action, Implied, Instruction, SelectedRoutine, WordOp},
    state::Width,
};
use super::{
    context::Context,
    driver::Driver,
    plan::{Delta, Plan, Rule},
};

pub(super) fn candidate(selected: &SelectedRoutine, start: usize) -> Result<Plan, String> {
    let records = selected.records();
    let window = records
        .get(start.checked_sub(1).ok_or("missing TSC")?..start + 3)
        .ok_or("incomplete stack adjustment")?;
    let instruction = |n: usize| match &window[n].action {
        Action::Instruction { form, .. } if window[n].parent.is_none() => Some(form),
        _ => None,
    };
    if instruction(0) != Some(&Instruction::Implied(Implied::Tsc))
        || instruction(3) != Some(&Instruction::Implied(Implied::Tcs))
    {
        return Err("not a contiguous TSC/arithmetic/TCS window".into());
    }
    let (step, amount) = match (instruction(1), instruction(2)) {
        (Some(Instruction::Implied(Implied::Clc)), Some(Instruction::Word(WordOp::AdcImm, n))) => {
            (Implied::IncA, *n)
        }
        (Some(Instruction::Implied(Implied::Sec)), Some(Instruction::Word(WordOp::SbcImm, n))) => {
            (Implied::DecA, *n)
        }
        _ => return Err("not a constant stack adjustment".into()),
    };
    if !(1..=3).contains(&amount) {
        return Err("stack adjustment is outside the profitable range".into());
    }
    let env = window[1].before.env;
    if !env.native
        || env.m != Width::Word
        || env.decimal != Some(false)
        || window[1].before.stack_a != Some(-env.depth)
        || window[2].before != window[1].before
        || window[2].after.env != env
        || window[2].after.stack_a
            != Some(
                -env.depth
                    + if step == Implied::IncA {
                        i64::from(amount)
                    } else {
                        -i64::from(amount)
                    },
            )
    {
        return Err("stack adjustment requires a checked A16 binary stack equation".into());
    }
    let mut delta = Delta::default();
    for r in &window[1..3] {
        let Action::Instruction { effects, .. } = &r.action else {
            unreachable!()
        };
        delta.registers.a |= effects.writes.a;
        delta.flags |= effects.flag_writes;
    }
    Ok(Plan {
        rule: Rule::SmallStack,
        first: selected.site(Node(start))?,
        last: selected.site(Node(start + 1))?,
        original: window[1..3].to_vec(),
        replacement: vec![Instruction::Implied(step); usize::from(amount)],
        removed_definitions: vec![],
        delta,
    })
}

pub(super) fn prove(context: &Context<'_>, plan: &Plan) -> Result<(), String> {
    let expected = candidate(context.selected, context.facts.validate(plan.first)?.0)?;
    if plan.last != expected.last || plan.replacement != expected.replacement {
        return Err("stack replacement does not preserve its equation".into());
    }
    for flag in [ConditionFlag::C, ConditionFlag::V] {
        if !context
            .flag_dead_after(plan.last, flag)
            .into_result()
            .map_err(|b| b.reason)?
        {
            return Err("stack arithmetic carry or overflow is live".into());
        }
    }
    // Binary ADC/SBC with explicit carry and repeated A16 steps have identical
    // A/N/Z modulo 65536. Neither replacement touches memory or X/Y. The driver
    // preserves event/CFG boundaries and replays all later decisions and TCS.
    Ok(())
}

pub(in crate::mir65816::emit) fn apply(mut code: Code, trace: bool) -> Result<Code, String> {
    #[cfg(feature = "native65816-state-proof")]
    let observations = code.rewrite_observations.clone();
    let count = code
        .selected
        .as_ref()
        .ok_or("missing stack selection")?
        .records()
        .len();
    let mut driver = Driver::new(count);
    let mut cursor = 1;
    while cursor + 2 < code.selected.as_ref().unwrap().records().len() {
        if let Ok(plan) = candidate(code.selected.as_ref().unwrap(), cursor) {
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

#[cfg(any(test, feature = "native65816-state-proof"))]
pub(in crate::mir65816::emit) fn probe(
    n: u16,
    subtract: bool,
    flag: Option<super::super::selected::Branch>,
    split: bool,
    trace: bool,
) -> Code {
    use super::super::{
        AllocatedFrame, analysis::homes::HomeContract, layout, tracked::TrackedEmitter65816,
    };
    let p = crate::compiler::native65816::prepare_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tools/native65816-runtime-tests/tests/fixtures/code_quality/identity.act"),
        false,
        &Default::default(),
    )
    .unwrap();
    let r = &p.mir.routines[0];
    let frame = AllocatedFrame::new(r).unwrap();
    let mut e = TrackedEmitter65816::default();
    #[cfg(feature = "native65816-state-proof")]
    if trace {
        e.trace();
    }
    let _ = trace;
    e.op(Implied::Tsc);
    if split {
        let label = e.label();
        e.mark(label);
    }
    e.op(if subtract { Implied::Sec } else { Implied::Clc });
    e.word(
        if subtract {
            WordOp::SbcImm
        } else {
            WordOp::AdcImm
        },
        n,
    );
    e.op(Implied::Tcs);
    if let Some(flag) = flag {
        let label = e.label();
        e.branch(flag, label);
        e.mark(label);
    }
    e.op(Implied::Tsc);
    // Keep the restoring arithmetic outside the four-instruction template.
    e.op(Implied::Nop);
    e.op(if subtract { Implied::Clc } else { Implied::Sec });
    e.word(
        if subtract {
            WordOp::AdcImm
        } else {
            WordOp::SbcImm
        },
        n,
    );
    e.op(Implied::Tcs);
    e.native_return(None).unwrap();
    layout::finalize(
        e.finish_selected(
            r.id,
            &frame,
            Some(HomeContract::from_verified(r, &frame).unwrap()),
        )
        .unwrap(),
        true,
    )
    .unwrap()
}

#[cfg(test)]
mod tests;
