use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Decl, Item, VarDecl};
use crate::diagnostic::Diagnostic;
use crate::embedded_vfs::EmbeddedSourceProvider;
use crate::includes::{
    ModuleLoadOptions, load_compilation_from_provider,
    load_program_with_expanded_source_from_provider,
};
use crate::runtime_link_manifest::{
    RuntimeLinkNode, embedded_sys_link_manifest, select_embedded_sys,
};
use crate::semantic::{SemanticOptions, analyze_compilation, analyze_compilation_with_options, ir};
use crate::source::{InMemorySourceProvider, SourceOrigin, Span};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RuntimeUnit {
    pub(crate) name: String,
    pub(crate) file_name: String,
    pub(crate) module_name: String,
    pub(crate) link_module: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeImage {
    pub(crate) semir: ir::SemProgram,
    /// Physical implementation unit for each case-normalized routine name.
    pub(crate) routine_units: BTreeMap<String, RuntimeUnit>,
}

#[derive(Debug, Clone)]
pub(crate) struct SelectedRuntimeImage {
    pub(crate) image: RuntimeImage,
    pub(crate) selection: crate::runtime_link_manifest::RuntimeLinkSelection,
}

/// Resolve the implementation-unit name used by an embedded runtime binding.
///
/// Binding sources use compact names such as `SYSBLK.Zero`; the runtime linker
/// derives every physical embedded-source and private-module name from that
/// value. Keeping this conversion in one place prevents each backend from
/// growing another SYSBLK-specific catalog.
pub(crate) fn resolve_runtime_unit(name: &str) -> Result<RuntimeUnit, Vec<Diagnostic>> {
    let name = name.to_ascii_uppercase();
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err(diagnostic(format!(
            "invalid embedded runtime unit `{name}` in standalone binding"
        )));
    }
    let module_name = format!("ACTION.RUNTIME.{name}");
    Ok(RuntimeUnit {
        file_name: format!("{}.act", name.to_ascii_lowercase()),
        link_module: module_name.replace('.', "_"),
        module_name,
        name,
    })
}

/// Compile one embedded Action! runtime source in an isolated semantic scope.
///
/// Historical runtime files use a pair of bare `MODULE` markers.  The linker
/// gives that source a private named-module identity before sending it through
/// the ordinary frontend, so every backend consumes the same resolved symbols.
pub(crate) fn compile_runtime_unit(
    file_name: &str,
    module_name: &str,
) -> Result<ir::SemProgram, Vec<Diagnostic>> {
    let source = EmbeddedSourceProvider
        .runtime_source(file_name)
        .ok_or_else(|| diagnostic(format!("embedded runtime source `{file_name}` is missing")))?;
    let text = crate::source::decode_source(source.bytes);
    let text = make_internal_named_module(&text, module_name, file_name)?;
    let origin = SourceOrigin::embedded(
        format!("runtime/internal/{file_name}"),
        format!("<runtime:{}>", file_name.to_ascii_uppercase()),
    );
    let provider = InMemorySourceProvider::default().with_source(origin.clone(), text);
    let loaded = load_compilation_from_provider(origin, &provider, &ModuleLoadOptions::default())
        .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    let model = analyze_compilation(&loaded)
        .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    Ok(ir::lower_compilation(&loaded, &model))
}

/// Compile a named module directly from the compiler's embedded VFS.
///
/// Runtime binding validation uses this to compare the selected implementation
/// against the authoritative public interface after lowering, including
/// function result types which are not represented by a routine's parameter
/// frame alone.
pub(crate) fn compile_embedded_module(file_name: &str) -> Result<ir::SemProgram, Vec<Diagnostic>> {
    let origin = SourceOrigin::embedded(
        file_name.to_ascii_lowercase(),
        format!("<embedded:{}>", file_name.to_ascii_uppercase()),
    );
    let loaded = load_compilation_from_provider(
        origin,
        &EmbeddedSourceProvider,
        &ModuleLoadOptions::default(),
    )
    .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    let model = analyze_compilation(&loaded)
        .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    Ok(ir::lower_compilation(&loaded, &model))
}

