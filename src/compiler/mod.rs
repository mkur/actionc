mod diagnostics;
pub(crate) mod validation;

use std::fs;
use std::path::Path;

use crate::codegen::{
    CODE_ORIGIN, CodegenOutput, CodegenProfile, format_load_file, generate_profile_at_origin,
    generate_profile_with_origin,
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
    let path = path.as_ref();
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

    let mode = resolve_mode(&loaded.source, options)?;
    let model = analyze(&loaded.program).map_err(|diagnostics| {
        CompileError::from_source_diagnostics(
            CompilerPhase::Semantic,
            diagnostics,
            &loaded.source,
            path,
            Some(&loaded.source_map),
        )
    })?;

    let output = match mode {
        CompileMode::Compatibility => compile_classic(
            &loaded.program,
            CodegenProfile::Compat,
            options.origin,
            &loaded.source,
            path,
            &loaded.source_map,
        )?,
        CompileMode::Optimized => compile_classic(
            &loaded.program,
            CodegenProfile::Modern,
            options.origin,
            &loaded.source,
            path,
            &loaded.source_map,
        )?,
        CompileMode::Mir6502 => {
            let diagnostics = legacy_routine_retargeting_diagnostics(&loaded.program);
            if !diagnostics.is_empty() {
                return Err(CompileError::from_source_diagnostics(
                    CompilerPhase::Nir,
                    diagnostics,
                    &loaded.source,
                    path,
                    Some(&loaded.source_map),
                ));
            }

            let semir = ir::lower_program(&loaded.program, &model);
            let origin = options
                .origin
                .unwrap_or_else(|| mir6502_default_origin_from_semir(&semir, CODE_ORIGIN));
            let nir = nir::lower_program(&semir);
            let nir = nir::optimize_program(&nir).map_err(CompileError::from_nir_diagnostics)?;
            mir6502::generate_output_with_config(&nir, origin, &mir6502::Mir6502Config::optimized())
                .map_err(CompileError::from_mir6502_diagnostics)?
        }
    };
    let object = format_load_file(&output);

    Ok(CompiledProgram {
        object,
        output,
        expanded_source: loaded.source,
    })
}

fn compile_classic(
    program: &crate::ast::Program,
    profile: CodegenProfile,
    origin: Option<u16>,
    source: &str,
    path: &Path,
    source_map: &crate::includes::SourceMap,
) -> Result<CodegenOutput, CompileError> {
    let result = match origin {
        Some(origin) => generate_profile_at_origin(program, origin, profile),
        None => generate_profile_with_origin(program, CODE_ORIGIN, profile),
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

fn resolve_mode(source: &str, options: &CompileOptions) -> Result<CompileMode, CompileError> {
    if let Some(mode) = options.mode {
        return Ok(mode);
    }

    let mut modern = false;
    let mut mir6502 = false;
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
            "profile modern" => modern = true,
            "backend classic" => mir6502 = false,
            "backend mir6502" => mir6502 = true,
            _ => {}
        }
    }

    match (modern, mir6502) {
        (false, false) => Ok(CompileMode::Compatibility),
        (true, false) => Ok(CompileMode::Optimized),
        (true, true) => Ok(CompileMode::Mir6502),
        (false, true) => Err(CompileError::configuration(
            "--backend mir6502 requires --profile modern",
        )),
    }
}
