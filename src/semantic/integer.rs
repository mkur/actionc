//! Target-independent integer division semantics. Storage width is not a
//! substitute for the operation's resolved type.
use super::types::ScalarType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticFault {
    DivisionByZero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DivMod {
    pub quotient: u16,
    pub remainder: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WideDivMod {
    pub quotient: u64,
    pub remainder: u64,
}

/// Fixed-width, target-independent arithmetic. Mask before checking the divisor;
/// use a wider host intermediate so MIN / -1 wraps without a host overflow.
pub fn divmod_bits(width: u8, signed: bool, left: u64, right: u64) -> Result<WideDivMod, ArithmeticFault> {
    assert!((1..=64).contains(&width));
    let mask = u64::MAX >> (64 - width);
    let number = |bits: u64| {
        let bits = bits & mask;
        if signed && bits & (1u64 << (width - 1)) != 0 {
            i128::from(bits) - (1i128 << width)
        } else {
            i128::from(bits)
        }
    };
    let (left, right) = (number(left), number(right));
    if right == 0 { return Err(ArithmeticFault::DivisionByZero); }
    Ok(WideDivMod {
        quotient: (left / right) as u64 & mask,
        remainder: (left % right) as u64 & mask,
    })
}

/// Operands have already undergone their source conversions. BYTE/CHAR are
/// always unsigned; CARD wins mixed promotion. INT_MIN/-1 wraps deliberately.
pub fn divmod(domain: ScalarType, left: u16, right: u16) -> Result<DivMod, ArithmeticFault> {
    let result = divmod_bits((domain.width_bytes() * 8) as u8,
        domain.signedness() == super::types::ScalarSignedness::Signed,
        u64::from(left), u64::from(right))?;
    Ok(DivMod {
        quotient: result.quotient as u16,
        remainder: result.remainder as u16,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustive_unsigned_bytes() {
        for a in 0..=255u16 {
            for b in 0..=255u16 {
                for ty in [ScalarType::Byte, ScalarType::Char] {
                    let actual = divmod(ty, a, b);
                    if b == 0 {
                        assert_eq!(actual, Err(ArithmeticFault::DivisionByZero));
                    } else {
                        let value = actual.unwrap();
                        assert_eq!(value.quotient, a / b);
                        assert_eq!(value.remainder, a - value.quotient * b);
                    }
                }
            }
        }
        assert_eq!(
            divmod(ScalarType::Byte, 1, 256),
            Err(ArithmeticFault::DivisionByZero)
        );
    }

    #[test]
    fn word_boundary_grid_and_signed_quadrants() {
        let words = [
            0u16, 1, 2, 3, 7, 127, 128, 255, 256, 257, 513, 32767, 32768, 40000, 50000, 65529,
            65533, 65534, 65535,
        ];
        for a in words {
            for b in words {
                for ty in [ScalarType::Card, ScalarType::Int] {
                    let actual = divmod(ty, a, b);
                    if b == 0 {
                        assert_eq!(actual, Err(ArithmeticFault::DivisionByZero));
                        continue;
                    }
                    let (a, b) = if ty == ScalarType::Int {
                        (i64::from(a as i16), i64::from(b as i16))
                    } else {
                        (i64::from(a), i64::from(b))
                    };
                    // Independent magnitude/sign oracle, not host signed / or %.
                    let magnitude = a.abs() / b.abs();
                    let q = if (a < 0) != (b < 0) {
                        -magnitude
                    } else {
                        magnitude
                    };
                    let r = a - q * b;
                    assert_eq!(
                        actual.unwrap(),
                        DivMod {
                            quotient: q as u16,
                            remainder: r as u16
                        }
                    );
                }
            }
        }
    }
}