/// Compile the resident library as one semantic program.
///
/// SYSALL is the historical composition root.  Loading its embedded INCLUDEs
/// into one frontend invocation lets ordinary Action! name resolution retain
/// calls between implementation units while the source map records which
/// physical unit owns every routine.
pub(crate) fn compile_runtime_image() -> Result<RuntimeImage, Vec<Diagnostic>> {
    let file_name = "actionc.act";
    let origin = SourceOrigin::embedded("runtime/actionc.act", "<runtime:ACTIONC.ACT>");
    let expanded = load_program_with_expanded_source_from_provider(origin, &EmbeddedSourceProvider)
        .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    let mut routine_units = BTreeMap::new();
    let mut symbols = BTreeSet::new();
    for routine in expanded
        .program
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            Item::Routine(routine) => Some(routine),
            _ => None,
        })
    {
        symbols.insert(routine.name.clone());
        for param in &routine.params {
            collect_var_names(param, &mut symbols);
        }
        for local in &routine.locals {
            collect_decl_names(local, &mut symbols);
        }
        let location = expanded.source_map.location(routine.span).ok_or_else(|| {
            diagnostic(format!(
                "embedded runtime routine `{}` has no physical source provenance",
                routine.name
            ))
        })?;
        let unit = runtime_unit_from_origin(&location.origin, &routine.name)?;
        let key = routine.name.to_ascii_uppercase();
        if routine_units.insert(key.clone(), unit).is_some() {
            return Err(diagnostic(format!(
                "embedded runtime image has multiple routines named `{key}`"
            )));
        }
    }
    for item in expanded
        .program
        .modules
        .iter()
        .flat_map(|module| &module.items)
    {
        match item {
            Item::Define(define) => {
                symbols.extend(define.entries.iter().map(|entry| entry.name.clone()));
            }
            Item::Declaration(decl) => collect_decl_names(decl, &mut symbols),
            _ => {}
        }
    }

    // A single private module keeps machine-code symbol references relocatable.
    // The original bare MODULE markers describe editor/library regions, not
    // namespaces, so flatten them before the ordinary named-module frontend.
    let text = separate_machine_symbol_references(&expanded.source, &symbols);
    let text = apply_runtime_source_errata(&text);
    let text = make_flat_internal_module(&text, "ACTION.RUNTIME.RESIDENT");
    let origin = SourceOrigin::embedded(
        "runtime/internal/sysall.act",
        "<runtime:SYSALL.ACT linked image>",
    );
    let provider = InMemorySourceProvider::default().with_source(origin.clone(), text);
    let loaded = load_compilation_from_provider(origin, &provider, &ModuleLoadOptions::default())
        .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    let model = analyze_compilation_with_options(
        &loaded,
        SemanticOptions {
            native_real: true,
            lexical_blocks: false,
        },
    )
    .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    let semir = ir::lower_compilation(&loaded, &model);
    Ok(RuntimeImage {
        semir,
        routine_units,
    })
}

