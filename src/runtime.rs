use std::fmt;

/// Selects where compiler/runtime support implementations come from.
///
/// This is deliberately independent of the code-generation backend.  The
/// cartridge remains the compatibility default until standalone coverage is
/// complete enough for a separate default-change decision.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Runtime {
    #[default]
    ActionCart,
    Standalone,
}

impl Runtime {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActionCart => "cart",
            Self::Standalone => "standalone",
        }
    }
}

impl fmt::Display for Runtime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
