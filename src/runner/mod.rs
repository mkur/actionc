use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;

use atrcopy_rs::{AtrImage, MYDOS_ATR};

use crate::compiler::{
    CompileError, CompileErrorKind, CompileMode, CompileOptions, CompilerPhase, DiagnosticSite,
    compile_file,
};

#[allow(dead_code)]
pub(crate) mod emulator;

const AUTORUN_NAME: &str = "AUTORUN.AR0";

pub fn actionc_run_main() {
    match run_cli(env::args_os().skip(1)) {
        Ok(RunCliOutcome::Completed { atr }) => {
            println!("ATR: {}", atr.display());
        }
        Ok(RunCliOutcome::Help) => print_help(),
        Err(error) => {
            print_error(&error);
            process::exit(error.exit_code());
        }
    }
}

#[derive(Debug)]
enum RunCliOutcome {
    Completed { atr: PathBuf },
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunCliOptions {
    source: PathBuf,
    output_atr: PathBuf,
    compile: CompileOptions,
}

fn run_cli(args: impl IntoIterator<Item = OsString>) -> Result<RunCliOutcome, RunnerError> {
    let Some(options) = parse_args(args)? else {
        return Ok(RunCliOutcome::Help);
    };
    let atr = prepare_atr(&options.source, &options.compile)?;
    write_file_atomically(&options.output_atr, &atr)?;
    Ok(RunCliOutcome::Completed {
        atr: options.output_atr,
    })
}

fn parse_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Option<RunCliOptions>, RunnerError> {
    let mut args = args.into_iter();
    let mut mode = None;
    let mut no_run = false;
    let mut output_atr = None;
    let mut source = None;

    while let Some(arg) = args.next() {
        if arg == OsStr::new("-h") || arg == OsStr::new("--help") {
            return Ok(None);
        }
        if arg == OsStr::new("--no-run") {
            no_run = true;
            continue;
        }
        if arg == OsStr::new("--mode") {
            let value = args.next().ok_or_else(|| {
                RunnerError::configuration("--mode requires compatibility, optimized, or mir6502")
            })?;
            mode = Some(parse_mode(&value)?);
            continue;
        }
        if let Some(value) = os_option_value(&arg, "--mode=") {
            mode = Some(parse_mode(value)?);
            continue;
        }
        if arg == OsStr::new("--out-atr") {
            let value = args
                .next()
                .ok_or_else(|| RunnerError::configuration("--out-atr requires a file path"))?;
            output_atr = Some(PathBuf::from(value));
            continue;
        }
        if let Some(value) = os_option_value(&arg, "--out-atr=") {
            output_atr = Some(PathBuf::from(value));
            continue;
        }
        if arg.to_string_lossy().starts_with('-') {
            return Err(RunnerError::configuration(format!(
                "unexpected argument: {}",
                arg.to_string_lossy()
            )));
        }
        if source.replace(PathBuf::from(&arg)).is_some() {
            return Err(RunnerError::configuration(format!(
                "unexpected argument: {}",
                arg.to_string_lossy()
            )));
        }
    }

    let source = source.ok_or_else(|| RunnerError::configuration("missing Action source file"))?;
    if !no_run {
        return Err(RunnerError::configuration(
            "emulator launch is not implemented yet; use --no-run",
        ));
    }
    let output_atr = output_atr.unwrap_or_else(|| default_atr_path(&source));
    let compile = mode.map_or_else(CompileOptions::default, CompileOptions::for_mode);

    Ok(Some(RunCliOptions {
        source,
        output_atr,
        compile,
    }))
}

fn parse_mode(value: &OsStr) -> Result<CompileMode, RunnerError> {
    match value.to_str() {
        Some("compatibility") => Ok(CompileMode::Compatibility),
        Some("optimized") => Ok(CompileMode::Optimized),
        Some("mir6502") => Ok(CompileMode::Mir6502),
        _ => Err(RunnerError::configuration(format!(
            "unknown mode: {}; expected compatibility, optimized, or mir6502",
            value.to_string_lossy()
        ))),
    }
}

fn os_option_value<'a>(value: &'a OsStr, prefix: &str) -> Option<&'a OsStr> {
    let value = value.to_str()?;
    value.strip_prefix(prefix).map(OsStr::new)
}

fn default_atr_path(source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("program"));
    let mut file_name = stem.to_os_string();
    file_name.push(".atr");
    PathBuf::from(file_name)
}

fn prepare_atr(source: &Path, options: &CompileOptions) -> Result<Vec<u8>, RunnerError> {
    let compiled = compile_file(source, options).map_err(RunnerError::Compile)?;
    let mut image = AtrImage::from_bytes(MYDOS_ATR.to_vec())
        .map_err(|message| RunnerError::Atr(format!("embedded MyDOS image: {message}")))?;
    image
        .upsert_file(AUTORUN_NAME, compiled.object_bytes())
        .map_err(|message| RunnerError::Atr(format!("add {AUTORUN_NAME}: {message}")))?;
    Ok(image.into_bytes())
}

