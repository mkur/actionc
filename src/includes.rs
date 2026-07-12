use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{Item, Module, Program};
use crate::diagnostic::Diagnostic;
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::source::{Span, decode_source};

pub struct LoadedProgram {
    pub program: Program,
    pub source: String,
    pub source_map: SourceMap,
}

#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
    segments: Vec<SourceSegment>,
}

#[derive(Debug, Clone)]
struct SourceFile {
    path: PathBuf,
    source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceSegment {
    expanded: Span,
    file_id: usize,
    original_start: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedSourceLocation {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub excerpt: String,
}

struct ExpandedSource {
    source: String,
    source_map: SourceMap,
}

impl SourceMap {
    pub fn location(&self, span: Span) -> Option<MappedSourceLocation> {
        let segment = self.segments.iter().find(|segment| {
            span.start >= segment.expanded.start && span.start < segment.expanded.end
        })?;
        let file = self.files.get(segment.file_id)?;
        let original_offset = segment
            .original_start
            .checked_add(span.start.checked_sub(segment.expanded.start)?)?;
        let (line, column, excerpt) = source_location_parts(&file.source, original_offset)?;
        Some(MappedSourceLocation {
            path: file.path.clone(),
            line,
            column,
            excerpt,
        })
    }

    fn add_file(&mut self, path: PathBuf, source: String) -> usize {
        let id = self.files.len();
        self.files.push(SourceFile { path, source });
        id
    }

    fn push_segment(&mut self, expanded: Span, file_id: usize, original_start: usize) {
        if expanded.start < expanded.end {
            self.segments.push(SourceSegment {
                expanded,
                file_id,
                original_start,
            });
        }
    }
}

pub fn load_program_with_includes(path: impl AsRef<Path>) -> Result<Program, Vec<Diagnostic>> {
    load_program_with_expanded_source(path).map(|loaded| loaded.program)
}

pub fn load_program_with_expanded_source(
    path: impl AsRef<Path>,
) -> Result<LoadedProgram, Vec<Diagnostic>> {
    let mut active = Vec::new();
    let expanded = load_expanded_source(path.as_ref(), &mut active)?;
    let tokens = tokenize(&expanded.source)?;
    let program = parse(&tokens)?;
    Ok(LoadedProgram {
        program,
        source: expanded.source,
        source_map: expanded.source_map,
    })
}

pub fn expand_includes(
    program: Program,
    base_dir: impl AsRef<Path>,
) -> Result<Program, Vec<Diagnostic>> {
    let mut active = Vec::new();
    expand_program(program, base_dir.as_ref(), &mut active)
}

fn load_file(path: &Path, active: &mut Vec<PathBuf>) -> Result<Program, Vec<Diagnostic>> {
    let resolved = resolve_case_insensitive(path).unwrap_or_else(|| path.to_path_buf());
    let key = file_key(&resolved);
    if active.contains(&key) {
        return Err(vec![Diagnostic::new(
            Span::new(0, 0),
            format!("recursive include of {}", resolved.display()),
        )]);
    }

    active.push(key);

    let result = read_parse_expand(&resolved, active);

    active.pop();
    result
}

fn load_expanded_source(
    path: &Path,
    active: &mut Vec<PathBuf>,
) -> Result<ExpandedSource, Vec<Diagnostic>> {
    let resolved = resolve_case_insensitive(path).unwrap_or_else(|| path.to_path_buf());
    let key = file_key(&resolved);
    if active.contains(&key) {
        return Err(vec![Diagnostic::new(
            Span::new(0, 0),
            format!("recursive include of {}", resolved.display()),
        )]);
    }

    active.push(key);

    let result = read_expand_source(&resolved, active);

    active.pop();
    result
}

fn read_expand_source(
    path: &Path,
    active: &mut Vec<PathBuf>,
) -> Result<ExpandedSource, Vec<Diagnostic>> {
    let source_bytes = fs::read(path).map_err(|err| {
        vec![Diagnostic::new(
            Span::new(0, 0),
            format!("failed to read {}: {err}", path.display()),
        )]
    })?;
    let source = decode_source(&source_bytes);
    let tokens = tokenize(&source)?;
    let program = parse(&tokens)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut source_map = SourceMap::default();
    let file_id = source_map.add_file(path.to_path_buf(), source.clone());
    expand_source_includes(&source, &program, base_dir, active, source_map, file_id)
}

fn expand_source_includes(
    source: &str,
    program: &Program,
    base_dir: &Path,
    active: &mut Vec<PathBuf>,
    mut source_map: SourceMap,
    file_id: usize,
) -> Result<ExpandedSource, Vec<Diagnostic>> {
    let mut includes = Vec::new();
    for module in &program.modules {
        for item in &module.items {
            if let Item::Include(include) = item {
                includes.push(include.clone());
            }
        }
    }

    if includes.is_empty() {
        let mut expanded = String::with_capacity(source.len());
        append_source_slice(
            &mut expanded,
            &mut source_map,
            file_id,
            source,
            0,
            source.len(),
        );
        return Ok(ExpandedSource {
            source: expanded,
            source_map,
        });
    }

    includes.sort_by_key(|include| include.span.start);

    let mut expanded = String::with_capacity(source.len());
    let mut cursor = 0;
    let mut diagnostics = Vec::new();

    for include in includes {
        append_source_slice(
            &mut expanded,
            &mut source_map,
            file_id,
            source,
            cursor,
            include.span.start,
        );
        let include_path = resolve_include(base_dir, &include.path);
        match load_expanded_source(&include_path, active) {
            Ok(included) => {
                append_expanded_source(&mut expanded, &mut source_map, included);
                if !expanded.ends_with('\n')
                    && source[include.span.end..]
                        .chars()
                        .next()
                        .is_some_and(|ch| !ch.is_whitespace())
                {
                    expanded.push('\n');
                }
            }
            Err(mut include_diagnostics) => {
                for diagnostic in &mut include_diagnostics {
                    if diagnostic.span == Span::new(0, 0) {
                        diagnostic.span = include.span;
                    } else {
                        diagnostic.message = format!(
                            "in included file {}: {}",
                            include_path.display(),
                            diagnostic.message
                        );
                    }
                }
                diagnostics.extend(include_diagnostics);
            }
        }
        cursor = include.span.end;
    }

    append_source_slice(
        &mut expanded,
        &mut source_map,
        file_id,
        source,
        cursor,
        source.len(),
    );

    if diagnostics.is_empty() {
        Ok(ExpandedSource {
            source: expanded,
            source_map,
        })
    } else {
        Err(diagnostics)
    }
}

fn append_source_slice(
    expanded: &mut String,
    source_map: &mut SourceMap,
    file_id: usize,
    source: &str,
    start: usize,
    end: usize,
) {
    if start >= end {
        return;
    }
    let expanded_start = expanded.len();
    expanded.push_str(&source[start..end]);
    source_map.push_segment(Span::new(expanded_start, expanded.len()), file_id, start);
}

fn append_expanded_source(
    expanded: &mut String,
    source_map: &mut SourceMap,
    included: ExpandedSource,
) {
    let expanded_base = expanded.len();
    let file_base = source_map.files.len();
    expanded.push_str(&included.source);
    source_map.files.extend(included.source_map.files);
    for segment in included.source_map.segments {
        source_map.push_segment(
            Span::new(
                expanded_base + segment.expanded.start,
                expanded_base + segment.expanded.end,
            ),
            file_base + segment.file_id,
            segment.original_start,
        );
    }
}

fn read_parse_expand(path: &Path, active: &mut Vec<PathBuf>) -> Result<Program, Vec<Diagnostic>> {
    let source_bytes = fs::read(path).map_err(|err| {
        vec![Diagnostic::new(
            Span::new(0, 0),
            format!("failed to read {}: {err}", path.display()),
        )]
    })?;
    let source = decode_source(&source_bytes);
    let tokens = tokenize(&source)?;
    let program = parse(&tokens)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    expand_program(program, base_dir, active)
}

fn expand_program(
    program: Program,
    base_dir: &Path,
    active: &mut Vec<PathBuf>,
) -> Result<Program, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut modules = Vec::new();

    for module in program.modules {
        let mut items = Vec::new();
        expand_items(module.items, base_dir, active, &mut items, &mut diagnostics);
        modules.push(Module { items });
    }

    if diagnostics.is_empty() {
        Ok(Program { modules })
    } else {
        Err(diagnostics)
    }
}

fn expand_items(
    source_items: Vec<Item>,
    base_dir: &Path,
    active: &mut Vec<PathBuf>,
    output: &mut Vec<Item>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for item in source_items {
        match item {
            Item::Include(include) => {
                let include_path = resolve_include(base_dir, &include.path);
                match load_file(&include_path, active) {
                    Ok(program) => {
                        for module in program.modules {
                            output.extend(module.items);
                        }
                    }
                    Err(mut include_diagnostics) => {
                        for diagnostic in &mut include_diagnostics {
                            if diagnostic.span == Span::new(0, 0) {
                                diagnostic.span = include.span;
                            } else {
                                diagnostic.message = format!(
                                    "in included file {}: {}",
                                    include_path.display(),
                                    diagnostic.message
                                );
                            }
                        }
                        diagnostics.extend(include_diagnostics);
                    }
                }
            }
            item => output.push(item),
        }
    }
}

fn resolve_include(base_dir: &Path, include_path: &str) -> PathBuf {
    let host_path = strip_atari_device(include_path);
    let path = Path::new(host_path);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };

