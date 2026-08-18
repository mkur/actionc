use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// Identifies one physical or virtual source object within a compilation.
///
/// Spans remain offsets in the compilation's expanded source arena.  SourceId
/// carries provenance at the source-map boundary without widening every span
/// used by the frontend and intermediate representations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceOrigin {
    Host(PathBuf),
    Embedded {
        virtual_path: String,
        display_name: String,
    },
}

impl SourceOrigin {
    pub fn host(path: impl Into<PathBuf>) -> Self {
        Self::Host(path.into())
    }

    pub fn embedded(virtual_path: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self::Embedded {
            virtual_path: virtual_path.into(),
            display_name: display_name.into(),
        }
    }

    pub fn host_path(&self) -> Option<&Path> {
        match self {
            Self::Host(path) => Some(path),
            Self::Embedded { .. } => None,
        }
    }

    pub fn virtual_path(&self) -> Option<&str> {
        match self {
            Self::Host(_) => None,
            Self::Embedded { virtual_path, .. } => Some(virtual_path),
        }
    }
}

impl fmt::Display for SourceOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(path) => write!(formatter, "{}", path.display()),
            Self::Embedded { display_name, .. } => formatter.write_str(display_name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceText {
    pub id: SourceId,
    pub origin: SourceOrigin,
    pub bytes: Vec<u8>,
}

impl SourceText {
    pub fn decode(&self) -> String {
        decode_source(&self.bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLoadError {
    message: String,
}

impl SourceLoadError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SourceLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SourceLoadError {}

/// Supplies bytes for a logical source origin.
///
/// Resolution and canonical-key hooks let the host provider retain historical
/// case-insensitive INCLUDE behavior without forcing in-memory or future
/// embedded providers to consult the host filesystem.
pub trait SourceProvider {
    fn read(&self, origin: &SourceOrigin) -> Result<Vec<u8>, SourceLoadError>;

    fn resolve(&self, origin: &SourceOrigin) -> SourceOrigin {
        origin.clone()
    }

    fn canonical_key(&self, origin: &SourceOrigin) -> SourceOrigin {
        origin.clone()
    }

    /// Resolve a compiler-owned embedded module before host search roots.
    fn resolve_embedded_module(
        &self,
        _canonical_components: &[String],
    ) -> Result<Option<SourceOrigin>, SourceLoadError> {
        Ok(None)
    }

    /// Resolve a canonical lowercase module path below ordered search roots.
    fn resolve_module(
        &self,
        _canonical_components: &[String],
        _search_roots: &[SourceOrigin],
    ) -> Result<Option<SourceOrigin>, SourceLoadError> {
        Ok(None)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HostSourceProvider;

impl SourceProvider for HostSourceProvider {
    fn read(&self, origin: &SourceOrigin) -> Result<Vec<u8>, SourceLoadError> {
        let SourceOrigin::Host(path) = origin else {
            return Err(SourceLoadError::new(
                "the host source provider cannot read an embedded source",
            ));
        };
        fs::read(path).map_err(|error| SourceLoadError::new(error.to_string()))
    }

    fn resolve(&self, origin: &SourceOrigin) -> SourceOrigin {
        let SourceOrigin::Host(path) = origin else {
            return origin.clone();
        };
        SourceOrigin::Host(resolve_case_insensitive(path).unwrap_or_else(|| path.clone()))
    }

    fn canonical_key(&self, origin: &SourceOrigin) -> SourceOrigin {
        let SourceOrigin::Host(path) = origin else {
            return origin.clone();
        };
        SourceOrigin::Host(fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
    }

    fn resolve_module(
        &self,
        canonical_components: &[String],
        search_roots: &[SourceOrigin],
    ) -> Result<Option<SourceOrigin>, SourceLoadError> {
        for root in search_roots {
            let SourceOrigin::Host(root) = root else {
                continue;
            };
            if let Some(path) = resolve_module_below_root(root, canonical_components)? {
                return Ok(Some(SourceOrigin::Host(path)));
            }
        }
        Ok(None)
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemorySourceProvider {
    sources: HashMap<SourceOrigin, Vec<u8>>,
}

impl InMemorySourceProvider {
    pub fn insert(&mut self, origin: SourceOrigin, bytes: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.sources.insert(origin, bytes.into())
    }

    pub fn with_source(mut self, origin: SourceOrigin, bytes: impl Into<Vec<u8>>) -> Self {
        self.insert(origin, bytes);
        self
    }
}

impl SourceProvider for InMemorySourceProvider {
    fn read(&self, origin: &SourceOrigin) -> Result<Vec<u8>, SourceLoadError> {
        self.sources.get(origin).cloned().ok_or_else(|| {
            SourceLoadError::new(format!("source `{origin}` is not present in memory"))
        })
    }

    fn resolve_embedded_module(
        &self,
        canonical_components: &[String],
    ) -> Result<Option<SourceOrigin>, SourceLoadError> {
        let relative = module_relative_path(canonical_components);
        Ok(self.sources.keys().find_map(|origin| match origin {
            SourceOrigin::Embedded { virtual_path, .. } if Path::new(virtual_path) == relative => {
                Some(origin.clone())
            }
            _ => None,
        }))
    }

    fn resolve_module(
        &self,
        canonical_components: &[String],
        search_roots: &[SourceOrigin],
    ) -> Result<Option<SourceOrigin>, SourceLoadError> {
        let relative = module_relative_path(canonical_components);
        for root in search_roots {
            let SourceOrigin::Host(root) = root else {
                continue;
            };
            let expected = SourceOrigin::Host(root.join(&relative));
            if self.sources.contains_key(&expected) {
                return Ok(Some(expected));
            }
            if let Some(actual) = self.sources.keys().find(|origin| match origin {
                SourceOrigin::Host(path) => path
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&root.join(&relative).to_string_lossy()),
                SourceOrigin::Embedded { .. } => false,
            }) {
                return Err(SourceLoadError::new(format!(
                    "module path must use canonical lowercase spelling `{}`; found `{actual}`",
                    root.join(&relative).display()
                )));
            }
        }
        Ok(None)
    }
}

fn module_relative_path(canonical_components: &[String]) -> PathBuf {
    let mut path = PathBuf::new();
    for component in canonical_components
        .iter()
        .take(canonical_components.len().saturating_sub(1))
    {
        path.push(component);
    }
    if let Some(last) = canonical_components.last() {
        path.push(format!("{last}.act"));
    }
    path
}

fn resolve_module_below_root(
    root: &Path,
    canonical_components: &[String],
) -> Result<Option<PathBuf>, SourceLoadError> {
    let mut current = root.to_path_buf();
    for (index, component) in canonical_components.iter().enumerate() {
        let expected = if index + 1 == canonical_components.len() {
            format!("{component}.act")
        } else {
            component.clone()
        };
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(SourceLoadError::new(format!(
                    "failed to search module directory `{}`: {error}",
                    current.display()
                )));
            }
        };
        let mut exact = None;
        let mut mismatched = None;
        for entry in entries {
            let entry = entry.map_err(|error| {
                SourceLoadError::new(format!(
                    "failed to inspect module directory `{}`: {error}",
                    current.display()
                ))
            })?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == expected {
                exact = Some(entry.path());
                break;
            }
            if name.eq_ignore_ascii_case(&expected) {
                mismatched = Some(entry.path());
            }
        }
        if let Some(path) = exact {
            current = path;
        } else if let Some(path) = mismatched {
            return Err(SourceLoadError::new(format!(
                "module path must use canonical lowercase spelling `{}`; found `{}`",
                current.join(&expected).display(),
                path.display()
            )));
        } else {
            return Ok(None);
        }
    }
    Ok(Some(current))
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

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

pub fn decode_source(bytes: &[u8]) -> String {
    let mut source = String::with_capacity(bytes.len());

    for &byte in bytes {
        match byte {
            // Atari text files commonly use EOL $9B instead of LF.
            0x9b => source.push('\n'),
            _ => source.push(byte as char),
        }
    }

    source
}

pub fn source_char_byte(ch: char) -> Option<u8> {
    let value = ch as u32;
    (value <= 0xFF).then_some(value as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_atascii_eol() {
        assert_eq!(
            decode_source(b"PROC main()\x9bRETURN()"),
            "PROC main()\nRETURN()"
        );
    }

    #[test]
    fn preserves_atascii_high_bit_bytes() {
        let source = decode_source(&[b'"', 0xD4, 0xEF, b'"']);

        assert_eq!(source.chars().collect::<Vec<_>>(), vec!['"', 'Ô', 'ï', '"']);
        assert_eq!(source_char_byte('Ô'), Some(0xD4));
        assert_eq!(source_char_byte('€'), None);
    }
}
