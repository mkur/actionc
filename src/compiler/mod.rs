pub(crate) mod artifacts;
mod diagnostics;
pub(crate) mod validation;

use std::fs;
use std::path::{Path, PathBuf};

use crate::codegen::{
    CODE_ORIGIN, CodegenOutput, CodegenProfile, format_load_file, generate_profile_at_origin,
    generate_profile_with_origin, generate_semir_profile_at_origin,
    generate_semir_profile_with_origin, generate_semir_standalone_profile_at_origin,
};
use crate::includes::{ModuleLoadOptions, load_compilation};
use crate::mir6502;
use crate::nir;
use crate::semantic::{
    SemanticOptions, analyze_compilation_with_options, ir, materialize::materialize_constants,
};
use crate::source::decode_source;

use self::validation::{legacy_routine_retargeting_diagnostics, standalone_resident_diagnostics};

pub use crate::runtime::Runtime;
pub use diagnostics::{
    CompileError, CompileErrorKind, CompilerDiagnostic, CompilerPhase, DiagnosticSite,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileMode {
    Compatibility,
    Optimized,
    Mir6502,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backend {
    Classic,
    Mir6502,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodegenSource {
    Ast,
    SemIr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompileRequest {
    pub(crate) profile: CodegenProfile,
    pub(crate) profile_explicit: bool,
    pub(crate) backend: Backend,
    pub(crate) backend_explicit: bool,
    pub(crate) runtime: Runtime,
    pub(crate) runtime_explicit: bool,
    pub(crate) codegen_source: CodegenSource,
    pub(crate) origin: Option<u16>,
    pub(crate) project_root: Option<PathBuf>,
    pub(crate) module_paths: Vec<PathBuf>,
}

impl Default for CompileRequest {
    fn default() -> Self {
        Self {
            profile: CodegenProfile::Compat,
            profile_explicit: false,
            backend: Backend::Classic,
            backend_explicit: false,
            runtime: Runtime::ActionCart,
            runtime_explicit: false,
            codegen_source: CodegenSource::Ast,
            origin: None,
            project_root: None,
            module_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedCompileRequest {
    profile: CodegenProfile,
    backend: Backend,
    runtime: Runtime,
    codegen_source: CodegenSource,
    origin: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompileOptions {
    mode: Option<CompileMode>,
    origin: Option<u16>,
    project_root: Option<PathBuf>,
    module_paths: Vec<PathBuf>,
    runtime: Runtime,
}

impl CompileOptions {
    pub fn for_mode(mode: CompileMode) -> Self {
        Self {
            mode: Some(mode),
            origin: None,
            project_root: None,
            module_paths: Vec::new(),
            runtime: Runtime::ActionCart,
        }
    }

    pub fn with_origin(mut self, origin: u16) -> Self {
        self.origin = Some(origin);
        self
    }

    pub fn mode(&self) -> Option<CompileMode> {
        self.mode
    }

    pub fn origin(&self) -> Option<u16> {
        self.origin
    }

    pub fn with_runtime(mut self, runtime: Runtime) -> Self {
        self.runtime = runtime;
        self
    }

    pub fn runtime(&self) -> Runtime {
        self.runtime
    }

    pub fn with_project_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_root = Some(path.into());
        self
    }

    pub fn with_module_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.module_paths.push(path.into());
        self
    }

    pub fn project_root(&self) -> Option<&Path> {
        self.project_root.as_deref()
    }

    pub fn module_paths(&self) -> &[PathBuf] {
        &self.module_paths
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledProgram {
    object: Vec<u8>,
    pub(crate) output: CodegenOutput,
    pub(crate) expanded_source: String,
}

impl CompiledProgram {
    pub fn object_bytes(&self) -> &[u8] {
        &self.object
    }

    pub fn source_listing(&self) -> String {
        artifacts::format_listing_with_source(&self.output, &self.expanded_source)
    }

    pub fn origin(&self) -> u16 {
        self.output.origin
    }

    pub fn run_address(&self) -> u16 {
        self.output.run_address
    }

    pub fn runtime(&self) -> Runtime {
        self.output.map.runtime
    }
}

pub fn compile_file(
    path: impl AsRef<Path>,
    options: &CompileOptions,
) -> Result<CompiledProgram, CompileError> {
    let request = compile_request_from_options(options);
    compile_file_with_request(path.as_ref(), &request)
}

pub(crate) fn compile_file_with_request(
    path: &Path,
    request: &CompileRequest,
) -> Result<CompiledProgram, CompileError> {
    let module_options = ModuleLoadOptions {
        project_root: request.project_root.clone(),
        module_paths: request.module_paths.clone(),
    };
    let loaded = load_compilation(path, &module_options).map_err(|diagnostics| {
        let source = fs::read(path)
            .map(|bytes| decode_source(&bytes))
            .unwrap_or_default();
        CompileError::from_source_diagnostics(
            CompilerPhase::Frontend,
            diagnostics,
            &source,
            path,
            None,
        )
    })?;
    let program = &loaded.root_module().program;

    let request = resolve_request(&loaded.source, request)?;
    let semantic_options = if request.profile == CodegenProfile::Modern {
        SemanticOptions::modern()
    } else {
        SemanticOptions::default()
    };
    let model =
        analyze_compilation_with_options(&loaded, semantic_options).map_err(|diagnostics| {
            CompileError::from_source_diagnostics(
                CompilerPhase::Semantic,
                diagnostics,
                &loaded.source,
                path,
                Some(&loaded.source_map),
            )
        })?;
    let semir = ir::lower_compilation(&loaded, &model);
    let uses_native_real = first_native_real_codegen_use(&model).is_some();
    let uses_lexical_blocks = !model.lexical_blocks.is_empty();
    if request.runtime == Runtime::Standalone {
        let diagnostics = standalone_resident_diagnostics(&semir);
        if !diagnostics.is_empty() {
            return Err(CompileError::from_source_diagnostics(
                CompilerPhase::Codegen,
                diagnostics,
                &loaded.source,
                path,
                Some(&loaded.source_map),
            ));
        }
    }
    let named = matches!(program.source_kind, crate::ast::SourceUnitKind::Named(_));

    let output = match request.backend {
        Backend::Classic => compile_classic(
            program,
            &semir,
            &model,
            named,
            uses_native_real,
            uses_lexical_blocks,
            request,
            &loaded.source,
            path,
            &loaded.source_map,
        )?,
        Backend::Mir6502 => compile_mir6502(
            program,
            &semir,
            request,
            &loaded.source,
            path,
            &loaded.source_map,
        )?,
    };
    let object = format_load_file(&output);

    Ok(CompiledProgram {
        object,
        output,
        expanded_source: loaded.source,
    })
}

fn first_native_real_codegen_use(
    model: &crate::semantic::SemanticModel,
) -> Option<crate::source::Span> {
    model
        .symbols
        .symbols
        .iter()
        .find_map(|symbol| {
            let uses_real = symbol
                .ty
                .as_ref()
                .is_some_and(|ty| matches!(ty.base, crate::semantic::ValueTypeBase::Real));
            (uses_real && symbol.class != crate::semantic::SymbolClass::Type).then_some(symbol.span)
        })
        .or_else(|| {
            model.expression_observations.iter().find_map(|expr| {
                expr.ty
                    .as_ref()
                    .is_some_and(|ty| matches!(ty.base, crate::semantic::ValueTypeBase::Real))
                    .then_some(expr.span)
            })
        })
}

fn compile_request_from_options(options: &CompileOptions) -> CompileRequest {
    let mut request = CompileRequest {
        origin: options.origin,
        project_root: options.project_root.clone(),
        module_paths: options.module_paths.clone(),
        runtime: options.runtime,
        runtime_explicit: options.runtime != Runtime::ActionCart,
        ..CompileRequest::default()
    };
    if let Some(mode) = options.mode {
        (request.profile, request.backend) = mode_profile_backend(mode);
        request.profile_explicit = true;
        request.backend_explicit = true;
    }
    request
}

pub(crate) fn mode_profile_backend(mode: CompileMode) -> (CodegenProfile, Backend) {
    match mode {
        CompileMode::Compatibility => (CodegenProfile::Compat, Backend::Classic),
        CompileMode::Optimized => (CodegenProfile::Modern, Backend::Classic),
        CompileMode::Mir6502 => (CodegenProfile::Modern, Backend::Mir6502),
    }
}

fn compile_classic(
    program: &crate::ast::Program,
    semir: &ir::SemProgram,
    model: &crate::semantic::SemanticModel,
    named: bool,
    uses_native_real: bool,
    uses_lexical_blocks: bool,
    request: ResolvedCompileRequest,
    source: &str,
    path: &Path,
    source_map: &crate::includes::SourceMap,
) -> Result<CodegenOutput, CompileError> {
    if request.runtime == Runtime::Standalone {
        let origin = request
            .origin
            .unwrap_or_else(|| mir6502_default_origin_from_semir(semir, CODE_ORIGIN));
        let mut output =
            generate_semir_standalone_profile_at_origin(semir, origin, request.profile).map_err(
                |diagnostics| {
                    CompileError::from_source_diagnostics(
                        CompilerPhase::Codegen,
                        diagnostics,
                        source,
                        path,
                        Some(source_map),
                    )
                },
            )?;
        output.map.runtime = request.runtime;
        return Ok(output);
    }

    let result = match request.codegen_source {
        CodegenSource::Ast if !named && !uses_native_real && !uses_lexical_blocks => {
            let materialized = materialize_constants(program, model);
            match request.origin {
                Some(origin) => generate_profile_at_origin(&materialized, origin, request.profile),
                None => generate_profile_with_origin(&materialized, CODE_ORIGIN, request.profile),
            }
        }
        CodegenSource::Ast => match request.origin {
            Some(origin) => generate_semir_profile_at_origin(semir, origin, request.profile),
            None => generate_semir_profile_with_origin(semir, CODE_ORIGIN, request.profile),
        },
        CodegenSource::SemIr => match request.origin {
            Some(origin) => generate_semir_profile_at_origin(semir, origin, request.profile),
            None => generate_semir_profile_with_origin(semir, CODE_ORIGIN, request.profile),
        },
    };
    let mut output = result.map_err(|diagnostics| {
        CompileError::from_source_diagnostics(
            CompilerPhase::Codegen,
            diagnostics,
            source,
            path,
            Some(source_map),
        )
    })?;
    output.map.runtime = request.runtime;
    Ok(output)
}

fn compile_mir6502(
    program: &crate::ast::Program,
    semir: &ir::SemProgram,
    request: ResolvedCompileRequest,
    source: &str,
    path: &Path,
    source_map: &crate::includes::SourceMap,
) -> Result<CodegenOutput, CompileError> {
    let diagnostics = legacy_routine_retargeting_diagnostics(program);
    if !diagnostics.is_empty() {
        return Err(CompileError::from_source_diagnostics(
            CompilerPhase::Nir,
            diagnostics,
            source,
            path,
            Some(source_map),
        ));
    }

    let origin = request
        .origin
        .unwrap_or_else(|| mir6502_default_origin_from_semir(semir, CODE_ORIGIN));
    let nir = nir::lower_program(semir);
    let nir = nir::optimize_program(&nir).map_err(CompileError::from_nir_diagnostics)?;
    let config = if request.profile == CodegenProfile::Modern {
        mir6502::Mir6502Config::optimized()
    } else {
        mir6502::Mir6502Config::default()
    };
    mir6502::generate_output_with_config_and_runtime(&nir, origin, &config, request.runtime)
        .map_err(CompileError::from_mir6502_diagnostics)
}

pub(crate) fn mir6502_default_origin_from_semir(program: &ir::SemProgram, fallback: u16) -> u16 {
    let mut cursor = fallback;
    let mut origin = fallback;
    for module in &program.modules {
        for item in &module.items {
            let ir::SemItem::Set(set) = item else {
                continue;
            };
            let Some(address) = sem_const_u16(&set.address) else {
                continue;
            };
            let Some(value) = sem_const_u16(&set.value) else {
                continue;
            };
            match address {
                0x000E | 0x0491 => {
                    cursor = value;
                    if value >= 0x0100 {
                        origin = value;
                    }
                }
                0x000F | 0x0492 => {
                    cursor = (cursor & 0x00FF) | ((value & 0x00FF) << 8);
                    if cursor >= 0x0100 {
                        origin = cursor;
                    }
                }
                _ => {}
            }
        }
    }
    origin
}

fn sem_const_u16(expr: &ir::SemExpr) -> Option<u16> {
    match &expr.kind {
        ir::SemExprKind::Literal(ir::SemLiteral::Number(number)) => number.value,
        ir::SemExprKind::Literal(ir::SemLiteral::Constant(value)) => Some(value.bits),
        _ => None,
    }
}

fn resolve_request(
    source: &str,
    request: &CompileRequest,
) -> Result<ResolvedCompileRequest, CompileError> {
    let mut profile = request.profile;
    let mut backend = request.backend;
    for line in source.lines() {
        let Some(annotation) = line.trim_start().strip_prefix(";@actionc") else {
            continue;
        };
        let normalized = annotation
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        match normalized.as_str() {
            "profile modern" if !request.profile_explicit => profile = CodegenProfile::Modern,
            "backend classic" if !request.backend_explicit => backend = Backend::Classic,
            "backend mir6502" if !request.backend_explicit => backend = Backend::Mir6502,
            _ => {}
        }
    }

    if profile == CodegenProfile::Compat && backend == Backend::Mir6502 {
        return Err(CompileError::configuration(
            "--backend mir6502 requires --profile modern",
        ));
    }

    Ok(ResolvedCompileRequest {
        profile,
        backend,
        runtime: request.runtime,
        codegen_source: request.codegen_source,
        origin: request.origin,
    })
}

#[cfg(test)]
mod relocation_tests {
    use std::path::PathBuf;

    use super::*;
    use crate::codegen::{CodegenOutput, CodegenRelocationKind};

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/listing/mads_reorigin_contract.act")
    }

    fn apply_origin(output: &CodegenOutput, origin: u16) -> Vec<u8> {
        let mut bytes = output.bytes.clone();
        for relocation in &output.relocations {
            let value_offset = usize::from(relocation.value_offset);
            let target = origin
                .wrapping_add(relocation.target_offset)
                .wrapping_add(relocation.addend as u16);
            match relocation.kind {
                CodegenRelocationKind::Word16 => {
                    bytes[value_offset..value_offset + 2].copy_from_slice(&target.to_le_bytes());
                }
                CodegenRelocationKind::Address8 | CodegenRelocationKind::Low8 => {
                    bytes[value_offset] = target as u8;
                }
                CodegenRelocationKind::High8 => {
                    bytes[value_offset] = (target >> 8) as u8;
                }
                CodegenRelocationKind::Relative8 => {
                    let next = relocation.value_offset.wrapping_add(1);
                    bytes[value_offset] = relocation.target_offset.wrapping_sub(next) as u8;
                }
            }
        }
        bytes
    }

    #[test]
    fn final_relocations_explain_origin_changes_in_the_listing_contract_fixture() {
        for mode in [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ] {
            for (baseline_origin, candidate_origin) in [(0x3000, 0x41c7), (0x2b40, 0x52d3)] {
                let baseline = compile_file(
                    fixture(),
                    &CompileOptions::for_mode(mode).with_origin(baseline_origin),
                )
                .unwrap_or_else(|error| {
                    panic!("compile baseline {mode:?} at ${baseline_origin:04X}: {error}")
                });
                let candidate = compile_file(
                    fixture(),
                    &CompileOptions::for_mode(mode).with_origin(candidate_origin),
                )
                .unwrap_or_else(|error| {
                    panic!("compile candidate {mode:?} at ${candidate_origin:04X}: {error}")
                });
                let relocated = apply_origin(&baseline.output, candidate.output.origin);
                let mismatches = relocated
                    .iter()
                    .zip(&candidate.output.bytes)
                    .enumerate()
                    .filter_map(|(offset, (actual, expected))| {
                        (actual != expected).then_some((offset, *actual, *expected))
                    })
                    .collect::<Vec<_>>();

                assert!(
                    mismatches.is_empty(),
                    "{mode:?} ${baseline_origin:04X}->${candidate_origin:04X} origin-dependent bytes lack relocation provenance: {mismatches:02X?}\nrelocations: {:#?}",
                    baseline.output.relocations
                );
            }
        }
    }

    #[test]
    fn semir_bridge_relocations_explain_origin_changes_in_the_listing_contract_fixture() {
        for (baseline_origin, candidate_origin) in [(0x3000, 0x41c7), (0x2b40, 0x52d3)] {
            let request = |origin| CompileRequest {
                profile: CodegenProfile::Modern,
                profile_explicit: true,
                backend: Backend::Classic,
                backend_explicit: true,
                runtime: Runtime::ActionCart,
                runtime_explicit: false,
                codegen_source: CodegenSource::SemIr,
                origin: Some(origin),
                project_root: None,
                module_paths: Vec::new(),
            };
            let baseline = compile_file_with_request(&fixture(), &request(baseline_origin))
                .unwrap_or_else(|error| {
                    panic!("compile SemIR bridge baseline at ${baseline_origin:04X}: {error}")
                });
            let candidate = compile_file_with_request(&fixture(), &request(candidate_origin))
                .unwrap_or_else(|error| {
                    panic!("compile SemIR bridge candidate at ${candidate_origin:04X}: {error}")
                });
            let relocated = apply_origin(&baseline.output, candidate.output.origin);

            assert_eq!(
                relocated, candidate.output.bytes,
                "SemIR bridge ${baseline_origin:04X}->${candidate_origin:04X} origin-dependent bytes lack relocation provenance\nrelocations: {:#?}",
                baseline.output.relocations
            );
        }
    }
}
