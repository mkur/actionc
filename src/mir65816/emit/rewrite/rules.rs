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
