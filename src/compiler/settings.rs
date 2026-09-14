//! Root-source defaults shared by compiler and CLI routing. Explicit caller
//! settings take precedence; these annotations never change language semantics.
use super::Backend;
use crate::{codegen::CodegenProfile, target::TargetId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendSetting {
    Atari(Backend),
    Mir68k,
}

#[derive(Debug, Default)]
pub(crate) struct SourceSettings {
    pub profile: Option<CodegenProfile>,
    pub backend: Option<BackendSetting>,
    pub target: Option<TargetId>,
}
impl SourceSettings {
    pub fn parse(source: &str) -> Self {
        let mut settings = Self::default();
        for line in source.lines() {
            let Some(annotation) = line.trim_start().strip_prefix(";@actionc") else {
                continue;
            };
            let words: Vec<_> = annotation
                .split_whitespace()
                .map(str::to_ascii_lowercase)
                .collect();
            let [key, value] = words.as_slice() else {
                continue;
            };
            match (key.as_str(), value.as_str()) {
                ("profile", "modern") => settings.profile = Some(CodegenProfile::Modern),
                ("backend", "classic") => {
                    settings.backend = Some(BackendSetting::Atari(Backend::Classic))
                }
                ("backend", "mir6502") => {
                    settings.backend = Some(BackendSetting::Atari(Backend::Mir6502))
                }
                ("backend", "mir68k") => settings.backend = Some(BackendSetting::Mir68k),
                ("target", value) => {
                    if let Ok(target) = value.parse() {
                        settings.target = Some(target)
                    }
                }
                _ => {}
            }
        }
        settings
    }
}
