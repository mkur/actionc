//! Exact conversion between decimal source text and Atari six-byte floating
//! point values.
//!
//! The Atari format is base 100 with five packed-BCD mantissa bytes. Conversion
//! deliberately uses decimal digits only: routing through a host `f32` or `f64`
//! would introduce host-dependent double rounding.

use std::fmt;

const EXPONENT_BIAS: i32 = 64;
const MIN_BASE100_EXPONENT: i32 = -49;
const MAX_BASE100_EXPONENT: i32 = 49;
const MANTISSA_DIGITS: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtariReal([u8; 6]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtariRealInteger {
    pub value: i128,
    pub exact: bool,
}

impl AtariReal {
    pub const ZERO: Self = Self([0; 6]);

    pub const fn from_bytes(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    pub const fn to_bytes(self) -> [u8; 6] {
        self.0
    }

    pub const fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0[0] == 0
    }

    pub const fn is_negative(self) -> bool {
        !self.is_zero() && self.0[0] & 0x80 != 0
    }

    pub const fn negated(self) -> Self {
        if self.is_zero() {
            self
        } else {
            let mut bytes = self.0;
            bytes[0] ^= 0x80;
            Self(bytes)
        }
    }

    /// Return the integer selected by Atari FPI's round-to-nearest magnitude
    /// rule. `None` means the represented magnitude exceeds `i128`.
    pub fn rounded_integer(self) -> Option<AtariRealInteger> {
        if self.is_zero() {
            return Some(AtariRealInteger {
                value: 0,
                exact: true,
            });
        }
        let exponent = i32::from(self.0[0] & 0x7F) - EXPONENT_BIAS;
        let mantissa = self.0[1..].iter().fold(0u128, |value, byte| {
            value * 100 + u128::from(byte >> 4) * 10 + u128::from(byte & 0x0F)
        });
        let pair_scale = exponent - 4;
        let (magnitude, exact) = if pair_scale >= 0 {
            let factor = checked_pow_100(pair_scale as u32)?;
            (mantissa.checked_mul(factor)?, true)
        } else {
            let denominator_pairs = pair_scale.unsigned_abs();
            if denominator_pairs >= 6 {
                (0, false)
            } else {
                let denominator = checked_pow_100(denominator_pairs)?;
                let quotient = mantissa / denominator;
                let remainder = mantissa % denominator;
                (
                    quotient + u128::from(remainder.saturating_mul(2) >= denominator),
                    remainder == 0,
                )
            }
        };
        let magnitude = i128::try_from(magnitude).ok()?;
        Some(AtariRealInteger {
            value: if self.is_negative() {
                -magnitude
            } else {
                magnitude
            },
            exact,
        })
    }

    pub fn from_decimal(text: &str) -> Result<Self, ParseAtariRealError> {
        ParsedDecimal::parse(text)?.encode()
    }
}

fn checked_pow_100(exponent: u32) -> Option<u128> {
    (0..exponent).try_fold(1u128, |value, _| value.checked_mul(100))
}

impl fmt::Debug for AtariReal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AtariReal")
            .field(&HexBytes(&self.0))
            .finish()
    }
}

struct HexBytes<'a>(&'a [u8; 6]);

impl fmt::Debug for HexBytes<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[")?;
        for (index, byte) in self.0.iter().enumerate() {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{byte:02X}")?;
        }
        formatter.write_str("]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseAtariRealErrorKind {
    MissingDigits,
    MissingExponentDigits,
    ExponentHasMoreThanTwoDigits,
    UnexpectedCharacter,
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseAtariRealError {
    pub kind: ParseAtariRealErrorKind,
    /// Byte offset in the ASCII source spelling associated with the error.
    pub offset: usize,
}

impl ParseAtariRealError {
    const fn new(kind: ParseAtariRealErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }
}

impl fmt::Display for ParseAtariRealError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            ParseAtariRealErrorKind::MissingDigits => {
                "real constant must contain at least one decimal digit"
            }
            ParseAtariRealErrorKind::MissingExponentDigits => {
                "real constant exponent must contain decimal digits"
            }
            ParseAtariRealErrorKind::ExponentHasMoreThanTwoDigits => {
                "real constant exponent may contain at most two digits"
            }
            ParseAtariRealErrorKind::UnexpectedCharacter => "unexpected character in real constant",
            ParseAtariRealErrorKind::Overflow => "real constant exceeds the Atari FPP range",
        };
        write!(formatter, "{message} at byte {}", self.offset)
    }
}

impl std::error::Error for ParseAtariRealError {}

struct ParsedDecimal<'a> {
    negative: bool,
    significant_digits: &'a [u8],
    decimal_power: i32,
    error_offset: usize,
}

