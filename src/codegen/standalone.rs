use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::runtime_bindings::{BindingTarget, binding_key, parse_bindings};

const INTERNAL_SYSBLK_MODULE: &str = "ACTION.RUNTIME.SYSBLK";
const INTERNAL_SYSLIB_MODULE: &str = "ACTION.RUNTIME.SYSLIB";

pub(crate) fn generate_semir_standalone_profile_at_origin(
    semir: &crate::semantic::ir::SemProgram,
    origin: u16,
    profile: CodegenProfile,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    let mut application = super::semir::semir_to_ast(semir)?;
    reject_absolute_helper_overrides(&application)?;
    let local_helper_overrides = local_helper_overrides(&application);

    // Give external interface declarations inert preflight addresses. They
    // must remain callable while classic code generation discovers its own
    // helper requirements, but their selected implementations are linked only
    // after that discovery pass.
    let mut preflight = application.clone();
    for module in &mut preflight.modules {
        for item in &mut module.items {
            if let Item::Routine(routine) = item
                && routine.is_external
            {
                routine.system_address = Some(number_expr(0xFFFF, routine.span));
            }
        }
    }
    let (_, classic_runtime_requirements) = super::driver::generate_with_options_and_requirements(
        &preflight,
        origin,
        true,
        profile,
        RuntimeTarget::StandaloneSlots,
    )?;

    let external_interfaces = external_interfaces(semir);
    let referenced_names = referenced_external_names(&application, &external_interfaces);
    let bindings = parse_bindings(crate::runtime::Runtime::Standalone)?;

    let mut sysblk_roots = BTreeSet::new();
    let mut external_roots = BTreeMap::new();
    for external_name in referenced_names {
        let interface = &external_interfaces[&external_name];
        let target = bindings
            .get(&binding_key(&interface.qualified_name))
            .ok_or_else(|| {
                diagnostic(format!(
                    "standalone runtime has no binding for external `{}`",
                    interface.qualified_name
                ))
            })?;
        let BindingTarget::RuntimeRoutine { unit, routine } = target else {
            return Err(diagnostic(format!(
                "standalone binding for external `{}` is an absolute address",
                interface.qualified_name
            )));
        };
        if !unit.eq_ignore_ascii_case("SYSBLK") {
            return Err(diagnostic(format!(
                "standalone binding for external `{}` references unsupported runtime unit `{unit}`",
                interface.qualified_name
            )));
        }
        sysblk_roots.insert(routine.clone());
        external_roots.insert(external_name, routine.clone());
    }

    let helper_roots = classic_runtime_requirements
        .iter()
        .map(|helper| helper.name().to_string())
        .collect::<BTreeSet<_>>();
    let selected_sysblk =
        select_runtime_names("sysblk.act", INTERNAL_SYSBLK_MODULE, &sysblk_roots)?;
    let selected_syslib =
        select_runtime_names("syslib.act", INTERNAL_SYSLIB_MODULE, &helper_roots)?;

    let sysblk = runtime_projection("sysblk.act", INTERNAL_SYSBLK_MODULE)?;
    let syslib = runtime_projection("syslib.act", INTERNAL_SYSLIB_MODULE)?;
    validate_external_signatures(&external_roots, &external_interfaces, &sysblk)?;

    let sysblk_names = sysblk.routine_names();
    let syslib_names = syslib.routine_names();
    let rename_external = external_roots
        .iter()
        .map(|(external, root)| {
            let implementation = sysblk_names
                .get(&root.to_ascii_uppercase())
                .ok_or_else(|| {
                    diagnostic(format!(
                        "embedded SYSBLK has no implementation routine `{root}`"
                    ))
                })?;
            Ok((external.clone(), implementation.clone()))
        })
        .collect::<Result<BTreeMap<_, _>, Vec<Diagnostic>>>()?;
    rewrite_program_names(&mut application, &rename_external);
    remove_external_routines(&mut application);

    let mut runtime_items = selected_routines(&sysblk.ast, &selected_sysblk, &sysblk_names);
    runtime_items.extend(selected_routines(
        &syslib.ast,
        &selected_syslib,
        &syslib_names,
    ));
    let helper_sets = selected_helper_sets(&syslib.ast, &helper_roots, &syslib_names);

    let mut modules = Vec::new();
    if !helper_sets.is_empty() {
        modules.push(Module { items: helper_sets });
    }
    if !runtime_items.is_empty() {
        modules.push(Module {
            items: runtime_items,
        });
    }
    modules.extend(application.modules);
    application.modules = modules;

    let mut output = super::driver::generate_with_options(
        &application,
        origin,
        true,
        profile,
        RuntimeTarget::StandaloneSlots,
    )?;
    append_runtime_binding_metadata(
        &mut output,
        &helper_roots,
        &syslib_names,
        &external_roots,
        &sysblk_names,
        &external_interfaces,
        &local_helper_overrides,
    );
    output.map.runtime = crate::runtime::Runtime::Standalone;
    Ok(output)
}

