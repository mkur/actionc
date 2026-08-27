use std::collections::{BTreeMap, BTreeSet};

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
    TopLevel(u32),
}

/// Extract source-level dependencies without pruning. Slice 3 will consume
/// this graph to filter SemIR; slice 2 deliberately uses it only for audit and
/// determinism checks.
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
    let mut builder = SemGraphBuilder {
        graph: LinkGraph::default(),
        routine_ids,
        storage_ids,
    };
    for id in &builder.routine_ids {
        builder.graph.add_node(SemLinkNode::Routine(*id));
    }
    for id in &builder.storage_ids {
        builder.graph.add_node(SemLinkNode::Storage(*id));
    }

    let mut top_level = 0u32;
    for module in &program.modules {
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
                    let owner = SemLinkNode::TopLevel(top_level);
                    top_level = top_level.wrapping_add(1);
                    builder.graph.add_node(owner);
                    builder.expression(owner, &set.address, LinkReason::StorageReference);
                    builder.expression(owner, &set.value, LinkReason::StorageReference);
                }
                SemItem::Statement(statement) => {
                    let owner = SemLinkNode::TopLevel(top_level);
                    top_level = top_level.wrapping_add(1);
                    builder.graph.add_node(owner);
                    builder.statement(owner, statement);
                }
                SemItem::Define(_)
                | SemItem::Const(_)
                | SemItem::Include(_)
                | SemItem::Unsupported { .. } => {}
            }
        }
    }
    builder.graph
}

struct SemGraphBuilder {
    graph: LinkGraph<SemLinkNode>,
    routine_ids: BTreeSet<SymbolId>,
    storage_ids: BTreeSet<SymbolId>,
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
                for reference in resolved_symbols {
                    self.symbol(owner, reference.symbol.id, LinkReason::MachineRelocation);
                }
            }
            SemStmt::InlineAsm { program, .. } => {
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
            SemStmt::Define(_) | SemStmt::Exit { .. } | SemStmt::Unsupported { .. } => {}
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
            SemExprKind::Missing
            | SemExprKind::Raw(_)
            | SemExprKind::UnresolvedName(_)
            | SemExprKind::CurrentLocation
            | SemExprKind::Literal(_) => {}
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
    }
}
