use std::path::PathBuf;

use super::{CommandSpec, EmulatorAdapter, EmulatorError, EmulatorKind, LaunchRequest, os_string};

#[derive(Debug, Clone)]
pub(crate) struct AltirraAdapter {
    executable: PathBuf,
}

impl AltirraAdapter {
    pub(crate) fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl EmulatorAdapter for AltirraAdapter {
    fn kind(&self) -> EmulatorKind {
        EmulatorKind::Altirra
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
        if request.os_rom.is_some() {
            return Err(EmulatorError::adapter(
                self.kind(),
                "external OS ROM selection is not supported; Altirra uses its temporary profile and built-in AltirraOS",
            ));
        }

        let mut command = CommandSpec::new(&self.executable)
            .arg("/tempprofile")
            .arg("/hardware:800xl")
            .arg("/kernel:llexl")
            .arg("/nobasic");
        if let Some(cartridge) = request.cartridge {
            command = command.arg("/cart").arg(os_string(cartridge.as_os_str()));
        }

        Ok(command.arg("/disk").arg(os_string(request.atr.as_os_str())))
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

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
    fn uses_a_temporary_xl_profile_and_separate_media_arguments() {
        let adapter = AltirraAdapter::new("C:/Program Files/Altirra/Altirra64.exe");
        let atr = Path::new("C:/run images/program.atr");
        let cartridge = Path::new("C:/ROM images/Action cartridge.rom");

        let command = adapter
            .command(&request(atr, Some(cartridge), None))
            .expect("build Altirra command");

        assert_eq!(
            command.arguments(),
            &[
                OsString::from("/tempprofile"),
                OsString::from("/hardware:800xl"),
                OsString::from("/kernel:llexl"),
                OsString::from("/nobasic"),
                OsString::from("/cart"),
                cartridge.as_os_str().to_os_string(),
                OsString::from("/disk"),
                atr.as_os_str().to_os_string(),
            ]
        );
    }

    #[test]
    fn no_cart_does_not_emit_a_cart_switch() {
        let adapter = AltirraAdapter::new("Altirra.exe");
        let atr = Path::new("program.atr");

        let command = adapter
            .command(&request(atr, None, None))
            .expect("build no-cartridge Altirra command");

        assert_eq!(
            command.arguments(),
            &[
                OsString::from("/tempprofile"),
                OsString::from("/hardware:800xl"),
                OsString::from("/kernel:llexl"),
                OsString::from("/nobasic"),
                OsString::from("/disk"),
                atr.as_os_str().to_os_string(),
            ]
        );
    }

    #[test]
    fn rejects_an_external_os_rom_instead_of_silently_ignoring_it() {
        let adapter = AltirraAdapter::new("Altirra64.exe");
        let error = adapter
            .command(&request(
                Path::new("program.atr"),
                None,
                Some(Path::new("altirraos-xl.rom")),
            ))
            .expect_err("external OS ROM should be rejected");

        assert_eq!(error.kind(), Some(EmulatorKind::Altirra));
        assert!(error.message().contains("not supported"));
    }
}