#[derive(Clone)]
struct ExternalInterface {
    qualified_name: String,
    signature: crate::semantic::ir::SemRoutineSignature,
}

fn external_interfaces(
    semir: &crate::semantic::ir::SemProgram,
) -> BTreeMap<String, ExternalInterface> {
    semir
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            crate::semantic::ir::SemItem::Routine(routine) if routine.is_external => Some((
                routine.symbol.name.clone(),
                ExternalInterface {
                    qualified_name: routine.symbol.qualified_name.clone(),
                    signature: routine.signature.clone(),
                },
            )),
            _ => None,
        })
        .collect()
}

struct RuntimeProjection {
    semir: crate::semantic::ir::SemProgram,
    ast: Program,
}

impl RuntimeProjection {
    fn routine_names(&self) -> BTreeMap<String, String> {
        self.semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .filter_map(|item| match item {
                crate::semantic::ir::SemItem::Routine(routine) => Some((
                    source_routine_name(&routine.symbol.qualified_name).to_ascii_uppercase(),
                    routine.symbol.name.clone(),
                )),
                _ => None,
            })
            .collect()
    }
}

fn runtime_projection(
    file_name: &str,
    module_name: &str,
) -> Result<RuntimeProjection, Vec<Diagnostic>> {
    let semir = crate::runtime_source::compile_runtime_unit(file_name, module_name)?;
    let ast = super::semir::semir_to_ast(&semir)?;
    Ok(RuntimeProjection { semir, ast })
}

fn select_runtime_names(
    file_name: &str,
    module_name: &str,
    roots: &BTreeSet<String>,
) -> Result<BTreeSet<String>, Vec<Diagnostic>> {
    crate::mir6502::standalone::selected_runtime_routine_names(file_name, module_name, roots)
        .map_err(|diagnostics| {
            diagnostics
                .into_iter()
                .map(|diagnostic| {
                    Diagnostic::new(
                        Span::new(0, 0),
                        format!("classic runtime selection: {}", diagnostic.message),
                    )
                })
                .collect()
        })
}

fn validate_external_signatures(
    roots: &BTreeMap<String, String>,
    interfaces: &BTreeMap<String, ExternalInterface>,
    runtime: &RuntimeProjection,
) -> Result<(), Vec<Diagnostic>> {
    let implementations = runtime
        .semir
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            crate::semantic::ir::SemItem::Routine(routine) => Some((
                source_routine_name(&routine.symbol.qualified_name).to_ascii_uppercase(),
                routine,
            )),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    for (external, root) in roots {
        let interface = &interfaces[external];
        let Some(implementation) = implementations.get(&root.to_ascii_uppercase()) else {
            return Err(diagnostic(format!(
                "embedded SYSBLK has no implementation routine `{root}`"
            )));
        };
        if interface.signature != implementation.signature {
            return Err(diagnostic(format!(
                "ABI mismatch between external `{}` and runtime implementation `{root}`",
                interface.qualified_name
            )));
        }
    }
    Ok(())
}

fn source_routine_name(qualified_name: &str) -> &str {
    qualified_name
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(qualified_name)
}

fn selected_routines(
    runtime: &Program,
    selected: &BTreeSet<String>,
    names: &BTreeMap<String, String>,
) -> Vec<Item> {
    let selected_names = selected
        .iter()
        .filter_map(|name| names.get(&name.to_ascii_uppercase()))
        .collect::<BTreeSet<_>>();
    runtime
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            Item::Routine(routine) if selected_names.contains(&routine.name) => {
                Some(Item::Routine(routine.clone()))
            }
            _ => None,
        })
        .collect()
}

