//! Reviewed Atari hardware calls. Reads remain observable even when their
//! result location and normal-return writes are known.
use super::sys_cio::{adjacent, linked_body_matches, linked_routine};
use super::sys_graphics::cart_matches;
use super::{CallFacts, DataRange};
use crate::ast::{FundType, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::SemRoutineSignature;
use crate::semantic::{ValueType, ValueTypeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HardwareRoutine {
    Paddle,
    PTrig,
    Stick,
    STrig,
    Rand,
    Sound,
    SndRst,
}

impl HardwareRoutine {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        match crate::runtime_bindings::binding_key(name).as_str() {
            "SYS.PADDLE" => Some(Self::Paddle),
            "SYS.PTRIG" => Some(Self::PTrig),
            "SYS.STICK" => Some(Self::Stick),
            "SYS.STRIG" => Some(Self::STrig),
            "SYS.RAND" => Some(Self::Rand),
            "SYS.SOUND" => Some(Self::Sound),
            "SYS.SNDRST" => Some(Self::SndRst),
            _ => None,
        }
    }

    pub(crate) fn byte_parameters(self) -> usize {
        match self {
            Self::Sound => 4,
            Self::SndRst => 0,
            _ => 1,
        }
    }

    pub(crate) fn returns_byte(self) -> bool {
        !matches!(self, Self::Sound | Self::SndRst)
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let byte = ValueType {
            base: ValueTypeBase::Fund(FundType::Byte),
            pointer: false,
        };
        signature.params.len() == self.byte_parameters()
            && signature.params.iter().all(|param| *param == byte)
            && if self.returns_byte() {
                matches!(signature.kind, RoutineKind::Func { .. })
                    && signature.return_type == Some(byte)
            } else {
                signature.kind == RoutineKind::Proc && signature.return_type.is_none()
            }
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        let (entry, bytes): (u16, &[u8]) = match self {
            Self::Paddle => (0xad37, &[0xaa, 0xbd, 0x70, 0x02, 0x85, 0xa0, 0x60]),
            Self::STrig => (0xad2f, &[0xaa, 0xbd, 0x10, 0xd0, 0x85, 0xa0, 0x60]),
            Self::PTrig => (
                0xa737,
                &[
                    0xa2, 0, 0xc9, 4, 0x30, 3, 0xe8, 0x29, 3, 0xa8, 0xbd, 0, 0xd3, 0x39, 0x4a,
                    0xa7, 0x85, 0xa0, 0x60, 4, 8, 0x40, 0x80,
                ],
            ),
            Self::Stick => (
                0xa74e,
                &[
                    0xa2, 0, 0xc9, 2, 0x30, 3, 0xe8, 0x29, 1, 0xa8, 0xbd, 0, 0xd3, 0x88, 0xd0, 4,
                    0x4a, 0x4a, 0x4a, 0x4a, 0x29, 0xf, 0x85, 0xa0, 0x60,
                ],
            ),
            Self::Rand => (
                0xa6f1,
                &[
                    0xae, 0x0a, 0xd2, 0xc9, 0, 0xf0, 9, 0x86, 0x84, 0xa2, 0, 0x86, 0x85, 0x20, 0,
                    0xa0, 0x86, 0xa0, 0x60,
                ],
            ),
            Self::Sound => (
                0xa704,
                &[
                    0x0a, 0x84, 0xa2, 0xa8, 0xc9, 7, 0x30, 5, 0xa0, 0x64, 0x20, 0xcb, 4, 0x8a,
                    0x99, 0, 0xd2, 0xa5, 0xa2, 0x0a, 0x0a, 0x0a, 0x0a, 0x05, 0xa3, 0x99, 1, 0xd2,
                    0x60,
                ],
            ),
            Self::SndRst => (
                0xa721,
                &[
                    0xad, 0x32, 2, 0x29, 0xef, 0x8d, 0x32, 2, 0x8d, 0x0f, 0xd2, 0xa9, 0, 0xa2, 8,
                    0x9d, 0, 0xd2, 0xca, 0x10, 0xfa, 0x60,
                ],
            ),
        };
        address == entry
            && cart_matches(entry, bytes)
            && (self != Self::Rand || cart_matches(0xa000, CART_MULTIPLY))
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage, unit: &str, name: &str) -> bool {
        let (entry, words) = match self {
            Self::Paddle => ("PADDLE", "bdaa 270 a085 60"),
            Self::STrig => ("STRIG", "bdaa d010 a085 60"),
            Self::PTrig => (
                "PTRIG",
                "a2 0 4c9 330 e8 329 a8 bd d300 39 *+5 a085 60 804 8040",
            ),
            Self::Stick => (
                "STICK",
                "a2 0 2c9 330 e8 129 bda8 d300 88 4d0 4a4a 4a4a f29 a085 60",
            ),
            Self::Rand => ("RAND", "ae d20a c9 0 9f0 8486 a2 0 8586 20 @MULTI a086 60"),
            Self::Sound => (
                "SOUND",
                "a a284 a8 7c9 530 64a0 20 @ERROR 998a d200 a2a5 a0a a0a a305 99 d201 60",
            ),
            Self::SndRst => (
                "SNDRST",
                "ad 232 ef29 8d 232 8d d20f a9 0 8a2 9d d200 ca fa10 60",
            ),
        };
        unit == "SYSMISC"
            && entry.eq_ignore_ascii_case(name)
            && linked_routine(image, unit, entry)
                .is_some_and(|routine| self.matches_signature(&routine.signature))
            && linked_body_matches(image, unit, entry, words)
            && (self != Self::Rand || linked_multiply_matches(image))
    }

    pub(crate) fn needs_argument_proof(self) -> bool {
        matches!(self, Self::PTrig | Self::Sound)
    }

    pub(crate) fn facts<Id>(self, input: Option<u8>) -> CallFacts<Id> {
        let range = |start, end| DataRange {
            start,
            end,
            global: None,
        };
        let supported = match self {
            // The signed branch in the legacy code can index past the mask
            // table for high port bytes, and Sound can invoke an error handler.
            Self::PTrig => input.is_some_and(|port| port <= 7),
            Self::Sound => input.is_some_and(|voice| voice <= 3),
            _ => true,
        };
        if !supported {
            return CallFacts::default();
        }
        let mut writes = if self.returns_byte() {
            vec![range(0xa0, 0xa0)]
        } else {
            Vec::new()
        };
        match self {
            Self::Rand if input != Some(0) => {
                // Union of the two reviewed multiplier workspaces.
                writes.extend([
                    range(0x82, 0x87),
                    range(0xc0, 0xc2),
                    range(0xc6, 0xc7),
                    range(0xd3, 0xd3),
                ]);
            }
            Self::Sound => {
                let base = 0xd200 + 2 * u16::from(input.expect("supported voice"));
                writes.extend([range(0xa2, 0xa2), range(base, base + 1)]);
            }
            Self::SndRst => writes.extend([
                range(0x0232, 0x0232),
                range(0xd200, 0xd208),
                range(0xd20f, 0xd20f),
            ]),
            _ => {}
        }
        let returns_a = matches!(self, Self::Paddle | Self::STrig | Self::PTrig | Self::Stick);
        let indexed = |base: u16| {
            let start = input.map_or(base, |index| base + u16::from(index));
            vec![range(start, input.map_or(base + 255, |_| start))]
        };
        let reads = match self {
            Self::Paddle => Some(indexed(0x0270)),
            Self::STrig => Some(indexed(0xd010)),
            Self::Stick => Some(vec![range(0xd300, 0xd301)]),
            // Its second read accesses the mask table relative to the entry.
            Self::PTrig => None,
            Self::Rand if input == Some(0) => Some(vec![range(0xd20a, 0xd20a)]),
            Self::Rand => Some(vec![
                range(0xd20a, 0xd20a),
                range(0x82, 0x87),
                range(0xc0, 0xc2),
                range(0xc6, 0xc7),
                range(0xd3, 0xd3),
            ]),
            Self::Sound => Some(vec![range(0xa2, 0xa3)]),
            Self::SndRst => Some(vec![range(0x0232, 0x0232)]),
        };
        CallFacts {
            abi_inputs_only: true,
            reads,
            writes: Some(writes),
            defines_on_return: self.returns_byte().then_some(0xa0).into_iter().collect(),
            returns_a_equals_a0: returns_a,
            preserves_regs: crate::ast::AnnotationRegisterSet {
                a: false,
                x: false,
                y: matches!(self, Self::Paddle | Self::STrig | Self::SndRst),
            },
            ..Default::default()
        }
    }
}

fn linked_multiply_matches(image: &RuntimeImage) -> bool {
    let matches = |name, words| linked_body_matches(image, "SYSLIB", name, words);
    matches(
        "MULTI",
        concat!(
            "20 @SMOPS 82a6 1bf0 c686 84a6 15f0 ca c786 8a2 a 8726 c606 690 c765 290 ",
            "87e6 ca f0d0 8685 82a5 85a6 20 @MULTB 83a5 84a6 20 @MULTB 4c @SETSIGN",
        ),
    ) && matches(
        "MULTB",
        "1bf0 ca c786 aa 15f0 c686 a9 0 8a2 a c606 290 c765 ca f6d0 18 8765 8785 86a5 87a6 60",
    ) && matches(
        "SMOPS",
        "d386 e0 0 310 20 @SS1 8285 8386 85a5 e10 aa d345 d385 84a5 20 @SS1 8485 8586 a9 0 8785 60",
    ) && matches("SETSIGN", "d3a4 1010")
        && matches("SS1", "8685 8786 38 a9 0 86e5 a8 a9 0 87e5 aa 98 60")
        && adjacent(image, "SETSIGN", "SS1")
}

const CART_MULTIPLY: &[u8] = &[
    0x20, 0x6c, 0xa0, 0xa6, 0x82, 0xf0, 0x1b, 0x86, 0xc0, 0xa6, 0x84, 0xf0, 0x15, 0xca, 0x86, 0xc1,
    0xa2, 0x08, 0x0a, 0x26, 0x87, 0x06, 0xc0, 0x90, 0x06, 0x65, 0xc1, 0x90, 0x02, 0xe6, 0x87, 0xca,
    0xd0, 0xf0, 0x85, 0x86, 0xa5, 0x82, 0xa6, 0x85, 0x20, 0x4a, 0xa0, 0xa5, 0x83, 0xa6, 0x84, 0x20,
    0x4a, 0xa0, 0xa4, 0xc2, 0x10, 0x13, 0x9d, 0x4a, 0xa0, 0x85, 0x86, 0x86, 0x87, 0x38, 0xa9, 0x00,
    0xe5, 0x86, 0xa8, 0xa9, 0x00, 0xe5, 0x87, 0xaa, 0x98, 0x60, 0xf0, 0x1b, 0xca, 0x86, 0xc1, 0xaa,
    0xf0, 0x15, 0x86, 0xc0, 0xa9, 0x00, 0xa2, 0x08, 0x0a, 0x06, 0xc0, 0x90, 0x02, 0x65, 0xc1, 0xca,
    0xd0, 0xf6, 0x18, 0x65, 0x87, 0x85, 0x87, 0xa5, 0x86, 0xa6, 0x87, 0x60, 0x86, 0xc2, 0xe0, 0x00,
    0x10, 0x03, 0x20, 0x39, 0xa0, 0x85, 0x82, 0x86, 0x83, 0xa5, 0x85, 0x10, 0x0e, 0xaa, 0x45, 0xc2,
    0x85, 0xc2, 0xa5, 0x84, 0x20, 0x39, 0xa0, 0x85, 0x84, 0x86, 0x85, 0xa9, 0x00, 0x85, 0x87, 0x60,
];
