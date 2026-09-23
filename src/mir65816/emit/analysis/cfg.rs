//! Intraprocedural selected-action graph. Compiler requests are zero-byte sites;
//! only typed transfers add nonsequential edges. PCs never identify a node.
use super::super::effects::Control;
use super::super::selected::*;
use super::super::state::{Environment, Width};
use super::super::{Code, Label, Target};
use super::sites::Node;
use crate::analysis::graph::DataflowGraph;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(in crate::mir65816::emit) struct SelectedCfg {
    nodes: BTreeSet<Node>,
    predecessors: BTreeMap<Node, BTreeSet<Node>>,
    successors: BTreeMap<Node, BTreeSet<Node>>,
    reachable: BTreeSet<Node>,
    postorder: Vec<Node>,
    reverse_postorder: Vec<Node>,
}
impl SelectedCfg {
    pub fn build(records: &[Record]) -> Result<Self, String> {
        #[cfg(any(test, feature = "native65816-state-proof"))]
        crate::mir65816::emit::work::add("cfg", 1);
        if !matches!(records.first().map(|r| &r.action), Some(Action::Entry)) {
            return Err("missing selected entry".into());
        }
        let mut allocated = BTreeSet::new();
        let mut labels = BTreeMap::new();
        let mut returns = None;
        let mut faults = None;
        let mut parents = Vec::new();
        let mut blocks: BTreeSet<Label> = BTreeSet::new();
        let mut expected = None;
        let mut expected_reachable = None;
        let mut actual: BTreeMap<Label, BTreeMap<Option<Label>, usize>> = BTreeMap::new();
        let mut active = None;
        let mut live = true;
        let mut pending_return = None;
        let mut edge = |source, target| {
            *actual.entry(target).or_default().entry(source).or_default() += 1;
        };
        for (i, r) in records.iter().enumerate() {
            if r.decision.is_some() && !matches!(r.action, Action::EndRequest(_)) {
                return Err("selected decision outside request end".into());
            }
            if r.parent != parents.last().copied() {
                return Err("selected request parent mismatch".into());
            }
            match &r.action {
                Action::Entry if i != 0 => return Err("duplicate selected entry".into()),
                Action::Request(request) => {
                    parents.push(Node(i));
                    match request {
                        Request::DeclareBlocks(declared) => {
                            if !blocks.is_empty() {
                                return Err("duplicate MIR block declaration".into());
                            }
                            blocks.extend(declared);
                        }
                        Request::ProveEntries {
                            predecessors,
                            reachable,
                        } => {
                            if expected.is_some()
                                || predecessors.keys().copied().collect::<BTreeSet<_>>() != blocks
                                || !reachable.is_subset(&blocks)
                            {
                                return Err("invalid selected MIR entry obligations".into());
                            }
                            expected = Some(predecessors.clone());
                            expected_reachable = Some(reachable.clone());
                        }
                        Request::Fallthrough(target) => {
                            if !blocks.contains(target) {
                                return Err("non-MIR fallthrough".into());
                            }
                            edge(active, *target);
                            live = false;
                        }
                        _ => {}
                    }
                }
                Action::EndRequest(begin) => {
                    if parents.pop() != Some(*begin) {
                        return Err("unbalanced selected request".into());
                    }
                    let returns_decision = matches!(
                        records[begin.0].action,
                        Action::Request(
                            Request::ConsumeWord(..)
                                | Request::ConsumeFrame(..)
                                | Request::StoreIncoming(..)
                                | Request::LoadX(..)
                        )
                    );
                    if r.decision.is_some() != returns_decision {
                        return Err("missing or unexpected selected consume decision".into());
                    }
                    if let Action::Request(Request::Mode(width)) = records[begin.0].action {
                        if r.after.env.m != width {
                            return Err("unfulfilled selected mode request".into());
                        }
                    }
                    if let Action::Request(Request::EstablishBody) = records[begin.0].action {
                        if r.before.env.anchor.is_some()
                            || r.after.env.anchor != Some(r.after.env.depth)
                            || r.after.env.pushes != 0
                            || r.after.stack_a != Some(-r.after.env.depth)
                        {
                            return Err("invalid selected body anchor".into());
                        }
                    } else if r.before.env != r.after.env {
                        return Err("compiler request changed the physical environment".into());
                    }
                }
                Action::Allocate(label) => {
                    if !allocated.insert(*label) {
                        return Err("duplicate selected label allocation".into());
                    }
                }
                Action::Bind(label) => {
                    if !allocated.contains(label) || labels.insert(*label, Node(i)).is_some() {
                        return Err("unallocated or duplicate selected label".into());
                    }
                    if blocks.contains(label) {
                        if live {
                            edge(active, *label);
                        }
                        active = Some(*label);
                    }
                    live = true;
                }
                Action::Instruction {
                    form,
                    effects,
                    continuation,
                } => {
                    validate_instruction(r, form)?;
                    if let Instruction::PushReturn(label) = form {
                        if pending_return.replace(*label).is_some() {
                            return Err("nested selected PER continuation".into());
                        }
                    }
                    if matches!(form, Instruction::IndirectTransfer(_))
                        && pending_return.take() != *continuation
                    {
                        return Err("indirect selected continuation differs from PER".into());
                    }
                    if *effects != form.effects(r.before.env) {
                        return Err("selected effects do not match typed form".into());
                    }
                    match effects.control {
                        Control::Branch { target, .. } | Control::Jump(Target::Label(target)) => {
                            if blocks.contains(&target) {
                                edge(active, target);
                            }
                        }
                        _ => {}
                    }
                    match effects.control {
                        Control::Jump(_) | Control::Return => live = false,
                        Control::Call { target: None } => {
                            if continuation.is_none() {
                                return Err("indirect call without continuation".into());
                            }
                            live = false;
                        }
                        _ if continuation.is_some() => {
                            return Err("unexpected selected continuation".into());
                        }
                        _ => {}
                    }
                }
                Action::ReturnExit => {
                    if returns.replace(Node(i)).is_some() {
                        return Err("duplicate return exit".into());
                    }
                }
                Action::FaultExit => {
                    if faults.replace(Node(i)).is_some() {
                        return Err("duplicate fault exit".into());
                    }
                }
                _ => {}
            }
        }
        if pending_return.is_some()
            || !parents.is_empty()
            || allocated != labels.keys().copied().collect()
        {
            return Err("unclosed selected requests or labels".into());
        }
        if let Some(expected) = expected {
            for label in &blocks {
                actual.entry(*label).or_default();
            }
            if expected != actual {
                return Err("selected MIR predecessor multiplicities changed".into());
            }
        }
        for r in records {
            let target = match &r.action {
                Action::Instruction {
                    form:
                        Instruction::Reference(_, Target::Label(l), ..) | Instruction::PushReturn(l),
                    ..
                } => Some(l),
                _ => None,
            };
            if target.is_some_and(|l| !labels.contains_key(l)) {
                return Err("unbound symbolic selected label".into());
            }
        }
        let returns = returns.ok_or("missing selected return exit")?;
        let faults = faults.ok_or("missing selected fault exit")?;
        let nodes: BTreeSet<_> = (0..records.len()).map(Node).collect();
        let mut successors: BTreeMap<_, BTreeSet<_>> =
            nodes.iter().map(|&n| (n, BTreeSet::new())).collect();
        let mut predecessors = successors.clone();
        let label = |l: Label| {
            labels
                .get(&l)
                .copied()
                .ok_or_else(|| format!("unbound selected target {l:?}"))
        };
        for (i, r) in records.iter().enumerate() {
            let next = Node(i + 1);
            let targets = match &r.action {
                Action::ReturnExit | Action::FaultExit => vec![],
                Action::Request(Request::Fallthrough(l)) => vec![label(*l)?],
                Action::Instruction {
                    effects,
                    continuation,
                    ..
                } => match effects.control {
                    Control::Next | Control::Call { target: Some(_) } => vec![next],
                    Control::Call { target: None } => {
                        vec![label(continuation.ok_or("missing call continuation")?)?]
                    }
                    Control::Branch { target, .. } => vec![label(target)?, next],
                    Control::Jump(Target::Label(l)) => vec![label(l)?],
                    Control::Jump(Target::StackOverflow) => vec![faults],
                    Control::Return => vec![returns],
                    Control::Jump(_) => return Err("unsupported selected transfer boundary".into()),
                },
                _ => vec![next],
            };
            for target in targets {
                records.get(target.0).ok_or("selected falloff")?;
                successors.get_mut(&Node(i)).unwrap().insert(target);
                predecessors.get_mut(&target).unwrap().insert(Node(i));
            }
        }
        let mut reachable = BTreeSet::new();
        let mut postorder = Vec::new();
        let mut pending = vec![(Node(0), false)];
        while let Some((node, finished)) = pending.pop() {
            if finished {
                postorder.push(node);
            } else if reachable.insert(node) {
                pending.push((node, true));
                pending.extend(successors[&node].iter().rev().map(|&n| (n, false)));
            }
        }
        for &node in &reachable {
            let r = &records[node.0];
            for target in &successors[&node] {
                let target_record = &records[target.0];
                if matches!(target_record.action, Action::ReturnExit | Action::FaultExit) {
                    continue;
                }
                let incoming = if matches!(target_record.action, Action::Bind(_)) {
                    target_record.after
                } else {
                    target_record.before
                };
                compatible(r.after.env, incoming.env)
                    .map_err(|e| format!("selected edge {}->{}: {e}", node.0, target.0))?;
                if incoming.stack_a.is_some() && incoming.stack_a != r.after.stack_a {
                    return Err("selected edge invents a stack equation".into());
                }
            }
        }
        if let Some(expected) = expected_reachable {
            let actual = labels
                .iter()
                .filter_map(|(label, node)| {
                    (blocks.contains(label) && reachable.contains(node)).then_some(*label)
                })
                .collect::<BTreeSet<_>>();
            if actual != expected {
                return Err("selected MIR reachability disagrees with obligations".into());
            }
        }
        for &node in &predecessors[&returns] {
            if reachable.contains(&node)
                && !matches!(&records[node.0].action, Action::Instruction { effects, .. } if effects.control == Control::Return)
            {
                return Err("reachable selected code falls off routine".into());
            }
        }
        let reverse_postorder = postorder.iter().rev().copied().collect();
        Ok(Self {
            nodes,
            predecessors,
            successors,
            reachable,
            postorder,
            reverse_postorder,
        })
    }
}

