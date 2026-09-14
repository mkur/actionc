//! Bare native image API, independent of Atari runtimes and load-file formats.
use super::{CompileError, CompilerPhase};
use crate::{
    backend::BackendLoweringError,
    includes::{ModuleLoadOptions, load_compilation},
    mir68k, nir,
    semantic::{self, SemanticOptions},
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct NativeCompileOptions {
    pub origin: u32,
    pub optimize: bool,
    pub promotion: nir::NirPromotionPolicy,
    /// Target optimization is independent of shared NIR optimization.
    pub codegen: mir68k::materialize::Options,
    pub target: crate::target::TargetId,
    pub project_root: Option<PathBuf>,
    pub module_paths: Vec<PathBuf>,
}
impl Default for NativeCompileOptions {
    fn default() -> Self {
        Self {
            origin: 0x10000,
            optimize: true,
            promotion: nir::NirPromotionPolicy::NativeLoops,
            codegen: Default::default(),
            target: crate::target::TargetId::Motorola68000,
            project_root: None,
            module_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NativeCompiledProgram {
    pub image: mir68k::image::NativeImage,
    pub machine: mir68k::machine::MachineProgram,
}

pub fn compile_file(
    path: impl AsRef<Path>,
    options: &NativeCompileOptions,
) -> Result<NativeCompiledProgram, CompileError> {
    let path = path.as_ref();
    if options.target != crate::target::TargetId::Motorola68000 {
        return Err(CompileError::configuration(
            "native emission currently supports only Motorola68000",
        ));
    }
    if options.origin & 1 != 0 || options.origin >= 0x1000000 {
        return Err(CompileError::configuration(
            "native origin must be even and within the 24-bit address space",
        ));
    }
    let loaded = load_compilation(
        path,
        &ModuleLoadOptions {
            project_root: options.project_root.clone(),
            module_paths: options.module_paths.clone(),
        },
    )
    .map_err(|d| {
        let source = std::fs::read(path)
            .map(|b| crate::source::decode_source(&b))
            .unwrap_or_default();
        CompileError::from_source_diagnostics(CompilerPhase::Frontend, d, &source, path, None)
    })?;
    let model = semantic::analyze_compilation_with_options(
        &loaded,
        SemanticOptions::modern().with_target(options.target),
    )
    .map_err(|d| {
        CompileError::from_source_diagnostics(
            CompilerPhase::Semantic,
            d,
            &loaded.source,
            path,
            Some(&loaded.source_map),
        )
    })?;
    let semir = semantic::ir::lower_compilation(&loaded, &model);
    if semir
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .any(|i| matches!(i, semantic::ir::SemItem::Statement(_)))
    {
        return Err(codegen_error(
            "native executable top-level statements require an explicit startup contract".into(),
        ));
    }
    if semir.origin.is_some() {
        return Err(CompileError::configuration(
            "native source SET origin is unsupported; use the full-width native origin option",
        ));
    }
    let semir = crate::linker::select_semir(&semir, crate::linker::SemLinkPolicy::EntryReachable)
        .map_err(codegen_error)?;
    // This is display metadata only. The backend receives verified NIR and
    // resolves code/storage exclusively by IDs; it never consults SemIR.
    let display_names: std::collections::BTreeMap<_, _> = semir
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .filter_map(|item| {
            let symbol = match item {
                semantic::ir::SemItem::Declaration(d) => &d.symbol,
                semantic::ir::SemItem::Routine(r) => &r.symbol,
                _ => return None,
            };
            Some((symbol.name.clone(), symbol.qualified_name.clone()))
        })
        .collect();
    let nir = nir::lower_program(&semir);
    let nir = if options.optimize {
        nir::optimize_program_with_promotion(&nir, options.promotion)
            .map_err(CompileError::from_nir_diagnostics)?
    } else {
        nir
    };
    let mir = mir68k::lower_program(&nir).map_err(|e| match e {
        BackendLoweringError::InvalidNir(d) => CompileError::from_nir_diagnostics(d),
        BackendLoweringError::UnsupportedTarget(t) => {
            CompileError::configuration(format!("unsupported native target {t}"))
        }
        BackendLoweringError::Backend(d) => CompileError::from_ir_diagnostics(
            CompilerPhase::Codegen,
            d.into_iter().map(|d| (d.routine, d.block, d.message)),
        ),
    })?;
    let machine = mir68k::materialize::materialize_with_options(&mir, options.codegen)
        .map_err(codegen_error)?;
    let mut image = mir68k::image::link(&mir, &machine, options.origin).map_err(codegen_error)?;
    for symbol in &mut image.symbols {
        let (base, suffix) = if let Some((base, local)) = symbol.name.split_once("::") {
            (base, format!("::{local}"))
        } else if let Some(base) = symbol.name.strip_suffix(".__backing") {
            (base, ".__backing".into())
        } else {
            (symbol.name.as_str(), String::new())
        };
        if let Some(display) = display_names.get(base) {
            symbol.name = format!("{display}{suffix}");
        }
    }
    Ok(NativeCompiledProgram { image, machine })
}

fn codegen_error(message: String) -> CompileError {
    CompileError::from_ir_diagnostics(CompilerPhase::Codegen, [(None, None, message)])
}
