//! Reviewed implementation facts for the public Atari graphics ABI.
use crate::ast::{FundType, MachineAddressAtom, MachineItem, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::{SemExprKind, SemItem, SemRoutineSignature, SemStmt};
use crate::semantic::{ValueType, ValueTypeBase};

use super::{CallFacts, DataRange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GraphicsRoutine {
    Position,
    SetColor,
    Plot,
    Locate,
    DrawTo,
    Fill,
    Graphics,
}

impl GraphicsRoutine {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        match crate::runtime_bindings::binding_key(name).as_str() {
            "SYS.POSITION" => Some(Self::Position),
            "SYS.SETCOLOR" => Some(Self::SetColor),
            "SYS.PLOT" => Some(Self::Plot),
            "SYS.LOCATE" => Some(Self::Locate),
            "SYS.DRAWTO" => Some(Self::DrawTo),
            "SYS.FILL" => Some(Self::Fill),
            "SYS.GRAPHICS" => Some(Self::Graphics),
            _ => None,
        }
    }

    pub(crate) fn params(self) -> &'static [FundType] {
        match self {
            Self::SetColor => &[FundType::Byte, FundType::Byte, FundType::Byte],
            Self::Graphics => &[FundType::Byte],
            _ => &[FundType::Card, FundType::Byte],
        }
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let result_matches = if self == Self::Locate {
            matches!(signature.kind, RoutineKind::Func { .. })
                && signature.return_type
                    == Some(ValueType {
                        base: ValueTypeBase::Fund(FundType::Byte),
                        pointer: false,
                    })
        } else {
            signature.kind == RoutineKind::Proc && signature.return_type.is_none()
        };
        result_matches
            && signature.params.len() == self.params().len()
            && signature
                .params
                .iter()
                .zip(self.params())
                .all(|(ty, base)| {
                    *ty == ValueType {
                        base: ValueTypeBase::Fund(*base),
                        pointer: false,
                    }
                })
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        let (entry, bytes): (u16, &[u8]) = match self {
            Self::Position => (
                0xa6ae,
                &[
                    0x85, 0x5b, 0x86, 0x5c, 0x84, 0x5a, 0x85, 0x55, 0x86, 0x56, 0x84, 0x54, 0x60,
                ],
            ),
            Self::SetColor => (
                0xa6ce,
                &[
                    0xc9, 0x05, 0x10, 0x16, 0x85, 0xa0, 0x98, 0x29, 0x0f, 0x85, 0xa2, 0x8a, 0x0a,
                    0x0a, 0x0a, 0x0a, 0x05, 0xa2, 0xa6, 0xa0, 0x9d, 0xc4, 0x02, 0x9d, 0x16, 0xd0,
                    0x60,
                ],
            ),
            Self::Plot => (
                0xa6c3,
                &[
                    0x20, 0xb4, 0xa6, 0xa9, 0x06, 0xae, 0xfd, 0x02, 0x4c, 0xd1, 0xa4,
                ],
            ),
            Self::Locate => (0xa6bb, &[0x20, 0xae, 0xa6, 0xa9, 0x06, 0x4c, 0xad, 0xa4]),
            Self::DrawTo => (0xa68c, &[0x20, 0x94, 0xa6, 0xa0, 0x11, 0x4c, 0xde, 0xa4]),
            Self::Fill => (0xa6e9, &[0x20, 0x94, 0xa6, 0xa0, 0x12, 0x4c, 0xde, 0xa4]),
            Self::Graphics => (
                0xa654,
                &[
                    0x48, 0xa9, 0x00, 0x20, 0x79, 0xa4, 0xa9, 0x0c, 0x85, 0xa3, 0xa9, 0x00, 0xa2,
                    0x84, 0xa0, 0xa6, 0x20, 0x04, 0xb0, 0x20, 0xa5, 0xa3, 0xa9, 0x06, 0x20, 0x79,
                    0xa4, 0x68, 0x85, 0xa4, 0x29, 0x30, 0x49, 0x1c, 0x85, 0xa3, 0xa9, 0x06, 0xa2,
                    0x88, 0xa0, 0xa6, 0x20, 0x04, 0xb0, 0x4c, 0xa5, 0xa3, 0x02, 0x45, 0x3a, 0x9b,
                    0x02, 0x53, 0x3a, 0x9b,
                ],
            ),
        };
        address == entry
            && cart_matches(entry, bytes)
            && match self {
                Self::Plot | Self::Locate => Self::Position.matches_cart(0xa6ae),
                Self::DrawTo | Self::Fill => {
                    Self::Position.matches_cart(0xa6ae)
                        && cart_matches(
                            0xa694,
                            &[
                                0x20, 0xb4, 0xa6, 0xad, 0xfd, 0x02, 0x8d, 0xfb, 0x02, 0xa9, 0x88,
                                0x85, 0xa5, 0xa9, 0xa6, 0x85, 0xa6, 0xa9, 0x00, 0x85, 0xa3, 0x85,
                                0xa4, 0xa9, 0x06, 0x60,
                            ],
                        )
                }
                Self::Position | Self::SetColor | Self::Graphics => true,
            }
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage, unit: &str, entry: &str) -> bool {
        if unit != "SYSGR" {
            return false;
        }
        let names = match self {
            Self::Position => &["POSITION", "POS1"][..],
            Self::SetColor => &["SETCOLOR"][..],
            Self::Plot => &["PLOT"][..],
            Self::Locate => &["LOCATE"][..],
            Self::DrawTo => &["DRAWTO"][..],
            Self::Fill => &["FILL"][..],
            Self::Graphics => &["GRAPHICS"][..],
        };
        if !entry.eq_ignore_ascii_case(names[0]) {
            return false;
        }

        let mut routines = image.semir.modules.iter().flat_map(|module| &module.items);
        let Some(first) = routines.find(|item| {
            matches!(item, SemItem::Routine(routine)
            if short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(entry))
        }) else {
            return false;
        };
        let entries = std::iter::once(first)
            .chain(routines)
            .take(names.len())
            .collect::<Vec<_>>();
        let matched = names.len() == entries.len() && names.iter().zip(entries).enumerate().all(|(index, (name, item))| {
            let SemItem::Routine(routine) = item else { return false; };
            if !short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(name)
                || image.routine_units.get(*name).is_none_or(|provider| provider.name != unit)
                || !routine.locals.is_empty()
                || !matches!(routine.system_address.as_ref().map(|value| &value.kind), Some(SemExprKind::CurrentLocation))
                || (index == 0 && !self.matches_signature(&routine.signature))
                || (index != 0 && !routine.signature.params.is_empty())
            {
                return false;
            }
            let [SemStmt::MachineBlock { items, resolved_symbols, .. }] = routine.body.as_slice() else { return false; };
            let expected = match self {
                Self::Position if index == 0 => "5b85 5c86 5a84",
                Self::Position => "5585 5686 5484 60",
                Self::SetColor => "5c9 1610 a085 98 f29 a285 8a a0a a0a a205 a0a6 9d 2c4 9d d016 60",
                Self::Plot => "20 @POS1 6a9 ae 2fd 4c @PUTD",
                Self::Locate => "20 @POSITION 6a9 4c @GETD",
                Self::DrawTo => "20 @GRIO 11a0 4c @XIO",
                Self::Fill => "20 @GRIO 12a0 4c @XIO",
                Self::Graphics => concat!(
                    "48 a9 0 20 @CLOSE ca9 a385 a9 0 ae @DEV_E ac @DEV_E+1 20 @OPEN ",
                    "6a9 20 @CLOSE 68 a485 3029 1c49 a385 6a9 ae @DEV_S ac @DEV_S+1 4c @OPEN",
                ),
            };
            matches_items(items, resolved_symbols, expected)
        });
        let pos1 = || matches_linked_helper(image, "POS1", "5585 5686 5484 60");
        matched
            && match self {
                Self::Plot => pos1(),
                Self::Locate => Self::Position.matches_linked(image, unit, "POSITION"),
                Self::DrawTo | Self::Fill => {
                    pos1()
                        && matches_linked_helper(
                            image,
                            "GRIO",
                            "20 @POS1 ad 2fd 8d 2fb ad @DEV_S a585 ad @DEV_S+1 a685 a9 0 a385 a485 6a9 60",
                        )
                }
                Self::Position | Self::SetColor | Self::Graphics => true,
            }
    }

    /// Reviewed transfer at the end of the graphics wrapper. The program's
    /// installed device and any returning error handler remain open boundaries.
    pub(crate) fn cio_boundary(self) -> Option<&'static str> {
        match self {
            Self::Plot => Some("PutD on S: (reads COLOR at $02FD)"),
            Self::Locate => Some("GetD on S: (successful pixel result at $A0)"),
            Self::DrawTo => Some("XIO $11 on S: (reads COLOR and writes $02FB)"),
            Self::Fill => Some("XIO $12 on S: (reads COLOR and writes $02FB)"),
            Self::Graphics => {
                Some("Close/Open on E: and S: (screen mode in AUX2, derived access in AUX1)")
            }
            Self::Position | Self::SetColor => None,
        }
    }

    pub(crate) fn facts<Id>(self, first_argument: Option<u8>) -> CallFacts<Id> {
        let range = |start, end| DataRange {
            start,
            end,
            global: None,
        };
        let writes = match self {
            Self::Position => Some(vec![range(0x54, 0x56), range(0x5a, 0x5c)]),
            Self::SetColor => first_argument
                .filter(|&register| register <= 4)
                .map(|register| {
                    let register = u16::from(register);
                    vec![
                        range(0xa0, 0xa0),
                        range(0xa2, 0xa2),
                        range(0x02c4 + register, 0x02c4 + register),
                        range(0xd016 + register, 0xd016 + register),
                    ]
                }),
            Self::Plot | Self::Locate | Self::DrawTo | Self::Fill | Self::Graphics => None,
        };
        CallFacts {
            abi_inputs_only: self.cio_boundary().is_none(),
            writes,
            preserves_regs: crate::ast::AnnotationRegisterSet {
                a: self == Self::Position,
                x: self == Self::Position,
                y: self.cio_boundary().is_none(),
            },
            ..Default::default()
        }
    }
}

