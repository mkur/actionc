//! Atari Error numbers, independent of the discriminants of RuntimeFault.
//! 100 retains the cartridge invalid-argument convention. 101..106 are
//! actionc extensions, below the CIO error range beginning at 128.
use crate::runtime_fault::RuntimeFault;

pub const fn code(fault: RuntimeFault) -> u8 {
    match fault {
        RuntimeFault::InvalidArgument => 100,
        RuntimeFault::DivisionByZero => 101,
        RuntimeFault::InvalidNumber => 102,
        RuntimeFault::NumberOutOfRange => 103,
        RuntimeFault::InputTruncated => 104,
        RuntimeFault::InvalidVariantTag => 105,
        RuntimeFault::InvalidVariantOverlap => 106,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_codes_are_stable_and_below_cio_errors() {
        assert_eq!(
            RuntimeFault::ALL.map(code),
            [100, 101, 102, 103, 104, 105, 106]
        );
    }
}
