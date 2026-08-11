pub(crate) mod artifacts;
mod diagnostics;
pub(crate) mod validation;

use std::fs;
use std::path::Path;

use crate::codegen::{
    CODE_ORIGIN, CodegenOutput, CodegenProfile, format_load_file, generate_profile_at_origin,
    generate_profile_with_origin, generate_semir_native_profile_with_origin,
    generate_semir_profile_at_origin, generate_semir_profile_with_origin,
};
use crate::includes::load_program_with_expanded_source;
use crate::mir6502;
use crate::nir;
use crate::semantic::{analyze, ir};
use crate::source::decode_source;

use self::validation::legacy_routine_retargeting_diagnostics;

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
    SemIrNative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompileRequest {
    pub(crate) profile: CodegenProfile,
    pub(crate) profile_explicit: bool,
    pub(crate) backend: Backend,
    pub(crate) backend_explicit: bool,
    pub(crate) codegen_source: CodegenSource,
    pub(crate) origin: Option<u16>,
}

impl Default for CompileRequest {
    fn default() -> Self {
        Self {
            profile: CodegenProfile::Compat,
            profile_explicit: false,
            backend: Backend::Classic,
            backend_explicit: false,
            codegen_source: CodegenSource::Ast,
            origin: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedCompileRequest {
    profile: CodegenProfile,
    backend: Backend,
    codegen_source: CodegenSource,
    origin: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompileOptions {
    mode: Option<CompileMode>,
    origin: Option<u16>,
}

impl CompileOptions {
    pub fn for_mode(mode: CompileMode) -> Self {
        Self {
            mode: Some(mode),
            origin: None,
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
    let loaded = load_program_with_expanded_source(path).map_err(|diagnostics| {
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

    let request = resolve_request(&loaded.source, request)?;
    let model = analyze(&loaded.program).map_err(|diagnostics| {
        CompileError::from_source_diagnostics(
            CompilerPhase::Semantic,
            diagnostics,
            &loaded.source,
            path,
            Some(&loaded.source_map),
        )
    })?;

    let output = match request.backend {
        Backend::Classic => compile_classic(
            &loaded.program,
            &model,
            request,
            &loaded.source,
            path,
            &loaded.source_map,
        )?,
        Backend::Mir6502 => compile_mir6502(
            &loaded.program,
            &model,
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

fn compile_request_from_options(options: &CompileOptions) -> CompileRequest {
    let mut request = CompileRequest {
        origin: options.origin,
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
    model: &crate::semantic::SemanticModel,
    request: ResolvedCompileRequest,
    source: &str,
    path: &Path,
    source_map: &crate::includes::SourceMap,
) -> Result<CodegenOutput, CompileError> {
    let result = match request.codegen_source {
        CodegenSource::Ast => match request.origin {
            Some(origin) => generate_profile_at_origin(program, origin, request.profile),
            None => generate_profile_with_origin(program, CODE_ORIGIN, request.profile),
        },
        CodegenSource::SemIr => {
            let semir = ir::lower_program(program, model);
            match request.origin {
                Some(origin) => generate_semir_profile_at_origin(&semir, origin, request.profile),
                None => generate_semir_profile_with_origin(&semir, CODE_ORIGIN, request.profile),
            }
        }
        CodegenSource::SemIrNative => {
            let semir = ir::lower_program(program, model);
            generate_semir_native_profile_with_origin(
                &semir,
                request.origin.unwrap_or(CODE_ORIGIN),
                request.profile,
            )
        }
    };
    result.map_err(|diagnostics| {
        CompileError::from_source_diagnostics(
            CompilerPhase::Codegen,
            diagnostics,
            source,
            path,
            Some(source_map),
        )
    })
}

fn compile_mir6502(
    program: &crate::ast::Program,
    model: &crate::semantic::SemanticModel,
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

    let semir = ir::lower_program(program, model);
    let origin = request
        .origin
        .unwrap_or_else(|| mir6502_default_origin_from_semir(&semir, CODE_ORIGIN));
    let nir = nir::lower_program(&semir);
    let nir = nir::optimize_program(&nir).map_err(CompileError::from_nir_diagnostics)?;
    let config = if request.profile == CodegenProfile::Modern {
        mir6502::Mir6502Config::optimized()
    } else {
        mir6502::Mir6502Config::default()
    };
    mir6502::generate_output_with_config(&nir, origin, &config)
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
        codegen_source: request.codegen_source,
        origin: request.origin,
    })
}
