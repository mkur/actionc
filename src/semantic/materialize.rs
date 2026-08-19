use crate::asm6502::{InlineAsmProgram, InlineAsmRelocationTarget};
use crate::ast::*;

use super::{ConstValue, ScalarType, ScopeId, SemanticModel, SemanticNameResolution};

/// Prepare the semantic AST for consumers that still operate on source AST
/// expressions.  CONST declarations have no runtime representation, and every
/// reference is replaced with the typed value established by semantic
/// analysis.  This is a compatibility bridge for the classic code generator;
/// SemIR and NIR carry the same values directly.
pub(crate) fn materialize_constants(program: &Program, model: &SemanticModel) -> Program {
    Materializer { model }.program(program)
}

struct Materializer<'a> {
    model: &'a SemanticModel,
}

impl Materializer<'_> {
    fn program(&mut self, program: &Program) -> Program {
        let global_scope = self.model.symbols.global_scope();
        Program {
            modules: program
                .modules
                .iter()
                .map(|module| Module {
                    items: module
                        .items
                        .iter()
                        .filter_map(|item| self.item(global_scope, item))
                        .collect(),
                })
                .collect(),
            source_kind: program.source_kind.clone(),
        }
    }

    fn item(&mut self, global_scope: ScopeId, item: &Item) -> Option<Item> {
        let mut item = item.clone();
        match &mut item {
            Item::Set(set) => {
                self.expr(global_scope, &mut set.address);
                self.expr(global_scope, &mut set.value);
            }
            Item::Declaration(Decl::Const(_)) => return None,
            Item::Declaration(declaration) => self.declaration(global_scope, declaration),
            Item::Routine(routine) => self.routine(global_scope, routine),
            Item::Statement(statement) => self.statement(global_scope, statement),
            Item::Define(_) | Item::Include(_) | Item::Unsupported { .. } => {}
        }
        Some(item)
    }

    fn routine(&mut self, global_scope: ScopeId, routine: &mut Routine) {
        let routine_symbol = self.model.symbols.lookup(global_scope, &routine.name);
        let routine_scope = self
            .model
            .routine_scopes
            .iter()
            .find(|scope| scope.symbol == routine_symbol)
            .map(|scope| scope.scope)
            .unwrap_or(global_scope);

        if let Some(system_address) = &mut routine.system_address {
            self.expr(global_scope, system_address);
        }
        for parameter in &mut routine.params {
            self.var_declaration(routine_scope, parameter);
        }
        routine
            .locals
            .retain(|decl| !matches!(decl, Decl::Const(_)));
        for declaration in &mut routine.locals {
            self.declaration(routine_scope, declaration);
        }
        for statement in &mut routine.body {
            self.statement(routine_scope, statement);
        }
    }

    fn declaration(&self, scope: ScopeId, declaration: &mut Decl) {
        match declaration {
            Decl::Var(declaration) => self.var_declaration(scope, declaration),
            Decl::Type(declaration) => {
                for field in &mut declaration.fields {
                    self.var_declaration(scope, field);
                }
            }
            Decl::Record(declaration) => {
                for field in &mut declaration.fields {
                    self.var_declaration(scope, field);
                }
            }
            Decl::Const(_) => {}
        }
    }

    fn var_declaration(&self, scope: ScopeId, declaration: &mut VarDecl) {
        // The classic AST bridge must retain volatility inferred by semantic
        // alias resolution, not only an explicit source qualifier.
        declaration.qualifiers.is_volatile |= declaration.entries.iter().any(|entry| {
            self.model
                .symbols
                .lookup(scope, &entry.name)
                .is_some_and(|id| self.model.symbols.symbols[id.0].is_volatile)
        });
        for entry in &mut declaration.entries {
            if let Some(size) = &mut entry.size {
                self.expr(scope, size);
            }
            if let Some(initializer) = &mut entry.initializer {
                self.expr(scope, initializer);
            }
        }
    }

    fn statement(&self, scope: ScopeId, statement: &mut Stmt) {
        match statement {
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.expr(scope, value);
                }
            }
            Stmt::Assign { target, value, .. } | Stmt::CompoundAssign { target, value, .. } => {
                self.expr(scope, target);
                self.expr(scope, value);
            }
            Stmt::Call { expr, .. } => self.expr(scope, expr),
            Stmt::If {
                branches,
                else_body,
                ..
            } => {
                for branch in branches {
                    self.expr(scope, &mut branch.condition);
                    for statement in &mut branch.body {
                        self.statement(scope, statement);
                    }
                }
                for statement in else_body {
                    self.statement(scope, statement);
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.expr(scope, condition);
                for statement in body {
                    self.statement(scope, statement);
                }
            }
            Stmt::DoUntil {
                body, condition, ..
            } => {
                for statement in body {
                    self.statement(scope, statement);
                }
                if let Some(condition) = condition {
                    self.expr(scope, condition);
                }
            }
            Stmt::For {
                target,
                start,
                end,
                step,
                body,
                ..
            } => {
                self.expr(scope, target);
                self.expr(scope, start);
                self.expr(scope, end);
                if let Some(step) = step {
                    self.expr(scope, step);
                }
                for statement in body {
                    self.statement(scope, statement);
                }
            }
            Stmt::InlineAsm { program, .. } => {
                *program = materialize_inline_asm_constants(program, scope, self.model);
            }
            Stmt::Define(_)
            | Stmt::Exit { .. }
            | Stmt::MachineBlock { .. }
            | Stmt::Unsupported { .. } => {}
        }
    }

    fn expr(&self, scope: ScopeId, expr: &mut Expr) {
        if let ExprKind::Name(name) = &expr.kind
            && let Some(value) = self.constant(scope, name)
        {
            let number = value.number_literal();
            expr.text = number.text.clone();
            expr.kind = ExprKind::Number(number);
            return;
        }

        match &mut expr.kind {
            ExprKind::Unary { expr, .. } | ExprKind::Cast { expr, .. } => self.expr(scope, expr),
            ExprKind::Binary { left, right, .. } => {
                self.expr(scope, left);
                self.expr(scope, right);
            }
            ExprKind::Call { callee, args } => {
                self.expr(scope, callee);
                for argument in args {
                    self.expr(scope, argument);
                }
            }
            ExprKind::Index { base, index } => {
                self.expr(scope, base);
                self.expr(scope, index);
            }
            ExprKind::Field { base, .. } => self.expr(scope, base),
            ExprKind::Missing
            | ExprKind::Raw
            | ExprKind::InitializerList(_)
            | ExprKind::CurrentLocation
            | ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::Char(_)
            | ExprKind::Name(_) => {}
        }
    }

    fn constant(&self, scope: ScopeId, name: &str) -> Option<ConstValue> {
        let symbol = self.model.symbols.lookup(scope, name)?;
        self.model.constants.get(&symbol).copied()
    }
}

