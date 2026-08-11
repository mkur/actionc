use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use atrcopy_rs::{AtrImage, MYDOS_ATR};

use crate::compiler::{
    CompileError, CompileErrorKind, CompileMode, CompileOptions, CompilerPhase, DiagnosticSite,
    compile_file,
};
use emulator::altirra::AltirraAdapter;
use emulator::atari800::Atari800Adapter;
use emulator::discovery::{EmulatorSelection, discover_from_process};
use emulator::{EmulatorAdapter, EmulatorError, EmulatorKind, LaunchRequest};

pub(crate) mod emulator;

const BOOTSTRAP_NAME: &str = "BOOT.AR0";
const PROGRAM_AFTER_BOOTSTRAP_NAME: &str = "PROGRAM.AR1";
const DIRECT_PROGRAM_NAME: &str = "PROGRAM.AR0";
const ACTION_LIBRARY_BOOTSTRAP_ORIGIN: u16 = 0x3000;
const ACTION_LIBRARY_BOOTSTRAP_CODE: &[u8] = &[
    0xA0, 0x00, // LDY #$00
    0x8C, 0xC9, 0x04, // STY $04C9 (Action! curbank)
    0x8C, 0x00, 0xD5, // STY $D500 (select the Action! library bank)
    0x60, // RTS
];
const BUNDLED_ACTION_CARTRIDGE: &[u8] = include_bytes!("../../roms/action.rom");
const BUNDLED_ALTIRRA_OS: &[u8] = include_bytes!("../../roms/altirraos-xl.rom");
const NO_CART_CONFIG: &[u8] = b"Atari 800 Emulator, Version 7.0.0\n\
CARTRIDGE_FILENAME=\n\
CARTRIDGE_TYPE=0\n\
CARTRIDGE_PIGGYBACK_FILENAME=\n\
CARTRIDGE_PIGGYBACK_TYPE=0\n";
static NEXT_RUN_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub fn actionc_run_main() {
    match run_cli(env::args_os().skip(1)) {
        Ok(RunCliOutcome::Completed {
            retained_atr,
            retained_directory,
        }) => {
            if let Some(atr) = retained_atr {
                println!("ATR: {}", atr.display());
            }
            if let Some(directory) = retained_directory {
                println!("Run directory: {}", directory.display());
            }
        }
        Ok(RunCliOutcome::Help) => print_help(),
        Ok(RunCliOutcome::Version) => {
            println!("{}", crate::build_info::version_line("actionc-run"))
        }
        Err(error) => {
            print_error(&error);
            process::exit(error.exit_code());
        }
    }
}