fn compatible(from: Environment, to: Environment) -> Result<(), String> {
    if (
        from.m,
        from.index,
        from.native,
        from.depth,
        from.anchor,
        from.pushes,
        from.current_domain,
    ) != (
        to.m,
        to.index,
        to.native,
        to.depth,
        to.anchor,
        to.pushes,
        to.current_domain,
    ) || to.decimal.is_some() && to.decimal != from.decimal
        || to.dbr.is_some() && to.dbr != from.dbr
        || to.irq_preserved && !from.irq_preserved
    {
        Err("incompatible mode, stack or environment boundary".into())
    } else {
        Ok(())
    }
}

fn validate_instruction(r: &Record, form: &Instruction) -> Result<(), String> {
    let before = r.before.env;
    let after = r.after.env;
    if !before.native || !before.current_domain {
        return Err("unsupported selected environment".into());
    }
    let mut expected = before;
    match form {
        Instruction::Byte(
            ByteOp::LdaImm | ByteOp::AdcImm | ByteOp::SbcImm | ByteOp::CmpImm | ByteOp::EorImm,
            _,
        )
        | Instruction::Reference(ReferenceOp::LdaByte, _, _, _)
            if before.m != Width::Byte =>
        {
            return Err("selected byte immediate in A16".into());
        }
        Instruction::Word(WordOp::LdyImm | WordOp::CpxImm, _) if before.index != Width::Word => {
            return Err("selected word index immediate in X8".into());
        }
        Instruction::Word(op, _)
            if !matches!(op, WordOp::LdyImm | WordOp::CpxImm) && before.m != Width::Word =>
        {
            return Err("selected word immediate in A8".into());
        }
        Instruction::Byte(op @ (ByteOp::Rep | ByteOp::Sep), mask) => {
            let set = *op == ByteOp::Sep;
            if mask & 0x20 != 0 {
                expected.m = if set { Width::Byte } else { Width::Word };
            }
            if mask & 0x10 != 0 {
                expected.index = if set { Width::Byte } else { Width::Word };
            }
            if mask & 8 != 0 {
                expected.decimal = Some(set);
            }
            if mask & 4 != 0 {
                expected.irq_preserved = false;
            }
        }
        Instruction::Implied(Implied::Tcs) => {
            if before.pushes != 0 {
                return Err("selected TCS during transfer".into());
            }
            expected.depth = -r
                .before
                .stack_a
                .ok_or("selected TCS without stack equation")?;
        }
        Instruction::Implied(Implied::Phk | Implied::Pha) | Instruction::PushReturn(_) => {
            if matches!(form, Instruction::PushReturn(_)) && before.pushes != 1 {
                return Err("selected PER without PHK phase".into());
            }
            let bytes = match form {
                Instruction::Implied(Implied::Phk) => 1,
                Instruction::PushReturn(_) => 2,
                _ => before.m.bytes(),
            };
            expected.depth += i64::from(bytes);
            expected.pushes = expected
                .pushes
                .checked_add(bytes)
                .ok_or("selected push overflow")?;
        }
        Instruction::NativeCall(..)
        | Instruction::Reference(ReferenceOp::Jsl, ..)
        | Instruction::IndirectTransfer(_) => {
            if before.m != Width::Word || before.index != Width::Word {
                return Err("selected call outside native mode contract".into());
            }
            if matches!(form, Instruction::IndirectTransfer(_)) {
                if before.pushes != 6 {
                    return Err("invalid selected indirect transfer stack".into());
                }
                expected.depth -= 6;
                expected.pushes = 0;
            } else if before.pushes != 0 {
                return Err("direct selected call during indirect setup".into());
            }
            expected.irq_preserved = false;
            expected.decimal = Some(false);
            expected.dbr = Some(0);
            expected.current_domain = true;
        }
        Instruction::NativeReturn(_) | Instruction::Implied(Implied::Rtl) => {
            if before.depth != 0
                || before.pushes != 0
                || before.m != Width::Word
                || before.index != Width::Word
            {
                return Err("selected return violates native stack/mode contract".into());
            }
        }
        _ => {}
    }
    if expected != after {
        return Err(format!(
            "selected instruction environment mismatch: {form:?}"
        ));
    }
    Ok(())
}