pub(super) fn materialize_inline_asm_constants(
    program: &InlineAsmProgram,
    scope: ScopeId,
    model: &SemanticModel,
) -> InlineAsmProgram {
    let mut program = program.clone();

    for item in &mut program.items {
        match item {
            MachineItem::Name(name) => {
                if let Some(value) = constant(model, scope, name) {
                    *item = MachineItem::Number(value.number_literal());
                }
            }
            MachineItem::AddressExpr(address) => {
                let MachineAddressAtom::Name(name) = &address.atom else {
                    continue;
                };
                if let Some(value) = constant(model, scope, name) {
                    address.atom = MachineAddressAtom::Number(value.number_literal());
                }
            }
            MachineItem::AddressByte { selector, name } => {
                if let Some(value) = constant(model, scope, name) {
                    let bits = match selector {
                        AddressByteSelector::Low => value.bits & 0x00ff,
                        AddressByteSelector::High => value.bits >> 8,
                    };
                    *item = MachineItem::Number(
                        ConstValue {
                            ty: ScalarType::Byte,
                            bits,
                        }
                        .number_literal(),
                    );
                }
            }
            MachineItem::Number(_)
            | MachineItem::StringLiteral(_)
            | MachineItem::CharLiteral(_)
            | MachineItem::Raw(_) => {}
        }
    }

    for relocation in &mut program.relocations {
        let InlineAsmRelocationTarget::Symbol(name) = &relocation.target else {
            continue;
        };
        if let Some(value) = constant(model, scope, name) {
            relocation.target = InlineAsmRelocationTarget::Absolute(value.bits);
        }
    }

    program
}

fn constant(model: &SemanticModel, scope: ScopeId, name: &str) -> Option<ConstValue> {
    let name = QualifiedName::new(name.split('.').map(str::to_string).collect());
    let SemanticNameResolution::Symbol(symbol) = model.resolve_name(scope, &name) else {
        return None;
    };
    model.constants.get(&symbol).copied()
}
