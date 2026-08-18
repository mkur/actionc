use std::collections::BTreeMap;

use crate::ast::{Expr, ExprKind, Item};
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

pub(crate) fn parse_bindings(
    runtime: Runtime,
) -> Result<BTreeMap<String, BindingTarget>, Vec<Diagnostic>> {
    let file_name = match runtime {
        Runtime::ActionCart => "std-cart.act",
        Runtime::Standalone => "std-standalone.act",
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
            if !interface.to_ascii_uppercase().starts_with("STD.") {
                return Err(diagnostic(format!(
                    "binding target `{interface}` is outside STD"
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
    Ok(bindings)
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
