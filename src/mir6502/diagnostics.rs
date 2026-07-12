#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDiagnostic {
    pub routine: Option<String>,
    pub block: Option<String>,
    pub message: String,
}

impl MirDiagnostic {
    pub(super) fn routine(routine: &str, message: impl Into<String>) -> Self {
        Self {
            routine: Some(routine.to_string()),
            block: None,
            message: message.into(),
        }
    }

    pub(super) fn block(routine: &str, block: &str, message: impl Into<String>) -> Self {
        Self {
            routine: Some(routine.to_string()),
            block: Some(block.to_string()),
            message: message.into(),
        }
    }
}
