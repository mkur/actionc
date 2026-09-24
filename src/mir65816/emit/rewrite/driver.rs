use super::super::{
    Code,
    analysis::sites::Node,
    effects::{Access, Control},
    layout, replay,
    selected::{Action, Instruction, Record, SelectedRoutine},
};
use super::{
    context::{Context, Proof},
    plan::{Delta, Plan, Rule},
};
use crate::analysis::graph::DataflowGraph;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::mir65816::emit) struct Statistics {
    pub attempted: usize,
    pub applied: usize,
    pub blocked: BTreeMap<String, usize>,
}
pub(in crate::mir65816::emit) struct Driver {
    pub statistics: Statistics,
    remaining: usize,
    #[cfg(test)]
    identity_used: bool,
}
impl Driver {
    pub fn new(limit: usize) -> Self {
        Self {
            statistics: Statistics::default(),
            remaining: limit,
            #[cfg(test)]
            identity_used: false,
        }
    }
    pub fn apply(&mut self, output: &mut Code, plan: &Plan, trace: bool) -> Proof<()> {
        self.statistics.attempted += 1;
        let result: Result<(), String> = (|| {
            if self.remaining == 0 {
                return Err("rewrite iteration limit".into());
            }
            #[cfg(test)]
            if plan.rule == Rule::Identity && self.identity_used {
                return Err("identity transaction is one-shot".into());
            }
            let scratch = self.prepare(output, plan, trace)?;
            // This is the sole publication point. All validation, replay, layout
            // and analysis construction have completed before any mutation.
            *output = scratch;
            self.remaining -= 1;
            #[cfg(test)]
            {
                self.identity_used |= plan.rule == Rule::Identity;
            }
            self.statistics.applied += 1;
            Ok(())
        })();
        if let Err(reason) = &result {
            *self.statistics.blocked.entry(reason.clone()).or_default() += 1;
        }
        Proof::checked(result, Some(plan.first))
    }
    fn prepare(&self, output: &Code, plan: &Plan, trace: bool) -> Result<Code, String> {
        let selected = output.selected.as_ref().ok_or("missing selected routine")?;
        selected.reconcile(output)?;
        let context = Context::new(selected)?;
        let start = context.facts.validate(plan.first)?.0;
        let end = context.facts.validate(plan.last)?.0;
        if start > end || selected.records()[start..=end] != plan.original {
            return Err("rewrite window/content mismatch".into());
        }
        let records = &selected.records()[start..=end];
        // Only this closed proof can retain an indirect memory barrier inside a
        // rewrite. It proves the complete access is preserved, including stores;
        // no definition is removed and no general alias permission is granted.
        let preserves_indirect = plan.rule == Rule::ZeroIndex;
        if preserves_indirect {
            super::zero_index::prove(&context, plan)?;
        }
        let mut original = Vec::new();
        for (i, record) in (start..=end).zip(records) {
            let Action::Instruction { form, effects, .. } = &record.action else {
                return Err("protected compiler event in rewrite window".into());
            };
            if record.parent.is_some()
                || effects.control != Control::Next
                || effects.environment_writes != 0
                || (effects.barrier && !preserves_indirect)
                || record.before != record.after
            {
                return Err("protected instruction/environment in rewrite window".into());
            }
            if i > start
                && (selected.cfg().predecessors(Node(i)) != &[Node(i - 1)].into()
                    || selected.cfg().successors(Node(i - 1)) != &[Node(i)].into())
            {
                return Err("rewrite window crosses selected blocks".into());
            }
            original.push(form.clone());
        }
        // Recompute declarations; authored deltas cannot waive proof checks.
        let changed = original != plan.replacement;
        let mut removed = Vec::new();
        let mut delta = Delta::default();
        if changed {
            for (i, record) in (start..=end).zip(records) {
                for access in &context.facts.homes.accesses[&Node(i)] {
                    if access.access != Access::Read && !preserves_indirect {
                        if access.uncertain {
                            return Err("uncertain removed definition".into());
                        }
                        for &home in &access.homes {
                            removed.push((home, selected.site(Node(i))?));
                        }
                    }
                }
                let Action::Instruction { effects, .. } = &record.action else {
                    unreachable!()
                };
                delta.registers.a |= effects.writes.a | effects.clobbers.a;
                delta.registers.x |= effects.writes.x | effects.clobbers.x;
                delta.registers.y |= effects.writes.y | effects.clobbers.y;
                delta.flags |= effects.flag_writes | effects.flag_clobbers;
            }
            for instruction in &plan.replacement {
                let effects = instruction.effects(records[0].before.env);
                if effects.environment_writes != 0
                    || effects.control != Control::Next
                    || (effects.barrier && !preserves_indirect)
                {
                    return Err("protected replacement effects".into());
                }
                delta.registers.a |= effects.writes.a | effects.clobbers.a;
                delta.registers.x |= effects.writes.x | effects.clobbers.x;
                delta.registers.y |= effects.writes.y | effects.clobbers.y;
                delta.flags |= effects.flag_writes | effects.flag_clobbers;
            }
        }
        if removed != plan.removed_definitions || delta != plan.delta {
            return Err("undeclared definitions or register/flag effects".into());
        }
        for &(home, store) in &removed {
            if !context
                .facts
                .definition_dead_outside_window(home, store, plan.last)?
            {
                return Err("removed definition has a live outside use".into());
            }
        }
        // Closed rules also validate replacement reads and live-state
        // equivalence. No arbitrary replacement wins through deadness alone.
        match plan.rule {
            Rule::ZeroIndex => {} // Complete memory/live-state proof above.
            #[cfg(test)]
            Rule::Identity if original == plan.replacement => {}
            Rule::Adjacent {
                request,
                temp,
                home,
            } if plan.replacement.is_empty() => {
                super::rules::prove_adjacent(&context, plan.first, request, temp, home)?;
                if records.len() != 1 {
                    return Err("adjacent rule requires one load".into());
                }
            }
            #[cfg(test)]
            Rule::NonDecreasingControl if original == plan.replacement => {}
            #[cfg(test)]
            Rule::RemoveNop
                if original == [Instruction::Implied(super::super::selected::Implied::Nop)]
                    && plan.replacement.is_empty() => {}
            _ => return Err("rule does not prove replacement equivalence".into()),
        }
        let edited = replace(selected, start, end + 1, &plan.replacement)?;
        // CFG/generation were rebuilt by edited(). Rebuild dataflow once on
        // the fresh finalized selection below; positions are not proof facts.
        let scratch = layout::finalize(replay::emit(&edited, trace)?, true)?;
        let decreasing = true;
        #[cfg(test)]
        let decreasing = decreasing && plan.rule != Rule::Identity;
        if decreasing && scratch.bytes.len() >= output.bytes.len() {
            return Err("rewrite metric did not decrease".into());
        }
        let rebuilt = Context::new(
            scratch
                .selected
                .as_ref()
                .ok_or("missing rewritten selection")?,
        )?;
        if rebuilt.facts.undefined_private_reads().len()
            > context.facts.undefined_private_reads().len()
        {
            return Err("rewrite introduced undefined private reads".into());
        }
        Ok(scratch)
    }
}

