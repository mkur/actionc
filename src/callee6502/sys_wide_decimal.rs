//! Reviewed source-level bounds of the compiler-owned wide decimal routines.
use super::{ArgumentBuffer, BufferLength, CallFacts};
use crate::ast::{FundType, RoutineKind};
use crate::embedded_vfs::EmbeddedSourceProvider;
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::{SemItem, SemRoutineSignature};
use crate::semantic::{ValueType, ValueTypeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WideDecimal {
    StrLC,
    StrLI,
    ValLC,
    ValLI,
}

impl WideDecimal {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        match crate::runtime_bindings::binding_key(name).as_str() {
            "SYS.STRLC" => Some(Self::StrLC),
            "SYS.STRLI" => Some(Self::StrLI),
            "SYS.VALLC" => Some(Self::ValLC),
            "SYS.VALLI" => Some(Self::ValLI),
            _ => None,
        }
    }

    pub(crate) fn parses(self) -> bool {
        matches!(self, Self::ValLC | Self::ValLI)
    }

    pub(crate) fn buffer(self) -> ArgumentBuffer {
        let bytes = match self {
            Self::StrLC => 11,
            Self::StrLI => 12,
            Self::ValLC | Self::ValLI => 256,
        };
        ArgumentBuffer {
            pointer: if self.parses() { 1 } else { 2 },
            length: BufferLength::Fixed(bytes),
            write: !self.parses(),
        }
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        let integer = ValueType {
            base: ValueTypeBase::Fund(match self {
                Self::StrLC | Self::ValLC => FundType::LongCard,
                Self::StrLI | Self::ValLI => FundType::LongInt,
            }),
            pointer: false,
        };
        let string = ValueType {
            base: ValueTypeBase::Fund(FundType::Char),
            pointer: true,
        };
        if self.parses() {
            matches!(signature.kind, RoutineKind::Func { .. })
                && signature.return_type == Some(integer)
                && signature.params == [string]
        } else {
            signature.kind == RoutineKind::Proc
                && signature.return_type.is_none()
                && signature.params == [integer, string]
        }
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage, unit: &str, name: &str) -> bool {
        // Independent of the generated link manifest. Regenerating its digest
        // must not silently bless changes to the reviewed Action! algorithms.
        let name = name.to_ascii_uppercase();
        let expected = match (self, unit) {
            (Self::StrLC, "SYSLONG") => "SYSSTRLC",
            (Self::StrLI, "SYSLONG") => "SYSSTRLI",
            (Self::ValLC, "SYSLONG") => "SYSVALLC",
            (Self::ValLI, "SYSLONG") => "SYSVALLI",
            (Self::ValLC, "SYSLONGC") => "CARTVALLC",
            (Self::ValLI, "SYSLONGC") => "CARTVALLI",
            _ => return false,
        };
        if name != expected
            || !reviewed_source(
                "syslong.act",
                "470c8db2056b16011ca2592c6e3e7b9fe807eb1fb1819e4737f60e51329268ca",
            )
            || unit == "SYSLONGC"
                && !reviewed_source(
                    "syslongc.act",
                    "bf88904b35dab035c63f9db9084d58f47a237015a6c3d63508419d4764fe2295",
                )
            || !image
                .routine_units
                .get(&name)
                .is_some_and(|provider| provider.name == unit)
        {
            return false;
        }
        image.semir.modules.iter().flat_map(|m| &m.items).any(|item| {
            matches!(item, SemItem::Routine(routine)
                if super::sys_graphics::short_name(&routine.symbol.qualified_name).eq_ignore_ascii_case(&name)
                    && self.matches_signature(&routine.signature))
        })
    }

    pub(crate) fn facts<Id>(self) -> CallFacts<Id> {
        CallFacts {
            abi_inputs_only: true,
            buffers: vec![self.buffer()],
            defines_on_return: if self.parses() {
                (0xa0..=0xa3).collect()
            } else {
                Default::default()
            },
            ..CallFacts::default()
        }
    }
}

pub(super) fn reviewed_source(file: &str, digest: &str) -> bool {
    EmbeddedSourceProvider
        .runtime_source(file)
        .is_some_and(|source| source.sha256 == digest)
}
