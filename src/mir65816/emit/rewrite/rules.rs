use super::super::{
    Location, TempId,
    analysis::sites::Node,
    copies::WordHome,
    selected::{ByteOp, Instruction, Request},
};
use super::super::{analysis::sites::SelectedSite, selected::Action};
use super::{
    context::{Context, Proof},
    plan::{Delta, Plan, Rule},
};

/// One-shot test transaction; never a fixed-point production rule.
pub(in crate::mir65816::emit) fn identity(
    context: &Context<'_>,
    first: SelectedSite,
    last: SelectedSite,
) -> Proof<Plan> {
    Proof::checked(
        (|| {
            let start = context.facts.validate(first)?.0;
            let end = context.facts.validate(last)?.0;
            if start > end {
                return Err("reversed rewrite window".into());
            }
            let original = context.selected.records()[start..=end].to_vec();
            let replacement = original
                .iter()
                .map(|r| match &r.action {
                    Action::Instruction { form, .. } => Ok(form.clone()),
                    _ => Err("rewrite window contains a protected compiler event".to_string()),
                })
                .collect::<Result<_, _>>()?;
            Ok(Plan {
                rule: Rule::Identity,
                first,
                last,
                original,
                replacement,
                removed_definitions: vec![],
                delta: Delta::default(),
            })
        })(),
        Some(first),
    )
}

pub(super) fn prove_adjacent(
    context: &Context<'_>,
    load: SelectedSite,
    request: SelectedSite,
    temp: TempId,
    home: Location,
) -> Result<(), String> {
    let load_node = context.facts.validate(load)?;
    let request_node = context.facts.validate(request)?;
    if request_node.0.checked_add(2) != Some(load_node.0) {
        return Err("load is not adjacent to its consume request".into());
    }
    let records = context.selected.records();
    let Action::Instruction { form, .. } = &records[load_node.0].action else {
        return Err("missing original load".into());
    };
    let offset = match form {
        Instruction::Byte(ByteOp::LdaStack, n) => WordHome::Stack(*n),
        Instruction::Byte(ByteOp::LdaDp, n) => WordHome::DirectPage(*n),
        _ => return Err("not a temporary word LDA".into()),
    };
    if records[request_node.0].action
        != Action::Request(Request::ConsumeWord(Some(temp), Some(home), Some(offset)))
        || records[request_node.0].parent.is_some()
        || records[request_node.0 + 1].action != Action::EndRequest(request_node)
        || records[request_node.0 + 1].decision != Some(true)
    {
        return Err("candidate consume attribution mismatch".into());
    }
    context
        .adjacent_load(request, Some(temp), Some(home), form)
        .into_result()
        .map_err(|b| b.reason)
}

pub(in crate::mir65816::emit) fn adjacent(
    context: &Context<'_>,
    load: SelectedSite,
) -> Proof<Plan> {
    match candidate(context.selected, load) {
        Proof::Proven(plan) => {
            let Rule::Adjacent {
                request,
                temp,
                home,
            } = plan.rule
            else {
                unreachable!()
            };
            Proof::checked(
                prove_adjacent(context, load, request, temp, home).map(|()| plan),
                Some(load),
            )
        }
        blocked => blocked,
    }
}

/// Structural discovery authors a proposal, never a permission. The driver
/// builds the generation's full context and recomputes its equivalence proof.
pub(super) fn candidate(
    selected: &super::super::selected::SelectedRoutine,
    load: SelectedSite,
) -> Proof<Plan> {
    Proof::checked(
        (|| {
            let node = selected.validate(load)?;
            let request_node = Node(node.0.checked_sub(2).ok_or("missing candidate request")?);
            let request = selected.site(request_node)?;
            let Action::Request(Request::ConsumeWord(Some(temp), Some(home), _)) =
                selected.records()[request_node.0].action
            else {
                return Err("missing typed temporary candidate".into());
            };
            let record = selected.records()[node.0].clone();
            let Action::Instruction { effects, .. } = &record.action else {
                return Err("missing candidate LDA".into());
            };
            let delta = Delta {
                registers: effects.writes,
                flags: effects.flag_writes,
            };
            Ok(Plan {
                rule: Rule::Adjacent {
                    request,
                    temp,
                    home,
                },
                first: load,
                last: load,
                original: vec![record],
                replacement: vec![],
                removed_definitions: vec![],
                delta,
            })
        })(),
        Some(load),
    )
}
