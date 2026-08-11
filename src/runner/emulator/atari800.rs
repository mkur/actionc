use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{CommandSpec, EmulatorAdapter, EmulatorError, EmulatorKind, LaunchRequest, os_string};

#[derive(Debug, Clone)]
pub(crate) struct Atari800Adapter {
    executable: PathBuf,
    no_cart_config: Option<PathBuf>,
    extra_args: Vec<OsString>,
}

impl Atari800Adapter {
    pub(crate) fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            no_cart_config: None,
            extra_args: Vec::new(),
        }
    }

    pub(crate) fn with_no_cart_config(mut self, path: impl Into<PathBuf>) -> Self {
        self.no_cart_config = Some(path.into());
        self
    }

    pub(crate) fn with_extra_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.extra_args = args.into_iter().map(Into::into).collect();
        self
    }
}

impl EmulatorAdapter for Atari800Adapter {
    fn kind(&self) -> EmulatorKind {
        EmulatorKind::Atari800
    }

    fn executable(&self) -> &Path {
        &self.executable
    }

    fn command(&self, request: &LaunchRequest<'_>) -> Result<CommandSpec, EmulatorError> {
        if self.executable.as_os_str().is_empty() {
            return Err(EmulatorError::adapter(
                self.kind(),
                "emulator executable path is empty",
            ));
        }
        if request.atr.as_os_str().is_empty() {
            return Err(EmulatorError::adapter(
                self.kind(),
                "ATR image path is empty",
            ));
        }

        let mut command = CommandSpec::new(&self.executable);
        if request.cartridge.is_none()
            && let Some(config) = &self.no_cart_config
        {
            command = command
                .arg("-config")
                .arg(config)
                .arg("-no-autosave-config");
        }

        command = command.arg("-xl");
        if let Some(os_rom) = request.os_rom {
            command = command.arg("-xlxe_rom").arg(os_string(os_rom.as_os_str()));
        }
        if let Some(cartridge) = request.cartridge {
            command = command.arg("-cart").arg(os_string(cartridge.as_os_str()));
        }

        Ok(command
            .args(self.extra_args.iter().cloned())
            .arg(os_string(request.atr.as_os_str())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(
        atr: &'a Path,
        cartridge: Option<&'a Path>,
        os_rom: Option<&'a Path>,
    ) -> LaunchRequest<'a> {
        LaunchRequest {
            atr,
            cartridge,
            os_rom,
        }
    }

    #[test]
    fn attaches_the_os_cartridge_and_atr_to_an_xl_machine() {
        let adapter = Atari800Adapter::new("atari800");
        let atr = Path::new("/run images/program.atr");
        let cartridge = Path::new("/ROM images/Action cartridge.rom");
        let os_rom = Path::new("/ROM images/Altirra OS.rom");

        let command = adapter
            .command(&request(atr, Some(cartridge), Some(os_rom)))
            .expect("build Atari800 command");

        assert_eq!(command.executable(), Path::new("atari800"));
        assert_eq!(
            command.arguments(),
            &[
                OsString::from("-xl"),
                OsString::from("-xlxe_rom"),
                os_rom.as_os_str().to_os_string(),
                OsString::from("-cart"),
                cartridge.as_os_str().to_os_string(),
                atr.as_os_str().to_os_string(),
            ]
        );
    }

    #[test]
    fn no_cart_uses_a_sanitized_config_without_reattaching_a_saved_cart() {
        let adapter = Atari800Adapter::new("atari800")
            .with_no_cart_config("/run files/no cart.cfg")
            .with_extra_args(["-pal"]);
        let atr = Path::new("/run files/program.atr");
        let os_rom = Path::new("/roms/altirraos-xl.rom");

        let command = adapter
            .command(&request(atr, None, Some(os_rom)))
            .expect("build no-cartridge Atari800 command");

        assert_eq!(
            command.arguments(),
            &[
                OsString::from("-config"),
                OsString::from("/run files/no cart.cfg"),
                OsString::from("-no-autosave-config"),
                OsString::from("-xl"),
                OsString::from("-xlxe_rom"),
                os_rom.as_os_str().to_os_string(),
                OsString::from("-pal"),
                atr.as_os_str().to_os_string(),
            ]
        );
        assert!(!command.arguments().contains(&OsString::from("-cart")));
    }

    #[test]
    fn rejects_empty_executable_and_atr_paths() {
        let atr = Path::new("program.atr");
        let error = Atari800Adapter::new("")
            .command(&request(atr, None, None))
            .expect_err("empty executable should fail");
        assert_eq!(error.kind(), Some(EmulatorKind::Atari800));
        assert_eq!(error.message(), "emulator executable path is empty");

        let adapter = Atari800Adapter::new("atari800");
        let error = adapter
            .command(&request(Path::new(""), None, None))
            .expect_err("empty ATR should fail");
        assert_eq!(error.message(), "ATR image path is empty");
    }
}