fn write_file_atomically(path: &Path, contents: &[u8]) -> Result<(), RunnerError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| RunnerError::Io {
            operation: "create output directory",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("output.atr");
    let temporary = path.with_file_name(format!(".{file_name}.actionc-run-{}.tmp", process::id()));
    fs::write(&temporary, contents).map_err(|source| RunnerError::Io {
        operation: "write temporary ATR",
        path: temporary.clone(),
        source,
    })?;
    if let Err(source) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(RunnerError::Io {
            operation: "install output ATR",
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

fn print_help() {
    eprintln!(
        "usage: actionc-run [--mode compatibility|optimized|mir6502] --no-run [--out-atr <file.atr>] <file.act>"
    );
}

fn print_error(error: &RunnerError) {
    if let RunnerError::Compile(error) = error {
        for diagnostic in error.diagnostics() {
            match &diagnostic.site {
                DiagnosticSite::Source {
                    path,
                    line,
                    column,
                    excerpt,
                    ..
                } => eprintln!(
                    "{}:{}:{}: {}{}",
                    path.display(),
                    line,
                    column,
                    diagnostic.message,
                    excerpt
                        .as_ref()
                        .map(|excerpt| format!(" | {excerpt}"))
                        .unwrap_or_default()
                ),
                DiagnosticSite::File { path } => {
                    eprintln!("{}: {}", path.display(), diagnostic.message)
                }
                DiagnosticSite::Ir { routine, block } => {
                    let phase = compiler_phase_name(diagnostic.phase);
                    match (routine, block) {
                        (Some(routine), Some(block)) => {
                            eprintln!("{phase} {routine}:{block}: {}", diagnostic.message)
                        }
                        (Some(routine), None) => {
                            eprintln!("{phase} {routine}: {}", diagnostic.message)
                        }
                        (None, _) => eprintln!("{phase}: {}", diagnostic.message),
                    }
                }
                DiagnosticSite::Unknown => eprintln!("{}", diagnostic.message),
            }
        }
    } else {
        eprintln!("{error}");
    }
}

fn compiler_phase_name(phase: CompilerPhase) -> &'static str {
    match phase {
        CompilerPhase::Configuration => "configuration",
        CompilerPhase::Input => "input",
        CompilerPhase::Frontend => "frontend",
        CompilerPhase::Semantic => "semantic",
        CompilerPhase::Nir => "nir",
        CompilerPhase::Mir6502 => "mir6502",
        CompilerPhase::Codegen => "codegen",
    }
}

#[derive(Debug)]
enum RunnerError {
    Configuration(String),
    Compile(CompileError),
    Atr(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl RunnerError {
    fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    fn exit_code(&self) -> i32 {
        match self {
            Self::Configuration(_) => 2,
            Self::Compile(error) if error.kind() == CompileErrorKind::Configuration => 2,
            Self::Compile(_) | Self::Atr(_) | Self::Io { .. } => 1,
        }
    }
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) | Self::Atr(message) => formatter.write_str(message),
            Self::Compile(error) => error.fmt(formatter),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {}: {source}", path.display()),
        }
    }
}

impl Error for RunnerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::Configuration(_) | Self::Atr(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello_world() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples")
            .join("hello-world.act")
    }

    #[test]
    fn prepared_atr_contains_the_compiler_object_as_autorun() {
        let source = hello_world();
        let options = CompileOptions::for_mode(CompileMode::Compatibility);
        let compiled = compile_file(&source, &options).expect("compile expected object");

        let atr = prepare_atr(&source, &options).expect("prepare runnable ATR");
        let image = AtrImage::from_bytes(atr).expect("parse prepared ATR");

        assert_eq!(
            image
                .read_file_named(AUTORUN_NAME)
                .expect("read prepared ATR")
                .expect("find AUTORUN.AR0"),
            compiled.object_bytes()
        );
    }

    #[test]
    fn parser_requires_no_run_until_an_adapter_is_available() {
        let error = parse_args([OsString::from("program.act")]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("--no-run"));
    }

    #[test]
    fn parser_maps_public_compiler_modes_and_default_atr_name() {
        let options = parse_args([
            OsString::from("--mode=mir6502"),
            OsString::from("--no-run"),
            OsString::from("some path/demo.act"),
        ])
        .expect("parse actionc-run options")
        .expect("not help");

        assert_eq!(options.source, PathBuf::from("some path/demo.act"));
        assert_eq!(options.output_atr, PathBuf::from("demo.atr"));
        assert_eq!(options.compile.mode(), Some(CompileMode::Mir6502));
    }
}
