use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Expr, ExprKind, Item, Program, Visibility};
use crate::diagnostic::Diagnostic;
use crate::embedded_vfs::EmbeddedSourceProvider;
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::runtime::Runtime;
use crate::source::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindingTarget {
    Absolute(u16),
    RuntimeRoutine { unit: String, routine: String },
}

pub(crate) fn parse_sys_interface() -> Result<Program, Vec<Diagnostic>> {
    let file_name = "sys.act";
    let source = EmbeddedSourceProvider
        .module_source(file_name)
        .ok_or_else(|| diagnostic(format!("embedded SYS interface `{file_name}` is missing")))?;
    let text = crate::source::decode_source(source.bytes);
    let tokens = tokenize(&text)?;
    parse(&tokens)
}

pub(crate) fn sys_interface_binding_keys() -> Result<BTreeSet<String>, Vec<Diagnostic>> {
    let program = parse_sys_interface()?;
    let mut keys = BTreeSet::new();
    for routine in program
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            Item::Routine(routine)
                if routine.is_external && routine.visibility == Visibility::Public =>
            {
                Some(routine)
            }
            _ => None,
        })
    {
        let name = format!("SYS.{}", routine.name);
        let key = binding_key(&name);
        if !keys.insert(key) {
            return Err(diagnostic(format!(
                "duplicate public external `{name}` in the SYS interface"
            )));
        }
    }
    Ok(keys)
}

pub(crate) fn parse_bindings(
    runtime: Runtime,
) -> Result<BTreeMap<String, BindingTarget>, Vec<Diagnostic>> {
    let file_name = match runtime {
        Runtime::ActionCart => "sys-cart.act",
        Runtime::Standalone => "sys-standalone.act",
    };
    let source = EmbeddedSourceProvider
        .binding_source(file_name)
        .ok_or_else(|| diagnostic(format!("embedded binding source `{file_name}` is missing")))?;
    let text = crate::source::decode_source(source.bytes);
    let tokens = tokenize(&text)?;
    let program = parse(&tokens)?;
    let mut bindings = BTreeMap::new();
    for module in &program.modules {
        for item in &module.items {
            let Item::Set(set) = item else {
                continue;
            };
            let Some(interface) = qualified_expr_name(&set.address) else {
                return Err(diagnostic(format!(
                    "binding target `{}` is not a qualified external name",
                    set.address.text
                )));
            };
            if !interface.to_ascii_uppercase().starts_with("SYS.") {
                return Err(diagnostic(format!(
                    "binding target `{interface}` is outside SYS"
                )));
            }
            let target = if let ExprKind::Number(number) = &set.value.kind {
                BindingTarget::Absolute(number.value.ok_or_else(|| {
                    diagnostic(format!("binding value `{}` is not numeric", set.value.text))
                })?)
            } else {
                let Some(implementation) = qualified_expr_name(&set.value) else {
                    return Err(diagnostic(format!(
                        "binding value `{}` is neither an address nor a runtime routine",
                        set.value.text
                    )));
                };
                let Some((unit, routine)) = implementation
                    .split_once('.')
                    .or_else(|| implementation.split_once('_'))
                else {
                    return Err(diagnostic(format!(
                        "standalone binding `{implementation}` must name UNIT.Routine"
                    )));
                };
                BindingTarget::RuntimeRoutine {
                    unit: unit.to_string(),
                    routine: routine.to_string(),
                }
            };
            let key = binding_key(&interface);
            if bindings.insert(key, target).is_some() {
                return Err(diagnostic(format!(
                    "duplicate runtime binding for `{interface}`"
                )));
            }
        }
    }
    validate_bindings(runtime, &bindings)?;
    Ok(bindings)
}

fn validate_bindings(
    runtime: Runtime,
    bindings: &BTreeMap<String, BindingTarget>,
) -> Result<(), Vec<Diagnostic>> {
    let interface = sys_interface_binding_keys()?;
    let binding_keys = bindings.keys().cloned().collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();

    for name in interface.difference(&binding_keys) {
        diagnostics.push(Diagnostic::new(
            Span::new(0, 0),
            format!("{runtime} runtime has no binding for public external `{name}`"),
        ));
    }
    for name in binding_keys.difference(&interface) {
        diagnostics.push(Diagnostic::new(
            Span::new(0, 0),
            format!("{runtime} runtime binds unknown SYS external `{name}`"),
        ));
    }
    for (name, target) in bindings {
        let valid = matches!(
            (runtime, target),
            (Runtime::ActionCart, BindingTarget::Absolute(_))
                | (Runtime::Standalone, BindingTarget::RuntimeRoutine { .. })
        );
        if !valid {
            diagnostics.push(Diagnostic::new(
                Span::new(0, 0),
                format!("{runtime} runtime has the wrong binding target kind for `{name}`"),
            ));
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

pub(crate) fn binding_key(name: &str) -> String {
    name.replace("::", ".").to_ascii_uppercase()
}

fn qualified_expr_name(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Name(name) => Some(name.clone()),
        ExprKind::Field { base, field } => {
            Some(format!("{}.{}", qualified_expr_name(base)?, field))
        }
        _ => None,
    }
}

fn diagnostic(message: impl Into<String>) -> Vec<Diagnostic> {
    vec![Diagnostic::new(Span::new(0, 0), message)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_runtime_binding_inventories_exactly_match_sys() {
        let interface = sys_interface_binding_keys().expect("SYS interface");
        let cart = parse_bindings(Runtime::ActionCart).expect("cart bindings");
        let standalone = parse_bindings(Runtime::Standalone).expect("standalone bindings");

        assert_eq!(cart.keys().cloned().collect::<BTreeSet<_>>(), interface);
        assert_eq!(
            standalone.keys().cloned().collect::<BTreeSet<_>>(),
            interface
        );
        assert!(
            cart.values()
                .all(|target| matches!(target, BindingTarget::Absolute(_)))
        );
        assert!(
            standalone
                .values()
                .all(|target| matches!(target, BindingTarget::RuntimeRoutine { .. }))
        );
    }

    #[test]
    fn binding_inventory_validation_rejects_missing_extra_and_wrong_kind_entries() {
        let interface = sys_interface_binding_keys().expect("SYS interface");
        let first = interface.iter().next().expect("non-empty SYS interface");
        let mut bindings = interface
            .iter()
            .map(|name| (name.clone(), BindingTarget::Absolute(0)))
            .collect::<BTreeMap<_, _>>();
        bindings.remove(first);
        bindings.insert(
            "SYS.NOTPUBLIC".to_string(),
            BindingTarget::RuntimeRoutine {
                unit: "SYSBLK".to_string(),
                routine: "Zero".to_string(),
            },
        );

        let diagnostics = validate_bindings(Runtime::ActionCart, &bindings)
            .expect_err("invalid binding inventory");
        let messages = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("has no binding"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("unknown SYS"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("wrong binding target kind"))
        );
    }
}
