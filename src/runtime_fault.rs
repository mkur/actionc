//! Target-independent reasons for compiler-generated, non-returning failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeFault {
    InvalidVariant,
}
