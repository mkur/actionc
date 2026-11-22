//! Reviewed SYS entry-to-CIOV paths. CIOV's selected device and the
//! returning error continuation are independent of this implementation.
use super::sys_graphics::{cart_matches, matches_items, short_name};
use crate::ast::{FundType, RoutineKind};
use crate::runtime::Runtime;
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::{SemExprKind, SemItem, SemRoutineSignature, SemStmt};
use crate::semantic::{ValueType, ValueTypeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CioRoutine {
    PutD,
    GetD,
    Close,
    Open,
    Xio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CioCommand {
    Fixed(u8),
    Argument(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CioBuffer {
    /// IOCB length is zero. GET forwards incoming Y in A to the device;
    /// PUT puts the public character argument in A.
    Character {
        output: bool,
    },
    CloseDescriptor,
    FilenameCopy {
        argument: u8,
        destination: FilenameBuffer,
    },
    /// The length byte is read before dispatch. Zero skips CIOV.
    CountedString {
        argument: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FilenameBuffer {
    PointerCell(u16),
    Fixed(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CioTransfer {
    pub command: CioCommand,
    pub buffer: CioBuffer,
    /// Possible wrapper writes before CIO dispatch or result/status staging.
    pub scratch: &'static [u8],
    /// Public AUX1/AUX2 argument indices. Zero AUX1 keeps both IOCB bytes.
    pub conditional_aux: Option<(u8, u8)>,
    pub successful_result: Option<u8>,
}

impl CioRoutine {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        match crate::runtime_bindings::binding_key(name).as_str() {
            "SYS.PUTD" => Some(Self::PutD),
            "SYS.GETD" => Some(Self::GetD),
            "SYS.CLOSE" => Some(Self::Close),
            "SYS.OPEN" => Some(Self::Open),
            "SYS.XIO" => Some(Self::Xio),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::PutD => "PUTD",
            Self::GetD => "GETD",
            Self::Close => "CLOSE",
            Self::Open => "OPEN",
            Self::Xio => "XIO",
        }
    }

    pub(crate) fn transfer(self, runtime: Runtime) -> CioTransfer {
        use CioCommand::{Argument, Fixed};
        let (command, buffer, scratch, conditional_aux): (_, _, &[u8], _) = match self {
            Self::PutD => (
                Fixed(0x0b),
                CioBuffer::Character { output: true },
                &[0xa0, 0xa1, 0xa4],
                None,
            ),
            Self::GetD => (
                Fixed(0x07),
                CioBuffer::Character { output: false },
                &[0xa0, 0xa4],
                None,
            ),
            Self::Close => (
                Fixed(0x0c),
                CioBuffer::CloseDescriptor,
                &[0xa3, 0xa5, 0xa6],
                None,
            ),
            Self::Open => {
                let destination = match runtime {
                    Runtime::ActionCart => FilenameBuffer::PointerCell(0x009b),
                    Runtime::Standalone => FilenameBuffer::Fixed(0x0500),
                };
                (
                    Fixed(0x03),
                    CioBuffer::FilenameCopy {
                        argument: 1,
                        destination,
                    },
                    &[0xa1, 0xa2, 0xa5, 0xa6],
                    Some((2, 3)),
                )
            }
            Self::Xio => (
                Argument(2),
                CioBuffer::CountedString { argument: 5 },
                &[],
                Some((3, 4)),
            ),
        };
        CioTransfer {
            command,
            buffer,
            scratch,
            conditional_aux,
            successful_result: (self == Self::GetD).then_some(0xa0),
        }
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let byte = ValueType {
            base: ValueTypeBase::Fund(FundType::Byte),
            pointer: false,
        };
        let character = ValueType {
            base: ValueTypeBase::Fund(FundType::Char),
            pointer: false,
        };
        let string = ValueType {
            pointer: true,
            ..character.clone()
        };
        match self {
            Self::PutD => {
                signature.kind == RoutineKind::Proc
                    && signature.return_type.is_none()
                    && signature.params == [byte, character]
            }
            Self::GetD => {
                matches!(signature.kind, RoutineKind::Func { .. })
                    && signature.return_type.as_ref() == Some(&character)
                    && signature.params == [byte]
            }
            Self::Close => {
                signature.kind == RoutineKind::Proc
                    && signature.return_type.is_none()
                    && signature.params == [byte]
            }
            Self::Open => {
                signature.kind == RoutineKind::Proc
                    && signature.return_type.is_none()
                    && signature.params == [byte.clone(), string, byte.clone(), byte]
            }
            Self::Xio => {
                signature.kind == RoutineKind::Proc
                    && signature.return_type.is_none()
                    && signature.params
                        == [
                            byte.clone(),
                            byte.clone(),
                            byte.clone(),
                            byte.clone(),
                            byte,
                            string,
                        ]
            }
        }
    }

    pub(crate) fn address(self) -> u16 {
        match self {
            Self::PutD => 0xa4d1,
            Self::GetD => 0xa4ad,
            Self::Close => 0xa479,
            Self::Open => 0xa444,
            Self::Xio => 0xa4de,
        }
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        let entry: &[u8] = match self {
            Self::PutD => &[0x86, 0xa1, 0xa4, 0xa1, 0xa2, 0x0b, 0x4c, 0xaf, 0xa4],
            Self::GetD => &[0xa2, 0x07],
            Self::Close => &[0x20, 0x26, 0xb0, 0x4c, 0xa5, 0xa3],
            Self::Open => &[
                0x48, 0x86, 0xa1, 0x84, 0xa2, 0xa8, 0xa9, 0x00, 0x99, 0xc0, 0x05, 0xa8, 0xb1, 0xa1,
                0x91, 0x9b, 0xa8, 0xc8, 0xa9, 0x9b, 0xd0, 0x02, 0xb1, 0xa1, 0x91, 0x9b, 0x88, 0xd0,
                0xf9, 0x68, 0xa6, 0x9b, 0xa4, 0x9c, 0x20, 0x04, 0xb0, 0x4c, 0xa5, 0xa3,
            ],
            Self::Xio => &[0x20, 0x3a, 0xb0, 0x4c, 0xa5, 0xa3],
        };
        address == self.address()
            && cart_matches(address, entry)
            && match self {
                Self::PutD | Self::GetD => cart_matches(
                    0xa4af,
                    &[
                        0x86, 0xa4, 0x0a, 0x0a, 0x0a, 0x0a, 0xaa, 0xa5, 0xa4, 0x9d, 0x42, 0x03,
                        0xa9, 0x00, 0x9d, 0x48, 0x03, 0x9d, 0x49, 0x03, 0x98, 0x20, 0x56, 0xe4,
                        0x85, 0xa0, 0x4c, 0xa5, 0xa3,
                    ],
                ),
                Self::Close => {
                    cart_matches(
                        0xb026,
                        &[
                            0xa2, 0xb0, 0x86, 0xa6, 0x91, 0xa5, 0xa0, 0x0c, 0xd0, 0x04, 0x84, 0xa6,
                            0xa0, 0x05, 0x86, 0xa5, 0xa2, 0x00, 0x86, 0xa3,
                        ],
                    ) && cart_xiostr_matches()
                }
                Self::Open => cart_open_device_matches(),
                Self::Xio => cart_xiostr_matches(),
            }
            && cart_status_matches()
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage, unit: &str, name: &str) -> bool {
        if unit != "SYSIO" || !name.eq_ignore_ascii_case(self.name()) {
            return false;
        }
        let Some(body) = linked_routine(image, unit, self.name()) else {
            return false;
        };
        if !self.matches_signature(&body.signature) {
            return false;
        }
        let ccio = || {
            linked_body_matches(
                image,
                "SYSIO",
                "CCIO",
                "a486 a0a a0a aa a4a5 9d 342 a9 0 9d 348 9d 349 98 20 e456 a085 4c @CHKERR",
            )
        };
        let matched = match self {
            Self::PutD => {
                body_matches(body, "a186 a1a4")
                    && adjacent(image, "PUTD", "PUTD1")
                    && linked_body_matches(image, "SYSIO", "PUTD1", "ba2 4c @CCIO")
                    && ccio()
            }
            Self::GetD => body_matches(body, "7a2") && adjacent(image, "GETD", "CCIO") && ccio(),
            Self::Close => {
                body_matches(body, "20 @CLOS 4c @CHKERR")
                    && linked_body_matches(image, "SYSLIB", "CLOS", "ffa2 a686 ca0 ad0")
                    && linked_body_matches(image, "SYSLIB", "OUTPUT", "a684 ba0 4d0")
                    && linked_body_matches(image, "SYSLIB", "IN", "a684 5a0 a586 a2 0 a386")
                    && adjacent(image, "CLOS", "OUTPUT")
                    && adjacent(image, "OUTPUT", "IN")
                    && adjacent(image, "IN", "XIOSTR")
                    && linked_xiostr_matches(image)
            }
            Self::Open => {
                linked_open_matches(image, body)
                    && linked_body_matches(image, "SYSLIB", "OPN", "a586 a684 3a0 4c @XIOSTR")
                    && linked_xiostr_matches(image)
            }
            Self::Xio => {
                body_matches(body, "20 @XIOSTR 4c @CHKERR") && linked_xiostr_matches(image)
            }
        };
        matched && linked_status_matches(image)
    }
}

pub(super) fn cart_open_device_matches() -> bool {
    cart_matches(0xb004, &[0x86, 0xa5, 0x84, 0xa6, 0xa0, 0x03, 0xd0, 0x2e]) && cart_xiostr_matches()
}

fn cart_xiostr_matches() -> bool {
    cart_matches(0xb025, &[0x60])
        && cart_matches(
            0xb03a,
            &[
                0x0a, 0x0a, 0x0a, 0x0a, 0xaa, 0x98, 0x9d, 0x42, 0x03, 0xa5, 0xa3, 0xf0, 0x0a, 0x9d,
                0x4a, 0x03, 0xa5, 0xa4, 0x9d, 0x4b, 0x03, 0xa9, 0x00, 0xa8, 0x9d, 0x49, 0x03, 0xb1,
                0xa5, 0x9d, 0x48, 0x03, 0xf0, 0xc9, 0x18, 0xa5, 0xa5, 0x69, 0x01, 0x9d, 0x44, 0x03,
                0xa5, 0xa6, 0x69, 0x00, 0x9d, 0x45, 0x03, 0x4c, 0x56, 0xe4,
            ],
        )
}

fn linked_xiostr_matches(image: &RuntimeImage) -> bool {
    linked_body_matches(
        image,
        "SYSLIB",
        "XIOSTR",
        concat!(
            "a0a a0a 98aa 9d 342 a3a5 af0 9d 34a a4a5 9d 34b a9 0 9da8 349 ",
            "a5b1 9d 348 12f0 18 a5a5 169 9d 344 a6a5 69 0 9d 345 4c e456 60",
        ),
    )
}

fn linked_open_matches(image: &RuntimeImage, body: &crate::semantic::ir::SemRoutine) -> bool {
    body_matches(body, concat!(
        "48 a186 a284 a8 a9 0 99 @EOF a8 a1b1 8d @OPENBUF a8 c8 9ba9 2d0 ",
        "a1b1 99 @OPENBUF 88 f8d0 68 a2 @OPENBUFL a0 @OPENBUFH 20 @OPN 4c @CHKERR",
    )) && [("OPENBUF", "$0500"), ("OPENBUFL", "$00"), ("OPENBUFH", "$05")]
        .iter().all(|(name, value)| image.semir.modules.iter().flat_map(|m| &m.items).any(|item|
            matches!(item, SemItem::Define(define)
                if short_name(&define.symbol.qualified_name).eq_ignore_ascii_case(name) && define.value == *value)))
}
fn cart_status_matches() -> bool {
    cart_matches(
        0xa3a5,
        &[
            0x10, 0x16, 0xc0, 0x88, 0xf0, 0x08, 0x98, 0xc0, 0x80, 0xf0, 0x11, 0x4c, 0xcb, 0x04,
            0x8a, 0x4a, 0x4a, 0x4a, 0x4a, 0xaa, 0x98, 0x9d, 0xc0, 0x05, 0xde, 0x95, 0xa3, 0x60,
            0xa2, 0x01, 0x86, 0x11, 0x48, 0x20, 0xda, 0xa7, 0x68, 0xa8, 0x60,
        ],
    )
}

fn linked_status_matches(image: &RuntimeImage) -> bool {
    linked_body_matches(
        image,
        "SYSIO",
        "CHKERR",
        "1610 88c0 8f0 98 80c0 11f0 4c @ERROR 8a 4a4a 4a4a 98aa 9d @EOF 60",
    ) && linked_body_matches(image, "SYSLIB", "ERROR", "6c a 0")
}

pub(super) fn linked_routine<'a>(
    image: &'a RuntimeImage,
    unit: &str,
    name: &str,
) -> Option<&'a crate::semantic::ir::SemRoutine> {
    if image.routine_units.get(name)?.name != unit {
        return None;
    }
    image
        .semir
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .find_map(|item| match item {
            SemItem::Routine(routine)
                if short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(name)
                    && routine.locals.is_empty()
                    && matches!(
                        routine.system_address.as_ref().map(|v| &v.kind),
                        Some(SemExprKind::CurrentLocation)
                    ) =>
            {
                Some(routine)
            }
            _ => None,
        })
}

fn body_matches(routine: &crate::semantic::ir::SemRoutine, expected: &str) -> bool {
    matches!(routine.body.as_slice(), [SemStmt::MachineBlock { items, resolved_symbols, .. }]
        if matches_items(items, resolved_symbols, expected))
}

pub(super) fn linked_body_matches(image: &RuntimeImage, unit: &str, name: &str, expected: &str) -> bool {
    linked_routine(image, unit, name).is_some_and(|routine| body_matches(routine, expected))
}

pub(super) fn adjacent(image: &RuntimeImage, first: &str, second: &str) -> bool {
    let mut items = image.semir.modules.iter().flat_map(|m| &m.items).peekable();
    items.any(|item| {
        matches!(item, SemItem::Routine(routine)
        if short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(first))
    }) && matches!(items.next(), Some(SemItem::Routine(routine))
            if short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(second))
}