impl DataflowGraph for SelectedCfg {
    type Node = Node;
    fn entry(&self) -> Option<Node> {
        Some(Node(0))
    }
    fn nodes(&self) -> &BTreeSet<Node> {
        &self.nodes
    }
    fn predecessors(&self, node: Node) -> &BTreeSet<Node> {
        &self.predecessors[&node]
    }
    fn successors(&self, node: Node) -> &BTreeSet<Node> {
        &self.successors[&node]
    }
    fn reachable(&self) -> &BTreeSet<Node> {
        &self.reachable
    }
    fn postorder(&self) -> &[Node] {
        &self.postorder
    }
    fn reverse_postorder(&self) -> &[Node] {
        &self.reverse_postorder
    }
}

/// Reconcile typed records, ownership and symbolic fixups. This reads no opcode
/// semantics from bytes: expected encoding comes exclusively from typed forms.
pub(in crate::mir65816::emit) fn reconcile(records: &[Record], code: &Code) -> Result<(), String> {
    let mut cursor = 0;
    let mut fixups = Vec::new();
    let mut returns = Vec::new();
    let mut labels = BTreeMap::new();
    let mut spans = BTreeMap::new();
    let mut source = None;
    let mut branches = Vec::new();
    let mut blocks: BTreeSet<Label> = BTreeSet::new();
    let mut active = None;
    let mut transfers = Vec::new();
    let mut fallthrough = None;
    for r in records {
        let at = r.encoded.start;
        if at != cursor || r.encoded.end > code.bytes.len() {
            return Err("selected encoding gap, overlap or invalid boundary".into());
        }
        if at != 0 && !code.boundaries.contains(&at) {
            return Err("selected site is not an emission boundary".into());
        }
        if r.source != source.map(|(s, _)| s) && !matches!(r.action, Action::SourceStart(_)) {
            return Err("selected source attribution mismatch".into());
        }
        if let Action::Instruction { form, .. } = &r.action {
            if fallthrough.is_some() {
                return Err("instruction inside selected fallthrough".into());
            }
            let mut reference = |op, target, addend, byte: Option<u8>| {
                fixups.push((at + 1, target, addend, byte));
                let mut b = vec![op];
                b.extend(std::iter::repeat_n(0, if byte.is_some() { 1 } else { 3 }));
                b
            };
            let expected = match form {
                Instruction::Implied(op) => vec![op.opcode()],
                Instruction::Byte(op, value) => vec![op.opcode(), *value],
                Instruction::Word(op, value) => {
                    let [lo, hi] = value.to_le_bytes();
                    vec![op.opcode(), lo, hi]
                }
                Instruction::Long(op, value) => {
                    let mut b = vec![op.opcode()];
                    b.extend(&value.to_le_bytes()[..3]);
                    b
                }
                Instruction::Reference(op, target, addend, byte) => {
                    reference(op.opcode(), *target, *addend, *byte)
                }
                Instruction::NativeCall(target, _) => reference(0x22, *target, 0, None),
                Instruction::NativeReturn(_) | Instruction::IndirectTransfer(_) => vec![0x6b],
                Instruction::PushReturn(label) => {
                    returns.push((at + 1, *label));
                    vec![0x62, 0, 0]
                }
                Instruction::Branch(op, label) => {
                    if r.encoded.len() == 2 {
                        let target = *code
                            .labels
                            .get(label)
                            .ok_or("unbound short selected branch")?;
                        let delta = i8::try_from(target as i64 - (at + 2) as i64)
                            .map_err(|_| "selected short branch out of range")?;
                        vec![op.opcode(), delta as u8]
                    } else {
                        fixups.push((at + 3, Target::Label(*label), 0, None));
                        vec![op.opcode() ^ 0x20, 4, 0x5c, 0, 0, 0]
                    }
                }
            };
            if code.bytes.get(r.encoded.clone()) != Some(expected.as_slice()) {
                return Err(format!("selected encoding mismatch at {at}: {form:?}"));
            }
            cursor = r.encoded.end;
        } else {
            if !r.encoded.is_empty() {
                return Err("compiler event emits bytes".into());
            }
            match &r.action {
                Action::Bind(l) => {
                    if fallthrough.take().is_some_and(|target| target != *l) {
                        return Err("selected fallthrough binds another label".into());
                    }
                    if labels.insert(*l, at).is_some() {
                        return Err("duplicate selected binding".into());
                    }
                    if blocks.contains(l) {
                        active = Some(*l);
                    }
                }
                Action::SourceStart(s) => {
                    if source.replace((*s, at)).is_some() {
                        return Err("nested selected source".into());
                    }
                }
                Action::SourceEnd {
                    source: s,
                    fused_terminator,
                } => {
                    let (old, start) = source.take().ok_or("unopened selected source")?;
                    if old != *s
                        || fused_terminator.is_some_and(|t| t != s.index + 1)
                        || spans.insert((s.block, s.index), start..at).is_some()
                    {
                        return Err("invalid selected source span".into());
                    }
                }
                Action::Request(Request::DeclareBlocks(b)) => blocks.extend(b),
                Action::Request(Request::Jump(l) | Request::Fallthrough(l))
                    if blocks.contains(l) =>
                {
                    if matches!(r.action, Action::Request(Request::Fallthrough(_))) {
                        fallthrough = Some(*l);
                    }
                    transfers.push((
                        active.ok_or("selected MIR transfer without source")?,
                        *l,
                        at,
                        matches!(r.action, Action::Request(Request::Fallthrough(_))),
                    ));
                }
                Action::Request(Request::Dispatch(op, l)) => branches.push((at, op.opcode(), *l)),
                _ => {}
            }
        }
    }
    if source.is_some()
        || fallthrough.is_some()
        || cursor != code.bytes.len()
        || labels != code.labels
        || spans != code.mir_spans
        || fixups
            != code
                .fixups
                .iter()
                .map(|f| (f.offset, f.target, f.addend, f.byte))
                .collect::<Vec<_>>()
        || returns != code.return_fixups
        || branches
            != code
                .conditional_branches
                .iter()
                .map(|b| (b.offset, b.predicate, b.target))
                .collect::<Vec<_>>()
        || transfers
            != code
                .mir_transfers
                .iter()
                .map(|t| (t.source, t.target, t.offset, t.fallthrough))
                .collect::<Vec<_>>()
    {
        return Err("selected bytes, fixups, labels, sources or transfers do not reconcile".into());
    }
    #[cfg(feature = "native65816-state-proof")]
    {
        for snapshot in &code.state_trace {
            if snapshot.pc > code.bytes.len()
                || snapshot.pc != 0 && !code.boundaries.contains(&snapshot.pc)
            {
                return Err("state trace is not on a selected boundary".into());
            }
        }
        if !code.instruction_effects.is_empty() {
            let instructions: Vec<_> = records
                .iter()
                .filter_map(|r| {
                    if let Action::Instruction { effects, .. } = &r.action {
                        Some((r.encoded.start, r.encoded.end, effects))
                    } else {
                        None
                    }
                })
                .collect();
            if instructions
                != code
                    .instruction_effects
                    .iter()
                    .map(|r| (r.start, r.end, &r.effects))
                    .collect::<Vec<_>>()
            {
                return Err("selected effects disagree with independent observer".into());
            }
        }
    }
    Ok(())
}