#[derive(Debug)]
enum RunCliOutcome {
    Completed {
        retained_atr: Option<PathBuf>,
        retained_directory: Option<PathBuf>,
    },
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunCliOptions {
    source: PathBuf,
    output_atr: Option<PathBuf>,
    compile: CompileOptions,
    no_run: bool,
    emulator: EmulatorSelection,
    emulator_path: Option<PathBuf>,
    cartridge: CartridgeChoice,
    keep: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CartridgeChoice {
    Bundled,
    File(PathBuf),
    None,
}

impl CartridgeChoice {
    fn is_attached(&self) -> bool {
        !matches!(self, Self::None)
    }
}

fn run_cli(args: impl IntoIterator<Item = OsString>) -> Result<RunCliOutcome, RunnerError> {
    let args = args.into_iter().collect::<Vec<_>>();
    if args.len() == 1 && (args[0] == OsStr::new("-V") || args[0] == OsStr::new("--version")) {
        return Ok(RunCliOutcome::Version);
    }
    let Some(options) = parse_args(args)? else {
        return Ok(RunCliOutcome::Help);
    };
    let atr_bytes = prepare_atr(
        &options.source,
        &options.compile,
        options.cartridge.is_attached(),
    )?;

    if options.no_run {
        let output_atr = options
            .output_atr
            .unwrap_or_else(|| default_atr_path(&options.source));
        write_file_atomically(&output_atr, &atr_bytes)?;
        return Ok(RunCliOutcome::Completed {
            retained_atr: Some(output_atr),
            retained_directory: None,
        });
    }

    launch_emulator(&options, &atr_bytes)
}

fn parse_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Option<RunCliOptions>, RunnerError> {
    let mut args = args.into_iter();
    let mut mode = None;
    let mut no_run = false;
    let mut output_atr = None;
    let mut emulator = EmulatorSelection::Auto;
    let mut emulator_path = None;
    let mut cartridge = None;
    let mut keep = false;
    let mut source = None;

    while let Some(arg) = args.next() {
        if arg == OsStr::new("-h") || arg == OsStr::new("--help") {
            return Ok(None);
        }
        if arg == OsStr::new("--no-run") {
            no_run = true;
            continue;
        }
        if arg == OsStr::new("--keep") {
            keep = true;
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
        if arg == OsStr::new("--emulator") {
            let value = args.next().ok_or_else(|| {
                RunnerError::configuration("--emulator requires auto, atari800, or altirra")
            })?;
            emulator = EmulatorSelection::parse(&value)
                .map_err(|error| RunnerError::configuration(error.to_string()))?;
            continue;
        }
        if let Some(value) = os_option_value(&arg, "--emulator=") {
            emulator = EmulatorSelection::parse(value)
                .map_err(|error| RunnerError::configuration(error.to_string()))?;
            continue;
        }
        if arg == OsStr::new("--emulator-path") {
            let value = args.next().ok_or_else(|| {
                RunnerError::configuration("--emulator-path requires an executable path")
            })?;
            emulator_path = Some(PathBuf::from(value));
            continue;
        }
        if let Some(value) = os_option_value(&arg, "--emulator-path=") {
            emulator_path = Some(PathBuf::from(value));
            continue;
        }
        if arg == OsStr::new("--cart") {
            let value = args.next().ok_or_else(|| {
                RunnerError::configuration("--cart requires a cartridge image path")
            })?;
            set_cartridge_choice(&mut cartridge, CartridgeChoice::File(PathBuf::from(value)))?;
            continue;
        }
        if let Some(value) = os_option_value(&arg, "--cart=") {
            set_cartridge_choice(&mut cartridge, CartridgeChoice::File(PathBuf::from(value)))?;
            continue;
        }
        if arg == OsStr::new("--no-cart") {
            set_cartridge_choice(&mut cartridge, CartridgeChoice::None)?;
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
    if no_run && keep {
        return Err(RunnerError::configuration(
            "--keep only applies when an emulator is launched",
        ));
    }
    let compile = mode.map_or_else(CompileOptions::default, CompileOptions::for_mode);

    Ok(Some(RunCliOptions {
        source,
        output_atr,
        compile,
        no_run,
        emulator,
        emulator_path,
        cartridge: cartridge.unwrap_or(CartridgeChoice::Bundled),
        keep,
    }))
}

fn set_cartridge_choice(
    current: &mut Option<CartridgeChoice>,
    choice: CartridgeChoice,
) -> Result<(), RunnerError> {
    if current.replace(choice).is_some() {
        return Err(RunnerError::configuration(
            "--cart and --no-cart may only be specified once",
        ));
    }
    Ok(())
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

fn prepare_atr(
    source: &Path,
    options: &CompileOptions,
    cartridge_attached: bool,
) -> Result<Vec<u8>, RunnerError> {
    let compiled = compile_file(source, options).map_err(RunnerError::Compile)?;
    let mut image = AtrImage::from_bytes(MYDOS_ATR.to_vec())
        .map_err(|message| RunnerError::Atr(format!("embedded MyDOS image: {message}")))?;

    let program_name = if cartridge_attached {
        let bootstrap = action_library_bootstrap_object();
        image
            .upsert_file(BOOTSTRAP_NAME, &bootstrap)
            .map_err(|message| RunnerError::Atr(format!("add {BOOTSTRAP_NAME}: {message}")))?;
        PROGRAM_AFTER_BOOTSTRAP_NAME
    } else {
        DIRECT_PROGRAM_NAME
    };
    image
        .upsert_file(program_name, compiled.object_bytes())
        .map_err(|message| RunnerError::Atr(format!("add {program_name}: {message}")))?;
    Ok(image.into_bytes())
}

fn action_library_bootstrap_object() -> Vec<u8> {
    let end = ACTION_LIBRARY_BOOTSTRAP_ORIGIN + ACTION_LIBRARY_BOOTSTRAP_CODE.len() as u16 - 1;
    let runad = 0x02E2_u16;
    let mut object = Vec::with_capacity(12 + ACTION_LIBRARY_BOOTSTRAP_CODE.len());

    object.extend_from_slice(&[0xFF, 0xFF]);
    object.extend_from_slice(&ACTION_LIBRARY_BOOTSTRAP_ORIGIN.to_le_bytes());
    object.extend_from_slice(&end.to_le_bytes());
    object.extend_from_slice(ACTION_LIBRARY_BOOTSTRAP_CODE);
    object.extend_from_slice(&runad.to_le_bytes());
    object.extend_from_slice(&runad.wrapping_add(1).to_le_bytes());
    object.extend_from_slice(&ACTION_LIBRARY_BOOTSTRAP_ORIGIN.to_le_bytes());
    object
}

fn launch_emulator(
    options: &RunCliOptions,
    atr_bytes: &[u8],
) -> Result<RunCliOutcome, RunnerError> {
    let discovered = discover_from_process(options.emulator, options.emulator_path.as_deref())
        .map_err(RunnerError::Emulator)?;
    let run_directory = RunDirectory::create(options.keep)?;
    let atr_path = options
        .output_atr
        .clone()
        .unwrap_or_else(|| run_directory.path().join("program.atr"));
    write_file_atomically(&atr_path, atr_bytes)?;

    let cartridge = match &options.cartridge {
        CartridgeChoice::Bundled => {
            let path = run_directory.path().join("action.car");
            write_run_asset(&path, BUNDLED_ACTION_CARTRIDGE)?;
            Some(path)
        }
        CartridgeChoice::File(path) => Some(existing_cartridge(path)?),
        CartridgeChoice::None => None,
    };

    let command = match discovered.kind {
        EmulatorKind::Atari800 => {
            let os_rom = run_directory.path().join("altirraos-xl.rom");
            write_run_asset(&os_rom, BUNDLED_ALTIRRA_OS)?;
            let mut adapter = Atari800Adapter::new(&discovered.executable);
            if cartridge.is_none() {
                let config = run_directory.path().join("atari800-no-cart.cfg");
                write_no_cart_config(&discovered.executable, &config)?;
                adapter = adapter.with_no_cart_config(config);
            }
            adapter.command(&LaunchRequest {
                atr: &atr_path,
                cartridge: cartridge.as_deref(),
                os_rom: Some(&os_rom),
            })?
        }
        EmulatorKind::Altirra => {
            AltirraAdapter::new(&discovered.executable).command(&LaunchRequest {
                atr: &atr_path,
                cartridge: cartridge.as_deref(),
                os_rom: None,
            })?
        }
    };

    println!(
        "Launching {}: {}",
        discovered.kind,
        command.executable().display()
    );
    let status = command
        .to_command()
        .status()
        .map_err(|source| RunnerError::ProcessSpawn {
            kind: discovered.kind,
            executable: command.executable().to_path_buf(),
            source,
        })?;
    if !status.success() {
        return Err(RunnerError::ProcessExit {
            kind: discovered.kind,
            executable: command.executable().to_path_buf(),
            status: status.to_string(),
        });
    }

    let retained_directory = options.keep.then(|| run_directory.path().to_path_buf());
    let retained_atr = options
        .output_atr
        .clone()
        .or_else(|| options.keep.then(|| atr_path.clone()));

    Ok(RunCliOutcome::Completed {
        retained_atr,
        retained_directory,
    })
}

fn existing_cartridge(path: &Path) -> Result<PathBuf, RunnerError> {
    let metadata = fs::metadata(path).map_err(|source| RunnerError::Io {
        operation: "read cartridge image",
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(RunnerError::configuration(format!(
            "cartridge image is not a regular file: {}",
            path.display()
        )));
    }
    fs::canonicalize(path).map_err(|source| RunnerError::Io {
        operation: "resolve cartridge image",
        path: path.to_path_buf(),
        source,
    })
}

fn write_run_asset(path: &Path, contents: &[u8]) -> Result<(), RunnerError> {
    fs::write(path, contents).map_err(|source| RunnerError::Io {
        operation: "write run asset",
        path: path.to_path_buf(),
        source,
    })
}

fn write_no_cart_config(executable: &Path, target: &Path) -> Result<(), RunnerError> {
    let portable = executable
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.join(".atari800.cfg"));
    let user = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| home.join(".atari800.cfg"));
    let source = portable
        .into_iter()
        .chain(user)
        .find(|candidate| candidate.is_file());

    if let Some(source) = source {
        let contents = fs::read_to_string(&source).map_err(|error| RunnerError::Io {
            operation: "read Atari800 configuration",
            path: source,
            source: error,
        })?;
        let sanitized = sanitize_atari800_config(&contents);
        write_run_asset(target, sanitized.as_bytes())
    } else {
        write_run_asset(target, NO_CART_CONFIG)
    }
}

fn sanitize_atari800_config(contents: &str) -> String {
    const REPLACEMENTS: [(&str, &str); 4] = [
        ("CARTRIDGE_FILENAME=", "CARTRIDGE_FILENAME="),
        ("CARTRIDGE_TYPE=", "CARTRIDGE_TYPE=0"),
        (
            "CARTRIDGE_PIGGYBACK_FILENAME=",
            "CARTRIDGE_PIGGYBACK_FILENAME=",
        ),
        ("CARTRIDGE_PIGGYBACK_TYPE=", "CARTRIDGE_PIGGYBACK_TYPE=0"),
    ];
    let mut found = [false; REPLACEMENTS.len()];
    let mut output = String::new();

    for line in contents.lines() {
        let mut replacement = None;
        for (index, (prefix, value)) in REPLACEMENTS.iter().enumerate() {
            if line.starts_with(prefix) {
                found[index] = true;
                replacement = Some(*value);
                break;
            }
        }
        output.push_str(replacement.unwrap_or(line));
        output.push('\n');
    }
    for (found, (_, value)) in found.into_iter().zip(REPLACEMENTS) {
        if !found {
            output.push_str(value);
            output.push('\n');
        }
    }
    output
}

#[derive(Debug)]
struct RunDirectory {
    base: PathBuf,
    path: PathBuf,
    keep: bool,
}

impl RunDirectory {
    fn create(keep: bool) -> Result<Self, RunnerError> {
        let base = env::temp_dir();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        for _ in 0..100 {
            let sequence = NEXT_RUN_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(
                "actionc-run-{}-{timestamp}-{sequence}",
                process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { base, path, keep }),
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(RunnerError::Io {
                        operation: "create run directory",
                        path,
                        source,
                    });
                }
            }
        }

        Err(RunnerError::configuration(
            "could not allocate a unique run directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RunDirectory {
    fn drop(&mut self) {
        if self.keep || self.path.parent() != Some(self.base.as_path()) {
            return;
        }
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            eprintln!(
                "warning: refusing to remove unexpected run path {}",
                self.path.display()
            );
            return;
        }
        if let Err(source) = fs::remove_dir_all(&self.path) {
            eprintln!(
                "warning: could not remove run directory {}: {source}",
                self.path.display()
            );
        }
    }
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
    if let Err(source) = install_temporary_file(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(RunnerError::Io {
            operation: "install output ATR",
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn install_temporary_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(windows)]
fn install_temporary_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    if !destination.exists() {
        return fs::rename(temporary, destination);
    }

    let file_name = destination
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("output.atr");
    let sequence = NEXT_RUN_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let backup = destination.with_file_name(format!(
        ".{file_name}.actionc-run-{}-{sequence}.bak",
        process::id()
    ));
    if backup.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("refusing to overwrite backup {}", backup.display()),
        ));
    }

    fs::rename(destination, &backup)?;
    if let Err(source) = fs::rename(temporary, destination) {
        if let Err(restore_error) = fs::rename(&backup, destination) {
            eprintln!(
                "warning: could not restore {} after failed replacement; original retained at {}: {restore_error}",
                destination.display(),
                backup.display()
            );
        }
        return Err(source);
    }
    if let Err(source) = fs::remove_file(&backup) {
        eprintln!(
            "warning: replaced {}, but could not remove backup {}: {source}",
            destination.display(),
            backup.display()
        );
    }
    Ok(())
}

fn print_help() {
    eprintln!(
        "usage: actionc-run [--mode compatibility|optimized|mir6502]\n\
         \x20                  [--emulator auto|atari800|altirra]\n\
         \x20                  [--emulator-path <path>]\n\
         \x20                  [--cart <path>|--no-cart]\n\
         \x20                  [--no-run] [--out-atr <file.atr>] [--keep]\n\
         \x20                  <file.act>\n\
         \x20     actionc-run --version"
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
    Emulator(EmulatorError),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    ProcessSpawn {
        kind: EmulatorKind,
        executable: PathBuf,
        source: io::Error,
    },
    ProcessExit {
        kind: EmulatorKind,
        executable: PathBuf,
        status: String,
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
            Self::Compile(_)
            | Self::Atr(_)
            | Self::Emulator(_)
            | Self::Io { .. }
            | Self::ProcessSpawn { .. }
            | Self::ProcessExit { .. } => 1,
        }
    }
}

impl From<EmulatorError> for RunnerError {
    fn from(error: EmulatorError) -> Self {
        Self::Emulator(error)
    }
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) | Self::Atr(message) => formatter.write_str(message),
            Self::Compile(error) => error.fmt(formatter),
            Self::Emulator(error) => error.fmt(formatter),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {}: {source}", path.display()),
            Self::ProcessSpawn {
                kind,
                executable,
                source,
            } => write!(
                formatter,
                "could not start {kind} at {}: {source}",
                executable.display()
            ),
            Self::ProcessExit {
                kind,
                executable,
                status,
            } => write!(
                formatter,
                "{kind} at {} exited unsuccessfully ({status})",
                executable.display()
            ),
        }
    }
}

impl Error for RunnerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Emulator(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::ProcessSpawn { source, .. } => Some(source),
            Self::Configuration(_) | Self::Atr(_) | Self::ProcessExit { .. } => None,
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
    fn action_library_bootstrap_is_a_load_file_that_returns_to_mydos() {
        assert_eq!(
            action_library_bootstrap_object(),
            vec![
                0xFF, 0xFF, // load-file marker
                0x00, 0x30, 0x08, 0x30, // $3000-$3008
                0xA0, 0x00, // LDY #$00
                0x8C, 0xC9, 0x04, // STY $04C9
                0x8C, 0x00, 0xD5, // STY $D500
                0x60, // RTS
                0xE2, 0x02, 0xE3, 0x02, // RUNAD
                0x00, 0x30, // $3000
            ]
        );
    }

    #[test]
    fn prepared_atr_runs_the_bootstrap_before_the_compiler_object() {
        let source = hello_world();
        let options = CompileOptions::for_mode(CompileMode::Compatibility);
        let compiled = compile_file(&source, &options).expect("compile expected object");

        let atr = prepare_atr(&source, &options, true).expect("prepare runnable ATR");
        let image = AtrImage::from_bytes(atr).expect("parse prepared ATR");

        assert_eq!(
            image
                .read_file_named(BOOTSTRAP_NAME)
                .expect("read prepared ATR")
                .expect("find BOOT.AR0"),
            action_library_bootstrap_object()
        );
        assert_eq!(
            image
                .read_file_named(PROGRAM_AFTER_BOOTSTRAP_NAME)
                .expect("read prepared ATR")
                .expect("find PROGRAM.AR1"),
            compiled.object_bytes()
        );
        assert!(
            image
                .read_file_named("AUTORUN.AR0")
                .expect("read prepared ATR")
                .is_none()
        );
    }

    #[test]
    fn prepared_atr_without_a_cartridge_runs_the_program_directly_as_ar0() {
        let source = hello_world();
        let options = CompileOptions::for_mode(CompileMode::Compatibility);
        let compiled = compile_file(&source, &options).expect("compile expected object");

        let atr = prepare_atr(&source, &options, false).expect("prepare runnable ATR");
        let image = AtrImage::from_bytes(atr).expect("parse prepared ATR");

        assert_eq!(
            image
                .read_file_named(DIRECT_PROGRAM_NAME)
                .expect("read prepared ATR")
                .expect("find PROGRAM.AR0"),
            compiled.object_bytes()
        );
        assert!(
            image
                .read_file_named(BOOTSTRAP_NAME)
                .expect("read prepared ATR")
                .is_none()
        );
        assert!(
            image
                .read_file_named(PROGRAM_AFTER_BOOTSTRAP_NAME)
                .expect("read prepared ATR")
                .is_none()
        );
    }

    #[test]
    fn parser_defaults_to_launching_with_auto_discovery_and_the_bundled_cart() {
        let options = parse_args([OsString::from("program.act")])
            .expect("parse default run")
            .expect("not help");

        assert!(!options.no_run);
        assert_eq!(options.emulator, EmulatorSelection::Auto);
        assert_eq!(options.cartridge, CartridgeChoice::Bundled);
        assert_eq!(options.output_atr, None);
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
        assert_eq!(options.output_atr, None);
        assert_eq!(default_atr_path(&options.source), PathBuf::from("demo.atr"));
        assert_eq!(options.compile.mode(), Some(CompileMode::Mir6502));
    }

    #[test]
    fn parser_maps_emulator_cart_and_retention_options() {
        let options = parse_args([
            OsString::from("--emulator=altirra"),
            OsString::from("--emulator-path"),
            OsString::from("C:/Program Files/Altirra/Altirra64.exe"),
            OsString::from("--cart"),
            OsString::from("C:/ROM images/action.car"),
            OsString::from("--out-atr=run image.atr"),
            OsString::from("--keep"),
            OsString::from("program.act"),
        ])
        .expect("parse emulator options")
        .expect("not help");

        assert_eq!(options.emulator, EmulatorSelection::Altirra);
        assert_eq!(
            options.emulator_path,
            Some(PathBuf::from("C:/Program Files/Altirra/Altirra64.exe"))
        );
        assert_eq!(
            options.cartridge,
            CartridgeChoice::File(PathBuf::from("C:/ROM images/action.car"))
        );
        assert_eq!(options.output_atr, Some(PathBuf::from("run image.atr")));
        assert!(options.keep);
    }

    #[test]
    fn parser_rejects_conflicting_cartridge_options() {
        let error = parse_args([
            OsString::from("--no-cart"),
            OsString::from("--cart=action.car"),
            OsString::from("program.act"),
        ])
        .expect_err("conflicting cartridge options should fail");

        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("--cart and --no-cart"));
    }

    #[test]
    fn no_cart_config_preserves_other_settings_and_clears_both_cartridges() {
        let source = "Atari 800 Emulator, Version 7.1.2\n\
ROM_XL/XE_CUSTOM=/roms/altirraos-xl.rom\n\
MACHINE_TYPE=Atari XL/XE\n\
CARTRIDGE_FILENAME=/roms/action.rom\n\
CARTRIDGE_TYPE=15\n";

        assert_eq!(
            sanitize_atari800_config(source),
            "Atari 800 Emulator, Version 7.1.2\n\
ROM_XL/XE_CUSTOM=/roms/altirraos-xl.rom\n\
MACHINE_TYPE=Atari XL/XE\n\
CARTRIDGE_FILENAME=\n\
CARTRIDGE_TYPE=0\n\
CARTRIDGE_PIGGYBACK_FILENAME=\n\
CARTRIDGE_PIGGYBACK_TYPE=0\n"
        );
    }
}
