//! Reviewed decimal conversion entries in SYSIO.

use super::sys_cio::{adjacent, linked_body_matches, linked_routine};
use super::sys_graphics::cart_matches;
use super::{CallFacts, DataRange};
use crate::ast::{FundType, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::SemRoutineSignature;
use crate::semantic::{ValueType, ValueTypeBase};

pub(crate) const FORMAT_STAGING: u16 = 0x0550;
/// The hardware stack and both documented math-pack RAM windows.
pub(crate) const FPP_ASCII_WORKSPACE: [(u16, u16); 3] =
    [(0x0100, 0x01ff), (0x057e, 0x05ff), (0x00d4, 0x00ff)];
/// Hardware stack, counted magnitude staging, and the two documented FPP RAM windows.
pub(crate) const FORMAT_WORKSPACE: [(u16, u16); 4] = [
    FPP_ASCII_WORKSPACE[0],
    (FORMAT_STAGING, FORMAT_STAGING + 5),
    FPP_ASCII_WORKSPACE[1],
    FPP_ASCII_WORKSPACE[2],
];

pub(super) fn cart_formatter_matches() -> bool {
    cart_matches(
        0xb10f,
        &[
            0x85, 0xd4, 0x86, 0xd5, 0x20, 0xaa, 0xd9, 0x20, 0xe6, 0xd8, 0xa0, 0xff, 0xa2, 0, 0xc8,
            0xe8, 0xb1, 0xf3, 0x9d, 0x50, 5, 0x10, 0xf7, 0x49, 0x80, 0x9d, 0x50, 5, 0x8e, 0x50, 5,
            0x60,
        ],
    )
}

pub(super) fn linked_formatter_matches(image: &RuntimeImage) -> bool {
    linked_body_matches(
        image,
        "SYSIO",
        "CTOSTR",
        "d485 d586 20 d9aa 20 d8e6 ffa0 a2 0 c8 e8 f3b1 9d 550 f710 8049 9d 550 8e 550 60",
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecimalRoutine {
    ValB,
    ValC,
    ValI,
    StrB,
    StrC,
    StrI,
}

impl DecimalRoutine {
    pub(crate) const PARSER_READ_BYTES: u16 = 256;

    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        match crate::runtime_bindings::binding_key(name).as_str() {
            "SYS.VALB" => Some(Self::ValB),
            "SYS.VALC" => Some(Self::ValC),
            "SYS.VALI" => Some(Self::ValI),
            "SYS.STRB" => Some(Self::StrB),
            "SYS.STRC" => Some(Self::StrC),
            "SYS.STRI" => Some(Self::StrI),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::ValB => "VALB",
            Self::ValC => "VALC",
            Self::ValI => "VALI",
            Self::StrB => "STRB",
            Self::StrC => "STRC",
            Self::StrI => "STRI",
        }
    }

    pub(crate) fn integer(self) -> FundType {
        match self {
            Self::ValB | Self::StrB => FundType::Byte,
            Self::ValC | Self::StrC => FundType::Card,
            Self::ValI | Self::StrI => FundType::Int,
        }
    }

    pub(crate) fn parses(self) -> bool {
        matches!(self, Self::ValB | Self::ValC | Self::ValI)
    }

    pub(crate) fn linked_aliases(self) -> &'static [Self] {
        match self {
            Self::ValB => &[Self::ValB, Self::ValC],
            Self::ValC => &[Self::ValC],
            _ => &[],
        }
    }

    pub(crate) fn fpp_services(self) -> Option<[u16; 2]> {
        (!self.parses()).then_some([0xd9aa, 0xd8e6])
    }

    pub(crate) fn buffer_argument(self) -> usize {
        usize::from(!self.parses())
    }

    pub(crate) fn buffer_byte_offset(self) -> u16 {
        match self {
            Self::StrB => 1,
            Self::StrC | Self::StrI => 2,
            _ => 0,
        }
    }

    pub(crate) fn buffer_bytes(self) -> u16 {
        match self {
            Self::StrB => 4,
            Self::StrC => 6,
            Self::StrI => 7,
            _ => Self::PARSER_READ_BYTES,
        }
    }

    pub(crate) fn may_retarget_destination(self, address: u16) -> bool {
        !self.parses()
            && [0xa2u16, 0xa3]
                .into_iter()
                .any(|byte| byte.wrapping_sub(address) < self.buffer_bytes())
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let string = ValueType {
            base: ValueTypeBase::Fund(FundType::Char),
            pointer: true,
        };
        let integer = ValueType {
            base: ValueTypeBase::Fund(self.integer()),
            pointer: false,
        };
        if self.parses() {
            matches!(signature.kind, RoutineKind::Func { .. })
                && signature.return_type == Some(integer)
                && signature.params == [string]
        } else {
            signature.kind == RoutineKind::Proc
                && signature.return_type.is_none()
                && signature.params == [integer, string]
        }
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        const VAL: &[u8] = &[
            0x85, 0xa4, 0x86, 0xa5, 0xa0, 0, 0x84, 0xa0, 0x84, 0xa1, 0x84, 0xa2, 0xb1, 0xa4, 0x85,
            0xa3, 0xe6, 0xa3, 0xa9, 0x20, 0xc8, 0xd1, 0xa4, 0xd0, 5, 0xc8, 0xc4, 0xa3, 0x30, 0xf7,
            0xb1, 0xa4, 0xc9, 0x2d, 0xd0, 3, 0x85, 0xa2, 0xc8, 0xc4, 0xa3, 0x10, 0x36, 0xb1, 0xa4,
            0xc9, 0x30, 0x30, 0x30, 0xc9, 0x3a, 0x10, 0x2c, 0x38, 0xe9, 0x30, 0xaa, 0xa5, 0xa1,
            0x48, 0xa5, 0xa0, 0x0a, 0x26, 0xa1, 0x0a, 0x26, 0xa1, 0x18, 0x65, 0xa0, 0x85, 0xa0,
            0x68, 0x65, 0xa1, 0x85, 0xa1, 0x06, 0xa0, 0x26, 0xa1, 0x18, 0x8a, 0x65, 0xa0, 0x85,
            0xa0, 0x90, 2, 0xe6, 0xa1, 0xc8, 0xc4, 0xa3, 0x30, 0xca, 0xa5, 0xa2, 0xf0, 0x0d, 0x38,
            0xa9, 0, 0xe5, 0xa0, 0x85, 0xa0, 0xa9, 0, 0xe5, 0xa1, 0x85, 0xa1, 0x60,
        ];
        if self.parses() {
            return address == 0xa59a && cart_matches(address, VAL);
        }
        let entry = match self {
            Self::StrB => 0xa544,
            Self::StrC => 0xa54c,
            Self::StrI => 0xa55b,
            _ => unreachable!(),
        };
        address == entry
            && cart_formatter_matches()
            && cart_matches(
                0xa54c,
                &[
                    0x84, 0xa2, 0x20, 0x0f, 0xb1, 0xc8, 0xb9, 0x50, 5, 0x91, 0xa2, 0x88, 0x10,
                    0xf8, 0x60,
                ],
            )
            && match self {
                Self::StrB => cart_matches(0xa544, &[0x86, 0xa2, 0x84, 0xa3, 0xa2, 0, 0xa4, 0xa2]),
                Self::StrI => cart_matches(
                    0xa55b,
                    &[
                        0xe0, 0, 0x10, 0xed, 0x85, 0xa0, 0x86, 0xa1, 0x84, 0xa2, 0x38, 0xa9, 0,
                        0xe5, 0xa0, 0xa8, 0xa9, 0, 0xe5, 0xa1, 0xaa, 0x98, 0x20, 0x0f, 0xb1, 0xe8,
                        0x8a, 0xa8, 0xb9, 0x4f, 5, 0x91, 0xa2, 0x88, 0xd0, 0xf8, 0x8a, 0x91, 0xa2,
                        0xc8, 0xa9, 0x2d, 0x91, 0xa2, 0x60,
                    ],
                ),
                _ => true,
            }
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage, unit: &str, name: &str) -> bool {
        const VAL_WORDS: &str = "a485 a586 a0 0 a084 a184 a284 a4b1 a385 a3e6 20a9 c8 a4d1 5d0 c8 a3c4 f730 a4b1 2dc9 3d0 a285 c8 a3c4 3610 a4b1 30c9 3030 3ac9 2c10 38 30e9 aa a1a5 48 a0a5 a a126 a a126 18 a065 a085 68 a165 a185 a006 a126 18 8a a065 a085 290 a1e6 c8 a3c4 ca30 a2a5 df0 38 a9 0 a0e5 a085 a9 0 a1e5 a185 60";
        if unit != "SYSIO" || !self.name().eq_ignore_ascii_case(name) {
            return false;
        }
        let signature_matches = |routine: Self| {
            linked_routine(image, unit, routine.name())
                .is_some_and(|body| routine.matches_signature(&body.signature))
        };
        let matches = |name, words| linked_body_matches(image, unit, name, words);
        if self.parses() {
            return signature_matches(Self::ValI)
                && matches("VALI", VAL_WORDS)
                && self.linked_aliases().iter().all(|&alias| {
                    let next = if alias == Self::ValB {
                        Self::ValC
                    } else {
                        Self::ValI
                    };
                    signature_matches(alias)
                        && linked_routine(image, unit, alias.name())
                            .is_some_and(|routine| routine.body.is_empty())
                        && adjacent(image, alias.name(), next.name())
                });
        }
        signature_matches(self)
            && signature_matches(Self::StrC)
            && linked_formatter_matches(image)
            && matches("STRC", "a284 20 @CTOSTR c8 b9 550 a291 88 f810 60")
            && match self {
                Self::StrB => {
                    adjacent(image, "STRB", "STRC") && matches("STRB", "a286 a384 a2 0 a2a4")
                }
                Self::StrI => {
                    adjacent(image, "STRC", "STRI")
                        && matches(
                            "STRI",
                            "e0 0 ed10 a085 a186 a284 38 a9 0 a0e5 a8 a9 0 a1e5 aa 98 20 @CTOSTR e8 8a a8 b9 54f a291 88 f8d0 8a a291 c8 2da9 a291 60",
                        )
                }
                _ => true,
            }
    }

    pub(crate) fn facts<Id>(self, buffer: Option<u16>, origin: Option<u16>) -> CallFacts<Id> {
        let Some(buffer) = buffer else {
            return CallFacts::default();
        };
        if !self.parses() {
            return self.format_facts(buffer, origin);
        }
        // Y always addresses one 256-byte window. The parser may inspect the
        // first payload byte even for an empty string. An explicit read of the
        // cartridge bank-control page makes the return path uncertain.
        if (0..Self::PARSER_READ_BYTES)
            .any(|offset| (0xd500..=0xd5ff).contains(&buffer.wrapping_add(offset)))
        {
            return CallFacts::default();
        }
        let range = |start, end| DataRange {
            start,
            end,
            global: None,
        };
        let end = u32::from(buffer) + u32::from(Self::PARSER_READ_BYTES) - 1;
        let mut reads = vec![
            range(0xa0, 0xa5),
            range(0x0100, 0x01ff),
            range(buffer, end.min(0xffff) as u16),
        ];
        if end > 0xffff {
            reads.push(range(0, end as u16));
        }
        CallFacts {
            abi_inputs_only: true,
            // All entries clear and compute both bytes. Only the declared
            // result width of the public interface is exposed to its callers.
            defines_on_return: (0xa0..=if self == Self::ValB { 0xa0 } else { 0xa1 }).collect(),
            reads: Some(reads),
            // The digit loop also uses a balanced PHA/PLA at the caller's
            // unknown stack depth. That temporary byte is still observable.
            writes: Some(vec![range(0xa0, 0xa5), range(0x0100, 0x01ff)]),
            ..CallFacts::default()
        }
    }

    fn format_facts<Id>(self, destination: u16, origin: Option<u16>) -> CallFacts<Id> {
        let Some(end) = destination.checked_add(self.buffer_bytes() - 1) else {
            return CallFacts::default();
        };
        let disjoint = |start, last| destination > last || end < start;
        // Reject a destination that can overwrite the pointer, staging,
        // executing code, hardware, or the math-pack workspace itself.
        if !origin.is_some_and(|origin| origin > 0x05ff && end < origin && end < 0xd000)
            || !disjoint(0xa0, 0xa3)
            || !FORMAT_WORKSPACE
                .iter()
                .all(|&(start, last)| disjoint(start, last))
        {
            return CallFacts::default();
        }
        let range = |start, end| DataRange {
            start,
            end,
            global: None,
        };
        let workspace = || {
            FORMAT_WORKSPACE
                .iter()
                .map(|&(start, end)| range(start, end))
        };
        let scratch = match self {
            Self::StrB => range(0xa2, 0xa3),
            Self::StrC => range(0xa2, 0xa2),
            Self::StrI => range(0xa0, 0xa2),
            _ => unreachable!(),
        };
        CallFacts {
            abi_inputs_only: true,
            reads: Some(
                std::iter::once(range(0xa0, 0xa3))
                    .chain(workspace())
                    .collect(),
            ),
            writes: Some(
                [scratch, range(destination, end)]
                    .into_iter()
                    .chain(workspace())
                    .collect(),
            ),
            ..CallFacts::default()
        }
    }
}
