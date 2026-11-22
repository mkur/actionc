//! Reviewed implementation facts for the public Atari graphics ABI.
use crate::ast::{FundType, MachineItem, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::{SemExprKind, SemItem, SemRoutineSignature, SemStmt};
use crate::semantic::{ValueType, ValueTypeBase};

use super::{CallFacts, DataRange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GraphicsRoutine {
    Position,
    SetColor,
}

impl GraphicsRoutine {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        match crate::runtime_bindings::binding_key(name).as_str() {
            "SYS.POSITION" => Some(Self::Position),
            "SYS.SETCOLOR" => Some(Self::SetColor),
            _ => None,
        }
    }

    pub(crate) fn params(self) -> &'static [FundType] {
        match self {
            Self::Position => &[FundType::Card, FundType::Byte],
            Self::SetColor => &[FundType::Byte, FundType::Byte, FundType::Byte],
        }
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        signature.kind == RoutineKind::Proc && signature.return_type.is_none()
            && signature.params.len() == self.params().len()
            && signature.params.iter().zip(self.params()).all(|(ty, base)| *ty == ValueType {
                base: ValueTypeBase::Fund(*base), pointer: false,
            })
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        const CART: &[u8] = include_bytes!("../../roms/action.rom");
        let (entry, bytes): (u16, &[u8]) = match self {
            Self::Position => (0xa6ae, &[0x85,0x5b,0x86,0x5c,0x84,0x5a,0x85,0x55,0x86,0x56,0x84,0x54,0x60]),
            Self::SetColor => (0xa6ce, &[
                0xc9,0x05,0x10,0x16,0x85,0xa0,0x98,0x29,0x0f,0x85,0xa2,0x8a,0x0a,0x0a,
                0x0a,0x0a,0x05,0xa2,0xa6,0xa0,0x9d,0xc4,0x02,0x9d,0x16,0xd0,0x60,
            ]),
        };
        // OSS type 15's initial bank containing the public runtime entries.
        let start = 0x1010 + usize::from(entry - 0xa000);
        address == entry && CART.starts_with(b"CART\0\0\0\x0f")
            && CART.get(start..start + bytes.len()) == Some(bytes)
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage, unit: &str, entry: &str) -> bool {
        if unit != "SYSGR" { return false; }
        let names = match self {
            Self::Position => &["POSITION", "POS1"][..],
            Self::SetColor => &["SETCOLOR"][..],
        };
        if !entry.eq_ignore_ascii_case(names[0]) { return false; }

        let mut routines = image.semir.modules.iter().flat_map(|module| &module.items);
        let Some(first) = routines.find(|item| matches!(item, SemItem::Routine(routine)
            if short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(entry))) else {
            return false;
        };
        names.iter().zip(std::iter::once(first).chain(routines)).enumerate().all(|(index, (name, item))| {
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
            let expected: &[u64] = match self {
                Self::Position if index == 0 => &[0x5b85, 0x5c86, 0x5a84],
                Self::Position => &[0x5585, 0x5686, 0x5484, 0x60],
                Self::SetColor => &[0x5c9,0x1610,0xa085,0x98,0xf29,0xa285,0x8a,0xa0a,0xa0a,0xa205,0xa0a6,0x9d,0x2c4,0x9d,0xd016,0x60],
            };
            resolved_symbols.is_empty() && items.len() == expected.len()
                && items.iter().zip(expected).all(|(item, expected)| matches!(item,
                    MachineItem::Number(number) if number.value == Some(*expected)))
        })
    }

    pub(crate) fn facts<Id>(self, first_argument: Option<u8>) -> CallFacts<Id> {
        let range = |start, end| DataRange { start, end, global: None };
        let writes = match self {
            Self::Position => Some(vec![range(0x54, 0x56), range(0x5a, 0x5c)]),
            Self::SetColor => first_argument.filter(|&register| register <= 4).map(|register| {
                let register = u16::from(register);
                vec![range(0xa0, 0xa0), range(0xa2, 0xa2),
                    range(0x02c4 + register, 0x02c4 + register),
                    range(0xd016 + register, 0xd016 + register)]
            }),
        };
        CallFacts {
            abi_inputs_only: true,
            writes,
            preserves_regs: crate::ast::AnnotationRegisterSet {
                a: self == Self::Position, x: self == Self::Position, y: true,
            },
            ..Default::default()
        }
    }
}

fn short_name(name: &str) -> &str {
    name.rsplit(['.', ':']).next().unwrap_or(name)
}