fn selected_helper_sets(
    runtime: &Program,
    roots: &BTreeSet<String>,
    names: &BTreeMap<String, String>,
) -> Vec<Item> {
    let root_names = roots
        .iter()
        .filter_map(|name| names.get(&name.to_ascii_uppercase()))
        .collect::<BTreeSet<_>>();
    runtime
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            Item::Set(set)
                if matches!(&set.value.kind, ExprKind::Name(name) if root_names.contains(name)) =>
            {
                Some(Item::Set(set.clone()))
            }
            _ => None,
        })
        .collect()
}

fn reject_absolute_helper_overrides(program: &Program) -> Result<(), Vec<Diagnostic>> {
    for module in &program.modules {
        for item in &module.items {
            let Item::Set(set) = item else {
                continue;
            };
            let Some(address) = expr_u16(&set.address) else {
                continue;
            };
            let Some(helper) = RuntimeHelperSlot::from_slot_address(address) else {
                continue;
            };
            if let Some(target) = expr_u16(&set.value) {
                return Err(vec![Diagnostic::new(
                    set.span,
                    format!(
                        "standalone runtime rejects absolute override ${target:04X} for `{}`",
                        helper.name()
                    ),
                )]);
            }
        }
    }
    Ok(())
}

fn local_helper_overrides(program: &Program) -> BTreeMap<RuntimeHelperSlot, String> {
    let mut overrides = BTreeMap::new();
    for module in &program.modules {
        for item in &module.items {
            let Item::Set(set) = item else {
                continue;
            };
            let Some(helper) =
                expr_u16(&set.address).and_then(RuntimeHelperSlot::from_slot_address)
            else {
                continue;
            };
            if let ExprKind::Name(name) = &set.value.kind {
                overrides.insert(helper, name.clone());
            }
        }
    }
    overrides
}

fn referenced_external_names(
    program: &Program,
    interfaces: &BTreeMap<String, ExternalInterface>,
) -> BTreeSet<String> {
    let candidates = interfaces.keys().cloned().collect::<BTreeSet<_>>();
    let mut referenced = BTreeSet::new();
    for module in &program.modules {
        for item in &module.items {
            if matches!(item, Item::Routine(routine) if routine.is_external) {
                continue;
            }
            collect_item_names(item, &candidates, &mut referenced);
        }
    }
    referenced
}

fn remove_external_routines(program: &mut Program) {
    for module in &mut program.modules {
        module
            .items
            .retain(|item| !matches!(item, Item::Routine(routine) if routine.is_external));
    }
}

fn append_runtime_binding_metadata(
    output: &mut CodegenOutput,
    helpers: &BTreeSet<String>,
    syslib_names: &BTreeMap<String, String>,
    external_roots: &BTreeMap<String, String>,
    sysblk_names: &BTreeMap<String, String>,
    interfaces: &BTreeMap<String, ExternalInterface>,
    local_overrides: &BTreeMap<RuntimeHelperSlot, String>,
) {
    for helper in helpers {
        let Some(link_name) = syslib_names.get(&helper.to_ascii_uppercase()) else {
            continue;
        };
        output.map.runtime_bindings.push(CodegenRuntimeBinding {
            helper: helper.clone(),
            implementation: format!("{INTERNAL_SYSLIB_MODULE}::{helper}"),
            address: routine_address(output, link_name),
            reason: "classic code generation requires a runtime helper".to_string(),
            origin: "embedded SYSLIB.ACT (GPL-3.0)".to_string(),
            suppressed_default: None,
        });
    }
    for (external, root) in external_roots {
        let Some(link_name) = sysblk_names.get(&root.to_ascii_uppercase()) else {
            continue;
        };
        output.map.runtime_bindings.push(CodegenRuntimeBinding {
            helper: interfaces[external].qualified_name.clone(),
            implementation: format!("{INTERNAL_SYSBLK_MODULE}::{root}"),
            address: routine_address(output, link_name),
            reason: "referenced external standard-library interface".to_string(),
            origin: "embedded SYSBLK.ACT (GPL-3.0)".to_string(),
            suppressed_default: None,
        });
    }
    for (helper, implementation) in local_overrides {
        output.map.runtime_bindings.push(CodegenRuntimeBinding {
            helper: helper.name().to_string(),
            implementation: implementation.clone(),
            address: routine_address(output, implementation),
            reason: "source-level local helper override".to_string(),
            origin: "application".to_string(),
            suppressed_default: Some(format!("{INTERNAL_SYSLIB_MODULE}::{}", helper.name())),
        });
    }
}

