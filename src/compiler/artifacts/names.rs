use super::*;

#[derive(Default)]
pub(super) struct ListingNames {
    declarations: crate::codegen::CodegenDeclarationNames,
    by_component: BTreeMap<String, String>,
}

impl ListingNames {
    pub(super) fn from_output(output: &CodegenOutput) -> Self {
        let mut names = Self {
            declarations: output.map.declaration_names.clone(),
            ..Self::default()
        };
        // MIR and compiler-owned helpers retain qualified declaration names.
        for routine in &output.map.routine_addresses {
            if let Some(component) = runtime_symbol_component(&routine.name)
                && let Some((_, spelling)) = routine.name.rsplit_once("::")
            {
                names.by_component.insert(component, spelling.to_string());
            }
        }
        // Classic projections use uppercase mangled identities. The runtime
        // linker retains original spellings for the whole selected closure.
        for (identity, spelling) in &output.map.runtime_routine_names {
            if let Some(component) = runtime_symbol_component(identity) {
                names.by_component.insert(component, spelling.clone());
            }
        }
        names
    }

    pub(super) fn component(&self, name: &str) -> Option<String> {
        let component = runtime_symbol_component(name)?;
        let (module, normalized) = component.split_once('_')?;
        let spelling = self
            .by_component
            .get(&component)
            .map(String::as_str)
            .or_else(|| name.rsplit_once("::").map(|(_, spelling)| spelling))
            .unwrap_or(normalized);
        Some(format!("{module}_{spelling}"))
    }

    pub(super) fn display(&self, name: &str) -> String {
        if let Some(owner) = name.strip_suffix(" storage") {
            return format!("{} storage", self.display(owner));
        }
        let key = name.to_ascii_uppercase();
        if let Some(display) = self
            .declarations
            .symbols
            .get(&key)
            .or_else(|| self.declarations.storage.get(&(None, key)))
        {
            return display.clone();
        }
        let Some(component) = self.component(name) else {
            return name.to_string();
        };
        let (module, routine) = component.split_once('_').expect("runtime module prefix");
        format!("ACTION.RUNTIME.{}::{routine}", module.to_ascii_uppercase())
    }

    pub(super) fn parameter_name<'a>(&'a self, routine: &str, name: &'a str) -> &'a str {
        self.declarations
            .storage
            .get(&(
                Some(routine.to_ascii_uppercase()),
                name.to_ascii_uppercase(),
            ))
            .map(String::as_str)
            .unwrap_or(name)
    }

    pub(super) fn storage_name<'a>(
        &'a self,
        symbol: &'a crate::codegen::CodegenStorageSymbol,
    ) -> &'a str {
        let scope = match &symbol.scope {
            CodegenSymbolScope::Global => None,
            CodegenSymbolScope::Routine(routine) => Some(routine.to_ascii_uppercase()),
        };
        self.declarations
            .storage
            .get(&(scope, symbol.name.to_ascii_uppercase()))
            .map(String::as_str)
            .unwrap_or(&symbol.name)
    }

    pub(super) fn binding_helper(&self, binding: &crate::codegen::CodegenRuntimeBinding) -> String {
        let implementation = self.display(&binding.implementation);
        if let Some((_, spelling)) = implementation.rsplit_once("::") {
            let prefix_len = binding
                .helper
                .rfind(['.', ':'])
                .map_or(0, |index| index + 1);
            if binding.helper[prefix_len..].eq_ignore_ascii_case(spelling) {
                return format!("{}{spelling}", &binding.helper[..prefix_len]);
            }
        }
        binding.helper.clone()
    }
}

/// Attach declaration spelling after code generation, without changing executable
/// names, maps' lookup identities, or relocation facts.
pub(crate) fn record_declaration_names(
    output: &mut CodegenOutput,
    program: &crate::semantic::ir::SemProgram,
) {
    use crate::semantic::ir::{SemItem, SemSymbolRef, visit_lexical_declarations};

    let names = &mut output.map.declaration_names;
    fn storage(
        names: &mut crate::codegen::CodegenDeclarationNames,
        routine: Option<&str>,
        symbol: &SemSymbolRef,
    ) {
        let display = if let Some(lexical) = &symbol.lexical_display_name {
            lexical.clone()
        } else if routine.is_some() {
            symbol
                .qualified_name
                .rsplit(['.', ':'])
                .find(|part| !part.is_empty())
                .unwrap_or(&symbol.name)
                .to_string()
        } else {
            symbol.qualified_name.clone()
        };
        if symbol.defining_module.is_some() {
            names.symbols.insert(
                symbol.name.to_ascii_uppercase(),
                symbol.qualified_name.clone(),
            );
        }
        let scope = routine.map(str::to_ascii_uppercase);
        names.storage.insert(
            (scope.clone(), symbol.name.to_ascii_uppercase()),
            display.clone(),
        );
        if let Some(lexical) = &symbol.lexical_display_name {
            names
                .storage
                .insert((scope, lexical.to_ascii_uppercase()), display);
        }
    }
    for item in program.modules.iter().flat_map(|module| &module.items) {
        match item {
            SemItem::Declaration(declaration) => storage(names, None, &declaration.symbol),
            SemItem::Routine(routine) => {
                names.symbols.insert(
                    routine.symbol.name.to_ascii_uppercase(),
                    routine.symbol.qualified_name.clone(),
                );
                names.symbols.insert(
                    routine.symbol.qualified_name.to_ascii_uppercase(),
                    routine.symbol.qualified_name.clone(),
                );
                for param in &routine.params {
                    storage(names, Some(&routine.symbol.name), &param.symbol);
                }
                for local in &routine.locals {
                    storage(names, Some(&routine.symbol.name), &local.symbol);
                }
                visit_lexical_declarations(&routine.body, &mut |_, declaration| {
                    storage(names, Some(&routine.symbol.name), &declaration.symbol);
                });
            }
            _ => {}
        }
    }
}
