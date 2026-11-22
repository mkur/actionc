//! Reviewed entries into the SYSSTR reverse-copy loop.
use super::sys_cio::{linked_body_matches, linked_routine};
use super::sys_graphics::cart_matches;
use super::{CallFacts, DataRange};
use crate::ast::{FundType, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::SemRoutineSignature;
use crate::semantic::{ValueType, ValueTypeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StringRoutine {
    SCopyS,
    SAssign,
}

#[derive(Clone, Copy)]
pub(crate) struct StringArguments {
    destination: Option<u16>,
    source: Option<u16>,
    first: Option<u8>,
    last: Option<u8>,
}

impl StringArguments {
    pub(crate) fn from_arguments(mut argument: impl FnMut(usize) -> Option<u16>) -> Self {
        Self {
            destination: argument(0),
            source: argument(1),
            first: argument(2).and_then(|value| u8::try_from(value).ok()),
            last: argument(3).and_then(|value| u8::try_from(value).ok()),
        }
    }
}

#[derive(Clone, Copy)]
struct Span {
    start: u16,
    bytes: u16,
}

impl Span {
    fn addresses(self) -> impl Iterator<Item = u16> {
        (0..self.bytes).map(move |offset| self.start.wrapping_add(offset))
    }

    fn ranges<Id>(self) -> impl Iterator<Item = DataRange<Id>> {
        let end = u32::from(self.start) + u32::from(self.bytes) - 1;
        let range = |start, end| DataRange {
            start,
            end,
            global: None,
        };
        [
            Some(range(self.start, end.min(0xffff) as u16)),
            (end > 0xffff).then(|| range(0, end as u16)),
        ]
        .into_iter()
        .flatten()
    }
}

impl StringRoutine {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        match crate::runtime_bindings::binding_key(name).as_str() {
            "SYS.SCOPYS" => Some(Self::SCopyS),
            "SYS.SASSIGN" => Some(Self::SAssign),
            _ => None,
        }
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let string = ValueType {
            base: ValueTypeBase::Fund(FundType::Char),
            pointer: true,
        };
        let byte = ValueType {
            base: ValueTypeBase::Fund(FundType::Byte),
            pointer: false,
        };
        signature.kind == RoutineKind::Proc
            && signature.return_type.is_none()
            && signature.params == [string.clone(), string, byte.clone(), byte]
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        let (entry, bytes): (u16, &[u8]) = match self {
            Self::SCopyS => (
                0xa8af,
                &[
                    0x85, 0xa0, 0x86, 0xa1, 0x84, 0xa2, 0xa0, 0, 0xb1, 0xa2, 0xc5, 0xa5, 0xb0, 2,
                    0x85, 0xa5, 0xc6, 0xa4, 0x18, 0xa5, 0xa2, 0x65, 0xa4, 0x85, 0xa2, 0x90, 2,
                    0xe6, 0xa3, 0x38, 0xa5, 0xa5, 0xe5, 0xa4, 0xb0, 2, 0xa9, 0, 0x4c, 0xa2, 0xa8,
                ],
            ),
            Self::SAssign => (
                0xa8d8,
                &[
                    0x85, 0xa0, 0x86, 0xa1, 0x84, 0xa2, 0xa0, 0, 0xb1, 0xa2, 0xf0, 0x0d, 0x85,
                    0xa6, 0xc6, 0xa4, 0x38, 0xa5, 0xa5, 0xe5, 0xa4, 0xf0, 2, 0xb0, 1, 0x60, 0xaa,
                    0xc5, 0xa6, 0x90, 8, 0x18, 0xa5, 0xa6, 0xaa, 0x65, 0xa4, 0x85, 0xa5, 0xa5,
                    0xa5, 0xd1, 0xa0, 0x90, 3, 0x91, 0xa0, 0x18, 0xa5, 0xa0, 0x65, 0xa4, 0x85,
                    0xa0, 0x90, 2, 0xe6, 0xa1, 0x8a, 0x4c, 0xa6, 0xa8,
                ],
            ),
        };
        address == entry
            && cart_matches(entry, bytes)
            && cart_matches(
                0xa8a2,
                &[
                    0x91, 0xa0, 0xf0, 8, 0xa8, 0xb1, 0xa2, 0x91, 0xa0, 0x88, 0xd0, 0xf9, 0x60,
                ],
            )
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage, unit: &str, name: &str) -> bool {
        let (entry, words) = match self {
            Self::SCopyS => (
                "SCOPYS",
                "a085 a186 a284 a0 0 a2b1 a5c5 2b0 a585 a4c6 18 a2a5 a465 a285 290 a3e6 38 a5a5 a4e5 2b0 a9 0 4c @SCOPY+10",
            ),
            Self::SAssign => (
                "SASSIGN",
                "a085 a186 a284 a0 0 a2b1 df0 a685 a4c6 38 a5a5 a4e5 2f0 1b0 aa60 a6c5 890 18 a6a5 aa a465 a585 a5a5 a0d1 390 a091 18 a0a5 a465 a085 290 a1e6 4c8a @SCOPY+14",
            ),
        };
        unit == "SYSSTR"
            && entry.eq_ignore_ascii_case(name)
            && linked_routine(image, unit, entry)
                .is_some_and(|routine| self.matches_signature(&routine.signature))
            && linked_body_matches(image, unit, entry, words)
            && linked_body_matches(
                image,
                unit,
                "SCOPY",
                "a085 a186 a284 a0 0 a2b1 a091 8f0 a8 a2b1 a091 88 f9d0 60",
            )
    }

    /// Length is read before the copy. The caller's inclusive bounds limit
    /// the payload independently of any aliasing between the two buffers.
    fn spans(self, args: StringArguments) -> (Vec<Span>, Vec<Span>) {
        let (first, count) = match (args.first, args.last) {
            (Some(first), Some(last)) => (first, last.saturating_sub(first.wrapping_sub(1))),
            _ => (1, 255),
        };
        let mut reads = Vec::new();
        let mut writes = Vec::new();
        if let Some(source) = args.source {
            reads.push(Span {
                start: source,
                bytes: 1,
            });
            if count != 0 {
                let offset = if self == Self::SCopyS { first } else { 1 };
                reads.push(Span {
                    start: source.wrapping_add(u16::from(offset)),
                    bytes: u16::from(count),
                });
            }
        }
        if let Some(destination) = args.destination {
            if self == Self::SCopyS {
                writes.push(Span {
                    start: destination,
                    bytes: 1 + u16::from(count),
                });
            } else if count != 0 {
                let length = Span {
                    start: destination,
                    bytes: 1,
                };
                reads.push(length);
                writes.push(length);
                writes.push(Span {
                    start: destination.wrapping_add(u16::from(first)),
                    bytes: u16::from(count),
                });
            }
        }
        (reads, writes)
    }

    pub(crate) fn numeric_observations(
        self,
        args: StringArguments,
    ) -> impl Iterator<Item = (u16, u16)> {
        let (reads, writes) = self.spans(args);
        reads
            .into_iter()
            .chain(writes)
            .map(|span| (span.start, span.bytes))
    }

    pub(crate) fn facts<Id>(self, args: StringArguments, origin: Option<u16>) -> CallFacts<Id> {
        let Some(origin) = origin else {
            return CallFacts::default();
        };
        if args.destination.is_none()
            || args.source.is_none()
            || args.first.is_none()
            || args.last.is_none()
        {
            return CallFacts::default();
        }
        let (reads, writes) = self.spans(args);
        if reads
            .iter()
            .flat_map(|span| span.addresses())
            .any(|byte| (0xd500..=0xd5ff).contains(&byte))
            || writes.iter().flat_map(|span| span.addresses()).any(|byte| {
                byte >= origin
                    || byte >= 0xd000
                    || (0x0100..=0x01ff).contains(&byte)
                    || (0x00a0..=0x00a6).contains(&byte)
            })
        {
            return CallFacts::default();
        }
        let scratch = |start, end| DataRange {
            start,
            end,
            global: None,
        };
        let fixed_writes = match self {
            Self::SCopyS => vec![scratch(0xa0, 0xa5)],
            Self::SAssign => vec![scratch(0xa0, 0xa2), scratch(0xa4, 0xa6)],
        };
        CallFacts {
            abi_inputs_only: true,
            reads: Some(
                std::iter::once(scratch(0xa0, 0xa6))
                    .chain(reads.into_iter().flat_map(Span::ranges))
                    .collect(),
            ),
            writes: Some(
                fixed_writes
                    .into_iter()
                    .chain(writes.into_iter().flat_map(Span::ranges))
                    .collect(),
            ),
            ..CallFacts::default()
        }
    }
}