fn routine_address(output: &CodegenOutput, name: &str) -> Option<u16> {
    output
        .routine_addresses
        .iter()
        .find(|routine| routine.name == name)
        .map(|routine| routine.address)
}

fn number_expr(value: u16, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Number(crate::lexer::NumberLiteral {
            text: format!("${value:04X}"),
            kind: crate::lexer::NumberKind::Card,
            value: Some(value),
        }),
        text: format!("${value:04X}"),
        span,
    }
}

fn expr_u16(expr: &Expr) -> Option<u16> {
    match &expr.kind {
        ExprKind::Number(number) => number.value,
        ExprKind::Cast { expr, .. } => expr_u16(expr),
        _ => None,
    }
}

fn diagnostic(message: impl Into<String>) -> Vec<Diagnostic> {
    vec![Diagnostic::new(Span::new(0, 0), message)]
}

fn rewrite_program_names(program: &mut Program, replacements: &BTreeMap<String, String>) {
    for module in &mut program.modules {
        for item in &mut module.items {
            rewrite_item_names(item, replacements);
        }
    }
}

fn rewrite_item_names(item: &mut Item, replacements: &BTreeMap<String, String>) {
    match item {
        Item::Set(set) => {
            rewrite_expr_names(&mut set.address, replacements);
            rewrite_expr_names(&mut set.value, replacements);
        }
        Item::Declaration(decl) => rewrite_decl_names(decl, replacements),
        Item::Routine(routine) => {
            if let Some(address) = &mut routine.system_address {
                rewrite_expr_names(address, replacements);
            }
            for param in &mut routine.params {
                rewrite_var_names(param, replacements);
            }
            for local in &mut routine.locals {
                rewrite_decl_names(local, replacements);
            }
            rewrite_stmt_list_names(&mut routine.body, replacements);
        }
        Item::Statement(stmt) => rewrite_stmt_names(stmt, replacements),
        Item::Define(_) | Item::Include(_) | Item::Unsupported { .. } => {}
    }
}

fn rewrite_decl_names(decl: &mut Decl, replacements: &BTreeMap<String, String>) {
    match decl {
        Decl::Var(var) => rewrite_var_names(var, replacements),
        Decl::Const(constant) => {
            for entry in &mut constant.entries {
                rewrite_expr_names(&mut entry.value, replacements);
            }
        }
        Decl::Type(decl) => {
            for field in &mut decl.fields {
                rewrite_var_names(field, replacements);
            }
        }
        Decl::Record(decl) => {
            for field in &mut decl.fields {
                rewrite_var_names(field, replacements);
            }
        }
    }
}

fn rewrite_var_names(var: &mut VarDecl, replacements: &BTreeMap<String, String>) {
    for entry in &mut var.entries {
        if let Some(size) = &mut entry.size {
            rewrite_expr_names(size, replacements);
        }
        if let Some(initializer) = &mut entry.initializer {
            rewrite_expr_names(initializer, replacements);
        }
    }
}

fn rewrite_stmt_list_names(statements: &mut [Stmt], replacements: &BTreeMap<String, String>) {
    for stmt in statements {
        rewrite_stmt_names(stmt, replacements);
    }
}

fn rewrite_stmt_names(stmt: &mut Stmt, replacements: &BTreeMap<String, String>) {
    match stmt {
        Stmt::Return(value) => {
            if let Some(value) = value {
                rewrite_expr_names(value, replacements);
            }
        }
        Stmt::Assign { target, value, .. } | Stmt::CompoundAssign { target, value, .. } => {
            rewrite_expr_names(target, replacements);
            rewrite_expr_names(value, replacements);
        }
        Stmt::Call { expr, .. } => rewrite_expr_names(expr, replacements),
        Stmt::MachineBlock { items, .. } => rewrite_machine_items(items, replacements),
        Stmt::InlineAsm { program, .. } => {
            rewrite_machine_items(&mut program.items, replacements);
            for relocation in &mut program.relocations {
                if let crate::asm6502::InlineAsmRelocationTarget::Symbol(name) =
                    &mut relocation.target
                    && let Some(replacement) = replacements.get(name)
                {
                    *name = replacement.clone();
                }
            }
        }
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                rewrite_expr_names(&mut branch.condition, replacements);
                rewrite_stmt_list_names(&mut branch.body, replacements);
            }
            rewrite_stmt_list_names(else_body, replacements);
        }
        Stmt::While {
            condition, body, ..
        } => {
            rewrite_expr_names(condition, replacements);
            rewrite_stmt_list_names(body, replacements);
        }
        Stmt::DoUntil {
            body, condition, ..
        } => {
            rewrite_stmt_list_names(body, replacements);
            if let Some(condition) = condition {
                rewrite_expr_names(condition, replacements);
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
            rewrite_expr_names(target, replacements);
            rewrite_expr_names(start, replacements);
            rewrite_expr_names(end, replacements);
            if let Some(step) = step {
                rewrite_expr_names(step, replacements);
            }
            rewrite_stmt_list_names(body, replacements);
        }
        Stmt::Define(_) | Stmt::Exit { .. } | Stmt::Unsupported { .. } => {}
    }
}

