use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::ast::{Item, Module, ModulePath, Program, SourceUnitKind, UseDecl};
use crate::diagnostic::Diagnostic;
use crate::lexer::tokenize;
use crate::parser::parse;
#[cfg(test)]
use crate::source::HostSourceProvider;
use crate::source::{
    CompilerSourceProvider, SourceId, SourceOrigin, SourceProvider, SourceText, Span,
};

#[derive(Debug, Clone)]
pub struct LoadedProgram {
    pub program: Program,
    pub source: String,
    pub source_map: SourceMap,
    pub root_source_id: SourceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(pub u32);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleLoadOptions {
    pub project_root: Option<PathBuf>,
    pub module_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct LoadedModule {
    pub id: ModuleId,
    pub declared_path: Option<ModulePath>,
    pub program: Program,
    pub origin: SourceOrigin,
    pub root_source_id: SourceId,
    pub source_span: Span,
    pub dependencies: Vec<ModuleId>,
}

#[derive(Debug, Clone)]
pub struct LoadedCompilation {
    pub root: ModuleId,
    pub modules: Vec<LoadedModule>,
    /// Dependencies precede using modules; each module occurs exactly once.
    pub graph_order: Vec<ModuleId>,
    pub source: String,
    pub source_map: SourceMap,
}

impl LoadedCompilation {
    pub fn root_module(&self) -> &LoadedModule {
        &self.modules[self.root.0 as usize]
    }
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

struct ParsedUnit {
    program: Program,
    origin: SourceOrigin,
    root_source_id: SourceId,
    source_span: Span,
}

struct CompilationLoader<'a> {
    source_loader: SourceLoader<'a>,
    root_origin: SourceOrigin,
    search_roots: Vec<SourceOrigin>,
    source: String,
    source_map: SourceMap,
    modules: Vec<LoadedModule>,
    modules_by_path: HashMap<String, ModuleId>,
    active: Vec<(String, String)>,
    graph_order: Vec<ModuleId>,
    implicit_sys: bool,
    allow_host_named_modules: bool,
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

    fn append_shifted(&mut self, other: SourceMap, offset: usize) {
        let file_base = self.files.len();
        self.files.extend(other.files);
        self.segments
            .extend(other.segments.into_iter().map(|segment| SourceSegment {
                expanded: Span::new(
                    segment.expanded.start + offset,
                    segment.expanded.end + offset,
                ),
                file_id: segment.file_id + file_base,
                original_start: segment.original_start,
            }));
    }
}

pub fn load_program_with_includes(path: impl AsRef<Path>) -> Result<Program, Vec<Diagnostic>> {
    load_program_with_expanded_source(path).map(|loaded| loaded.program)
}

pub fn load_program_with_expanded_source(
    path: impl AsRef<Path>,
) -> Result<LoadedProgram, Vec<Diagnostic>> {
    let provider = CompilerSourceProvider::default();
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
    let expanded = loader.load_expanded_source(origin, &mut active, true)?;
    let tokens = tokenize(&expanded.source)?;
    let program = parse(&tokens)?;
    Ok(LoadedProgram {
        program,
        source: expanded.source,
        source_map: expanded.source_map,
        root_source_id: expanded.root_source_id,
    })
}

pub fn load_compilation(
    path: impl AsRef<Path>,
    options: &ModuleLoadOptions,
) -> Result<LoadedCompilation, Vec<Diagnostic>> {
    let allow_host_named_modules = cfg!(feature = "experimental-named-modules");
    if !allow_host_named_modules
        && (options.project_root.is_some() || !options.module_paths.is_empty())
    {
        return Err(vec![Diagnostic::new(
            Span::new(0, 0),
            "named modules are disabled in this build; rebuild with the experimental-named-modules feature",
        )]);
    }
    let provider = CompilerSourceProvider::default();
    CompilationLoader::new(
        &provider,
        SourceOrigin::host(path.as_ref().to_path_buf()),
        options,
        allow_host_named_modules,
    )
    .with_implicit_sys()
    .load()
}

pub fn load_compilation_from_provider(
    root_origin: SourceOrigin,
    provider: &dyn SourceProvider,
    options: &ModuleLoadOptions,
) -> Result<LoadedCompilation, Vec<Diagnostic>> {
    CompilationLoader::new(provider, root_origin, options, true).load()
}

pub fn expand_includes(
    program: Program,
    base_dir: impl AsRef<Path>,
) -> Result<Program, Vec<Diagnostic>> {
    let provider = CompilerSourceProvider::default();
    let mut loader = SourceLoader::new(&provider);
    let mut active = Vec::new();
    let root_origin = SourceOrigin::host(base_dir.as_ref().join(".actionc-include-root"));
    loader.expand_program(program, &root_origin, &mut active)
}

impl<'a> CompilationLoader<'a> {
    fn new(
        provider: &'a dyn SourceProvider,
        root_origin: SourceOrigin,
        options: &ModuleLoadOptions,
        allow_host_named_modules: bool,
    ) -> Self {
        let project_root = options
            .project_root
            .clone()
            .or_else(|| {
                root_origin
                    .host_path()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
            })
            .unwrap_or_else(|| PathBuf::from("."));
        let search_roots = coalesce_search_roots(
            provider,
            std::iter::once(SourceOrigin::host(project_root))
                .chain(options.module_paths.iter().cloned().map(SourceOrigin::host)),
        );
        Self {
            source_loader: SourceLoader::new(provider),
            root_origin,
            search_roots,
            source: String::new(),
            source_map: SourceMap::default(),
            modules: Vec::new(),
            modules_by_path: HashMap::new(),
            active: Vec::new(),
            graph_order: Vec::new(),
            implicit_sys: false,
            allow_host_named_modules,
        }
    }

    fn with_implicit_sys(mut self) -> Self {
        self.implicit_sys = true;
        self
    }

    fn load(mut self) -> Result<LoadedCompilation, Vec<Diagnostic>> {
        let root_origin = self.source_loader.provider.resolve(&self.root_origin);
        let root = self.load_unit(root_origin)?;
        let root_id = ModuleId(0);
        let root_path = named_path(&root.program).cloned();
        if root_path.is_some() && !self.allow_host_named_modules {
            return Err(vec![Diagnostic::new(
                root.source_span,
                "named modules are disabled in this build; rebuild with the experimental-named-modules feature",
            )]);
        }
        self.modules.push(LoadedModule {
            id: root_id,
            declared_path: root_path.clone(),
            program: root.program,
            origin: root.origin,
            root_source_id: root.root_source_id,
            source_span: root.source_span,
            dependencies: Vec::new(),
        });

        if let Some(path) = root_path {
            let key = path.canonical_name();
            self.modules_by_path.insert(key.clone(), root_id);
            self.active.push((key, path.display_name()));
            self.load_implicit_sys(root_id)?;
            self.load_dependencies(root_id)?;
            self.active.pop();
        } else {
            self.load_implicit_sys(root_id)?;
        }
        self.graph_order.push(root_id);

        Ok(LoadedCompilation {
            root: root_id,
            modules: self.modules,
            graph_order: self.graph_order,
            source: self.source,
            source_map: self.source_map,
        })
    }

    fn load_implicit_sys(&mut self, using_module: ModuleId) -> Result<(), Vec<Diagnostic>> {
        if !self.implicit_sys
            || self.modules[using_module.0 as usize]
                .declared_path
                .as_ref()
                .is_some_and(|path| path.canonical_name() == "sys")
        {
            return Ok(());
        }

        let span = self.modules[using_module.0 as usize].source_span;
        let dependency = self.load_use(&UseDecl {
            path: ModulePath::new(vec!["SYS".to_string()], span),
            alias: None,
            all: false,
            span,
        })?;
        self.modules[using_module.0 as usize]
            .dependencies
            .push(dependency);
        Ok(())
    }

    fn load_dependencies(&mut self, using_module: ModuleId) -> Result<(), Vec<Diagnostic>> {
        let uses = named_uses(&self.modules[using_module.0 as usize].program).to_vec();
        for use_decl in uses {
            let dependency = self.load_use(&use_decl)?;
            let dependencies = &mut self.modules[using_module.0 as usize].dependencies;
            if !dependencies.contains(&dependency) {
                dependencies.push(dependency);
            }
        }
        Ok(())
    }

    fn load_use(&mut self, use_decl: &UseDecl) -> Result<ModuleId, Vec<Diagnostic>> {
        let key = use_decl.path.canonical_name();
        if let Some(cycle_start) = self.active.iter().position(|(active, _)| active == &key) {
            let mut chain = self.active[cycle_start..]
                .iter()
                .map(|(_, display)| display.clone())
                .collect::<Vec<_>>();
            chain.push(use_decl.path.display_name());
            return Err(vec![Diagnostic::new(
                use_decl.span,
                format!("module dependency cycle: {}", chain.join(" -> ")),
            )]);
        }
        if let Some(id) = self.modules_by_path.get(&key) {
            return Ok(*id);
        }

        let origin = self.resolve_use(use_decl)?;
        let unit = self.load_unit(origin)?;
        let SourceUnitKind::Named(declaration) = &unit.program.source_kind else {
            return Err(vec![Diagnostic::new(
                use_decl.span,
                format!(
                    "module `{}` resolved to a legacy source file; files named by USE must declare a named module",
                    use_decl.path.display_name()
                ),
            )]);
        };
        if declaration.path.canonical_components != use_decl.path.canonical_components {
            return Err(vec![Diagnostic::new(
                declaration.path.span,
                format!(
                    "requested module `{}` but the file declares `{}`",
                    use_decl.path.display_name(),
                    declaration.path.display_name()
                ),
            )]);
        }

        let id = ModuleId(
            u32::try_from(self.modules.len())
                .expect("one compilation cannot contain more than u32::MAX modules"),
        );
        let declared_path = declaration.path.clone();
        self.modules.push(LoadedModule {
            id,
            declared_path: Some(declared_path.clone()),
            program: unit.program,
            origin: unit.origin,
            root_source_id: unit.root_source_id,
            source_span: unit.source_span,
            dependencies: Vec::new(),
        });
        self.modules_by_path.insert(key.clone(), id);
        self.active.push((key, declared_path.display_name()));
        self.load_dependencies(id)?;
        self.active.pop();
        self.graph_order.push(id);
        Ok(id)
    }

    fn resolve_use(&self, use_decl: &UseDecl) -> Result<SourceOrigin, Vec<Diagnostic>> {
        let canonical = &use_decl.path.canonical_components;
        match self
            .source_loader
            .provider
            .resolve_embedded_module(canonical)
        {
            Ok(Some(origin)) => return Ok(origin),
            Ok(None) => {}
            Err(error) => return Err(vec![Diagnostic::new(use_decl.span, error.to_string())]),
        }

        let reserved = canonical
            .first()
            .is_some_and(|root| matches!(root.as_str(), "sys" | "atari"));
        if reserved {
            return Err(vec![Diagnostic::new(
                use_decl.span,
                format!(
                    "reserved embedded module `{}` is not available",
                    use_decl.path.display_name()
                ),
            )]);
        }

        match self
            .source_loader
            .provider
            .resolve_module(canonical, &self.search_roots)
        {
            Ok(Some(origin)) => Ok(origin),
            Ok(None) => Err(vec![Diagnostic::new(
                use_decl.span,
                format!(
                    "cannot find module `{}` in the project root or configured module paths",
                    use_decl.path.display_name()
                ),
            )]),
            Err(error) => Err(vec![Diagnostic::new(use_decl.span, error.to_string())]),
        }
    }

    fn load_unit(&mut self, origin: SourceOrigin) -> Result<ParsedUnit, Vec<Diagnostic>> {
        let resolved = self.source_loader.provider.resolve(&origin);
        let mut include_stack = Vec::new();
        let expanded =
            self.source_loader
                .load_expanded_source(resolved.clone(), &mut include_stack, true)?;

        if !self.source.is_empty() {
            self.source.push('\n');
        }
        let offset = self.source.len();
        let source_span = Span::new(offset, offset + expanded.source.len());
        let mut tokens = tokenize(&expanded.source).map_err(|mut diagnostics| {
            rebase_diagnostics(&mut diagnostics, offset);
            diagnostics
        })?;
        for token in &mut tokens {
            token.span.start += offset;
            token.span.end += offset;
        }
        let program = parse(&tokens)?;
        let root_source_id = expanded.root_source_id;
        self.source.push_str(&expanded.source);
        self.source_map.append_shifted(expanded.source_map, offset);

        Ok(ParsedUnit {
            program,
            origin: resolved,
            root_source_id,
            source_span,
        })
    }
}

fn coalesce_search_roots(
    provider: &dyn SourceProvider,
    roots: impl IntoIterator<Item = SourceOrigin>,
) -> Vec<SourceOrigin> {
    let mut keys = HashSet::new();
    roots
        .into_iter()
        .filter(|root| {
            let resolved = provider.resolve(root);
            keys.insert(provider.canonical_key(&resolved))
        })
        .collect()
}

fn named_path(program: &Program) -> Option<&ModulePath> {
    match &program.source_kind {
        SourceUnitKind::Named(module) => Some(&module.path),
        SourceUnitKind::Legacy => None,
    }
}

fn named_uses(program: &Program) -> &[UseDecl] {
    match &program.source_kind {
        SourceUnitKind::Named(module) => &module.uses,
        SourceUnitKind::Legacy => &[],
    }
}

fn rebase_diagnostics(diagnostics: &mut [Diagnostic], offset: usize) {
    for diagnostic in diagnostics {
        diagnostic.span.start += offset;
        diagnostic.span.end += offset;
    }
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
        allow_named: bool,
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
        let result = self.read_expand_source(resolved, active, allow_named);
        active.pop();
        result
    }

    fn read_expand_source(
        &mut self,
        origin: SourceOrigin,
        active: &mut Vec<SourceOrigin>,
        allow_named: bool,
    ) -> Result<ExpandedSource, Vec<Diagnostic>> {
        let source_text = self.read_source(origin)?;
        let source = source_text.decode();
        let tokens = tokenize(&source)?;
        let program = parse(&tokens)?;
        if !allow_named && let SourceUnitKind::Named(module) = &program.source_kind {
            return Err(vec![Diagnostic::new(
                module.span,
                "an included fragment cannot declare a named module; use USE instead",
            )]);
        }
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
            match self.load_expanded_source(include_origin.clone(), active, false) {
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
        if let SourceUnitKind::Named(module) = &program.source_kind {
            return Err(vec![Diagnostic::new(
                module.span,
                "an included fragment cannot declare a named module; use USE instead",
            )]);
        }
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
        let source_kind = program.source_kind;

        for module in program.modules {
            let mut items = Vec::new();
            self.expand_items(module.items, owner, active, &mut items, &mut diagnostics);
            modules.push(Module { items });
        }

        if diagnostics.is_empty() {
            Ok(Program {
                modules,
                source_kind,
            })
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
    let host_path = strip_atari_device(include_path);
    if let Some(owner_path) = owner.host_path() {
        let base_dir = owner_path.parent().unwrap_or_else(|| Path::new("."));
        let path = Path::new(host_path);
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            base_dir.join(path)
        };
        return Ok(SourceOrigin::host(candidate));
    }

    let Some(owner_path) = owner.virtual_path() else {
        return Err(format!(
            "INCLUDE `{include_path}` cannot be resolved from {owner}"
        ));
    };
    embedded_include_origin(owner_path, host_path)
}

fn embedded_include_origin(owner_path: &str, include_path: &str) -> Result<SourceOrigin, String> {
    let include_path = include_path.replace('\\', "/");
    let path = Path::new(&include_path);
    if path.is_absolute() {
        return Err(format!(
            "embedded INCLUDE `{include_path}` must be relative"
        ));
    }

    let mut components = Path::new(owner_path)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
        .filter_map(|component| match component {
            Component::Normal(component) => Some(component.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for component in path.components() {
        match component {
            Component::Normal(component) => {
                components.push(component.to_string_lossy().to_ascii_lowercase())
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "embedded INCLUDE `{include_path}` cannot escape its virtual directory"
                ));
            }
        }
    }
    if components.is_empty() {
        return Err(format!(
            "embedded INCLUDE `{include_path}` has no file name"
        ));
    }
    let virtual_path = components.join("/");
    Ok(SourceOrigin::embedded(
        virtual_path.clone(),
        format!("<embedded:{}>", virtual_path.to_ascii_uppercase()),
    ))
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
    fn embedded_include_is_relative_case_normalized_and_mapped() {
        let root = SourceOrigin::embedded("runtime/all.act", "<embedded:RUNTIME/ALL.ACT>");
        let fragment = SourceOrigin::embedded("runtime/part.act", "<embedded:RUNTIME/PART.ACT>");
        let provider = InMemorySourceProvider::default()
            .with_source(
                root.clone(),
                b"BYTE before\nINCLUDE \"PART.ACT\"\n".to_vec(),
            )
            .with_source(fragment.clone(), b"BYTE included\n".to_vec());

        let loaded = load_program_with_expanded_source_from_provider(root, &provider)
            .expect("embedded include");
        let Item::Declaration(crate::ast::Decl::Var(var)) = &loaded.program.modules[0].items[1]
        else {
            panic!("expected included declaration");
        };
        assert_eq!(
            loaded.source_map.location(var.span).unwrap().origin,
            fragment
        );
    }

    #[test]
    fn embedded_include_cannot_escape_its_virtual_directory() {
        let owner = SourceOrigin::embedded("runtime/all.act", "<embedded:RUNTIME/ALL.ACT>");
        let error = include_origin(&owner, "../private.act").expect_err("escaped include");
        assert!(error.contains("cannot escape its virtual directory"));
    }

    #[test]
    fn included_fragment_cannot_declare_a_named_module() {
        let root = SourceOrigin::host(PathBuf::from("project/main.act"));
        let fragment = SourceOrigin::host(PathBuf::from("project/fragment.act"));
        let provider = InMemorySourceProvider::default()
            .with_source(
                root.clone(),
                b"MODULE ROOT\nINCLUDE \"fragment.act\"\nENDMODULE\n".to_vec(),
            )
            .with_source(
                fragment,
                b"MODULE NESTED\nPUBLIC BYTE value\nENDMODULE\n".to_vec(),
            );

        let Err(diagnostics) = load_program_with_expanded_source_from_provider(root, &provider)
        else {
            panic!("expected named include rejection");
        };
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("included fragment cannot declare a named module")
        }));
    }

    #[test]
    fn included_declarations_inherit_the_named_owner_context() {
        let root = SourceOrigin::host(PathBuf::from("project/main.act"));
        let fragment = SourceOrigin::host(PathBuf::from("project/fragment.act"));
        let provider = InMemorySourceProvider::default()
            .with_source(
                root.clone(),
                b"MODULE ROOT\nINCLUDE \"fragment.act\"\nENDMODULE\n".to_vec(),
            )
            .with_source(fragment, b"PUBLIC VOLATILE BYTE WSYNC=$D40A\n".to_vec());

        let loaded = load_program_with_expanded_source_from_provider(root, &provider).unwrap();
        let Item::Declaration(crate::ast::Decl::Var(var)) = &loaded.program.modules[0].items[0]
        else {
            panic!("expected included register declaration");
        };
        assert_eq!(var.visibility, crate::ast::Visibility::Public);
        assert!(var.qualifiers.is_volatile);
    }

    #[test]
    fn loads_named_modules_once_in_deterministic_dependency_order() {
        let root = SourceOrigin::host(PathBuf::from("project/main.act"));
        let provider = InMemorySourceProvider::default()
            .with_source(
                root.clone(),
                b"MODULE APP\nUSE LIB.B\nUSE LIB.A\nENDMODULE\n".to_vec(),
            )
            .with_source(
                SourceOrigin::host(PathBuf::from("project/lib/b.act")),
                b"MODULE LIB.B\nUSE LIB.COMMON\nENDMODULE\n".to_vec(),
            )
            .with_source(
                SourceOrigin::host(PathBuf::from("project/lib/a.act")),
                b"MODULE LIB.A\nUSE LIB.COMMON\nENDMODULE\n".to_vec(),
            )
            .with_source(
                SourceOrigin::host(PathBuf::from("project/lib/common.act")),
                b"MODULE LIB.COMMON\nPUBLIC BYTE value\nENDMODULE\n".to_vec(),
            );

        let loaded =
            load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
        let names = loaded
            .modules
            .iter()
            .map(|module| {
                module
                    .declared_path
                    .as_ref()
                    .map(ModulePath::display_name)
                    .unwrap_or_else(|| "<legacy>".to_string())
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["APP", "LIB.B", "LIB.COMMON", "LIB.A"]);
        assert_eq!(
            loaded.graph_order,
            [ModuleId(2), ModuleId(1), ModuleId(3), ModuleId(0)]
        );
        assert_eq!(loaded.modules[0].dependencies, [ModuleId(1), ModuleId(3)]);
        assert_eq!(loaded.modules[1].dependencies, [ModuleId(2)]);
        assert_eq!(loaded.modules[3].dependencies, [ModuleId(2)]);
    }

    #[test]
    fn module_search_prefers_project_root_then_explicit_paths() {
        let root = SourceOrigin::host(PathBuf::from("source/main.act"));
        let project_module = SourceOrigin::host(PathBuf::from("project/lib/math.act"));
        let fallback_module = SourceOrigin::host(PathBuf::from("fallback/lib/math.act"));
        let provider = InMemorySourceProvider::default()
            .with_source(
                root.clone(),
                b"MODULE APP\nUSE LIB.MATH\nENDMODULE\n".to_vec(),
            )
            .with_source(
                project_module.clone(),
                b"MODULE LIB.MATH\nPUBLIC BYTE projectValue\nENDMODULE\n".to_vec(),
            )
            .with_source(
                fallback_module,
                b"MODULE LIB.MATH\nPUBLIC BYTE fallbackValue\nENDMODULE\n".to_vec(),
            );
        let options = ModuleLoadOptions {
            project_root: Some(PathBuf::from("project")),
            module_paths: vec![PathBuf::from("fallback")],
        };

        let loaded = load_compilation_from_provider(root, &provider, &options).unwrap();
        assert_eq!(loaded.modules[1].origin, project_module);
    }

    #[test]
    fn repeated_physical_module_roots_are_coalesced_canonically() {
        let dir = temp_dir("actionc-module-root-coalescing");
        fs::create_dir_all(&dir).unwrap();
        let roots = coalesce_search_roots(
            &HostSourceProvider,
            [
                SourceOrigin::host(dir.clone()),
                SourceOrigin::host(dir.join(".")),
                SourceOrigin::host(dir.clone()),
            ],
        );
        assert_eq!(roots, [SourceOrigin::host(dir)]);
    }

    #[test]
    fn rejects_module_cycles_with_the_complete_closing_chain() {
        let root = SourceOrigin::host(PathBuf::from("project/main.act"));
        let provider = InMemorySourceProvider::default()
            .with_source(root.clone(), b"MODULE APP\nUSE LIB.A\nENDMODULE\n".to_vec())
            .with_source(
                SourceOrigin::host(PathBuf::from("project/lib/a.act")),
                b"MODULE LIB.A\nUSE LIB.B\nENDMODULE\n".to_vec(),
            )
            .with_source(
                SourceOrigin::host(PathBuf::from("project/lib/b.act")),
                b"MODULE LIB.B\nUSE APP\nENDMODULE\n".to_vec(),
            );

        let Err(diagnostics) =
            load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default())
        else {
            panic!("expected module cycle");
        };
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("APP -> LIB.A -> LIB.B -> APP"))
        );
    }

    #[test]
    fn validates_module_file_identity_and_canonical_path_case() {
        let root = SourceOrigin::host(PathBuf::from("project/main.act"));
        let wrong_declaration = InMemorySourceProvider::default()
            .with_source(
                root.clone(),
                b"MODULE APP\nUSE LIB.MATH\nENDMODULE\n".to_vec(),
            )
            .with_source(
                SourceOrigin::host(PathBuf::from("project/lib/math.act")),
                b"MODULE LIB.OTHER\nENDMODULE\n".to_vec(),
            );
        let Err(diagnostics) = load_compilation_from_provider(
            root.clone(),
            &wrong_declaration,
            &ModuleLoadOptions::default(),
        ) else {
            panic!("expected declaration mismatch");
        };
        assert!(diagnostics[0].message.contains("declares `LIB.OTHER`"));

        let wrong_case = InMemorySourceProvider::default()
            .with_source(
                root.clone(),
                b"MODULE APP\nUSE LIB.MATH\nENDMODULE\n".to_vec(),
            )
            .with_source(
                SourceOrigin::host(PathBuf::from("project/Lib/Math.ACT")),
                b"MODULE LIB.MATH\nENDMODULE\n".to_vec(),
            );
        let Err(diagnostics) =
            load_compilation_from_provider(root, &wrong_case, &ModuleLoadOptions::default())
        else {
            panic!("expected canonical path case diagnostic");
        };
        assert!(diagnostics[0].message.contains("canonical lowercase"));
    }

    #[test]
    fn reserved_module_roots_cannot_be_shadowed_by_host_sources() {
        for (module, path) in [
            ("ATARI.GTIA", "project/atari/gtia.act"),
            ("SYS", "project/sys.act"),
        ] {
            let root = SourceOrigin::host(PathBuf::from("project/main.act"));
            let provider = InMemorySourceProvider::default()
                .with_source(
                    root.clone(),
                    format!("MODULE APP\nUSE {module}\nENDMODULE\n").into_bytes(),
                )
                .with_source(
                    SourceOrigin::host(PathBuf::from(path)),
                    format!("MODULE {module}\nENDMODULE\n").into_bytes(),
                );

            let Err(diagnostics) =
                load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default())
            else {
                panic!("expected reserved-module rejection for {module}");
            };
            assert!(diagnostics[0].message.contains("reserved embedded module"));
        }
    }

    #[test]
    fn embedded_reserved_modules_precede_host_lookup() {
        let root = SourceOrigin::host(PathBuf::from("project/main.act"));
        let embedded = SourceOrigin::embedded("atari/gtia.act", "<embedded:ATARI.GTIA>");
        let provider = InMemorySourceProvider::default()
            .with_source(
                root.clone(),
                b"MODULE APP\nUSE ATARI.GTIA\nENDMODULE\n".to_vec(),
            )
            .with_source(
                embedded.clone(),
                b"MODULE ATARI.GTIA\nPUBLIC VOLATILE BYTE PRIOR=$D01B\nENDMODULE\n".to_vec(),
            )
            .with_source(
                SourceOrigin::host(PathBuf::from("project/atari/gtia.act")),
                b"MODULE ATARI.GTIA\nENDMODULE\n".to_vec(),
            );

        let loaded =
            load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
        assert_eq!(loaded.modules[1].origin, embedded);
    }

    #[test]
    fn host_lookup_reports_case_mismatches_without_platform_dependence() {
        let dir = temp_dir("actionc-module-case");
        fs::create_dir_all(dir.join("Lib")).unwrap();
        fs::write(
            dir.join("main.act"),
            "MODULE APP\nUSE LIB.MATH\nENDMODULE\n",
        )
        .unwrap();
        fs::write(dir.join("Lib/Math.ACT"), "MODULE LIB.MATH\nENDMODULE\n").unwrap();

        let provider = CompilerSourceProvider::default();
        let Err(diagnostics) = load_compilation_from_provider(
            SourceOrigin::host(dir.join("main.act")),
            &provider,
            &ModuleLoadOptions::default(),
        ) else {
            panic!("expected canonical host path case diagnostic");
        };
        assert!(diagnostics[0].message.contains("canonical lowercase"));
    }

    #[test]
    fn aggregate_source_map_preserves_used_module_origins() {
        let root = SourceOrigin::host(PathBuf::from("project/main.act"));
        let library = SourceOrigin::host(PathBuf::from("project/lib/data.act"));
        let provider = InMemorySourceProvider::default()
            .with_source(
                root.clone(),
                b"MODULE APP\nUSE LIB.DATA\nENDMODULE\n".to_vec(),
            )
            .with_source(
                library.clone(),
                b"MODULE LIB.DATA\nPUBLIC BYTE value\nENDMODULE\n".to_vec(),
            );

        let loaded =
            load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
        let Item::Declaration(crate::ast::Decl::Var(var)) =
            &loaded.modules[1].program.modules[0].items[0]
        else {
            panic!("expected declaration from used module");
        };
        let location = loaded.source_map.location(var.span).unwrap();
        assert_eq!(location.origin, library);
        assert_eq!(location.excerpt, "PUBLIC BYTE value");
        assert_ne!(
            loaded.modules[0].root_source_id,
            loaded.modules[1].root_source_id
        );
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
