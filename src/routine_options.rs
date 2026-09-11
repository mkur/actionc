//! Source routine optimization preferences, independent of callable types and ABI.
use crate::source::Span;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InlinePreference {
    #[default]
    Auto,
    Prefer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InlineHint {
    pub preference: InlinePreference,
    /// Diagnostic metadata only; never executable semantics.
    pub span: Option<Span>,
}

impl InlineHint {
    pub fn requested(self) -> bool {
        self.preference == InlinePreference::Prefer
    }
}
