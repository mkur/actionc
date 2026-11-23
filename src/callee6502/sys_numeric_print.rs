//! Reviewed integer output from the public entries to the formatter and CIO.

use super::sys_cio::{
    CioDevice, CioRoutine, adjacent, cart_output_dispatch_matches, cart_put_character_matches,
    linked_body_matches, linked_output_dispatch_matches, linked_put_character_matches,
    linked_routine,
};
use super::sys_decimal::{cart_formatter_matches, linked_formatter_matches};
use super::sys_graphics::cart_matches;
use crate::ast::{FundType, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::SemRoutineSignature;
use crate::semantic::{ValueType, ValueTypeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericPrint {
    PrintB,
    PrintC,
    PrintI,
    PrintBE,
    PrintCE,
    PrintIE,
    PrintBD,
    PrintCD,
    PrintID,
    PrintBDE,
    PrintCDE,
    PrintIDE,
}

impl NumericPrint {
    pub(crate) fn for_interface(key: &str) -> Option<Self> {
        use NumericPrint::*;
        match key {
            "SYS.PRINTB" => Some(PrintB),
            "SYS.PRINTC" => Some(PrintC),
            "SYS.PRINTI" => Some(PrintI),
            "SYS.PRINTBE" => Some(PrintBE),
            "SYS.PRINTCE" => Some(PrintCE),
            "SYS.PRINTIE" => Some(PrintIE),
            "SYS.PRINTBD" => Some(PrintBD),
            "SYS.PRINTCD" => Some(PrintCD),
            "SYS.PRINTID" => Some(PrintID),
            "SYS.PRINTBDE" => Some(PrintBDE),
            "SYS.PRINTCDE" => Some(PrintCDE),
            "SYS.PRINTIDE" => Some(PrintIDE),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        self.entry().0
    }

    pub(crate) fn address(self) -> u16 {
        self.entry().1
    }

    pub(crate) fn integer(self) -> FundType {
        use NumericPrint::*;
        match self {
            PrintB | PrintBE | PrintBD | PrintBDE => FundType::Byte,
            PrintC | PrintCE | PrintCD | PrintCDE => FundType::Card,
            PrintI | PrintIE | PrintID | PrintIDE => FundType::Int,
        }
    }

    pub(crate) fn device(self) -> CioDevice {
        use NumericPrint::*;
        match self {
            PrintBD | PrintCD | PrintID | PrintBDE | PrintCDE | PrintIDE => CioDevice::Argument(0),
            _ => CioDevice::Current,
        }
    }

    pub(crate) fn ends_line(self) -> bool {
        use NumericPrint::*;
        matches!(
            self,
            PrintBE | PrintCE | PrintIE | PrintBDE | PrintCDE | PrintIDE
        )
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let scalar = |fund| ValueType {
            base: ValueTypeBase::Fund(fund),
            pointer: false,
        };
        let integer = scalar(self.integer());
        signature.kind == RoutineKind::Proc
            && signature.return_type.is_none()
            && match self.device() {
                CioDevice::Current => signature.params == [integer],
                CioDevice::Argument(_) => signature.params == [scalar(FundType::Byte), integer],
            }
    }

    // Each entry describes the cartridge bytes and the equivalent linked block
    // before verifying any further fallthrough, jump, or call dependency.
    fn entry(self) -> (&'static str, u16, &'static [u8], &'static str) {
        use NumericPrint::*;
        match self {
            PrintB => ("PRINTB", 0xa4e4, &[0xa2, 0], "a2 0"),
            PrintC => (
                "PRINTC",
                0xa4e6,
                &[0x20, 0x48, 0xb1, 0x4c, 0xa5, 0xa3],
                "20 @CTOSTR a5 @DEVICE",
            ),
            PrintBE => ("PRINTBE", 0xa4ec, &[0xa2, 0], "a2 0"),
            PrintCE => (
                "PRINTCE",
                0xa4ee,
                &[0x20, 0xe6, 0xa4, 0x4c, 0xcc, 0xa4],
                "20 @PRINTC 4c @PUTE",
            ),
            PrintBD => ("PRINTBD", 0xa4f4, &[0xa0, 0], "a0 0"),
            PrintCD => (
                "PRINTCD",
                0xa4f6,
                &[
                    0x85, 0xa0, 0x8a, 0x84, 0xa2, 0xa6, 0xa2, 0x20, 0x0f, 0xb1, 0xa5, 0xa0, 0x20,
                    0x4d, 0xb1, 0x4c, 0xa5, 0xa3,
                ],
                "a085 8a a284 a2a6 20 @CTOSTR a0a5 4c @PNUM",
            ),
            PrintBDE => ("PRINTBDE", 0xa508, &[0xa0, 0], "a0 0"),
            PrintCDE => (
                "PRINTCDE",
                0xa50a,
                &[0x20, 0xf6, 0xa4, 0xa5, 0xa0, 0x4c, 0xda, 0xa4],
                "20 @PRINTCD a0a5 4c @PUTDE",
            ),
            PrintI => (
                "PRINTI",
                0xa512,
                &[0x86, 0xa2, 0xaa, 0xa4, 0xa2, 0xa5, 0xb7],
                "a286 aa a2a4 a5 @DEVICE",
            ),
            PrintID => (
                "PRINTID",
                0xa519,
                &[
                    0xc0, 0, 0x10, 0xd9, 0x48, 0x86, 0xa1, 0x84, 0xa2, 0xa0, 0x2d, 0x20, 0xd5,
                    0xa4, 0x38, 0xa9, 0, 0xe5, 0xa1, 0xaa, 0xa9, 0, 0xe5, 0xa2, 0xa8, 0x68, 0x4c,
                    0xf6, 0xa4,
                ],
                "c0 0 1610 48 a186 a284 2da0 20 @PUTD1 38 a9 0 a1e5 aa a9 0 a2e5 a8 68 4c @PRINTCD",
            ),
            PrintIE => (
                "PRINTIE",
                0xa536,
                &[0x20, 0x12, 0xa5, 0x4c, 0xcc, 0xa4],
                "20 @PRINTI 4c @PUTE",
            ),
            PrintIDE => (
                "PRINTIDE",
                0xa53c,
                &[0x20, 0x19, 0xa5, 0xa5, 0xa0, 0x4c, 0xda, 0xa4],
                "20 @PRINTID a0a5 4c @PUTDE",
            ),
        }
    }

    fn fallthrough(self) -> Option<Self> {
        use NumericPrint::*;
        match self {
            PrintB => Some(PrintC),
            PrintBE => Some(PrintCE),
            PrintBD => Some(PrintCD),
            PrintBDE => Some(PrintCDE),
            PrintI => Some(PrintID),
            _ => None,
        }
    }

    fn eol_entries(self) -> Option<(Self, CioRoutine)> {
        use NumericPrint::*;
        match self {
            PrintCE => Some((PrintC, CioRoutine::PutE)),
            PrintCDE => Some((PrintCD, CioRoutine::PutDE)),
            PrintIE => Some((PrintI, CioRoutine::PutE)),
            PrintIDE => Some((PrintID, CioRoutine::PutDE)),
            _ => None,
        }
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        let (_, expected_address, bytes, _) = self.entry();
        if address != expected_address || !cart_matches(address, bytes) {
            return false;
        }
        if let Some(next) = self.fallthrough() {
            return next.matches_cart(address + bytes.len() as u16);
        }
        if let Some((number, end)) = self.eol_entries() {
            return number.matches_cart(number.address()) && end.matches_cart(end.address());
        }
        let cart_number_dispatch = || {
            cart_matches(0xb14d, &[0xa2, 0x50, 0xa0, 0x05, 0x4c, 0x6e, 0xb0])
                && cart_output_dispatch_matches()
                && cart_formatter_matches()
        };
        match self {
            Self::PrintC => {
                cart_matches(0xb148, &[0x20, 0x0f, 0xb1, 0xa5, 0xb7]) && cart_number_dispatch()
            }
            Self::PrintCD => cart_number_dispatch(),
            Self::PrintID => {
                cart_put_character_matches() && Self::PrintCD.matches_cart(Self::PrintCD.address())
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage) -> bool {
        let unit = "SYSIO";
        let (name, _, _, expected) = self.entry();
        if !linked_routine(image, unit, name)
            .is_some_and(|body| self.matches_signature(&body.signature))
            || !linked_body_matches(image, unit, name, expected)
        {
            return false;
        }
        if let Some(next) = self.fallthrough() {
            return adjacent(image, name, next.name()) && next.matches_linked(image);
        }
        if let Some((number, end)) = self.eol_entries() {
            return number.matches_linked(image) && end.matches_linked(image, unit, end.name());
        }
        let linked_number_dispatch = || {
            linked_body_matches(image, unit, "PNUM", "50a2 5a0 20 @OUTPUT 4c @CHKERR")
                && linked_output_dispatch_matches(image)
                && linked_formatter_matches(image)
        };
        match self {
            Self::PrintC => adjacent(image, "PRINTC", "PNUM") && linked_number_dispatch(),
            Self::PrintCD => linked_number_dispatch(),
            Self::PrintID => {
                linked_put_character_matches(image) && Self::PrintCD.matches_linked(image)
            }
            _ => unreachable!(),
        }
    }
}
