//! Reviewed numeric input entries, from record setup to the decimal parser.

use super::sys_cio::{CioDevice, CioRoutine, adjacent, linked_body_matches, linked_routine};
use super::sys_decimal::DecimalRoutine;
use super::sys_graphics::cart_matches;
use crate::ast::{FundType, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::SemRoutineSignature;
use crate::semantic::{ValueType, ValueTypeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericInput {
    InputB,
    InputC,
    InputI,
    InputBD,
    InputCD,
    InputID,
}

impl NumericInput {
    pub(crate) const BUFFER: u16 = 0x0550;
    pub(crate) const MAXIMUM: u8 = 19;
    // This is also the order of the linked fallthrough entries.
    const ENTRY_CHAIN: [Self; 6] = [
        Self::InputB,
        Self::InputC,
        Self::InputI,
        Self::InputBD,
        Self::InputCD,
        Self::InputID,
    ];

    pub(crate) fn for_interface(key: &str) -> Option<Self> {
        match key {
            "SYS.INPUTB" => Some(Self::InputB),
            "SYS.INPUTC" => Some(Self::InputC),
            "SYS.INPUTI" => Some(Self::InputI),
            "SYS.INPUTBD" => Some(Self::InputBD),
            "SYS.INPUTCD" => Some(Self::InputCD),
            "SYS.INPUTID" => Some(Self::InputID),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::InputB => "INPUTB",
            Self::InputC => "INPUTC",
            Self::InputI => "INPUTI",
            Self::InputBD => "INPUTBD",
            Self::InputCD => "INPUTCD",
            Self::InputID => "INPUTID",
        }
    }

    pub(crate) fn device(self) -> CioDevice {
        match self {
            Self::InputB | Self::InputC | Self::InputI => CioDevice::Current,
            Self::InputBD | Self::InputCD | Self::InputID => CioDevice::Argument(0),
        }
    }

    fn integer(self) -> FundType {
        match self {
            Self::InputB | Self::InputBD => FundType::Byte,
            Self::InputC | Self::InputCD => FundType::Card,
            Self::InputI | Self::InputID => FundType::Int,
        }
    }

    pub(crate) fn successful_result(self) -> &'static [u8] {
        if self.integer() == FundType::Byte {
            &[0xa0]
        } else {
            &[0xa0, 0xa1]
        }
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let scalar = |fund| ValueType {
            base: ValueTypeBase::Fund(fund),
            pointer: false,
        };
        matches!(signature.kind, RoutineKind::Func { .. })
            && signature.return_type == Some(scalar(self.integer()))
            && match self.device() {
                CioDevice::Current => signature.params.is_empty(),
                CioDevice::Argument(_) => signature.params == [scalar(FundType::Byte)],
            }
    }

    fn linked_entry_chain(self) -> &'static [Self] {
        &Self::ENTRY_CHAIN[self as usize..]
    }

    pub(crate) fn linked_abi_only_aliases(self) -> impl Iterator<Item = &'static str> {
        self.linked_entry_chain()
            .iter()
            .filter(|entry| matches!(entry, Self::InputBD | Self::InputCD))
            .map(|entry| entry.name())
            .chain(
                DecimalRoutine::ValB
                    .linked_aliases()
                    .iter()
                    .map(|entry| entry.name()),
            )
    }

    pub(crate) fn address(self) -> u16 {
        if self.device() == CioDevice::Current {
            0xa588
        } else {
            0xa58a
        }
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        address == self.address()
            && (self.device() != CioDevice::Current || cart_matches(0xa588, &[0xa5, 0xb7]))
            && cart_matches(
                0xa58a,
                &[
                    0xa2, 0x13, 0x8e, 0x50, 0x05, 0xa2, 0x50, 0xa0, 0x05, 0x20, 0xa7, 0xa4, 0xa9,
                    0x50, 0xa2, 0x05,
                ],
            )
            && CioRoutine::InputD.matches_cart(CioRoutine::InputD.address())
            && DecimalRoutine::ValI.matches_cart(0xa59a)
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage) -> bool {
        let unit = "SYSIO";
        let chain = self.linked_entry_chain();
        chain.iter().all(|&entry| {
            linked_routine(image, unit, entry.name()).is_some_and(|body| {
                entry.matches_signature(&body.signature)
                    && match entry {
                        Self::InputI => {
                            linked_body_matches(image, unit, entry.name(), "a5 @DEVICE")
                        }
                        Self::InputID => linked_body_matches(
                            image,
                            unit,
                            entry.name(),
                            "13a2 8e 550 50a2 5a0 20 @INPUTD 50a9 5a2",
                        ),
                        _ => body.body.is_empty(),
                    }
            })
        }) && chain
            .windows(2)
            .all(|entries| adjacent(image, entries[0].name(), entries[1].name()))
            && adjacent(image, "INPUTID", "VALB")
            && CioRoutine::InputD.matches_linked(image, unit, "INPUTD")
            && DecimalRoutine::ValB.matches_linked(image, unit, "VALB")
    }
}
