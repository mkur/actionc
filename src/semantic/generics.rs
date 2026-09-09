//! Finite nominal generic instances. Source names resolve to definition IDs;
//! the instance cache compares those IDs and canonical concrete ValueTypes.
use super::*;

pub const MAX_GENERIC_DEPTH: usize = 64;
pub const MAX_GENERIC_INSTANCES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GenericDefinition {
    pub(super) scope: ScopeId,
    declaration: TypeDecl,
    validated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericTypeInstance {
    pub definition: SymbolId,
    pub arguments: Vec<ValueType>,
    pub owner: SymbolId,
    pub scope: ScopeId,
    weight: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GenericTypeFacts {
    pub instances: Vec<GenericTypeInstance>,
    pub(super) definitions: HashMap<SymbolId, GenericDefinition>,
    pub(super) bindings: HashMap<SymbolId, ValueType>,
    active: Vec<usize>,
}

impl GenericTypeFacts {
    pub(super) fn instance(
        &self,
        definition: SymbolId,
        arguments: &[ValueType],
    ) -> Option<SymbolId> {
        // Bounded, insertion-ordered interning; no printable/mangled name keys.
        self.instances
            .iter()
            .find(|i| i.definition == definition && i.arguments == arguments)
            .map(|i| i.owner)
    }
    fn weight(&self, ty: &ValueType) -> usize {
        let base = ty
            .as_aggregate_identity()
            .and_then(|id| id.symbol)
            .and_then(|id| self.instances.iter().find(|i| i.owner == id))
            .map_or(1, |i| i.weight);
        base.saturating_add(usize::from(ty.pointer))
    }
}

fn argument_name(ty: &ValueType) -> String {
    let name = match &ty.base {
        ValueTypeBase::Fund(fund) => format!("{fund:?}").to_ascii_uppercase(),
        ValueTypeBase::Enum(identity) => identity.name.clone(),
        ValueTypeBase::Named(identity) => identity.name.clone(),
        ValueTypeBase::Real => "REAL".into(),
        ValueTypeBase::Callable(callable) => {
            let head = callable
                .return_type
                .as_ref()
                .map_or_else(|| "PROC".into(), |ty| format!("{} FUNC", argument_name(ty)));
            format!(
                "{head} POINTER({})",
                callable
                    .params
                    .iter()
                    .map(argument_name)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        ValueTypeBase::Error => "<error>".into(),
    };
    if ty.pointer {
        format!("{name} POINTER")
    } else {
        name
    }
}

impl Analyzer {
    pub(super) fn prepare_type_applications(&mut self, scope: ScopeId, ty: &TypeRef, span: Span) {
        match &ty.base {
            TypeBase::Applied { .. } => self.validate_type_ref(scope, ty, span),
            TypeBase::Callable(callable) => {
                for param in &callable.params {
                    self.prepare_type_applications(scope, &param.ty, span);
                }
                if let RoutineKind::Func { return_type } = &callable.kind {
                    self.prepare_type_applications(scope, return_type, span);
                }
            }
            _ => {}
        }
    }

    pub(super) fn register_generic_definition(&mut self, scope: ScopeId, declaration: &TypeDecl) {
        if !self.options.algebraic_types.generic_types {
            self.diagnostics.push(Diagnostic::new(
                declaration.span,
                "generic TYPE definitions require the modern generic-types capability",
            ));
            return;
        }
        let Some(id) = self.symbols.lookup_exact(scope, &declaration.name) else {
            return;
        };
        if self.generics.definitions.contains_key(&id) {
            return;
        }
        let mut names = HashSet::new();
        for name in &declaration.parameters {
            if !names.insert(normalize_name(name)) {
                self.diagnostics.push(Diagnostic::new(
                    declaration.span,
                    "duplicate generic TYPE parameter",
                ));
            }
        }
        if matches!(declaration.definition, TypeDefinition::Enum(_)) {
            self.diagnostics.push(Diagnostic::new(
                declaration.span,
                "generic TYPE definitions must be records or variants",
            ));
        }
        self.generics.definitions.insert(
            id,
            GenericDefinition {
                scope,
                declaration: declaration.clone(),
                validated: false,
            },
        );
    }

    pub(super) fn validate_generic_template(&mut self, id: SymbolId) {
        let Some(template) = self.generics.definitions.get_mut(&id) else {
            return;
        };
        if template.validated {
            return;
        }
        template.validated = true;
        let template = template.clone();
        let parameters: HashSet<_> = template
            .declaration
            .parameters
            .iter()
            .map(|n| normalize_name(n))
            .collect();
        let fields: Vec<_> = match &template.declaration.definition {
            TypeDefinition::Record(fields) => fields.iter().collect(),
            TypeDefinition::Variant(alternatives) => {
                alternatives.iter().flat_map(|a| &a.fields).collect()
            }
            TypeDefinition::Enum(_) => Vec::new(),
        };
        for field in fields {
            self.validate_template_type(template.scope, &parameters, &field.ty, field.span);
        }
    }

    fn validate_template_type(
        &mut self,
        scope: ScopeId,
        parameters: &HashSet<String>,
        ty: &TypeRef,
        span: Span,
    ) {
        match &ty.base {
            TypeBase::Named(name)
                if name
                    .simple_name()
                    .is_some_and(|n| parameters.contains(&normalize_name(n))) => {}
            TypeBase::Named(_) => self.validate_type_ref(scope, ty, span),
            TypeBase::Applied {
                definition,
                arguments,
            } => {
                let Some(id) = self.resolve_qualified_symbol(scope, definition, span) else {
                    return;
                };
                match self.generics.definitions.get(&id) {
                    Some(template) if template.declaration.parameters.len() == arguments.len() => {}
                    Some(_) => self.diagnostics.push(Diagnostic::new(
                        span,
                        "generic template application has the wrong number of type arguments",
                    )),
                    None => self.diagnostics.push(Diagnostic::new(
                        span,
                        "template application requires a generic TYPE definition",
                    )),
                }
                for argument in arguments {
                    self.validate_template_type(scope, parameters, argument, span);
                }
            }
            TypeBase::Callable(callable) => {
                for param in &callable.params {
                    self.validate_template_type(scope, parameters, &param.ty, span);
                }
                if let RoutineKind::Func { return_type } = &callable.kind {
                    self.validate_template_type(scope, parameters, return_type, span);
                }
            }
            _ => {}
        }
    }

    pub(super) fn generic_application_type(
        &self,
        scope: ScopeId,
        definition: &QualifiedName,
        arguments: &[TypeRef],
    ) -> ValueType {
        let SemanticNameResolution::Symbol(definition) =
            resolve_semantic_name(&self.symbols, &self.modules, scope, definition)
        else {
            return ValueType::error();
        };
        let arguments: Vec<_> = arguments
            .iter()
            .map(|ty| self.value_type_from_type_ref(scope, ty))
            .collect();
        self.generics
            .instance(definition, &arguments)
            .map(|id| ValueType::aggregate(self.symbols.symbols[id.0].aggregate_identity(id)))
            .unwrap_or_else(ValueType::error)
    }

    pub(super) fn instantiate_generic(
        &mut self,
        scope: ScopeId,
        definition: &QualifiedName,
        arguments: &[TypeRef],
        span: Span,
    ) -> ValueType {
        if !self.options.algebraic_types.generic_types {
            self.diagnostics.push(Diagnostic::new(
                span,
                "generic type applications require the modern generic-types capability",
            ));
            return ValueType::error();
        }
        let Some(definition_id) = self.resolve_qualified_symbol(scope, definition, span) else {
            return ValueType::error();
        };
        let Some(template) = self.generics.definitions.get(&definition_id).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!("`{definition}` is not a generic TYPE"),
            ));
            return ValueType::error();
        };
        self.validate_generic_template(definition_id);
        if arguments.len() != template.declaration.parameters.len() {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!(
                    "generic TYPE `{definition}` expects {} type arguments, got {}",
                    template.declaration.parameters.len(),
                    arguments.len()
                ),
            ));
            return ValueType::error();
        }
        let mut types = Vec::new();
        for argument in arguments {
            self.validate_type_ref(scope, argument, span);
            let ty = self.value_type_from_type_ref(scope, argument);
            if ty.is_error() {
                return ty;
            }
            if !ty.pointer && !self.ensure_named_record_layout(&ty, span) {
                return ValueType::error();
            }
            if self.value_storage_width(&ty).is_none()
                || matches!(&argument.base, TypeBase::Named(name) if is_string_type_name(name))
            {
                self.diagnostics.push(Diagnostic::new(span, "generic type arguments must be complete value types, not ARRAY/STRING storage or routines"));
                return ValueType::error();
            }
            types.push(ty);
        }
        if let Some(id) = self.generics.instance(definition_id, &types) {
            return ValueType::aggregate(self.symbols.symbols[id.0].aggregate_identity(id));
        }
        let weight = types
            .iter()
            .fold(1usize, |w, ty| w.saturating_add(self.generics.weight(ty)));
        if self.generics.active.iter().any(|i| {
            let previous = &self.generics.instances[*i];
            previous.definition == definition_id && weight > previous.weight
        }) {
            self.diagnostics.push(Diagnostic::new(
                span,
                "expanding recursive generic specialization is not supported",
            ));
            return ValueType::error();
        }
        if self.generics.active.len() >= MAX_GENERIC_DEPTH
            || self.generics.instances.len() >= MAX_GENERIC_INSTANCES
        {
            self.diagnostics.push(Diagnostic::new(
                span,
                "generic instantiation budget exceeded (depth 64, instances 1024)",
            ));
            return ValueType::error();
        }
        let instance_scope = self
            .symbols
            .add_scope(ScopeKind::LexicalBlock, Some(template.scope));
        let index = self.generics.instances.len();
        let name = format!("__actionc_type_instance_{index}");
        let source = self.symbols.symbols[definition_id.0].clone();
        let display_name = format!(
            "{}<{}>",
            source.qualified_name,
            types
                .iter()
                .map(argument_name)
                .collect::<Vec<_>>()
                .join(",")
        );
        let owner = self
            .symbols
            .declare_with_identity(
                instance_scope,
                name.clone(),
                SymbolClass::Type,
                None,
                span,
                source.defining_module,
                Visibility::Private,
                format!("{}::instance::{index}", source.canonical_qualified_key),
                display_name,
            )
            .expect("fresh generic instance scope");
        let value = ValueType::aggregate(self.symbols.symbols[owner.0].aggregate_identity(owner));
        self.symbols.symbols[owner.0].ty = Some(value.clone());
        for (parameter, ty) in template.declaration.parameters.iter().zip(&types) {
            match self.symbols.declare(
                instance_scope,
                parameter.clone(),
                SymbolClass::Type,
                Some(ty.clone()),
                span,
            ) {
                Ok(id) => {
                    self.generics.bindings.insert(id, ty.clone());
                }
                Err(_) => {
                    self.diagnostics.push(Diagnostic::new(
                        span,
                        "generic parameter conflicts with its instance identity",
                    ));
                }
            }
        }
        // Publish the placeholder before resolving any field, including pointer
        // pointees. Regular recursive and mutually recursive instances share it.
        self.generics.instances.push(GenericTypeInstance {
            definition: definition_id,
            arguments: types,
            owner,
            scope: instance_scope,
            weight,
        });
        self.generics.active.push(index);
        self.register_generic_layout(
            instance_scope,
            &name,
            &template.declaration.definition,
            span,
        );
        self.ensure_named_record_layout(&value, span);
        self.generics.active.pop();
        self.finish_generic_layout(owner);
        value
    }
}
