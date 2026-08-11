use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::diagnostic::Diagnostic;
use crate::includes::SourceMap;
use crate::mir6502::MirDiagnostic;
use crate::nir::NirDiagnostic;
use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileErrorKind {
    Configuration,
    Compilation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilerPhase {
    Configuration,
    Input,
    Frontend,
    Semantic,
    Nir,
    Mir6502,
    Codegen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerDiagnostic {
    pub phase: CompilerPhase,
    pub message: String,
    pub site: DiagnosticSite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticSite {
    Source {
        path: PathBuf,
        line: usize,
        column: usize,
        byte_range: Option<Range<usize>>,
        excerpt: Option<String>,
    },
    File {
        path: PathBuf,
    },
    Ir {
        routine: Option<String>,
        block: Option<String>,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    kind: CompileErrorKind,
    diagnostics: Vec<CompilerDiagnostic>,
}

impl CompileError {
    pub fn kind(&self) -> CompileErrorKind {
        self.kind
    }

    pub fn diagnostics(&self) -> &[CompilerDiagnostic] {
        &self.diagnostics
    }

    pub(super) fn configuration(message: impl Into<String>) -> Self {
        Self {
            kind: CompileErrorKind::Configuration,
            diagnostics: vec![CompilerDiagnostic {
                phase: CompilerPhase::Configuration,
                message: message.into(),
                site: DiagnosticSite::Unknown,
            }],
        }
    }

    pub(super) fn from_source_diagnostics(
        phase: CompilerPhase,
        diagnostics: Vec<Diagnostic>,
        source: &str,
        fallback_path: &Path,
        source_map: Option<&SourceMap>,
    ) -> Self {
        let diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| CompilerDiagnostic {
                phase,
                message: diagnostic.message,
                site: diagnostic_site(diagnostic.span, source, fallback_path, source_map),
            })
            .collect();
        Self {
            kind: CompileErrorKind::Compilation,
            diagnostics,
        }
    }

    pub(super) fn from_nir_diagnostics(diagnostics: Vec<NirDiagnostic>) -> Self {
        Self::from_ir_diagnostics(
            CompilerPhase::Nir,
            diagnostics
                .into_iter()
                .map(|diagnostic| (diagnostic.routine, diagnostic.block, diagnostic.message)),
        )
    }

    pub(super) fn from_mir6502_diagnostics(diagnostics: Vec<MirDiagnostic>) -> Self {
        Self::from_ir_diagnostics(
            CompilerPhase::Mir6502,
            diagnostics
                .into_iter()
                .map(|diagnostic| (diagnostic.routine, diagnostic.block, diagnostic.message)),
        )
    }

    fn from_ir_diagnostics(
        phase: CompilerPhase,
        diagnostics: impl IntoIterator<Item = (Option<String>, Option<String>, String)>,
    ) -> Self {
        Self {
            kind: CompileErrorKind::Compilation,
            diagnostics: diagnostics
                .into_iter()
                .map(|(routine, block, message)| CompilerDiagnostic {
                    phase,
                    message,
                    site: DiagnosticSite::Ir { routine, block },
                })
                .collect(),
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(first) = self.diagnostics.first() else {
            return formatter.write_str("compilation failed");
        };
        formatter.write_str(&first.message)?;
        if self.diagnostics.len() > 1 {
            write!(
                formatter,
                " (and {} more diagnostic{})",
                self.diagnostics.len() - 1,
                if self.diagnostics.len() == 2 { "" } else { "s" }
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}

fn diagnostic_site(
    span: Span,
    source: &str,
    fallback_path: &Path,
    source_map: Option<&SourceMap>,
) -> DiagnosticSite {
    if let Some(location) = source_map.and_then(|source_map| source_map.location(span)) {
        return DiagnosticSite::Source {
            path: location.path,
            line: location.line,
            column: location.column,
            byte_range: Some(span.start..span.end),
            excerpt: Some(location.excerpt),
        };
    }
    let Some((line, column, excerpt)) = source_location_parts(source, span.start) else {
        return DiagnosticSite::File {
            path: fallback_path.to_path_buf(),
        };
    };
    DiagnosticSite::Source {
        path: fallback_path.to_path_buf(),
        line,
        column,
        byte_range: Some(span.start..span.end),
        excerpt: Some(excerpt),
    }
}

fn source_location_parts(source: &str, offset: usize) -> Option<(usize, usize, String)> {
    if offset > source.len() {
        return None;
    }
    let mut line = 1usize;
    let mut column = 1usize;
    for (current, ch) in source.char_indices() {
        if current >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    let line_start = source[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[offset..]
        .find('\n')
        .map(|index| offset + index)
        .unwrap_or(source.len());
    Some((
        line,
        column,
        source[line_start..line_end].trim().to_string(),
    ))
}
