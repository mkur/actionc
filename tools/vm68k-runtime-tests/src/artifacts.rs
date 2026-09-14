//! Compatibility entry point for compiler-owned native artifacts.
use actionc::compiler::native::{NativeCompiledProgram, artifacts};
use std::path::{Path, PathBuf};

pub fn dump(program: &NativeCompiledProgram, prefix: &Path) -> Result<PathBuf, String> {
    let mut manifest = prefix.as_os_str().to_os_string();
    manifest.push(".json");
    let mut listing = prefix.as_os_str().to_os_string();
    listing.push(".machine.txt");
    let manifest = PathBuf::from(manifest);
    artifacts::write(program, &manifest, Some(Path::new(&listing)), &[])?;
    Ok(manifest)
}
