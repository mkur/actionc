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

/// Operands have already undergone their source conversions. BYTE/CHAR are
/// always unsigned; CARD wins mixed promotion. INT_MIN/-1 wraps deliberately.
pub fn divmod(domain: ScalarType, left: u16, right: u16) -> Result<DivMod, ArithmeticFault> {
    let (left, right) = match domain {
        ScalarType::Byte | ScalarType::Char => (i32::from(left as u8), i32::from(right as u8)),
        ScalarType::Card => (i32::from(left), i32::from(right)),
        ScalarType::Int => (i32::from(left as i16), i32::from(right as i16)),
    };
    if right == 0 {
        return Err(ArithmeticFault::DivisionByZero);
    }
    // i32 intermediates also represent +32768, avoiding host overflow in the
    // one signed overflow case. Rust division truncates toward zero.
    Ok(DivMod {
        quotient: (left / right) as u16,
        remainder: (left % right) as u16,
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