fn rewrite_expr_names(expr: &mut Expr, replacements: &BTreeMap<String, String>) {
    match &mut expr.kind {
        ExprKind::Name(name) => {
            if let Some(replacement) = replacements.get(name) {
                *name = replacement.clone();
                expr.text = replacement.clone();
            }
        }
        ExprKind::InitializerList(elements) => {
            for element in elements {
                if let InitializerElementKind::Address { target, .. } = &mut element.kind
                    && let Some(replacement) = replacements.get(target.display_name())
                {
                    *target = QualifiedName::simple(replacement.clone());
                }
            }
        }
        ExprKind::Unary { expr, .. } | ExprKind::Cast { expr, .. } => {
            rewrite_expr_names(expr, replacements)
        }
        ExprKind::Binary { left, right, .. } => {
            rewrite_expr_names(left, replacements);
            rewrite_expr_names(right, replacements);
        }
        ExprKind::Call { callee, args } => {
            rewrite_expr_names(callee, replacements);
            for arg in args {
                rewrite_expr_names(arg, replacements);
            }
        }
        ExprKind::Index { base, index } => {
            rewrite_expr_names(base, replacements);
            rewrite_expr_names(index, replacements);
        }
        ExprKind::Field { base, .. } => rewrite_expr_names(base, replacements),
        ExprKind::Missing
        | ExprKind::Raw
        | ExprKind::CurrentLocation
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Char(_) => {}
    }
}

fn rewrite_machine_items(items: &mut [MachineItem], replacements: &BTreeMap<String, String>) {
    for item in items {
        let name = match item {
            MachineItem::Name(name) | MachineItem::AddressByte { name, .. } => Some(name),
            MachineItem::AddressExpr(MachineAddressExpr {
                atom: MachineAddressAtom::Name(name),
                ..
            }) => Some(name),
            _ => None,
        };
        if let Some(name) = name
            && let Some(replacement) = replacements.get(name.display_name())
        {
            *name = QualifiedName::simple(replacement.clone());
        }
    }
}

fn collect_item_names(item: &Item, candidates: &BTreeSet<String>, output: &mut BTreeSet<String>) {
    match item {
        Item::Set(set) => {
            collect_expr_names(&set.address, candidates, output);
            collect_expr_names(&set.value, candidates, output);
        }
        Item::Declaration(decl) => collect_decl_names(decl, candidates, output),
        Item::Routine(routine) => {
            if let Some(address) = &routine.system_address {
                collect_expr_names(address, candidates, output);
            }
            for param in &routine.params {
                collect_var_names(param, candidates, output);
            }
            for local in &routine.locals {
                collect_decl_names(local, candidates, output);
            }
            collect_stmt_list_names(&routine.body, candidates, output);
        }
        Item::Statement(stmt) => collect_stmt_names(stmt, candidates, output),
        Item::Define(_) | Item::Include(_) | Item::Unsupported { .. } => {}
    }
}

fn collect_decl_names(decl: &Decl, candidates: &BTreeSet<String>, output: &mut BTreeSet<String>) {
    match decl {
        Decl::Var(var) => collect_var_names(var, candidates, output),
        Decl::Const(constant) => {
            for entry in &constant.entries {
                collect_expr_names(&entry.value, candidates, output);
            }
        }
        Decl::Type(decl) => {
            for field in &decl.fields {
                collect_var_names(field, candidates, output);
            }
        }
        Decl::Record(decl) => {
            for field in &decl.fields {
                collect_var_names(field, candidates, output);
            }
        }
    }
}

