//! Versioned native image transport. This contains no emulator or executable IR.
mod model;
use super::NativeCompiledProgram;
use crate::{
    mir68k::image::{ImageView, Segment, ZeroFill, verify_regions},
    target::{TargetId, TargetLayout},
};
pub use model::{
    ArtifactSymbol, FORMAT, Identity, LayoutKind, Location, Manifest, SegmentSpec, SymbolLayout,
    VERSION,
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone)]
pub struct NativeArtifact {
    pub manifest: Manifest,
    pub segments: Vec<Segment>,
}
impl ImageView for NativeArtifact {
    fn entry(&self) -> u32 {
        self.manifest.entry
    }
    fn segments(&self) -> &[Segment] {
        &self.segments
    }
    fn zero_fill(&self) -> &[ZeroFill] {
        &self.manifest.zero_fill
    }
    fn verify(&self) -> Result<(), String> {
        self.manifest.verify()?;
        if self.segments.len() != self.manifest.segments.len() {
            return Err("segment count mismatch".into());
        }
        for (actual, spec) in self.segments.iter().zip(&self.manifest.segments) {
            if actual.address != spec.address
                || actual.bytes.len() != spec.size as usize
                || actual.writable != spec.writable
                || actual.executable != spec.executable
            {
                return Err("native payload/manifest mismatch".into());
            }
        }
        verify_regions(
            TargetLayout::for_target(TargetId::Motorola68000),
            self.entry(),
            self.segments(),
            self.zero_fill(),
        )
    }
}
impl NativeArtifact {
    pub fn symbol(&self, name: &str) -> Result<&ArtifactSymbol, String> {
        self.manifest.symbol(name)
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes = read_bounded(path, model::ADDRESS_LIMIT as usize)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|e| format!("invalid native manifest: {e}"))?;
        manifest.verify()?;
        let directory = parent(path).canonicalize().map_err(|e| e.to_string())?;
        let mut segments = Vec::new();
        for spec in &manifest.segments {
            let file = directory
                .join(&spec.file)
                .canonicalize()
                .map_err(|e| format!("{}: {e}", spec.file))?;
            if file.parent() != Some(directory.as_path()) {
                return Err("payload escapes manifest directory".into());
            }
            let bytes = read_bounded(&file, spec.size as usize)?;
            if bytes.len() != spec.size as usize {
                return Err("native payload length mismatch".into());
            }
            segments.push(Segment {
                address: spec.address,
                bytes,
                writable: spec.writable,
                executable: spec.executable,
            });
        }
        let artifact = Self { manifest, segments };
        artifact.verify()?;
        Ok(artifact)
    }
}
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!(
            "{} exceeds declared/allowed length",
            path.display()
        ));
    }
    Ok(bytes)
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Resolve output identity even when the final file does not yet exist.
pub fn destination(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return path.canonicalize().map_err(|e| e.to_string());
    }
    if path.components().next_back() == Some(std::path::Component::ParentDir) {
        return destination(parent(path))?
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "output escapes filesystem root".into());
    }
    let name = path.file_name().ok_or("output needs a file name")?;
    Ok(destination(parent(path))?.join(name))
}

struct Staged {
    files: Vec<PathBuf>,
}
impl Staged {
    fn create(&mut self, path: PathBuf, bytes: &[u8]) -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        self.files.push(path);
        file.write_all(bytes).map_err(|e| e.to_string())
    }
}
impl Drop for Staged {
    fn drop(&mut self) {
        for path in &self.files {
            let _ = fs::remove_file(path);
        }
    }
}
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Publish a manifest last. New payload names never replace files referenced by
/// an older manifest. Old generations are retained for callers to manage.
pub fn write(
    program: &NativeCompiledProgram,
    path: &Path,
    listing: Option<&Path>,
    protected: &[&Path],
) -> Result<(), String> {
    program.image.verify()?;
    let out = destination(path)?;
    let list = listing.map(destination).transpose()?;
    if list.as_ref() == Some(&out) {
        return Err("manifest and listing paths must differ".into());
    }
    for source in protected
        .iter()
        .copied()
        .chain(program.source_paths.iter().map(PathBuf::as_path))
    {
        let source = destination(source)?;
        if source == out || list.as_ref() == Some(&source) {
            return Err("output would overwrite source".into());
        }
    }
    for output in std::iter::once(&out).chain(list.iter()) {
        if output.is_dir() {
            return Err("output names a directory".into());
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let generation = format!(
        "actionc-native-{}-{now}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let directory = parent(&out);
    let mut manifest = Manifest::from_image(&program.image);
    for (i, s) in program.image.segments.iter().enumerate() {
        manifest.segments.push(SegmentSpec {
            file: format!("{generation}-{i}.bin"),
            address: s.address,
            size: u32::try_from(s.bytes.len()).map_err(|_| "segment too large")?,
            writable: s.writable,
            executable: s.executable,
        });
    }
    if let Some(listing) = &list {
        if parent(listing) == directory {
            manifest.machine_listing = Some(
                listing
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or("listing file name is not UTF-8")?
                    .into(),
            );
        }
    }
    manifest.verify()?;
    let mut json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    json.push(b'\n');
    if json.len() > model::ADDRESS_LIMIT as usize {
        return Err("native manifest exceeds allowed length".into());
    }
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let mut staged = Staged { files: vec![] };
    for (spec, segment) in manifest.segments.iter().zip(&program.image.segments) {
        staged.create(directory.join(&spec.file), &segment.bytes)?;
    }
    let manifest_temp = directory.join(format!("{generation}.json.tmp"));
    staged.create(manifest_temp.clone(), &json)?;
    if let Some(listing) = list {
        fs::create_dir_all(parent(&listing)).map_err(|e| e.to_string())?;
        let temp = parent(&listing).join(format!("{generation}.listing.tmp"));
        staged.create(temp.clone(), format!("{:#?}\n", program.machine).as_bytes())?;
        fs::rename(&temp, &listing).map_err(|e| e.to_string())?;
    }
    fs::rename(&manifest_temp, &out).map_err(|e| e.to_string())?;
    staged.files.clear();
    Ok(())
}
