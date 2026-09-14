//! Bare MC68000 fault transport, independent of Atari Error numbers.
use crate::runtime_fault::RuntimeFault;
pub const FAULT_TRAP: u8 = 14;
pub const fn fault_code(reason: RuntimeFault) -> u32 {
    match reason {
        RuntimeFault::InvalidArgument => 1,
        RuntimeFault::DivisionByZero => 2,
        RuntimeFault::InvalidNumber => 3,
        RuntimeFault::NumberOutOfRange => 4,
        RuntimeFault::InputTruncated => 5,
        RuntimeFault::InvalidVariantTag => 6,
        RuntimeFault::InvalidVariantOverlap => 7,
    }
}
pub fn decode_fault(code: u32) -> Option<RuntimeFault> {
    RuntimeFault::ALL
        .into_iter()
        .find(|reason| fault_code(*reason) == code)
}
