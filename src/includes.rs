use std::path::Path;

use crate::ast::{Item, Module, Program};
use crate::diagnostic::Diagnostic;
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::source::{HostSourceProvider, SourceId, SourceOrigin, SourceProvider, SourceText, Span};

pub struct LoadedProgram {
    pub program: Program,
    pub source: String,
    pub source_map: SourceMap,
    pub root_source_id: SourceId,
}

#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
    segments: Vec<SourceSegment>,
}

#[derive(Debug, Clone)]
struct SourceFile {
    id: SourceId,
    origin: SourceOrigin,
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
    pub source_id: SourceId,
    pub origin: SourceOrigin,
    pub line: usize,
    pub column: usize,
    pub excerpt: String,
}

struct ExpandedSource {
    source: String,
    source_map: SourceMap,
    root_source_id: SourceId,
}

struct SourceLoader<'a> {
    provider: &'a dyn SourceProvider,
    next_source_id: u32,
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
            source_id: file.id,
            origin: file.origin.clone(),
            line,
            column,
            excerpt,
        })
    }

    pub fn source_origin(&self, source_id: SourceId) -> Option<&SourceOrigin> {
        self.files
            .iter()
            .find(|file| file.id == source_id)
            .map(|file| &file.origin)
    }

    fn add_file(&mut self, source_text: &SourceText, source: String) -> usize {
        let id = self.files.len();
        self.files.push(SourceFile {
            id: source_text.id,
            origin: source_text.origin.clone(),
            source,
        });
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
    let provider = HostSourceProvider;
    load_program_with_expanded_source_from_provider(
        SourceOrigin::host(path.as_ref().to_path_buf()),
        &provider,
    )
}

pub fn load_program_with_expanded_source_from_provider(
    origin: SourceOrigin,
    provider: &dyn SourceProvider,
) -> Result<LoadedProgram, Vec<Diagnostic>> {
    let mut loader = SourceLoader::new(provider);
    let mut active = Vec::new();
    let expanded = loader.load_expanded_source(origin, &mut active)?;
    let tokens = tokenize(&expanded.source)?;
    let program = parse(&tokens)?;
    Ok(LoadedProgram {
        program,
        source: expanded.source,
        source_map: expanded.source_map,
        root_source_id: expanded.root_source_id,
    })
}

pub fn expand_includes(
    program: Program,
    base_dir: impl AsRef<Path>,
) -> Result<Program, Vec<Diagnostic>> {
    let provider = HostSourceProvider;
    let mut loader = SourceLoader::new(&provider);
    let mut active = Vec::new();
    let root_origin = SourceOrigin::host(base_dir.as_ref().join(".actionc-include-root"));
    loader.expand_program(program, &root_origin, &mut active)
}

impl<'a> SourceLoader<'a> {
    fn new(provider: &'a dyn SourceProvider) -> Self {
        Self {
            provider,
            next_source_id: 0,
        }
    }

    fn read_source(&mut self, origin: SourceOrigin) -> Result<SourceText, Vec<Diagnostic>> {
        let bytes = self.provider.read(&origin).map_err(|error| {
            vec![Diagnostic::new(
                Span::new(0, 0),
                format!("failed to read {origin}: {error}"),
            )]
        })?;
        let id = SourceId(self.next_source_id);
        self.next_source_id = self
            .next_source_id
            .checked_add(1)
            .expect("one compilation cannot contain more than u32::MAX source objects");
        Ok(SourceText { id, origin, bytes })
    }

    fn load_file(
        &mut self,
        origin: SourceOrigin,
        active: &mut Vec<SourceOrigin>,
    ) -> Result<Program, Vec<Diagnostic>> {
        let resolved = self.provider.resolve(&origin);
        let key = self.provider.canonical_key(&resolved);
        if active.contains(&key) {
            return Err(vec![Diagnostic::new(
                Span::new(0, 0),
                format!("recursive include of {resolved}"),
            )]);
        }

        active.push(key);
        let result = self.read_parse_expand(resolved, active);
        active.pop();
        result
    }

    fn load_expanded_source(
        &mut self,
        origin: SourceOrigin,
        active: &mut Vec<SourceOrigin>,
    ) -> Result<ExpandedSource, Vec<Diagnostic>> {
        let resolved = self.provider.resolve(&origin);
        let key = self.provider.canonical_key(&resolved);
        if active.contains(&key) {
            return Err(vec![Diagnostic::new(
                Span::new(0, 0),
                format!("recursive include of {resolved}"),
            )]);
        }

        active.push(key);
        let result = self.read_expand_source(resolved, active);
        active.pop();
        result
    }

    fn read_expand_source(
        &mut self,
        origin: SourceOrigin,
        active: &mut Vec<SourceOrigin>,
    ) -> Result<ExpandedSource, Vec<Diagnostic>> {
        let source_text = self.read_source(origin)?;
        let source = source_text.decode();
        let tokens = tokenize(&source)?;
        let program = parse(&tokens)?;
        let mut source_map = SourceMap::default();
        let file_id = source_map.add_file(&source_text, source.clone());
        self.expand_source_includes(&source_text, &source, &program, active, source_map, file_id)
    }

