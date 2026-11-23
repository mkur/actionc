//! Encoder-independent source bounds for the shared SYS REAL converters.
use super::{
    ArgumentBuffer, BufferLength, CallFacts, DataRange, sys_decimal::FPP_ASCII_WORKSPACE,
    sys_wide_decimal::reviewed_source,
};
use crate::ast::{FundType, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::{SemItem, SemRoutineSignature};
use crate::semantic::{ValueType, ValueTypeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RealDecimal {
    StrR,
    ValR,
}

impl RealDecimal {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        match crate::runtime_bindings::binding_key(name).as_str() {
            "SYS.STRR" => Some(Self::StrR),
            "SYS.VALR" => Some(Self::ValR),
            _ => None,
        }
    }

    pub(crate) fn fpp_entry(self) -> u16 {
        match self {
            Self::StrR => 0xd8e6, // FASC
            Self::ValR => 0xd800, // AFP
        }
    }

    pub(super) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let pointer = |base| ValueType {
            base,
            pointer: true,
        };
        let string = pointer(ValueTypeBase::Fund(FundType::Char));
        let real = pointer(ValueTypeBase::Real);
        signature.kind == RoutineKind::Proc
            && signature.return_type.is_none()
            && signature.params
                == match self {
                    Self::StrR => [real, string],
                    Self::ValR => [string, real],
                }
    }

    pub(super) fn matches_linked(self, image: &RuntimeImage, unit: &str, name: &str) -> bool {
        let entry = match self {
            Self::StrR => "SYSSTRR",
            Self::ValR => "SYSVALR",
        };
        unit == "SYSREAL"
            && name.eq_ignore_ascii_case(entry)
            && image.routine_units.get(entry).is_some_and(|provider| provider.name == unit)
            && reviewed_source("sysreal.act", "e05fbceef3b3a27d878869a1705a834843bf122f0dc67fe01a06afbf5c9669c0")
            && image.semir.modules.iter().flat_map(|module| &module.items).any(|item| {
                matches!(item, SemItem::Routine(routine)
                    if super::sys_graphics::short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(entry)
                        && self.matches_signature(&routine.signature))
            })
    }

    /// StrR requires a normalized REAL. OS-B can omit the terminator for a
    /// malformed mantissa, leaving the caller's copy loop with an open read.
    pub(crate) fn facts<Id>(self, normalized_real: bool) -> CallFacts<Id> {
        let (read_bytes, written_bytes) = match self {
            Self::ValR => (256, 6),
            Self::StrR if normalized_real => (6, 17),
            Self::StrR => return CallFacts::default(),
        };
        let workspace = || {
            FPP_ASCII_WORKSPACE
                .into_iter()
                .map(|(start, end)| DataRange {
                    global: None,
                    start,
                    end,
                })
                .collect()
        };
        CallFacts {
            abi_inputs_only: true,
            reads: Some(workspace()),
            writes: Some(workspace()),
            buffers: vec![
                ArgumentBuffer {
                    pointer: 1,
                    length: BufferLength::Fixed(read_bytes),
                    write: false,
                },
                ArgumentBuffer {
                    pointer: 2,
                    length: BufferLength::Fixed(written_bytes),
                    write: true,
                },
            ],
            ..CallFacts::default()
        }
    }
}
