//! Reviewed source bounds for SYS.ValR and its AFP dependency.
use super::{
    ArgumentBuffer, BufferLength, CallFacts, DataRange, sys_decimal::FPP_ASCII_WORKSPACE,
    sys_wide_decimal::reviewed_source,
};
use crate::ast::{FundType, RoutineKind};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::{SemItem, SemRoutineSignature};
use crate::semantic::{ValueType, ValueTypeBase};

pub(crate) const AFP: u16 = 0xd800;

pub(super) fn matches_signature(signature: &SemRoutineSignature) -> bool {
    let pointer = |base| ValueType {
        base,
        pointer: true,
    };
    signature.kind == RoutineKind::Proc
        && signature.return_type.is_none()
        && signature.params
            == [
                pointer(ValueTypeBase::Fund(FundType::Char)),
                pointer(ValueTypeBase::Real),
            ]
}

pub(super) fn matches_linked(image: &RuntimeImage, unit: &str, name: &str) -> bool {
    unit == "SYSREAL"
        && name.eq_ignore_ascii_case("SysValR")
        && image.routine_units.get("SYSVALR").is_some_and(|provider| provider.name == unit)
        && reviewed_source("sysreal.act", "e05fbceef3b3a27d878869a1705a834843bf122f0dc67fe01a06afbf5c9669c0")
        && image.semir.modules.iter().flat_map(|module| &module.items).any(|item| {
            matches!(item, SemItem::Routine(routine)
                if super::sys_graphics::short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(name)
                    && matches_signature(&routine.signature))
        })
}

pub(super) fn facts<Id>() -> CallFacts<Id> {
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
                length: BufferLength::Fixed(256),
                write: false,
            },
            ArgumentBuffer {
                pointer: 2,
                length: BufferLength::Fixed(6),
                write: true,
            },
        ],
        ..CallFacts::default()
    }
}
