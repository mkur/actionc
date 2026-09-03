//! Target-neutral facts attached to embedded foreign-code payloads.
//!
//! Instruction decoding and target relocation selection remain backend work.

use crate::target::{ByteSize, TargetId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignCodeMode {
    Analyzed,
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignRelocationEncoding {
    /// A complete address whose byte order is selected by the tagged target.
    Address { width: ByteSize },
    /// An unsigned integer that must fit without truncation.
    Unsigned { width: ByteSize },
    /// One byte selected using a convention owned by the tagged target.
    TargetByte { target: TargetId, byte_index: u8 },
}

impl ForeignRelocationEncoding {
    pub const fn width(self) -> ByteSize {
        match self {
            Self::Address { width } | Self::Unsigned { width } => width,
            Self::TargetByte { .. } => ByteSize::ONE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignSymbolUse {
    Address,
    Constant,
    Read,
    Write,
    ReadWrite,
    IndexedRead,
    IndexedWrite,
    IndexedReadWrite,
    Call,
    Control,
    PointerRead,
}
