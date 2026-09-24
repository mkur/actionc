//! Freestanding 65816 compilation. Frontend policy lives here; instruction
//! selection and linking consume only verified target IR and stable identities.
use super::{CompileError, CompilerPhase};
use crate::{
    backend::BackendLoweringError,
    includes::{ModuleLoadOptions, load_compilation},
    mir65816, nir, semantic,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Prepared {
    pub mir: mir65816::Mir65816Program,
    pub source_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Compiled {
    pub image: mir65816::image::Image,
    pub machine: mir65816::emit::MachineProgram,
    pub source_paths: Vec<PathBuf>,
}

/// Self-contained experimental o65 output, separate from JSON image v3.
#[derive(Debug, Clone)]
pub struct O65Compiled {
    pub bytes: Vec<u8>,
    pub source_paths: Vec<PathBuf>,
}

pub fn prepare_file(
    path: impl AsRef<Path>,
    optimize: bool,
    modules: &ModuleLoadOptions,
) -> Result<Prepared, CompileError> {
    let path = path.as_ref();
    let loaded = load_compilation(path, modules).map_err(|d| {
        let source = std::fs::read(path)
            .map(|b| crate::source::decode_source(&b))
            .unwrap_or_default();
        CompileError::from_source_diagnostics(CompilerPhase::Frontend, d, &source, path, None)
    })?;
    let model = semantic::analyze_compilation_with_options(
        &loaded,
        semantic::SemanticOptions::modern().with_target(crate::target::TargetId::Wdc65816Native),
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
    // The shared declaration-address resolver still uses a 16-bit address
    // cursor. A wider bare declaration initializer can otherwise become data
    // silently. Keep that source form out of this emitter's advertised subset.
    for declaration in semir
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .flat_map(|item| match item {
            semantic::ir::SemItem::Declaration(d) => std::slice::from_ref(d),
            semantic::ir::SemItem::Routine(r) => r.locals.as_slice(),
            _ => &[],
        })
    {
        if declaration
            .initializer
            .as_ref()
            .is_some_and(wide_bare_initializer)
        {
            return Err(codegen(format!(
                "{}: absolute declaration aliases above bank zero are unsupported; use an explicit 24-bit pointer for banked memory, or brackets for initialized data",
                declaration.symbol.name
            )));
        }
    }
    if semir
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .any(|i| matches!(i, semantic::ir::SemItem::Statement(_)))
    {
        return Err(codegen(
            "native top-level statements require an explicit startup contract".into(),
        ));
    }
    if semir.origin.is_some() || semir.modules.iter().flat_map(|m| &m.items).any(|item| matches!(item, semantic::ir::SemItem::Set(set) if matches!(super::sem_const_u16(&set.address), Some(0x000e | 0x000f | 0x0491 | 0x0492)))) {
        return Err(CompileError::configuration("native source ORG/SET origin is unsupported; use the 65816 layout file"));
    }
    // Keep exported routines callable by independently assembled code, even if
    // no Action! routine refers to them. Do not identify entries by their names.
    let program = nir::lower_program(&semir);
    let program = if optimize {
        nir::optimize_program_with_promotion(&program, nir::NirPromotionPolicy::Native65816)
            .map_err(CompileError::from_nir_diagnostics)?
    } else {
        program
    };
    let mir = mir65816::lower_program(&program).map_err(|e| match e {
        BackendLoweringError::InvalidNir(d) => CompileError::from_nir_diagnostics(d),
        BackendLoweringError::UnsupportedTarget(t) => {
            CompileError::configuration(format!("unsupported native target {t}"))
        }
        BackendLoweringError::Backend(d) => CompileError::from_ir_diagnostics(
            CompilerPhase::Codegen,
            d.into_iter().map(|d| (d.routine, d.block, d.message)),
        ),
    })?;
    let mir = mir65816::arithmetic::prepare(&mir).map_err(codegen)?;
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
    Ok(Prepared { mir, source_paths })
}

impl Prepared {
    pub fn compile_o65(
        &self,
        options: &mir65816::o65::Options,
    ) -> Result<O65Compiled, CompileError> {
        let artifact = mir65816::o65::prepare(&self.mir, options).map_err(codegen)?;
        let bytes = mir65816::o65::write(&artifact).map_err(codegen)?;
        mir65816::o65::inspect(&bytes).map_err(codegen)?;
        Ok(O65Compiled {
            bytes,
            source_paths: self.source_paths.clone(),
        })
    }

    pub fn compile(&self, layout: &mir65816::image::LinkOptions) -> Result<Compiled, CompileError> {
        let machine = mir65816::emit::materialize(&self.mir).map_err(codegen)?;
        let image = mir65816::image::link(&self.mir, &machine, layout).map_err(codegen)?;
        Ok(Compiled {
            image,
            machine,
            source_paths: self.source_paths.clone(),
        })
    }
}

/// One self-contained JSON image. Validate everything before opening output;
/// publish by rename and protect every loaded source plus the layout file.
pub fn write(program: &Compiled, output: &Path, protected: &[&Path]) -> Result<(), String> {
    let bytes = program.image.to_json()?;
    publish(&bytes, &program.source_paths, output, protected)
}

pub fn write_o65(program: &O65Compiled, output: &Path, protected: &[&Path]) -> Result<(), String> {
    mir65816::o65::inspect(&program.bytes)?;
    publish(&program.bytes, &program.source_paths, output, protected)
}

fn publish(
    bytes: &[u8],
    source_paths: &[PathBuf],
    output: &Path,
    protected: &[&Path],
) -> Result<(), String> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let destination = super::native::artifacts::destination(output)?;
    for path in source_paths
        .iter()
        .map(PathBuf::as_path)
        .chain(protected.iter().copied())
    {
        if path.canonicalize().map_err(|e| e.to_string())? == destination {
            return Err("65816 output would overwrite an input".into());
        }
    }
    let temporary = destination.with_file_name(format!(
        ".actionc65816-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = file.write_all(bytes).and_then(|_| {
        drop(file);
        std::fs::rename(&temporary, &destination)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}

fn codegen(message: String) -> CompileError {
    CompileError::from_ir_diagnostics(CompilerPhase::Codegen, [(None, None, message)])
}

fn wide_bare_initializer(expr: &semantic::ir::SemExpr) -> bool {
    use semantic::ir::{SemExprKind, SemLiteral};
    match &expr.kind {
        SemExprKind::Literal(SemLiteral::Number(number)) => number.value.is_some_and(|v| v > 65535),
        SemExprKind::Literal(SemLiteral::Constant(value)) => value.bits > 65535,
        SemExprKind::Cast { expr, .. } | SemExprKind::Unary { expr, .. } => {
            wide_bare_initializer(expr)
        }
        SemExprKind::Binary { left, right, .. } => {
            wide_bare_initializer(left) || wide_bare_initializer(right)
        }
        // Bracketed values and resolved symbol references have distinct source
        // meanings; this guard does not reinterpret them as numeric addresses.
        _ => false,
    }
}