pub(super) fn cart_matches(entry: u16, bytes: &[u8]) -> bool {
    const CART: &[u8] = include_bytes!("../../roms/action.rom");
    // OSS type 15's initial bank containing the public runtime entries.
    let start = match entry {
        0xa000..=0xafff => 0x1010 + usize::from(entry - 0xa000),
        0xb000..=0xbfff => 0x0010 + usize::from(entry - 0xb000),
        _ => return false,
    };
    CART.starts_with(b"CART\0\0\0\x0f") && CART.get(start..start + bytes.len()) == Some(bytes)
}

fn matches_linked_helper(image: &RuntimeImage, name: &str, words: &str) -> bool {
    image
        .routine_units
        .get(name)
        .is_some_and(|unit| unit.name == "SYSGR")
        && image
            .semir
            .modules
            .iter()
            .flat_map(|m| &m.items)
            .any(|item| {
                let SemItem::Routine(routine) = item else {
                    return false;
                };
                short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(name)
                    && routine.signature.kind == RoutineKind::Proc
                    && routine.signature.params.is_empty()
                    && routine.locals.is_empty()
                    && matches!(
                        routine.system_address.as_ref().map(|v| &v.kind),
                        Some(SemExprKind::CurrentLocation)
                    )
                    && matches!(routine.body.as_slice(),
                    [SemStmt::MachineBlock { items, resolved_symbols, .. }]
                        if matches_items(items, resolved_symbols, words))
            })
}