    fn expand_source_includes(
        &mut self,
        source_text: &SourceText,
        source: &str,
        program: &Program,
        active: &mut Vec<SourceOrigin>,
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
                root_source_id: source_text.id,
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
            let include_origin = match include_origin(&source_text.origin, &include.path) {
                Ok(origin) => origin,
                Err(message) => {
                    diagnostics.push(Diagnostic::new(include.span, message));
                    cursor = include.span.end;
                    continue;
                }
            };
            match self.load_expanded_source(include_origin.clone(), active) {
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
                                "in included file {include_origin}: {}",
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
                root_source_id: source_text.id,
            })
        } else {
            Err(diagnostics)
        }
    }

    fn read_parse_expand(
        &mut self,
        origin: SourceOrigin,
        active: &mut Vec<SourceOrigin>,
    ) -> Result<Program, Vec<Diagnostic>> {
        let source_text = self.read_source(origin)?;
        let source = source_text.decode();
        let tokens = tokenize(&source)?;
        let program = parse(&tokens)?;
        self.expand_program(program, &source_text.origin, active)
    }

    fn expand_program(
        &mut self,
        program: Program,
        owner: &SourceOrigin,
        active: &mut Vec<SourceOrigin>,
    ) -> Result<Program, Vec<Diagnostic>> {
        let mut diagnostics = Vec::new();
        let mut modules = Vec::new();

        for module in program.modules {
            let mut items = Vec::new();
            self.expand_items(module.items, owner, active, &mut items, &mut diagnostics);
            modules.push(Module { items });
        }

        if diagnostics.is_empty() {
            Ok(Program { modules })
        } else {
            Err(diagnostics)
        }
    }

    fn expand_items(
        &mut self,
        source_items: Vec<Item>,
        owner: &SourceOrigin,
        active: &mut Vec<SourceOrigin>,
        output: &mut Vec<Item>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for item in source_items {
            match item {
                Item::Include(include) => {
                    let include_origin = match include_origin(owner, &include.path) {
                        Ok(origin) => origin,
                        Err(message) => {
                            diagnostics.push(Diagnostic::new(include.span, message));
                            continue;
                        }
                    };
                    match self.load_file(include_origin.clone(), active) {
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
                                        "in included file {include_origin}: {}",
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

fn include_origin(owner: &SourceOrigin, include_path: &str) -> Result<SourceOrigin, String> {
    let Some(owner_path) = owner.host_path() else {
        return Err(format!(
            "host INCLUDE `{include_path}` cannot be resolved from {owner}"
        ));
    };
    let base_dir = owner_path.parent().unwrap_or_else(|| Path::new("."));
    let host_path = strip_atari_device(include_path);
    let path = Path::new(host_path);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };
    Ok(SourceOrigin::host(candidate))
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
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::ast::Item;
    use crate::semantic::analyze;
    use crate::source::InMemorySourceProvider;

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
                .origin
                .host_path()
                .unwrap()
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
    fn host_and_in_memory_providers_produce_equivalent_programs() {
        let dir = temp_dir("actionc-provider-equivalence");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("main.act"),
            "BYTE before\nINCLUDE \"lib.act\"\nBYTE after\n",
        )
        .unwrap();
        fs::write(dir.join("lib.act"), "BYTE included\n").unwrap();
        let host = load_program_with_expanded_source(dir.join("main.act")).unwrap();

        let root_origin = SourceOrigin::host(PathBuf::from("virtual-project/main.act"));
        let include_origin = SourceOrigin::host(PathBuf::from("virtual-project/lib.act"));
        let provider = InMemorySourceProvider::default()
            .with_source(
                root_origin.clone(),
                b"BYTE before\nINCLUDE \"lib.act\"\nBYTE after\n".to_vec(),
            )
            .with_source(include_origin.clone(), b"BYTE included\n".to_vec());
        let memory =
            load_program_with_expanded_source_from_provider(root_origin.clone(), &provider)
                .unwrap();

        assert_eq!(memory.program, host.program);
        assert_eq!(memory.source, host.source);
        assert_eq!(
            memory.source_map.source_origin(memory.root_source_id),
            Some(&root_origin)
        );

        let root_item = &memory.program.modules[0].items[0];
        let included_item = &memory.program.modules[0].items[1];
        let Item::Declaration(crate::ast::Decl::Var(root_var)) = root_item else {
            panic!("expected root declaration");
        };
        let Item::Declaration(crate::ast::Decl::Var(included_var)) = included_item else {
            panic!("expected included declaration");
        };
        let root_location = memory.source_map.location(root_var.span).unwrap();
        let included_location = memory.source_map.location(included_var.span).unwrap();
        assert_eq!(root_location.origin, root_origin);
        assert_eq!(included_location.origin, include_origin);
        assert_eq!(root_location.source_id, memory.root_source_id);
        assert_ne!(root_location.source_id, included_location.source_id);
    }

    #[test]
    fn embedded_source_origin_survives_mapping() {
        let origin = SourceOrigin::embedded("modules/test.act", "<embedded:TEST>");
        let provider =
            InMemorySourceProvider::default().with_source(origin.clone(), b"BYTE value\n".to_vec());

        let loaded =
            load_program_with_expanded_source_from_provider(origin.clone(), &provider).unwrap();
        let Item::Declaration(crate::ast::Decl::Var(var)) = &loaded.program.modules[0].items[0]
        else {
            panic!("expected embedded declaration");
        };
        let location = loaded.source_map.location(var.span).unwrap();

        assert_eq!(location.source_id, loaded.root_source_id);
        assert_eq!(location.origin, origin);
        assert_eq!(location.origin.to_string(), "<embedded:TEST>");
        assert_eq!(location.excerpt, "BYTE value");
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