    resolve_case_insensitive(&candidate).unwrap_or(candidate)
}

fn strip_atari_device(path: &str) -> &str {
    let Some((device, rest)) = path.split_once(':') else {
        return path;
    };

    if is_atari_device(device) {
        rest.trim_start_matches(['/', '\\'])
    } else {
        path
    }
}

fn is_atari_device(device: &str) -> bool {
    let mut chars = device.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }

    chars.all(|ch| ch.is_ascii_digit()) && device.len() <= 2
}

fn resolve_case_insensitive(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }

    let parent = path.parent()?;
    let name = path.file_name()?.to_str()?;
    let resolved_parent = resolve_case_insensitive(parent)?;

    for entry in fs::read_dir(resolved_parent).ok()? {
        let entry = entry.ok()?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|entry_name| entry_name.eq_ignore_ascii_case(name))
        {
            return Some(entry.path());
        }
    }

    None
}

fn file_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
    let excerpt = source[line_start..line_end].trim().to_string();
    Some((line, column, excerpt))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::ast::Item;
    use crate::semantic::analyze;

    use super::*;

    #[test]
    fn expands_atari_device_include_at_include_site() {
        let dir = temp_dir("actionc-include-site");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("LIB.ACT"), "BYTE included\n").unwrap();
        fs::write(
            dir.join("main.act"),
            "BYTE before\nINCLUDE \"D:lib.act\"\nBYTE after\n",
        )
        .unwrap();

        let program = load_program_with_includes(dir.join("main.act")).unwrap();
        let items = &program.modules[0].items;
        assert_eq!(items.len(), 3);
        assert!(!items.iter().any(|item| matches!(item, Item::Include(_))));
        analyze(&program).unwrap();
    }

    #[test]
    fn expanded_source_matches_included_item_spans() {
        let dir = temp_dir("actionc-expanded-source");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("LIB.ACT"), "BYTE included\n").unwrap();
        fs::write(
            dir.join("main.act"),
            "BYTE before\nINCLUDE \"D:lib.act\"\nBYTE after\n",
        )
        .unwrap();

        let loaded = load_program_with_expanded_source(dir.join("main.act")).unwrap();
        let items = &loaded.program.modules[0].items;
        assert_eq!(items.len(), 3);
        assert!(!items.iter().any(|item| matches!(item, Item::Include(_))));

        let Item::Declaration(crate::ast::Decl::Var(var)) = &items[1] else {
            panic!("expected included declaration");
        };
        assert_eq!(
            &loaded.source[var.span.start..var.span.end],
            "BYTE included"
        );

        let location = loaded.source_map.location(var.span).unwrap();
        assert!(
            location
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .eq_ignore_ascii_case("LIB.ACT")
        );
        assert_eq!(location.line, 1);
        assert_eq!(location.column, 1);
        assert_eq!(location.excerpt, "BYTE included");
    }

    #[test]
    fn reports_recursive_includes() {
        let dir = temp_dir("actionc-include-cycle");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.act"), "INCLUDE \"b.act\"\n").unwrap();
        fs::write(dir.join("b.act"), "INCLUDE \"a.act\"\n").unwrap();

        let diagnostics = load_program_with_includes(dir.join("a.act")).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("recursive include"))
        );
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{suffix}"))
    }
}