/// Compare the checked-in machine word/relocation sequence. This never infers
/// instruction effects. A symbolic item must also have the reviewed resolved target.
pub(super) fn matches_items(
    items: &[MachineItem],
    symbols: &[crate::semantic::ir::SemMachineSymbolRef],
    expected: &str,
) -> bool {
    let expected = expected.split_ascii_whitespace();
    items.len() == expected.clone().count()
        && items
            .iter()
            .zip(expected)
            .enumerate()
            .all(|(index, (item, expected))| {
                if expected == "*+5" {
                    return matches!(item, MachineItem::AddressExpr(expr)
                        if matches!(expr.atom, MachineAddressAtom::Current)
                            && expr.offset == 5 && expr.selector.is_none() && !expr.explicit_address);
                }
                let Some(name) = expected.strip_prefix('@') else {
                    return matches!(item, MachineItem::Number(number)
                    if number.value == u64::from_str_radix(expected, 16).ok());
                };
                let (name, offset) = if let Some((name, offset)) = name.rsplit_once('+') {
                    let Ok(offset) = offset.parse() else { return false; };
                    (name, offset)
                } else {
                    (name, 0)
                };
                let actual = match item {
                    MachineItem::Name(name) if offset == 0 => name,
                    MachineItem::AddressExpr(expr)
                        if expr.offset == offset
                            && expr.selector.is_none()
                            && !expr.explicit_address =>
                    {
                        let MachineAddressAtom::Name(name) = &expr.atom else {
                            return false;
                        };
                        name
                    }
                    _ => return false,
                };
                actual.to_string().eq_ignore_ascii_case(name)
                    && symbols.iter().any(|s| {
                        s.item_index == index
                            && short_name(&s.symbol.qualified_name).eq_ignore_ascii_case(name)
                    })
            })
}

pub(super) fn short_name(name: &str) -> &str {
    name.rsplit(['.', ':']).next().unwrap_or(name)
}
