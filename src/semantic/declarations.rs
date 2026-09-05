//! Named-module constants and record layouts share an explicit dependency
//! lifecycle. Dependencies are requested by semantic consumers, not recovered
//! by a second AST expression evaluator or by speculative diagnostic retries.

use super::*;

#[derive(Default)]
pub(super) struct NamedLayoutDeclarations {
    nodes: HashMap<SymbolId, LayoutDeclaration>,
    records_by_name: HashMap<String, SymbolId>,
    active: Vec<SymbolId>,
}

struct LayoutDeclaration {
    scope: ScopeId,
    order: usize,
    state: ResolutionState,
    dependency_failed: bool,
    kind: LayoutDeclarationKind,
}

#[derive(Clone)]
enum LayoutDeclarationKind {
    Record {
        name: String,
        fields: Vec<VarDecl>,
    },
    Constant {
        declared_type: Option<ConstDeclaredType>,
        entry: ConstEntry,
    },
}

#[derive(Clone, Copy)]
enum ResolutionState {
    Pending,
    Resolving,
    Complete,
    Failed,
}

impl Analyzer {
    pub(super) fn resolve_named_layout_declarations(&mut self, scope: ScopeId, program: &Program) {
        debug_assert!(self.named_layout_declarations.nodes.is_empty());
        let mut records = Vec::new();
        let mut constants = Vec::new();
        for region in &program.modules {
            for item in &region.items {
                match item {
                    Item::Declaration(Decl::Type(decl)) => {
                        if let Some(id) = self.register_named_layout_declaration(
                            scope,
                            &decl.name,
                            LayoutDeclarationKind::Record {
                                name: decl.name.clone(),
                                fields: decl.fields.clone(),
                            },
                        ) {
                            records.push(id);
                        }
                    }
                    Item::Declaration(Decl::Record(decl)) => {
                        if let Some(id) = self.register_named_layout_declaration(
                            scope,
                            &decl.name,
                            LayoutDeclarationKind::Record {
                                name: decl.name.clone(),
                                fields: decl.fields.clone(),
                            },
                        ) {
                            records.push(id);
                        }
                    }
                    Item::Declaration(Decl::Const(decl)) => {
                        for entry in &decl.entries {
                            if let Some(id) = self.register_named_layout_declaration(
                                scope,
                                &entry.name,
                                LayoutDeclarationKind::Constant {
                                    declared_type: decl.declared_type,
                                    entry: entry.clone(),
                                },
                            ) {
                                constants.push(id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Preserve the old root traversal and field-ID order where no new
        // dependency is needed. Each node is evaluated at most once.
        for id in records.into_iter().chain(constants) {
            self.resolve_named_layout_declaration(id, self.symbols.symbols[id.0].span);
        }
        self.named_layout_declarations = NamedLayoutDeclarations::default();
    }

    fn register_named_layout_declaration(
        &mut self,
        scope: ScopeId,
        name: &str,
        kind: LayoutDeclarationKind,
    ) -> Option<SymbolId> {
        let id = self.symbols.lookup_exact(scope, name)?;
        let pending = &mut self.named_layout_declarations;
        // Duplicate declarations were diagnosed when identities were collected.
        // Do not replace the defining declaration or its resolution state.
        if pending.nodes.contains_key(&id) {
            return None;
        }
        if matches!(kind, LayoutDeclarationKind::Record { .. }) {
            pending.records_by_name.insert(
                normalize_name(&self.symbols.symbols[id.0].qualified_name),
                id,
            );
        }
        pending.nodes.insert(
            id,
            LayoutDeclaration {
                scope,
                order: pending.nodes.len(),
                state: ResolutionState::Pending,
                dependency_failed: false,
                kind,
            },
        );
        Some(id)
    }

    pub(super) fn ensure_named_constant(&mut self, id: SymbolId, span: Span) -> bool {
        let pending = &self.named_layout_declarations;
        let Some(node) = pending.nodes.get(&id) else {
            return true;
        };
        if let Some(active) = pending.active.last()
            && *active != id
            && node.order >= pending.nodes[active].order
        {
            // Check source visibility even when another layout has already
            // evaluated this constant. Scheduling must not enable forward CONSTs.
            self.diagnostics.push(Diagnostic::new(
                span,
                format!(
                    "constant `{}` is not available before its declaration",
                    self.symbols.symbols[id.0].name
                ),
            ));
            return false;
        }
        self.resolve_named_layout_declaration(id, span)
    }

    /// Callers requesting a pointer's own width must skip this method; callers
    /// selecting a field need the pointee record layout as well.
    pub(super) fn ensure_named_record_layout(&mut self, ty: &ValueType, span: Span) -> bool {
        let Some(name) = ty.as_record_base_name() else {
            return true;
        };
        let Some(id) = self
            .named_layout_declarations
            .records_by_name
            .get(&normalize_name(name))
            .copied()
        else {
            return true;
        };
        self.resolve_named_layout_declaration(id, span)
    }

    fn resolve_named_layout_declaration(&mut self, id: SymbolId, span: Span) -> bool {
        let success = self.resolve_named_layout_declaration_node(id, span);
        if !success && let Some(active) = self.named_layout_declarations.active.last().copied() {
            self.named_layout_declarations
                .nodes
                .get_mut(&active)
                .unwrap()
                .dependency_failed = true;
        }
        success
    }

    fn resolve_named_layout_declaration_node(&mut self, id: SymbolId, span: Span) -> bool {
        let Some(node) = self.named_layout_declarations.nodes.get(&id) else {
            return true;
        };
        match node.state {
            ResolutionState::Complete => return true,
            ResolutionState::Failed => return false,
            ResolutionState::Resolving => {
                let active = &self.named_layout_declarations.active;
                let start = active.iter().position(|active| *active == id).unwrap_or(0);
                let chain = active[start..]
                    .iter()
                    .chain(std::iter::once(&id))
                    .map(|id| self.symbols.symbols[id.0].qualified_name.as_str())
                    .collect::<Vec<_>>()
                    .join(" -> ");
                self.diagnostics.push(Diagnostic::new(
                    span,
                    format!("cyclic constant/record layout dependency: {chain}"),
                ));
                return false;
            }
            ResolutionState::Pending => {}
        }
        let scope = node.scope;
        let kind = node.kind.clone();
        self.named_layout_declarations
            .nodes
            .get_mut(&id)
            .unwrap()
            .state = ResolutionState::Resolving;
        self.named_layout_declarations.active.push(id);
        let diagnostic_count = self.diagnostics.len();
        match kind {
            LayoutDeclarationKind::Record { name, fields } => {
                self.validate_predeclared_record_type(scope, &name, &fields);
            }
            LayoutDeclarationKind::Constant {
                declared_type,
                entry,
            } => {
                self.evaluate_declared_const(scope, id, declared_type, &entry);
            }
        }
        let success = self.diagnostics.len() == diagnostic_count
            && !self.named_layout_declarations.nodes[&id].dependency_failed;
        self.named_layout_declarations.active.pop();
        self.named_layout_declarations
            .nodes
            .get_mut(&id)
            .unwrap()
            .state = if success {
            ResolutionState::Complete
        } else {
            ResolutionState::Failed
        };
        success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use crate::source::{InMemorySourceProvider, SourceOrigin};

    fn analyze_named(
        source: &str,
        options: SemanticOptions,
    ) -> Result<SemanticModel, Vec<Diagnostic>> {
        let root = SourceOrigin::host("project/main.act");
        let provider =
            InMemorySourceProvider::default().with_source(root.clone(), source.as_bytes().to_vec());
        let compilation =
            load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
        analyze_compilation_with_options(&compilation, options)
    }

    fn array_options(target: TargetId) -> SemanticOptions {
        SemanticOptions {
            embedded_record_arrays: true,
            ..SemanticOptions::modern().with_target(target)
        }
    }

    #[test]
    fn named_record_arrays_resolve_same_module_constant_chains() {
        let model = analyze_named(
            "MODULE Data CONST BYTE Unit=50 CONST Count=Unit*2 \
             PUBLIC TYPE Buffer=[INT ARRAY x(Count),y(Count)] ENDMODULE",
            array_options(TargetId::Atari6502),
        )
        .unwrap();
        let record = model.layout.record_for_name("Data.Buffer").unwrap();
        assert_eq!(record.size, 400);
        assert_eq!(record.fields[1].offset, 200);
        assert_eq!(model.fields.len(), 2);
    }

    #[test]
    fn named_layout_dependencies_compose_bounds_and_later_record_queries() {
        let source = "MODULE Data \
            CONST Count=SIZEOF(Point)+OFFSETOF(Point,word)+ALIGNOF(Point) \
            TYPE Buffer=[BYTE ARRAY values(Count)] \
            TYPE Point=[BYTE tag CARD word] ENDMODULE";
        for (target, size) in [(TargetId::Atari6502, 5), (TargetId::Motorola68000, 8)] {
            let model = analyze_named(source, array_options(target)).unwrap();
            let record = model.layout.record_for_name("Data.Buffer").unwrap();
            assert_eq!(record.size, size, "{target:?}");
            assert_eq!(
                model.fields.len(),
                3,
                "dependencies must not duplicate fields"
            );
        }
    }

    #[test]
    fn named_layout_dependencies_keep_forward_constants_rejected_even_if_cached() {
        for source in [
            "MODULE Data CONST Bad=Later CONST Later=4 TYPE Buffer=[BYTE ARRAY values(Later)] ENDMODULE",
            "MODULE Data TYPE Buffer=[BYTE ARRAY values(Count)] CONST Count=4 ENDMODULE",
            "MODULE Data CONST First=Second, Second=4 ENDMODULE",
        ] {
            let errors = analyze_named(source, array_options(TargetId::Atari6502)).unwrap_err();
            assert!(
                errors.iter().any(|error| error
                    .message
                    .contains("not available before its declaration")),
                "{source}: {errors:?}"
            );
        }
    }

    #[test]
    fn named_layout_dependencies_diagnose_cycles_without_retrying() {
        for source in [
            "MODULE Data CONST Count=SIZEOF(Buffer) TYPE Buffer=[BYTE ARRAY values(Count)] ENDMODULE",
            "MODULE Data TYPE First=[Second next] TYPE Second=[First next] ENDMODULE",
            "MODULE Data CONST Count=Count+1 TYPE Buffer=[BYTE ARRAY values(Count)] ENDMODULE",
        ] {
            let errors = analyze_named(source, array_options(TargetId::Atari6502)).unwrap_err();
            let cycles = errors
                .iter()
                .filter(|error| {
                    error
                        .message
                        .contains("cyclic constant/record layout dependency")
                })
                .count();
            assert_eq!(cycles, 1, "{source}: {errors:?}");
            assert_eq!(
                errors,
                analyze_named(source, array_options(TargetId::Atari6502)).unwrap_err()
            );
        }
    }

    #[test]
    fn named_layout_dependencies_do_not_follow_pointer_widths_into_pointees() {
        let source = "MODULE Data CONST Count=SIZEOF(Node POINTER) \
            TYPE Node=[Node POINTER next BYTE ARRAY data(Count)] ENDMODULE";
        for (target, size) in [
            (TargetId::Atari6502, 4),
            (TargetId::Wdc65816Native, 6),
            (TargetId::Motorola68000, 8),
        ] {
            let model = analyze_named(source, array_options(target)).unwrap();
            assert_eq!(
                model.layout.record_for_name("Data.Node").unwrap().size,
                size
            );
        }
    }

    #[test]
    fn named_layout_dependencies_preserve_public_query_constants() {
        let source = "MODULE Data CONST Width=SIZEOF(Pair), Offset=OFFSETOF(Pair,word) \
            TYPE Pair=[BYTE tag CARD word] ENDMODULE";
        for options in [SemanticOptions::default(), SemanticOptions::modern()] {
            let model = analyze_named(source, options).unwrap();
            let scope = model
                .modules
                .iter()
                .find(|module| module.path.display_name() == "Data")
                .unwrap()
                .scope;
            assert_eq!(
                model.constants[&model.symbols.lookup(scope, "Width").unwrap()].bits,
                3
            );
            assert_eq!(
                model.constants[&model.symbols.lookup(scope, "Offset").unwrap()].bits,
                1
            );
        }
    }

    #[test]
    fn named_layout_dependencies_do_not_re_evaluate_failed_constants() {
        let source = "MODULE Data CONST Count=1/0 \
            TYPE First=[BYTE ARRAY values(Count)] TYPE Second=[BYTE ARRAY values(Count)] ENDMODULE";
        let errors = analyze_named(source, array_options(TargetId::Atari6502)).unwrap_err();
        assert_eq!(
            errors
                .iter()
                .filter(|error| error.message.contains("division by zero"))
                .count(),
            1,
            "{errors:?}"
        );
    }
}