impl<'a> ParsedDecimal<'a> {
    fn parse(text: &'a str) -> Result<Self, ParseAtariRealError> {
        if !text.is_ascii() {
            let offset = text
                .char_indices()
                .find_map(|(offset, character)| (!character.is_ascii()).then_some(offset))
                .unwrap_or(0);
            return Err(ParseAtariRealError::new(
                ParseAtariRealErrorKind::UnexpectedCharacter,
                offset,
            ));
        }

        let bytes = text.as_bytes();
        let mut position = 0usize;
        let negative = match bytes.first().copied() {
            Some(b'+') => {
                position += 1;
                false
            }
            Some(b'-') => {
                position += 1;
                true
            }
            _ => false,
        };

        let integer_start = position;
        consume_digits(bytes, &mut position);
        let integer_digits = position - integer_start;

        let mut fractional_digits = 0usize;
        if bytes.get(position) == Some(&b'.') {
            position += 1;
            let fraction_start = position;
            consume_digits(bytes, &mut position);
            fractional_digits = position - fraction_start;
        }

        if integer_digits + fractional_digits == 0 {
            return Err(ParseAtariRealError::new(
                ParseAtariRealErrorKind::MissingDigits,
                position,
            ));
        }

        let significand_end = position;
        let mut explicit_exponent = 0i32;
        if matches!(bytes.get(position), Some(b'E' | b'e')) {
            position += 1;
            let exponent_negative = match bytes.get(position).copied() {
                Some(b'+') => {
                    position += 1;
                    false
                }
                Some(b'-') => {
                    position += 1;
                    true
                }
                _ => false,
            };
            let exponent_start = position;
            consume_digits(bytes, &mut position);
            let exponent_digits = position - exponent_start;
            if exponent_digits == 0 {
                return Err(ParseAtariRealError::new(
                    ParseAtariRealErrorKind::MissingExponentDigits,
                    position,
                ));
            }
            if exponent_digits > 2 {
                return Err(ParseAtariRealError::new(
                    ParseAtariRealErrorKind::ExponentHasMoreThanTwoDigits,
                    exponent_start + 2,
                ));
            }
            explicit_exponent = bytes[exponent_start..position]
                .iter()
                .fold(0i32, |value, digit| value * 10 + i32::from(digit - b'0'));
            if exponent_negative {
                explicit_exponent = -explicit_exponent;
            }
        }

        if position != bytes.len() {
            return Err(ParseAtariRealError::new(
                ParseAtariRealErrorKind::UnexpectedCharacter,
                position,
            ));
        }

        // The significant digit slice must omit the decimal point. Borrowing a
        // contiguous input slice is possible only for all-integer/all-fraction
        // spellings, so encoding reads both sides through a small digit cursor.
        let significant_digits = &bytes[integer_start..significand_end];
        let decimal_power = explicit_exponent - fractional_digits as i32;
        Ok(Self {
            negative,
            significant_digits,
            decimal_power,
            error_offset: text.len(),
        })
    }

    fn encode(self) -> Result<AtariReal, ParseAtariRealError> {
        let digits = self
            .significant_digits
            .iter()
            .copied()
            .filter(|byte| byte.is_ascii_digit())
            .collect::<Vec<_>>();
        let Some(first_nonzero) = digits.iter().position(|digit| *digit != b'0') else {
            return Ok(AtariReal::ZERO);
        };
        let digits = &digits[first_nonzero..];

        let scientific_power = digits.len() as i32 - 1 + self.decimal_power;
        let base100_exponent = scientific_power.div_euclid(2);
        if base100_exponent < MIN_BASE100_EXPONENT {
            return Ok(AtariReal::ZERO);
        }
        if base100_exponent > MAX_BASE100_EXPONENT {
            return Err(ParseAtariRealError::new(
                ParseAtariRealErrorKind::Overflow,
                self.error_offset,
            ));
        }

        let mut mantissa_digits = [0u8; MANTISSA_DIGITS];
        let mut destination = 0usize;
        if scientific_power.rem_euclid(2) == 0 {
            destination = 1;
        }
        for digit in digits.iter().take(MANTISSA_DIGITS - destination) {
            mantissa_digits[destination] = digit - b'0';
            destination += 1;
        }

        let mut bytes = [0u8; 6];
        bytes[0] = (base100_exponent + EXPONENT_BIAS) as u8;
        if self.negative {
            bytes[0] |= 0x80;
        }
        for (index, pair) in mantissa_digits.chunks_exact(2).enumerate() {
            bytes[index + 1] = pair[0] << 4 | pair[1];
        }
        Ok(AtariReal(bytes))
    }
}