/// Reindex symbolic parent links; encoded positions are deliberately discarded
/// by fresh replay. Only the checked driver can reach this edit operation.
fn replace(
    selected: &SelectedRoutine,
    start: usize,
    end: usize,
    replacement: &[Instruction],
) -> Result<SelectedRoutine, String> {
    let old = selected.records();
    let mut records = Vec::new();
    let mut map = BTreeMap::new();
    for (i, record) in old.iter().enumerate() {
        if i == start {
            for form in replacement {
                records.push(instruction_record(form, record));
            }
        }
        if (start..end).contains(&i) {
            continue;
        }
        map.insert(Node(i), Node(records.len()));
        records.push(record.clone());
    }
    reindex(selected, records, &map)
}

fn instruction_record(form: &Instruction, at: &Record) -> Record {
    Record {
        action: Action::Instruction {
            form: form.clone(),
            effects: form.effects(at.before.env),
            continuation: None,
        },
        parent: None,
        source: at.source,
        encoded: at.encoded.start..at.encoded.start,
        before: at.before,
        after: at.before,
        decision: None,
    }
}

fn reindex(
    selected: &SelectedRoutine,
    mut records: Vec<Record>,
    map: &BTreeMap<Node, Node>,
) -> Result<SelectedRoutine, String> {
    for record in &mut records {
        if let Some(parent) = record.parent {
            record.parent = Some(*map.get(&parent).ok_or("removed request parent")?);
        }
        if let Action::EndRequest(parent) = &mut record.action {
            *parent = *map.get(parent).ok_or("removed request start")?;
        }
    }
    selected.edited(records)
}

/// Materialize a previously recorded load candidate for analysis. No arbitrary
/// instruction insertion is exposed; publication still requires driver replay.
#[cfg(test)]
pub(super) fn insert_load(
    selected: &SelectedRoutine,
    before: Node,
    load: &Instruction,
) -> Result<SelectedRoutine, String> {
    selected.site(before)?;
    if !matches!(
        load,
        Instruction::Byte(
            super::super::selected::ByteOp::LdaStack | super::super::selected::ByteOp::LdaDp,
            _
        )
    ) {
        return Err("candidate is not a physical word LDA".into());
    }
    replace(selected, before.0, before.0, std::slice::from_ref(load))
}

/// Reconstruct the original stream once, reusing the driver's symbolic edits.
/// Only physical load insertion is admitted; fresh replay still gates use.
pub(super) fn expand_loads(
    selected: &SelectedRoutine,
    loads: &[(Node, &Instruction)],
) -> Result<SelectedRoutine, String> {
    if loads.is_empty() {
        return Ok(selected.clone());
    }
    if loads.windows(2).any(|p| p[0].0 >= p[1].0) {
        return Err("load expansion sites are duplicated or out of order".into());
    }
    for &(node, load) in loads {
        selected.site(node)?;
        if selected.records()[node.0].parent.is_some()
            || !matches!(
                load,
                Instruction::Byte(
                    super::super::selected::ByteOp::LdaStack
                        | super::super::selected::ByteOp::LdaDp,
                    _
                )
            )
        {
            return Err("expansion requires a top-level physical word LDA".into());
        }
    }
    #[cfg(any(test, feature = "native65816-state-proof"))]
    super::super::work::add("original_expansion", 1);
    let mut records = Vec::with_capacity(selected.records().len() + loads.len());
    let mut map = BTreeMap::new();
    let mut pending = loads.iter().peekable();
    for (i, record) in selected.records().iter().enumerate() {
        if let Some(&&(node, load)) = pending.peek() {
            if node == Node(i) {
                records.push(instruction_record(load, record));
                pending.next();
            }
        }
        map.insert(Node(i), Node(records.len()));
        records.push(record.clone());
    }
    reindex(selected, records, &map)
}