fn collect_var_names(var: &VarDecl, candidates: &BTreeSet<String>, output: &mut BTreeSet<String>) {
    for entry in &var.entries {
        if let Some(size) = &entry.size {
            collect_expr_names(size, candidates, output);
        }
        if let Some(initializer) = &entry.initializer {
            collect_expr_names(initializer, candidates, output);
        }
    }
}

fn collect_stmt_list_names(
    statements: &[Stmt],
    candidates: &BTreeSet<String>,
    output: &mut BTreeSet<String>,
) {
    for stmt in statements {
        collect_stmt_names(stmt, candidates, output);
    }
}

fn collect_stmt_names(stmt: &Stmt, candidates: &BTreeSet<String>, output: &mut BTreeSet<String>) {
    match stmt {
        Stmt::Return(value) => {
            if let Some(value) = value {
                collect_expr_names(value, candidates, output);
            }
        }
        Stmt::Assign { target, value, .. } | Stmt::CompoundAssign { target, value, .. } => {
            collect_expr_names(target, candidates, output);
            collect_expr_names(value, candidates, output);
        }
        Stmt::Call { expr, .. } => collect_expr_names(expr, candidates, output),
        Stmt::MachineBlock { items, .. } => collect_machine_names(items, candidates, output),
        Stmt::InlineAsm { program, .. } => {
            collect_machine_names(&program.items, candidates, output)
        }
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                collect_expr_names(&branch.condition, candidates, output);
                collect_stmt_list_names(&branch.body, candidates, output);
            }
            collect_stmt_list_names(else_body, candidates, output);
        }
        Stmt::While {
            condition, body, ..
        } => {
            collect_expr_names(condition, candidates, output);
            collect_stmt_list_names(body, candidates, output);
        }
        Stmt::DoUntil {
            body, condition, ..
        } => {
            collect_stmt_list_names(body, candidates, output);
            if let Some(condition) = condition {
                collect_expr_names(condition, candidates, output);
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
            collect_expr_names(target, candidates, output);
            collect_expr_names(start, candidates, output);
            collect_expr_names(end, candidates, output);
            if let Some(step) = step {
                collect_expr_names(step, candidates, output);
            }
            collect_stmt_list_names(body, candidates, output);
        }
        Stmt::Define(_) | Stmt::Exit { .. } | Stmt::Unsupported { .. } => {}
    }
}

fn collect_expr_names(expr: &Expr, candidates: &BTreeSet<String>, output: &mut BTreeSet<String>) {
    match &expr.kind {
        ExprKind::Name(name) => {
            if candidates.contains(name) {
                output.insert(name.clone());
            }
        }
        ExprKind::InitializerList(elements) => {
            for element in elements {
                if let InitializerElementKind::Address { target, .. } = &element.kind {
                    let name = target.display_name();
                    if candidates.contains(name) {
                        output.insert(name.to_string());
                    }
                }
            }
        }
        ExprKind::Unary { expr, .. } | ExprKind::Cast { expr, .. } => {
            collect_expr_names(expr, candidates, output)
        }
        ExprKind::Binary { left, right, .. } => {
            collect_expr_names(left, candidates, output);
            collect_expr_names(right, candidates, output);
        }
        ExprKind::Call { callee, args } => {
            collect_expr_names(callee, candidates, output);
            for arg in args {
                collect_expr_names(arg, candidates, output);
            }
        }
        ExprKind::Index { base, index } => {
            collect_expr_names(base, candidates, output);
            collect_expr_names(index, candidates, output);
        }
        ExprKind::Field { base, .. } => collect_expr_names(base, candidates, output),
        ExprKind::Missing
        | ExprKind::Raw
        | ExprKind::CurrentLocation
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Char(_) => {}
    }
}

fn collect_machine_names(
    items: &[MachineItem],
    candidates: &BTreeSet<String>,
    output: &mut BTreeSet<String>,
) {
    for item in items {
        let name = match item {
            MachineItem::Name(name) | MachineItem::AddressByte { name, .. } => Some(name),
            MachineItem::AddressExpr(MachineAddressExpr {
                atom: MachineAddressAtom::Name(name),
                ..
            }) => Some(name),
            _ => None,
        };
        if let Some(name) = name {
            let name = name.display_name();
            if candidates.contains(name) {
                output.insert(name.to_string());
            }
        }
    }
}
