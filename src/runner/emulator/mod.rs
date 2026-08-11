use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) mod altirra;
pub(crate) mod atari800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EmulatorKind {
    Atari800,
    Altirra,
}

impl EmulatorKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Atari800 => "Atari800",
            Self::Altirra => "Altirra",
        }
    }
}

impl fmt::Display for EmulatorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LaunchRequest<'a> {
    pub(crate) atr: &'a Path,
    pub(crate) cartridge: Option<&'a Path>,
    pub(crate) os_rom: Option<&'a Path>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    executable: PathBuf,
    args: Vec<OsString>,
}

impl CommandSpec {
    pub(crate) fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            args: Vec::new(),
        }
    }

    pub(crate) fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub(crate) fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub(crate) fn executable(&self) -> &Path {
        &self.executable
    }

    pub(crate) fn arguments(&self) -> &[OsString] {
        &self.args
    }

    pub(crate) fn to_command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.args(&self.args);
        command
    }
}

pub(crate) trait EmulatorAdapter {
    fn kind(&self) -> EmulatorKind;

    fn executable(&self) -> &Path;

    fn command(&self, request: &LaunchRequest<'_>) -> Result<CommandSpec, EmulatorError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmulatorError {
    kind: Option<EmulatorKind>,
    message: String,
}

impl EmulatorError {
    pub(crate) fn adapter(kind: EmulatorKind, message: impl Into<String>) -> Self {
        Self {
            kind: Some(kind),
            message: message.into(),
        }
    }

    pub(crate) fn discovery(message: impl Into<String>) -> Self {
        Self {
            kind: None,
            message: message.into(),
        }
    }

    pub(crate) fn kind(&self) -> Option<EmulatorKind> {
        self.kind
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for EmulatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(kind) = self.kind {
            write!(formatter, "{kind}: ")?;
        }
        formatter.write_str(&self.message)
    }
}

impl Error for EmulatorError {}

pub(crate) fn os_string(value: &OsStr) -> OsString {
    value.to_os_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestAdapter {
        executable: PathBuf,
    }

    impl EmulatorAdapter for TestAdapter {
        fn kind(&self) -> EmulatorKind {
            EmulatorKind::Atari800
        }

        fn executable(&self) -> &Path {
            &self.executable
        }

        fn command(&self, request: &LaunchRequest<'_>) -> Result<CommandSpec, EmulatorError> {
            let mut spec = CommandSpec::new(&self.executable)
                .arg("--disk")
                .arg(request.atr);
            if let Some(cartridge) = request.cartridge {
                spec = spec.arg("--cartridge").arg(cartridge);
            }
            if let Some(os_rom) = request.os_rom {
                spec = spec.arg("--os-rom").arg(os_rom);
            }
            Ok(spec)
        }
    }

    #[test]
    fn command_spec_preserves_paths_with_spaces_as_single_arguments() {
        let adapter = TestAdapter {
            executable: PathBuf::from("C:/Program Files/Emulator/emulator.exe"),
        };
        let atr = Path::new("C:/run images/hello world.atr");
        let cartridge = Path::new("C:/ROM images/Action cartridge.car");
        let os_rom = Path::new("C:/ROM images/Altirra OS.rom");

        let spec = adapter
            .command(&LaunchRequest {
                atr,
                cartridge: Some(cartridge),
                os_rom: Some(os_rom),
            })
            .expect("build test command");

        assert_eq!(spec.executable(), adapter.executable());
        assert_eq!(
            spec.arguments(),
            &[
                OsString::from("--disk"),
                atr.as_os_str().to_os_string(),
                OsString::from("--cartridge"),
                cartridge.as_os_str().to_os_string(),
                OsString::from("--os-rom"),
                os_rom.as_os_str().to_os_string(),
            ]
        );

        let command = spec.to_command();
        assert_eq!(command.get_program(), adapter.executable().as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            spec.arguments()
                .iter()
                .map(OsString::as_os_str)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn adapter_and_discovery_errors_retain_context() {
        let adapter = EmulatorError::adapter(EmulatorKind::Altirra, "missing disk image");
        assert_eq!(adapter.kind(), Some(EmulatorKind::Altirra));
        assert_eq!(adapter.message(), "missing disk image");
        assert_eq!(adapter.to_string(), "Altirra: missing disk image");

        let discovery = EmulatorError::discovery("no supported emulator found");
        assert_eq!(discovery.kind(), None);
        assert_eq!(discovery.to_string(), "no supported emulator found");
    }

    #[test]
    fn command_builder_appends_argument_iterators_without_shell_parsing() {
        let spec = CommandSpec::new("emulator")
            .args(["--first", "one value"])
            .arg(OsString::from("two value"));

        assert_eq!(
            spec.arguments(),
            &[
                OsString::from("--first"),
                OsString::from("one value"),
                OsString::from("two value"),
            ]
        );
    }
}
