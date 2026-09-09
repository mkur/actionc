//! Target-independent reasons for compiler-generated, non-returning failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuntimeFault {
    InvalidArgument,
    DivisionByZero,
    InvalidNumber,
    NumberOutOfRange,
    InputTruncated,
    InvalidVariantTag,
    InvalidVariantOverlap,
}

impl RuntimeFault {
    pub const ALL: [Self; 7] = [
        Self::InvalidArgument,
        Self::DivisionByZero,
        Self::InvalidNumber,
        Self::NumberOutOfRange,
        Self::InputTruncated,
        Self::InvalidVariantTag,
        Self::InvalidVariantOverlap,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::InvalidArgument => "InvalidArgument",
            Self::DivisionByZero => "DivisionByZero",
            Self::InvalidNumber => "InvalidNumber",
            Self::NumberOutOfRange => "NumberOutOfRange",
            Self::InputTruncated => "InputTruncated",
            Self::InvalidVariantTag => "InvalidVariantTag",
            Self::InvalidVariantOverlap => "InvalidVariantOverlap",
        }
    }
}
