//! Bare native image API, independent of Atari runtimes and load-file formats.
pub mod artifacts;
pub mod runtime;
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
    /// Host inputs are protected by artifact publication, not embedded in images.
    pub source_paths: Vec<PathBuf>,
}

/// Verified target input, before choosing a platform adapter or link layout.
#[derive(Debug, Clone)]
pub struct PreparedNativeProgram {
    pub mir: mir68k::Mir68kProgram,
    pub source_paths: Vec<PathBuf>,
    display_names: std::collections::BTreeMap<String, String>,
}

pub fn compile_file(
    path: impl AsRef<Path>,
    options: &NativeCompileOptions,
) -> Result<NativeCompiledProgram, CompileError> {
    if options.origin & 1 != 0 || options.origin >= 0x1000000 {
        return Err(CompileError::configuration(
            "native origin must be even and within the 24-bit address space",
        ));
    }
    let prepared = prepare_file(path, options)?;
    let machine = mir68k::materialize::materialize_with_options(&prepared.mir, options.codegen)
        .map_err(codegen_error)?;
    let mut image =
        mir68k::image::link(&prepared.mir, &machine, options.origin).map_err(codegen_error)?;
    prepared.qualify_symbols(&mut image.symbols);
    Ok(NativeCompiledProgram {
        image,
        machine,
        source_paths: prepared.source_paths,
    })
}

pub fn prepare_file(
    path: impl AsRef<Path>,
    options: &NativeCompileOptions,
) -> Result<PreparedNativeProgram, CompileError> {
    let path = path.as_ref();
    if options.target != crate::target::TargetId::Motorola68000 {
        return Err(CompileError::configuration(
            "native emission currently supports only Motorola68000",
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
    let legacy_origin = semir.modules.iter().flat_map(|m| &m.items).any(|item| {
        matches!(item, semantic::ir::SemItem::Set(set)
            if matches!(super::sem_const_u16(&set.address), Some(0x000e | 0x000f | 0x0491 | 0x0492)))
    });
    if semir.origin.is_some() || legacy_origin {
        return Err(CompileError::configuration(
            "native source ORG/SET origin is unsupported; use the full-width native origin option",
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
    let source_paths = loaded
        .source_map
        .source_origins()
        .filter_map(|origin| {
            if let crate::source::SourceOrigin::Host(path) = origin {
                Some(path.clone())
            } else {
                None
            }
        })
        .collect();
    Ok(PreparedNativeProgram {
        mir,
        source_paths,
        display_names,
    })
}

impl PreparedNativeProgram {
    fn qualify_symbols(&self, symbols: &mut [mir68k::image::Symbol]) {
        for symbol in symbols {
            let (base, suffix) = if let Some((base, local)) = symbol.name.split_once("::") {
                (base, format!("::{local}"))
            } else if let Some(base) = symbol.name.strip_suffix(".__backing") {
                (base, ".__backing".into())
            } else {
                (symbol.name.as_str(), String::new())
            };
            if let Some(display) = self.display_names.get(base) {
                symbol.name = format!("{display}{suffix}");
            }
        }
    }
}

fn codegen_error(message: String) -> CompileError {
    CompileError::from_ir_diagnostics(CompilerPhase::Codegen, [(None, None, message)])
}
