//! Relocatable Shell executable API; no fictitious absolute link origin.
use super::{
    NativeCompileOptions, codegen_error, prepare_file,
    runtime::{self, NativeRuntime},
};
use crate::{compiler::CompileError, mir68k, nir};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone)]
pub struct Options {
    pub optimize: bool,
    pub promotion: nir::NirPromotionPolicy,
    pub codegen: mir68k::materialize::Options,
    pub project_root: Option<PathBuf>,
    pub module_paths: Vec<PathBuf>,
}
impl Default for Options {
    fn default() -> Self {
        Self::from(&NativeCompileOptions::default())
    }
}
impl From<&NativeCompileOptions> for Options {
    fn from(options: &NativeCompileOptions) -> Self {
        Self {
            optimize: options.optimize,
            promotion: options.promotion,
            codegen: options.codegen,
            project_root: options.project_root.clone(),
            module_paths: options.module_paths.clone(),
        }
    }
}
pub struct CompiledProgram {
    pub executable: mir68k::hunk::Executable,
    pub machine: mir68k::machine::MachineProgram,
    pub source_paths: Vec<PathBuf>,
}
pub fn compile_file(
    path: impl AsRef<Path>,
    options: &Options,
) -> Result<CompiledProgram, CompileError> {
    let prepared = prepare_file(
        path,
        &NativeCompileOptions {
            optimize: options.optimize,
            promotion: options.promotion,
            codegen: options.codegen,
            project_root: options.project_root.clone(),
            module_paths: options.module_paths.clone(),
            ..Default::default()
        },
    )?;
    let bindings = runtime::bind(&prepared.mir, NativeRuntime::AmigaDos).map_err(codegen_error)?;
    let machine = mir68k::amiga::materialize_with_bindings(
        &prepared.mir,
        options.codegen,
        bindings.console_bindings(),
    )
    .map_err(codegen_error)?;
    let mut object = mir68k::object::emit(&prepared.mir, &machine).map_err(codegen_error)?;
    prepared.qualify_object(&mut object);
    let executable = mir68k::hunk::emit(&object).map_err(codegen_error)?;
    Ok(CompiledProgram {
        executable,
        machine,
        source_paths: prepared.source_paths,
    })
}

static NEXT: AtomicU64 = AtomicU64::new(0);
/// Stage complete output before replacement; source/include/module identities
/// use the same canonical destination checks as bare native artifacts.
pub fn write(
    program: &CompiledProgram,
    path: &Path,
    listing: Option<&Path>,
    protected: &[&Path],
) -> Result<(), String> {
    use super::artifacts::{Staged, destination, parent};
    let out = destination(path)?;
    let list = listing.map(destination).transpose()?;
    if list.as_ref() == Some(&out) {
        return Err("executable and listing paths must differ".into());
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
        "actionc-amiga-{}-{now}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    fs::create_dir_all(parent(&out)).map_err(|e| e.to_string())?;
    let mut staged = Staged { files: vec![] };
    let temp = parent(&out).join(format!("{generation}.tmp"));
    staged.create(temp.clone(), &program.executable.bytes)?;
    if let Some(list) = list {
        fs::create_dir_all(parent(&list)).map_err(|e| e.to_string())?;
        let list_temp = parent(&list).join(format!("{generation}.listing.tmp"));
        staged.create(
            list_temp.clone(),
            format!("{:#?}\n", program.machine).as_bytes(),
        )?;
        fs::rename(list_temp, list).map_err(|e| e.to_string())?;
    }
    fs::rename(temp, out).map_err(|e| e.to_string())?;
    Ok(())
}
