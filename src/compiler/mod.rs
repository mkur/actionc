mod diagnostics;

use std::fs;
use std::path::Path;

use crate::codegen::{
    CODE_ORIGIN, CodegenOutput, CodegenProfile, format_load_file, generate_profile_with_origin,
};
use crate::includes::load_program_with_expanded_source;
use crate::semantic::analyze;
use crate::source::decode_source;

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
    if mode != CompileMode::Compatibility {
        return Err(CompileError::configuration(format!(
            "compiler API mode {mode:?} is not implemented yet"
        )));
    }

    analyze(&loaded.program).map_err(|diagnostics| {
        CompileError::from_source_diagnostics(
            CompilerPhase::Semantic,
            diagnostics,
            &loaded.source,
            path,
            Some(&loaded.source_map),
        )
    })?;

    let origin = options.origin.unwrap_or(CODE_ORIGIN);
    let output = generate_profile_with_origin(&loaded.program, origin, CodegenProfile::Compat)
        .map_err(|diagnostics| {
            CompileError::from_source_diagnostics(
                CompilerPhase::Codegen,
                diagnostics,
                &loaded.source,
                path,
                Some(&loaded.source_map),
            )
        })?;
    let object = format_load_file(&output);

    Ok(CompiledProgram {
        object,
        output,
        expanded_source: loaded.source,
    })
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