fn consume_digits(bytes: &[u8], position: &mut usize) {
    while bytes.get(*position).is_some_and(u8::is_ascii_digit) {
        *position += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &str) -> [u8; 6] {
        AtariReal::from_decimal(text)
            .unwrap_or_else(|error| panic!("parse {text:?}: {error}"))
            .to_bytes()
    }

    #[test]
    fn matches_canonical_atari_fpp_oracle_vectors() {
        let cases = [
            ("0", [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            ("-0", [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            ("1", [0x40, 0x01, 0x00, 0x00, 0x00, 0x00]),
            ("-1", [0xc0, 0x01, 0x00, 0x00, 0x00, 0x00]),
            (".5", [0x3f, 0x50, 0x00, 0x00, 0x00, 0x00]),
            ("1.25", [0x40, 0x01, 0x25, 0x00, 0x00, 0x00]),
            ("10", [0x40, 0x10, 0x00, 0x00, 0x00, 0x00]),
            ("100", [0x41, 0x01, 0x00, 0x00, 0x00, 0x00]),
            ("1234567890", [0x44, 0x12, 0x34, 0x56, 0x78, 0x90]),
            ("9.999999999E97", [0x70, 0x99, 0x99, 0x99, 0x99, 0x99]),
            ("1E99", [0x71, 0x10, 0x00, 0x00, 0x00, 0x00]),
            ("1E-98", [0x0f, 0x01, 0x00, 0x00, 0x00, 0x00]),
            ("1E-99", [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        ];

        for (text, expected) in cases {
            assert_eq!(bytes(text), expected, "{text:?}");
        }
    }

    #[test]
    fn matches_afp_truncation_instead_of_host_rounding() {
        assert_eq!(bytes("1.2345678904"), [0x40, 0x01, 0x23, 0x45, 0x67, 0x89]);
        assert_eq!(bytes("1.2345678905"), [0x40, 0x01, 0x23, 0x45, 0x67, 0x89]);
        assert_eq!(bytes("1.234567895"), [0x40, 0x01, 0x23, 0x45, 0x67, 0x89]);
        assert_eq!(bytes("12.34567895"), [0x40, 0x12, 0x34, 0x56, 0x78, 0x95]);
        assert_eq!(bytes("12.345678956"), [0x40, 0x12, 0x34, 0x56, 0x78, 0x95]);
    }

    #[test]
    fn validates_the_entire_decimal_spelling() {
        let cases = [
            ("", ParseAtariRealErrorKind::MissingDigits),
            ("+", ParseAtariRealErrorKind::MissingDigits),
            (".", ParseAtariRealErrorKind::MissingDigits),
            ("1E", ParseAtariRealErrorKind::MissingExponentDigits),
            ("1E+", ParseAtariRealErrorKind::MissingExponentDigits),
            (
                "1E100",
                ParseAtariRealErrorKind::ExponentHasMoreThanTwoDigits,
            ),
            ("1X", ParseAtariRealErrorKind::UnexpectedCharacter),
            ("99E99", ParseAtariRealErrorKind::Overflow),
        ];

        for (text, expected) in cases {
            assert_eq!(
                AtariReal::from_decimal(text).unwrap_err().kind,
                expected,
                "{text:?}"
            );
        }
    }

    #[test]
    fn handles_decimal_point_and_exponent_without_binary_floating_point() {
        assert_eq!(bytes("0.00123"), [0x3e, 0x12, 0x30, 0x00, 0x00, 0x00]);
        assert_eq!(bytes("123e-3"), [0x3f, 0x12, 0x30, 0x00, 0x00, 0x00]);
        assert_eq!(bytes("+001.2300"), [0x40, 0x01, 0x23, 0x00, 0x00, 0x00]);
        assert!(AtariReal::from_decimal("-1").unwrap().is_negative());
        assert!(!AtariReal::ZERO.is_negative());
    }

    #[test]
    fn exposes_exact_integer_and_fpi_rounding_facts_without_host_float() {
        let cases = [
            ("0", 0, true),
            ("-123", -123, true),
            ("1.25", 1, false),
            ("1.5", 2, false),
            ("-1.5", -2, false),
            ("32768", 32_768, true),
            ("1E-98", 0, false),
        ];
        for (text, expected, exact) in cases {
            let integer = AtariReal::from_decimal(text)
                .expect("REAL")
                .rounded_integer()
                .expect("integer magnitude");
            assert_eq!((integer.value, integer.exact), (expected, exact), "{text}");
        }
        assert_eq!(
            AtariReal::from_decimal("1.5")
                .expect("REAL")
                .negated()
                .rounded_integer()
                .expect("integer")
                .value,
            -2
        );
        assert!(
            AtariReal::from_decimal("1E99")
                .expect("REAL")
                .rounded_integer()
                .is_none()
        );
    }
}
