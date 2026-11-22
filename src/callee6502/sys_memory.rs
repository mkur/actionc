//! Reviewed numeric memory entries. Public result widths and physical
//! accesses differ for the two Peek entry names.
use super::sys_cio::{adjacent, linked_body_matches, linked_routine};
use super::sys_graphics::cart_matches;
use super::{CallFacts, DataRange};
use crate::ast::{FundType, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::SemRoutineSignature;
use crate::semantic::{ValueType, ValueTypeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryRoutine {
    Peek,
    PeekC,
    Poke,
    PokeC,
}

impl MemoryRoutine {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        match crate::runtime_bindings::binding_key(name).as_str() {
            "SYS.PEEK" => Some(Self::Peek),
            "SYS.PEEKC" => Some(Self::PeekC),
            "SYS.POKE" => Some(Self::Poke),
            "SYS.POKEC" => Some(Self::PokeC),
            _ => None,
        }
    }

    pub(crate) fn parameters(self) -> &'static [FundType] {
        use FundType::{Byte, Card};
        match self {
            Self::Peek | Self::PeekC => &[Card],
            Self::Poke => &[Card, Byte],
            Self::PokeC => &[Card, Card],
        }
    }

    pub(crate) fn result(self) -> Option<FundType> {
        match self {
            Self::Peek => Some(FundType::Byte),
            Self::PeekC => Some(FundType::Card),
            Self::Poke | Self::PokeC => None,
        }
    }

    fn is_read(self) -> bool {
        self.result().is_some()
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let scalar = |fund| ValueType {
            base: ValueTypeBase::Fund(fund),
            pointer: false,
        };
        signature
            .params
            .iter()
            .cloned()
            .eq(self.parameters().iter().copied().map(scalar))
            && signature.return_type == self.result().map(scalar)
            && matches!(signature.kind, RoutineKind::Func { .. }) == self.is_read()
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        let (entry, bytes): (u16, &[u8]) = match self {
            Self::Peek | Self::PeekC => (
                0xa767,
                &[
                    0x85, 0xa2, 0x86, 0xa3, 0xa0, 0, 0xb1, 0xa2, 0x85, 0xa0, 0xc8, 0xb1, 0xa2,
                    0x85, 0xa1, 0x60,
                ],
            ),
            Self::Poke => (
                0xa777,
                &[0x85, 0xa0, 0x86, 0xa1, 0x98, 0xa0, 0, 0x91, 0xa0, 0x60],
            ),
            Self::PokeC => (
                0xa781,
                &[0x20, 0x77, 0xa7, 0xc8, 0xa5, 0xa3, 0x91, 0xa0, 0x60],
            ),
        };
        address == entry
            && cart_matches(entry, bytes)
            && (self != Self::PokeC || Self::Poke.matches_cart(0xa777))
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage, unit: &str, name: &str) -> bool {
        let entry = match self {
            Self::Peek => "PEEK",
            Self::PeekC => "PEEKC",
            Self::Poke => "POKE",
            Self::PokeC => "POKEC",
        };
        if unit != "SYSMISC"
            || !entry.eq_ignore_ascii_case(name)
            || !linked_routine(image, unit, entry)
                .is_some_and(|routine| self.matches_signature(&routine.signature))
        {
            return false;
        }
        let matches = |name, words| linked_body_matches(image, unit, name, words);
        match self {
            Self::Peek => {
                linked_routine(image, unit, "PEEK").is_some_and(|routine| routine.body.is_empty())
                    && adjacent(image, "PEEK", "PEEKC")
                    && Self::PeekC.matches_linked(image, unit, "PEEKC")
            }
            Self::PeekC => matches("PEEKC", "a285 a386 a0 0 a2b1 a085 c8 a2b1 a185 60"),
            Self::Poke => matches("POKE", "a085 a186 a098 9100 60a0"),
            Self::PokeC => {
                matches("POKEC", "20 @POKE a5c8 91a3 60a0")
                    && Self::Poke.matches_linked(image, unit, "POKE")
            }
        }
    }

    pub(crate) fn access_bytes(self) -> u16 {
        if self == Self::Poke { 1 } else { 2 }
    }

    fn redirects_second_store(self, address: u16) -> bool {
        self == Self::PokeC && matches!(address, 0x00a0 | 0x00a1)
    }

    /// Numeric bytes the invocation can observe even when the call site
    /// cannot use a complete write footprint for residence.
    pub(crate) fn numeric_observations(
        self,
        address: u16,
        low_value: Option<u8>,
    ) -> Vec<(u16, u16)> {
        if !self.redirects_second_store(address) {
            return vec![(address, self.access_bytes())];
        }
        // The first store changes a byte of the pointer. The second uses
        // that changed pointer with Y=1.
        let second = |low: u8| {
            if address == 0x00a0 {
                u16::from(low) + 1
            } else {
                u16::from_le_bytes([0xa2, low])
            }
        };
        let mut observations = vec![(address, 1)];
        if let Some(low) = low_value {
            observations.push((second(low), 1));
        } else if address == 0x00a0 {
            observations.push((1, 256));
        } else {
            observations.extend((0..=255).map(|low| (second(low), 1)));
        }
        observations
    }

    pub(crate) fn facts<Id>(self, address: Option<u16>, origin: Option<u16>) -> CallFacts<Id> {
        let Some(address) = address else {
            return CallFacts::default();
        };
        let target_bytes = (0..self.access_bytes()).map(|offset| address.wrapping_add(offset));
        // Reading cartridge bank-control registers can change which code
        // executes. A write additionally needs a bound disjoint from loaded
        // code, stack, and hardware. Both providers use the same scope.
        let safe = if self.is_read() {
            target_bytes
                .clone()
                .all(|byte| !(0xd500..=0xd5ff).contains(&byte))
        } else {
            origin.is_some_and(|origin| {
                target_bytes.clone().all(|byte| {
                    byte < origin && byte < 0xd000 && !(0x0100..=0x01ff).contains(&byte)
                })
            }) && !self.redirects_second_store(address)
        };
        if !safe {
            return CallFacts::default();
        }

        let range = |start, end| DataRange {
            start,
            end,
            global: None,
        };
        let target_ranges = target_bytes.map(|byte| range(byte, byte));
        let (reads, writes) = if self.is_read() {
            let mut reads = vec![range(0xa2, 0xa3)];
            reads.extend(target_ranges);
            (reads, vec![range(0xa0, 0xa3)])
        } else {
            let mut writes = vec![range(0xa0, 0xa1)];
            writes.extend(target_ranges);
            let mut reads = vec![range(0xa0, 0xa1)];
            if self == Self::PokeC {
                reads.push(range(0xa3, 0xa3));
            }
            (reads, writes)
        };
        let result_bytes = match self {
            Self::Peek => 1,
            Self::PeekC => 2,
            Self::Poke | Self::PokeC => 0,
        };
        CallFacts {
            abi_inputs_only: true,
            reads: Some(reads),
            writes: Some(writes),
            defines_on_return: (0xa0..0xa0 + result_bytes).collect(),
            preserves_regs: crate::ast::AnnotationRegisterSet {
                a: false,
                x: true,
                y: false,
            },
            ..Default::default()
        }
    }
}