/// Select the embedded resident provider before either backend lowers it.
/// The checked-in graph owns dependencies; this function only binds stable
/// provider names to the freshly lowered SemIR and preserves source order.
pub(crate) fn select_runtime_image(
    roots_by_unit: &BTreeMap<RuntimeUnit, BTreeSet<String>>,
) -> Result<SelectedRuntimeImage, Vec<Diagnostic>> {
    let mut image = compile_runtime_image()?;
    let mut roots = BTreeSet::new();
    for (unit, expected_names) in roots_by_unit {
        for expected in expected_names {
            if image.routine_units.get(&expected.to_ascii_uppercase()) != Some(unit) {
                return Err(diagnostic(format!(
                    "embedded {} has no implementation routine `{expected}`",
                    unit.name
                )));
            }
            roots.insert(expected.to_ascii_uppercase());
        }
    }

    validate_runtime_manifest_bindings(&image.semir)?;
    let selection = select_embedded_sys(roots).map_err(|message| diagnostic(message))?;
    if !selection.statics.is_empty() {
        return Err(diagnostic(format!(
            "embedded SYS link manifest contains source-unbound static nodes: {}",
            selection.statics.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }

    for module in &mut image.semir.modules {
        module.items.retain(|item| match item {
            ir::SemItem::Routine(routine) if runtime_routine_emits_code(routine) => selection
                .routines
                .contains(&runtime_symbol_name(&routine.symbol)),
            // Fixed-address routines, DEFINEs, constants, and type metadata
            // are compile-time ABI facts needed by classic projection. They
            // emit no resident bytes by themselves.
            ir::SemItem::Routine(_) => true,
            ir::SemItem::Declaration(declaration)
                if matches!(
                    declaration.storage,
                    ir::SemDeclarationStorage::Type { .. }
                        | ir::SemDeclarationStorage::Record { .. }
                ) =>
            {
                true
            }
            ir::SemItem::Declaration(declaration) => selection
                .globals
                .contains(&runtime_symbol_name(&declaration.symbol)),
            ir::SemItem::Define(_) | ir::SemItem::Const(_) | ir::SemItem::Include(_) => true,
            ir::SemItem::Set(_) | ir::SemItem::Statement(_) | ir::SemItem::Unsupported { .. } => {
                false
            }
        });
    }
    image.semir.entry_routine = None;
    image
        .routine_units
        .retain(|name, _| selection.routines.contains(name));

    Ok(SelectedRuntimeImage { image, selection })
}

fn validate_runtime_manifest_bindings(program: &ir::SemProgram) -> Result<(), Vec<Diagnostic>> {
    let manifest = embedded_sys_link_manifest().map_err(|message| diagnostic(message))?;
    let expected_routines = manifest
        .graph
        .nodes()
        .iter()
        .filter_map(|node| match node {
            RuntimeLinkNode::Routine(name) if name != "<PROGRAM>" => Some(name.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let expected_globals = manifest
        .graph
        .nodes()
        .iter()
        .filter_map(|node| match node {
            RuntimeLinkNode::Global(name) => Some(name.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut actual_routines = BTreeSet::new();
    let mut actual_globals = BTreeSet::new();
    for item in program.modules.iter().flat_map(|module| &module.items) {
        match item {
            ir::SemItem::Define(define) => {
                let name = runtime_symbol_name(&define.symbol);
                if expected_globals.contains(&name) && !actual_globals.insert(name.clone()) {
                    return Err(diagnostic(format!(
                        "embedded runtime has multiple source globals named `{name}`"
                    )));
                }
            }
            ir::SemItem::Routine(routine) => {
                let name = runtime_symbol_name(&routine.symbol);
                if !actual_routines.insert(name.clone()) {
                    return Err(diagnostic(format!(
                        "embedded runtime has multiple source routines named `{name}`"
                    )));
                }
                for declaration in &routine.locals {
                    insert_runtime_global_binding(
                        declaration,
                        &expected_globals,
                        &mut actual_globals,
                    )?;
                }
                for statement in &routine.body {
                    collect_runtime_statement_globals(
                        statement,
                        &expected_globals,
                        &mut actual_globals,
                    )?;
                }
            }
            ir::SemItem::Declaration(declaration)
                if matches!(
                    declaration.storage,
                    ir::SemDeclarationStorage::Scalar | ir::SemDeclarationStorage::Array { .. }
                ) =>
            {
                let name = runtime_symbol_name(&declaration.symbol);
                if expected_globals.contains(&name) && !actual_globals.insert(name.clone()) {
                    return Err(diagnostic(format!(
                        "embedded runtime has multiple source globals named `{name}`"
                    )));
                }
            }
            _ => {}
        }
    }
    if actual_routines != expected_routines || actual_globals != expected_globals {
        let missing = expected_routines
            .difference(&actual_routines)
            .chain(expected_globals.difference(&actual_globals))
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = actual_routines
            .difference(&expected_routines)
            .chain(actual_globals.difference(&expected_globals))
            .cloned()
            .collect::<Vec<_>>();
        return Err(diagnostic(format!(
            "embedded SYS link manifest source-node mismatch; missing [{}], unexpected [{}]",
            missing.join(", "),
            unexpected.join(", ")
        )));
    }
    Ok(())
}

fn collect_runtime_statement_globals(
    statement: &ir::SemStmt,
    expected: &BTreeSet<String>,
    globals: &mut BTreeSet<String>,
) -> Result<(), Vec<Diagnostic>> {
    match statement {
        ir::SemStmt::LexicalBlock {
            declarations, body, ..
        } => {
            for declaration in declarations {
                insert_runtime_global_binding(declaration, expected, globals)?;
            }
            for statement in body {
                collect_runtime_statement_globals(statement, expected, globals)?;
            }
        }
        ir::SemStmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                for statement in &branch.body {
                    collect_runtime_statement_globals(statement, expected, globals)?;
                }
            }
            for statement in else_body {
                collect_runtime_statement_globals(statement, expected, globals)?;
            }
        }
        ir::SemStmt::While { body, .. }
        | ir::SemStmt::DoUntil { body, .. }
        | ir::SemStmt::For { body, .. } => {
            for statement in body {
                collect_runtime_statement_globals(statement, expected, globals)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn insert_runtime_global_binding(
    declaration: &ir::SemDeclaration,
    expected: &BTreeSet<String>,
    globals: &mut BTreeSet<String>,
) -> Result<(), Vec<Diagnostic>> {
    if !matches!(
        declaration.storage,
        ir::SemDeclarationStorage::Scalar | ir::SemDeclarationStorage::Array { .. }
    ) {
        return Ok(());
    }
    let name = runtime_symbol_name(&declaration.symbol);
    if !expected.contains(&name) {
        return Ok(());
    }
    if !globals.insert(name.clone()) {
        return Err(diagnostic(format!(
            "embedded runtime has multiple source globals named `{name}`"
        )));
    }
    Ok(())
}

fn runtime_routine_emits_code(routine: &ir::SemRoutine) -> bool {
    routine
        .system_address
        .as_ref()
        .is_none_or(|address| matches!(address.kind, ir::SemExprKind::CurrentLocation))
}

fn runtime_symbol_name(symbol: &ir::SemSymbolRef) -> String {
    symbol
        .qualified_name
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(&symbol.name)
        .to_ascii_uppercase()
}

fn runtime_unit_from_origin(
    origin: &SourceOrigin,
    routine_name: &str,
) -> Result<RuntimeUnit, Vec<Diagnostic>> {
    let virtual_path = origin.virtual_path().ok_or_else(|| {
        diagnostic(format!(
            "embedded runtime routine `{routine_name}` unexpectedly came from {origin}"
        ))
    })?;
    let unit_name = virtual_path
        .strip_prefix("runtime/")
        .and_then(|path| path.strip_suffix(".act"))
        .ok_or_else(|| {
            diagnostic(format!(
                "embedded runtime routine `{routine_name}` came from invalid path `{virtual_path}`"
            ))
        })?;
    resolve_runtime_unit(unit_name)
}

fn make_flat_internal_module(source: &str, module_name: &str) -> String {
    let mut output = format!("MODULE {module_name}\n");
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.to_ascii_uppercase().starts_with("MODULE ;")
            || trimmed.eq_ignore_ascii_case("MODULE")
        {
            output.push('\n');
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    output.push_str("ENDMODULE\n");
    output
}

fn collect_decl_names(decl: &Decl, output: &mut BTreeSet<String>) {
    match decl {
        Decl::Var(var) => collect_var_names(var, output),
        Decl::Const(constant) => {
            output.extend(constant.entries.iter().map(|entry| entry.name.clone()));
        }
        Decl::Type(decl) => {
            output.insert(decl.name.clone());
            for field in &decl.fields {
                collect_var_names(field, output);
            }
        }
        Decl::Record(decl) => {
            output.insert(decl.name.clone());
            for field in &decl.fields {
                collect_var_names(field, output);
            }
        }
    }
}

fn collect_var_names(var: &VarDecl, output: &mut BTreeSet<String>) {
    output.extend(var.entries.iter().map(|entry| entry.name.clone()));
}

/// Correct verified interface defects in the historical standalone source
/// without rewriting the preserved source corpus. The Action! 3.6 compiler
/// accepts `PrintBDE(device,value)` and emits `LDA device; LDX value; JSR
/// $A508`, while SYSIO.ACT accidentally omits the device parameter. `Error`
/// accepts an error code in A plus optional X/Y handler context, as exercised
/// by CATCH.ACT, while SYSLIB.ACT declares only the first byte. `PrintH` stores
/// its shifting value in `$A4/$A5`, but its call to `Put` reaches `CCIO`, which
/// overwrites `$A4`; move that private value to the unused `$A2/$A3` pair. The
/// normalized routines remain current-location machine-code entries because
/// their bodies consume the Action ABI directly and must not acquire an
/// `SArgs` prologue. SYSGR loads `dev_E` and `dev_S` as pointer words, matching
/// the original compiler's global STRING representation; materialize explicit
/// pointer arrays so the modern frontend preserves that machine-code contract.
fn apply_runtime_source_errata(source: &str) -> String {
    source
        .replace("PROC PrintBDE=*(BYTE n)", "PROC PrintBDE=*(BYTE d,n)")
        .replace("PROC Error(BYTE err)", "PROC Error=*(BYTE err,x,y)")
        .replace("$A485$A586$4A9$A685$24A9", "$A285$A386$4A9$A685$24A9")
        .replace(
            "$A9$0$4A2$A406$A526$2A$CA$F8D0",
            "$A9$0$4A2$A206$A326$2A$CA$F8D0",
        )
        .replace(
            "STRING dev_S=\"S:\", dev_E=\"E:\"",
            "BYTE ARRAY dev_S_value=\"S:\"\n\
             BYTE ARRAY dev_E_value=\"E:\"\n\
             CARD ARRAY dev_S(1)=[@dev_S_value]\n\
             CARD ARRAY dev_E(1)=[@dev_E_value]",
        )
}

/// The original Action! machine-code notation permits an opcode byte and a
/// symbol with no intervening space (`$A5device`).  The general lexer quite
/// reasonably reads the leading hexadecimal letters of the symbol as part of
/// the number, so normalize only compiler-owned runtime sources before their
/// second frontend pass.
fn separate_machine_symbol_references(source: &str, symbols: &BTreeSet<String>) -> String {
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    while cursor < source.len() {
        let remaining = &source[cursor..];
        if remaining.starts_with('$')
            && remaining
                .as_bytes()
                .get(1..3)
                .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
            && remaining
                .as_bytes()
                .get(3)
                .is_some_and(|byte| byte.is_ascii_lowercase() || *byte == b'_')
        {
            output.push_str(&remaining[..3]);
            output.push(' ');
            cursor += 3;
            continue;
        }
        if remaining.starts_with('$')
            && remaining
                .as_bytes()
                .get(1..3)
                .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
            && let Some(symbol) = symbols.iter().find(|symbol| {
                remaining[3..].starts_with(symbol.as_str())
                    && remaining[3 + symbol.len()..]
                        .chars()
                        .next()
                        .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
            })
        {
            output.push_str(&remaining[..3]);
            output.push(' ');
            output.push_str(symbol);
            cursor += 3 + symbol.len();
            continue;
        }
        let ch = remaining
            .chars()
            .next()
            .expect("remaining source character");
        output.push(ch);
        cursor += ch.len_utf8();
    }
    output
}

fn make_internal_named_module(
    source: &str,
    module_name: &str,
    file_name: &str,
) -> Result<String, Vec<Diagnostic>> {
    let mut converted_first_marker = false;
    let mut output = String::with_capacity(source.len() + 32);
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.to_ascii_uppercase().starts_with("MODULE ;") {
            if !converted_first_marker {
                output.push_str("MODULE ");
                output.push_str(module_name);
                converted_first_marker = true;
            } else {
                output.push_str("ENDMODULE");
            }
        } else if converted_first_marker && trimmed.eq_ignore_ascii_case("MODULE") {
            output.push_str("ENDMODULE");
        } else {
            output.push_str(line);
        }
        output.push('\n');
    }
    if !converted_first_marker {
        return Err(diagnostic(format!(
            "embedded runtime source `{file_name}` has no legacy MODULE marker"
        )));
    }
    Ok(output)
}

fn frontend_diagnostics(file_name: &str, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| {
            Diagnostic::new(
                diagnostic.span,
                format!("embedded {file_name} frontend: {}", diagnostic.message),
            )
        })
        .collect()
}

fn diagnostic(message: impl Into<String>) -> Vec<Diagnostic> {
    vec![Diagnostic::new(Span::new(0, 0), message)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_unit_identity_is_derived_from_a_binding_name() {
        assert_eq!(
            resolve_runtime_unit("sysblk").expect("runtime unit"),
            RuntimeUnit {
                name: "SYSBLK".to_string(),
                file_name: "sysblk.act".to_string(),
                module_name: "ACTION.RUNTIME.SYSBLK".to_string(),
                link_module: "ACTION_RUNTIME_SYSBLK".to_string(),
            }
        );
    }

    #[test]
    fn runtime_unit_identity_cannot_escape_the_embedded_vfs() {
        let diagnostics = resolve_runtime_unit("../SYSBLK").expect_err("invalid unit");
        assert!(
            diagnostics[0]
                .message
                .contains("invalid embedded runtime unit")
        );
    }

    #[test]
    fn embedded_sys_interface_retains_external_function_results() {
        let program = compile_embedded_module("sys.act").expect("compile SYS interface");
        let scompare = program
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find_map(|item| match item {
                ir::SemItem::Routine(routine)
                    if routine
                        .symbol
                        .qualified_name
                        .eq_ignore_ascii_case("SYS.SCompare") =>
                {
                    Some(routine)
                }
                _ => None,
            })
            .expect("SYS.SCompare interface");
        assert!(scompare.is_external);
        assert_eq!(
            scompare.signature.return_type,
            Some(crate::ast::FundType::Int)
        );
    }

    #[test]
    fn runtime_image_resolves_calls_across_physical_units() {
        let image = compile_runtime_image().expect("compile resident runtime image");
        assert_eq!(image.routine_units["RAND"].name, "SYSMISC");
        assert_eq!(image.routine_units["MULTI"].name, "SYSLIB");
        assert_eq!(image.routine_units["GRAPHICS"].name, "SYSGR");
        assert_eq!(image.routine_units["OPEN"].name, "SYSIO");
    }

    #[test]
    fn resident_provider_is_filtered_in_semir_before_backend_lowering() {
        let unit = resolve_runtime_unit("SYSBLK").expect("SYSBLK unit");
        let selected = select_runtime_image(&BTreeMap::from([(
            unit,
            BTreeSet::from(["Zero".to_string()]),
        )]))
        .expect("select Zero provider SemIR");
        let routines = selected
            .image
            .semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .filter_map(|item| match item {
                ir::SemItem::Routine(routine) if runtime_routine_emits_code(routine) => {
                    Some(runtime_symbol_name(&routine.symbol))
                }
                _ => None,
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(
            routines,
            BTreeSet::from(["SETBLOCK".to_string(), "ZERO".to_string()])
        );
        assert_eq!(routines, selected.selection.routines);
        assert!(selected.selection.globals.is_empty());
        assert!(selected.image.semir.entry_routine.is_none());
        assert!(selected.image.semir.modules.iter().all(|module| {
            module
                .items
                .iter()
                .all(|item| !matches!(item, ir::SemItem::Set(_) | ir::SemItem::Statement(_)))
        }));
    }

    #[test]
    fn runtime_image_applies_verified_interface_errata() {
        let sysio = EmbeddedSourceProvider
            .runtime_source("sysio.act")
            .expect("embedded SYSIO source");
        let normalized = apply_runtime_source_errata(&crate::source::decode_source(sysio.bytes));
        assert!(normalized.contains("$A285$A386$4A9$A685$24A9"));
        assert!(normalized.contains("$A9$0$4A2$A206$A326$2A$CA$F8D0"));
        assert!(!normalized.contains("$A485$A586$4A9$A685$24A9"));
        assert!(!normalized.contains("$A9$0$4A2$A406$A526$2A$CA$F8D0"));
        let sysgr = EmbeddedSourceProvider
            .runtime_source("sysgr.act")
            .expect("embedded SYSGR source");
        let normalized_graphics =
            apply_runtime_source_errata(&crate::source::decode_source(sysgr.bytes));
        assert!(normalized_graphics.contains("CARD ARRAY dev_S(1)=[@dev_S_value]"));
        assert!(normalized_graphics.contains("CARD ARRAY dev_E(1)=[@dev_E_value]"));

        let image = compile_runtime_image().expect("compile resident runtime image");
        let routine = image
            .semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find_map(|item| match item {
                ir::SemItem::Routine(routine)
                    if routine
                        .symbol
                        .qualified_name
                        .rsplit(['.', ':'])
                        .find(|part| !part.is_empty())
                        .is_some_and(|name| name.eq_ignore_ascii_case("PrintBDE")) =>
                {
                    Some(routine)
                }
                _ => None,
            })
            .expect("resident PrintBDE");
        assert_eq!(routine.signature.params.len(), 2);
        assert!(routine.signature.params.iter().all(|param| {
            matches!(
                &param.base,
                crate::semantic::ValueTypeBase::Fund(crate::ast::FundType::Byte)
            ) && !param.pointer
        }));

        let error = image
            .semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find_map(|item| match item {
                ir::SemItem::Routine(routine)
                    if routine
                        .symbol
                        .qualified_name
                        .rsplit(['.', ':'])
                        .find(|part| !part.is_empty())
                        .is_some_and(|name| name.eq_ignore_ascii_case("Error")) =>
                {
                    Some(routine)
                }
                _ => None,
            })
            .expect("resident Error");
        assert_eq!(error.signature.params.len(), 3);
        assert!(error.signature.params.iter().all(|param| {
            matches!(
                &param.base,
                crate::semantic::ValueTypeBase::Fund(crate::ast::FundType::Byte)
            ) && !param.pointer
        }));
    }
}
