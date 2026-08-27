use std::collections::{BTreeMap, BTreeSet};

use crate::includes::ModuleId;
use crate::semantic::SymbolId;
use crate::semantic::ir::{
    SemArrayDecay, SemCall, SemCallable, SemCondition, SemDeclaration, SemDeclarationStorage,
    SemExpr, SemExprKind, SemImplicitAddressOf, SemInitializerElementKind, SemInlineAsmTarget,
    SemItem, SemLValue, SemLValueKind, SemProgram, SemStmt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum LinkReason {
    DirectCall,
    RoutineAddress,
    StorageReference,
    AliasBacking,
    InitializerRelocation,
    MachineRelocation,
    EntryAlias,
    MachineFallthrough,
    RequiredPrefix,
    ConservativeLayout,
}

impl LinkReason {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::DirectCall => "direct-call",
            Self::RoutineAddress => "routine-address",
            Self::StorageReference => "storage-reference",
            Self::AliasBacking => "alias-backing",
            Self::InitializerRelocation => "initializer-relocation",
            Self::MachineRelocation => "machine-relocation",
            Self::EntryAlias => "entry-alias",
            Self::MachineFallthrough => "machine-fallthrough",
            Self::RequiredPrefix => "required-prefix",
            Self::ConservativeLayout => "conservative-layout",
        }
    }

    pub(crate) fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "direct-call" => Self::DirectCall,
            "routine-address" => Self::RoutineAddress,
            "storage-reference" => Self::StorageReference,
            "alias-backing" => Self::AliasBacking,
            "initializer-relocation" => Self::InitializerRelocation,
            "machine-relocation" => Self::MachineRelocation,
            "entry-alias" => Self::EntryAlias,
            "machine-fallthrough" => Self::MachineFallthrough,
            "required-prefix" => Self::RequiredPrefix,
            "conservative-layout" => Self::ConservativeLayout,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LinkEdge<Node> {
    pub(crate) target: Node,
    pub(crate) reason: LinkReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkGraph<Node> {
    nodes: BTreeSet<Node>,
    edges: BTreeMap<Node, BTreeSet<LinkEdge<Node>>>,
}

impl<Node> Default for LinkGraph<Node> {
    fn default() -> Self {
        Self {
            nodes: BTreeSet::new(),
            edges: BTreeMap::new(),
        }
    }
}

impl<Node: Clone + Ord> LinkGraph<Node> {
    pub(crate) fn add_node(&mut self, node: Node) {
        self.nodes.insert(node);
    }

    pub(crate) fn add_edge(&mut self, source: Node, target: Node, reason: LinkReason) {
        self.nodes.insert(source.clone());
        self.nodes.insert(target.clone());
        self.edges
            .entry(source)
            .or_default()
            .insert(LinkEdge { target, reason });
    }

    pub(crate) fn nodes(&self) -> &BTreeSet<Node> {
        &self.nodes
    }

    pub(crate) fn edges_from(&self, source: &Node) -> impl Iterator<Item = &LinkEdge<Node>> {
        self.edges.get(source).into_iter().flatten()
    }

    pub(crate) fn edges(&self) -> impl Iterator<Item = (&Node, &LinkEdge<Node>)> {
        self.edges
            .iter()
            .flat_map(|(source, edges)| edges.iter().map(move |edge| (source, edge)))
    }

    pub(crate) fn closure(
        &self,
        roots: impl IntoIterator<Item = Node>,
    ) -> Result<LinkSelection<Node>, Node> {
        let mut pending = BTreeSet::new();
        let mut retained = BTreeSet::new();
        let mut reasons = BTreeMap::new();
        for root in roots {
            if !self.nodes.contains(&root) {
                return Err(root);
            }
            pending.insert(root.clone());
            reasons.entry(root).or_insert(LinkRetention::Root);
        }
        while let Some(source) = pending.pop_first() {
            if !retained.insert(source.clone()) {
                continue;
            }
            for edge in self.edges_from(&source) {
                if !self.nodes.contains(&edge.target) {
                    return Err(edge.target.clone());
                }
                if !retained.contains(&edge.target) {
                    reasons.entry(edge.target.clone()).or_insert_with(|| {
                        LinkRetention::Dependency {
                            source: source.clone(),
                            reason: edge.reason,
                        }
                    });
                    pending.insert(edge.target.clone());
                }
            }
        }
        Ok(LinkSelection { retained, reasons })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkRetention<Node> {
    Root,
    Dependency { source: Node, reason: LinkReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkSelection<Node> {
    pub(crate) retained: BTreeSet<Node>,
    pub(crate) reasons: BTreeMap<Node, LinkRetention<Node>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SemLinkNode {
    Routine(SymbolId),
    Storage(SymbolId),
    TopLevel {
        module: Option<ModuleId>,
        ordinal: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemLinkPolicy {
    RetainAll,
    EntryReachable,
}

/// Extract source-level dependencies independently of the selection policy.
/// Optimized builds consume this graph to filter SemIR; compatibility builds
/// seed every executable node through the same closure implementation.
pub(crate) fn semir_link_graph(program: &SemProgram) -> LinkGraph<SemLinkNode> {
    let routine_ids = program
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            SemItem::Routine(routine) => Some(routine.symbol.id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let storage_ids = program
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            SemItem::Declaration(declaration) => Some(declaration.symbol.id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut graph = LinkGraph::default();
    let mut node_modules = BTreeMap::new();
    for module in &program.modules {
        let mut top_level = 0u32;
        for item in &module.items {
            match item {
                SemItem::Routine(routine) => {
                    let node = SemLinkNode::Routine(routine.symbol.id);
                    graph.add_node(node);
                    node_modules.insert(node, module.id);
                }
                SemItem::Declaration(declaration) => {
                    let node = SemLinkNode::Storage(declaration.symbol.id);
                    graph.add_node(node);
                    node_modules.insert(node, module.id);
                }
                SemItem::Set(_) | SemItem::Statement(_) => {
                    let node = SemLinkNode::TopLevel {
                        module: module.id,
                        ordinal: top_level,
                    };
                    top_level = top_level.wrapping_add(1);
                    graph.add_node(node);
                    node_modules.insert(node, module.id);
                }
                SemItem::Define(_)
                | SemItem::Const(_)
                | SemItem::Include(_)
                | SemItem::Unsupported { .. } => {}
            }
        }
    }

    let mut builder = SemGraphBuilder {
        graph,
        routine_ids,
        storage_ids,
        opaque_owners: BTreeSet::new(),
    };
    for module in &program.modules {
        let mut top_level = 0u32;
        for item in &module.items {
            match item {
                SemItem::Routine(routine) => {
                    let owner = SemLinkNode::Routine(routine.symbol.id);
                    for declaration in &routine.locals {
                        builder.declaration(owner, declaration);
                    }
                    for statement in &routine.body {
                        builder.statement(owner, statement);
                    }
                    if let Some(address) = &routine.system_address {
                        builder.expression(owner, address, LinkReason::InitializerRelocation);
                    }
                }
                SemItem::Declaration(declaration) => {
                    builder.declaration(SemLinkNode::Storage(declaration.symbol.id), declaration);
                }
                SemItem::Set(set) => {
                    let owner = SemLinkNode::TopLevel {
                        module: module.id,
                        ordinal: top_level,
                    };
                    top_level = top_level.wrapping_add(1);
                    builder.expression(owner, &set.address, LinkReason::StorageReference);
                    builder.expression(owner, &set.value, LinkReason::StorageReference);
                }
                SemItem::Statement(statement) => {
                    let owner = SemLinkNode::TopLevel {
                        module: module.id,
                        ordinal: top_level,
                    };
                    top_level = top_level.wrapping_add(1);
                    builder.statement(owner, statement);
                }
                SemItem::Unsupported { .. } => {}
                SemItem::Define(_) | SemItem::Const(_) | SemItem::Include(_) => {}
            }
        }
    }

    // Raw machine bodies can hide relative branches or byte-prefix ownership
    // which SemIR cannot prove. Retain their provider module as one explicit,
    // conservative layout group. Unreachable opaque routines do not become
    // roots merely because they exist.
    for owner in builder.opaque_owners.clone() {
        let Some(module) = node_modules.get(&owner) else {
            continue;
        };
        for (target, target_module) in &node_modules {
            if target != &owner && target_module == module {
                builder
                    .graph
                    .add_edge(owner, *target, LinkReason::ConservativeLayout);
            }
        }
    }
    builder.graph
}

pub(crate) fn select_semir(
    program: &SemProgram,
    policy: SemLinkPolicy,
) -> Result<SemProgram, String> {
    select_semir_with_plan(program, policy).map(|(program, _)| program)
}

pub(crate) fn select_semir_with_plan(
    program: &SemProgram,
    policy: SemLinkPolicy,
) -> Result<(SemProgram, LinkSelection<SemLinkNode>), String> {
    let graph = semir_link_graph(program);
    let roots = match policy {
        SemLinkPolicy::RetainAll => graph.nodes().iter().copied().collect::<BTreeSet<_>>(),
        SemLinkPolicy::EntryReachable => {
            let mut roots = graph
                .nodes()
                .iter()
                .filter(|node| {
                    matches!(
                        node,
                        SemLinkNode::TopLevel { module, .. }
                            if *module == program.root_module
                    )
                })
                .copied()
                .collect::<BTreeSet<_>>();
            roots.extend(graph.edges().filter_map(|(source, edge)| {
                (matches!(source, SemLinkNode::Storage(_))
                    && matches!(edge.target, SemLinkNode::Routine(_))
                    && edge.reason == LinkReason::InitializerRelocation)
                    .then_some(*source)
            }));
            if let Some(entry) = program.entry_routine {
                roots.insert(SemLinkNode::Routine(entry));
            }
            roots
        }
    };
    let selection = graph
        .closure(roots)
        .map_err(|node| format!("SemIR link root or dependency {node:?} is missing"))?;
    let mut selected = program.clone();
    for module in &mut selected.modules {
        let mut top_level = 0u32;
        module.items.retain(|item| match item {
            SemItem::Routine(routine) => selection
                .retained
                .contains(&SemLinkNode::Routine(routine.symbol.id)),
            SemItem::Declaration(declaration)
                if matches!(
                    declaration.storage,
                    SemDeclarationStorage::Type { .. } | SemDeclarationStorage::Record { .. }
                ) =>
            {
                true
            }
            SemItem::Declaration(declaration) => selection
                .retained
                .contains(&SemLinkNode::Storage(declaration.symbol.id)),
            SemItem::Set(_) | SemItem::Statement(_) => {
                let node = SemLinkNode::TopLevel {
                    module: module.id,
                    ordinal: top_level,
                };
                top_level = top_level.wrapping_add(1);
                selection.retained.contains(&node)
            }
            // These are compile-time facts or diagnostics and emit no target
            // storage by themselves.
            SemItem::Define(_)
            | SemItem::Const(_)
            | SemItem::Include(_)
            | SemItem::Unsupported { .. } => true,
        });
    }
    let retained_entry = selected.entry_routine.is_none_or(|entry| {
        selected
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .any(|item| matches!(item, SemItem::Routine(routine) if routine.symbol.id == entry))
    });
    if !retained_entry {
        return Err("SemIR link selection removed the program entry".to_string());
    }
    Ok((selected, selection))
}

struct SemGraphBuilder {
    graph: LinkGraph<SemLinkNode>,
    routine_ids: BTreeSet<SymbolId>,
    storage_ids: BTreeSet<SymbolId>,
    opaque_owners: BTreeSet<SemLinkNode>,
}

impl SemGraphBuilder {
    fn symbol(&mut self, owner: SemLinkNode, symbol: SymbolId, reason: LinkReason) {
        let target = if self.routine_ids.contains(&symbol) {
            SemLinkNode::Routine(symbol)
        } else if self.storage_ids.contains(&symbol) {
            SemLinkNode::Storage(symbol)
        } else {
            return;
        };
        if target != owner {
            self.graph.add_edge(owner, target, reason);
        }
    }

    fn declaration(&mut self, owner: SemLinkNode, declaration: &SemDeclaration) {
        if let Some(initializer) = &declaration.initializer {
            self.expression(owner, initializer, LinkReason::InitializerRelocation);
        }
        self.declaration_storage(owner, &declaration.storage);
    }

    fn declaration_storage(&mut self, owner: SemLinkNode, storage: &SemDeclarationStorage) {
        match storage {
            SemDeclarationStorage::Array { length, .. } => {
                if let Some(length) = length {
                    self.expression(owner, length, LinkReason::InitializerRelocation);
                }
            }
            SemDeclarationStorage::Type { fields, .. }
            | SemDeclarationStorage::Record { fields, .. } => {
                for field in fields {
                    self.declaration_storage(owner, &field.storage);
                }
            }
            SemDeclarationStorage::Scalar => {}
        }
    }

    fn statement(&mut self, owner: SemLinkNode, statement: &SemStmt) {
        match statement {
            SemStmt::LexicalBlock {
                declarations, body, ..
            } => {
                for declaration in declarations {
                    self.declaration(owner, declaration);
                }
                for statement in body {
                    self.statement(owner, statement);
                }
            }
            SemStmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.expression(owner, value, LinkReason::StorageReference);
                }
            }
            SemStmt::Assign { target, value, .. }
            | SemStmt::CompoundAssign { target, value, .. } => {
                self.lvalue(owner, target, LinkReason::StorageReference);
                self.expression(owner, value, LinkReason::StorageReference);
            }
            SemStmt::Call { call, .. } => self.call(owner, call),
            SemStmt::MachineBlock {
                resolved_symbols, ..
            } => {
                self.opaque_owners.insert(owner);
                for reference in resolved_symbols {
                    self.symbol(owner, reference.symbol.id, LinkReason::MachineRelocation);
                }
            }
            SemStmt::InlineAsm { program, .. } => {
                self.opaque_owners.insert(owner);
                for relocation in &program.relocations {
                    if let SemInlineAsmTarget::Symbol(symbol) = &relocation.target {
                        self.symbol(owner, symbol.id, LinkReason::MachineRelocation);
                    }
                }
            }
            SemStmt::If {
                branches,
                else_body,
                ..
            } => {
                for branch in branches {
                    self.condition(owner, &branch.condition);
                    for statement in &branch.body {
                        self.statement(owner, statement);
                    }
                }
                for statement in else_body {
                    self.statement(owner, statement);
                }
            }
            SemStmt::While {
                condition, body, ..
            } => {
                self.condition(owner, condition);
                for statement in body {
                    self.statement(owner, statement);
                }
            }
            SemStmt::DoUntil {
                body, condition, ..
            } => {
                for statement in body {
                    self.statement(owner, statement);
                }
                if let Some(condition) = condition {
                    self.condition(owner, condition);
                }
            }
            SemStmt::For {
                target,
                start,
                end,
                step,
                body,
                ..
            } => {
                self.lvalue(owner, target, LinkReason::StorageReference);
                self.expression(owner, start, LinkReason::StorageReference);
                self.expression(owner, end, LinkReason::StorageReference);
                if let Some(step) = step {
                    self.expression(owner, step, LinkReason::StorageReference);
                }
                for statement in body {
                    self.statement(owner, statement);
                }
            }
            SemStmt::Unsupported { .. } => {
                self.opaque_owners.insert(owner);
            }
            SemStmt::Define(_) | SemStmt::Exit { .. } => {}
        }
    }

    fn condition(&mut self, owner: SemLinkNode, condition: &SemCondition) {
        self.expression(owner, &condition.expr, LinkReason::StorageReference);
    }

    fn call(&mut self, owner: SemLinkNode, call: &SemCall) {
        match &call.callee {
            SemCallable::User(symbol) | SemCallable::Builtin(symbol) => {
                self.symbol(owner, symbol.id, LinkReason::DirectCall);
            }
            SemCallable::Indirect { target, .. } => {
                self.expression(owner, target, LinkReason::RoutineAddress);
            }
            SemCallable::Runtime { .. } => {}
        }
        for argument in &call.args {
            self.expression(owner, argument, LinkReason::StorageReference);
        }
    }

    fn expression(&mut self, owner: SemLinkNode, expression: &SemExpr, reason: LinkReason) {
        match &expression.kind {
            SemExprKind::InitializerList(elements) => {
                for element in elements {
                    if let SemInitializerElementKind::Address { target, .. } = &element.kind {
                        self.symbol(owner, target.id, LinkReason::InitializerRelocation);
                    }
                }
            }
            SemExprKind::Symbol(symbol) | SemExprKind::AddressOfSymbol(symbol) => {
                let reason = if self.routine_ids.contains(&symbol.id) {
                    LinkReason::RoutineAddress
                } else {
                    reason
                };
                self.symbol(owner, symbol.id, reason);
            }
            SemExprKind::LValue(place) | SemExprKind::AddressOf(place) => {
                self.lvalue(owner, place, reason);
            }
            SemExprKind::ImplicitAddressOf(SemImplicitAddressOf { place, .. }) => {
                self.lvalue(owner, place, reason);
            }
            SemExprKind::ArrayDecay(SemArrayDecay { array, .. }) => {
                self.lvalue(owner, array, reason);
            }
            SemExprKind::Cast { expr, .. } | SemExprKind::Unary { expr, .. } => {
                self.expression(owner, expr, reason);
            }
            SemExprKind::Binary { left, right, .. } => {
                self.expression(owner, left, reason);
                self.expression(owner, right, reason);
            }
            SemExprKind::Call(call) => self.call(owner, call),
            SemExprKind::Missing | SemExprKind::CurrentLocation | SemExprKind::Literal(_) => {}
            SemExprKind::Raw(_) | SemExprKind::UnresolvedName(_) => {
                self.opaque_owners.insert(owner);
            }
        }
    }

    fn lvalue(&mut self, owner: SemLinkNode, value: &SemLValue, reason: LinkReason) {
        if let Some(symbol) = value
            .storage
            .as_ref()
            .and_then(|storage| storage.symbol.as_ref())
        {
            self.symbol(owner, symbol.id, reason);
        }
        match &value.kind {
            SemLValueKind::Symbol(symbol) => self.symbol(owner, symbol.id, reason),
            SemLValueKind::Deref { pointer } => self.expression(owner, pointer, reason),
            SemLValueKind::Index { base, index, .. } => {
                self.expression(owner, base, reason);
                self.expression(owner, index, reason);
            }
            SemLValueKind::Field { base, .. } => self.lvalue(owner, base, reason),
            SemLValueKind::UnresolvedName(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::semantic::{SemanticOptions, analyze_with_options};

    #[test]
    fn semir_graph_audits_transitive_calls_and_storage_without_pruning() {
        let tokens =
            tokenize("BYTE g PROC Leaf() g=1 RETURN PROC Dead() RETURN PROC Start() Leaf() RETURN")
                .expect("tokenize link graph source");
        let ast = parse(&tokens).expect("parse link graph source");
        let model = analyze_with_options(&ast, SemanticOptions::modern())
            .expect("analyze link graph source");
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        let graph = semir_link_graph(&semir);
        let routine = |name: &str| {
            semir
                .modules
                .iter()
                .flat_map(|module| &module.items)
                .find_map(|item| match item {
                    SemItem::Routine(routine) if routine.symbol.name == name => {
                        Some(routine.symbol.id)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing routine {name}"))
        };
        let storage = semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find_map(|item| match item {
                SemItem::Declaration(declaration) if declaration.symbol.name == "g" => {
                    Some(declaration.symbol.id)
                }
                _ => None,
            })
            .expect("global g");
        let selection = graph
            .closure([SemLinkNode::Routine(routine("Start"))])
            .expect("entry is a graph node");

        assert!(
            selection
                .retained
                .contains(&SemLinkNode::Routine(routine("Start")))
        );
        assert!(
            selection
                .retained
                .contains(&SemLinkNode::Routine(routine("Leaf")))
        );
        assert!(selection.retained.contains(&SemLinkNode::Storage(storage)));
        assert!(
            !selection
                .retained
                .contains(&SemLinkNode::Routine(routine("Dead")))
        );
        assert_eq!(
            semir.modules[0].items.len(),
            4,
            "audit mode must not prune SemIR"
        );

        let selected = select_semir(&semir, SemLinkPolicy::EntryReachable)
            .expect("select entry-reachable SemIR");
        let retained = selected.modules[0]
            .items
            .iter()
            .filter_map(|item| match item {
                SemItem::Routine(routine) => Some(routine.symbol.name.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(retained, BTreeSet::from(["Leaf", "Start"]));

        let compatible = select_semir(&semir, SemLinkPolicy::RetainAll).expect("retain all SemIR");
        assert_eq!(
            compatible.modules[0].items.len(),
            semir.modules[0].items.len()
        );
    }

    #[test]
    fn selected_opaque_machine_routine_conservatively_retains_its_module() {
        let tokens =
            tokenize("PROC Hidden() RETURN PROC Machine() [$EA] PROC Start() Machine() RETURN")
                .expect("tokenize opaque machine source");
        let ast = parse(&tokens).expect("parse opaque machine source");
        let model = analyze_with_options(&ast, SemanticOptions::modern())
            .expect("analyze opaque machine source");
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        let selected = select_semir(&semir, SemLinkPolicy::EntryReachable)
            .expect("select opaque machine source");
        let retained = selected.modules[0]
            .items
            .iter()
            .filter_map(|item| match item {
                SemItem::Routine(routine) => Some(routine.symbol.name.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(retained, BTreeSet::from(["Hidden", "Machine", "Start"]));
    }
}
