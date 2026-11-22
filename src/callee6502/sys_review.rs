//! Dispatch reviewed SYS facts after resolving the typed public binding.
use super::CallFacts;
use super::sys_cio::CioRoutine;
use super::sys_decimal::DecimalRoutine;
use super::sys_graphics::GraphicsRoutine;
use super::sys_hardware::HardwareRoutine;
use super::sys_memory::MemoryRoutine;
use super::sys_strings::{StringArguments, StringRoutine};
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::SemRoutineSignature;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SysReview {
    Graphics(GraphicsRoutine),
    Cio(CioRoutine),
    Hardware(HardwareRoutine),
    Memory(MemoryRoutine),
    Strings(StringRoutine),
    Decimal(DecimalRoutine),
}

impl SysReview {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        GraphicsRoutine::for_interface(name)
            .map(Self::Graphics)
            .or_else(|| CioRoutine::for_interface(name).map(Self::Cio))
            .or_else(|| HardwareRoutine::for_interface(name).map(Self::Hardware))
            .or_else(|| MemoryRoutine::for_interface(name).map(Self::Memory))
            .or_else(|| StringRoutine::for_interface(name).map(Self::Strings))
            .or_else(|| DecimalRoutine::for_interface(name).map(Self::Decimal))
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        match self {
            Self::Graphics(routine) => routine.matches_signature(signature),
            Self::Cio(routine) => routine.matches_signature(signature),
            Self::Hardware(routine) => routine.matches_signature(signature),
            Self::Memory(routine) => routine.matches_signature(signature),
            Self::Strings(routine) => routine.matches_signature(signature),
            Self::Decimal(routine) => routine.matches_signature(signature),
        }
    }

    pub(crate) fn matches_cart(self, address: u16) -> bool {
        match self {
            Self::Graphics(routine) => {
                routine.matches_cart(address)
                    && match routine {
                        // The cartridge's graphics routine calls the internal
                        // open entry directly and then dispatches status itself.
                        GraphicsRoutine::Graphics => {
                            CioRoutine::Close.matches_cart(CioRoutine::Close.address())
                                && super::sys_cio::cart_open_device_matches()
                        }
                        _ => dependencies(routine)
                            .iter()
                            .all(|dep| dep.matches_cart(dep.address())),
                    }
            }
            Self::Cio(routine) => routine.matches_cart(address),
            Self::Hardware(routine) => routine.matches_cart(address),
            Self::Memory(routine) => routine.matches_cart(address),
            Self::Strings(routine) => routine.matches_cart(address),
            Self::Decimal(routine) => routine.matches_cart(address),
        }
    }

    pub(crate) fn matches_linked(self, image: &RuntimeImage, unit: &str, name: &str) -> bool {
        match self {
            Self::Graphics(routine) => {
                routine.matches_linked(image, unit, name)
                    && dependencies(routine)
                        .iter()
                        .all(|dep| dep.matches_linked(image, "SYSIO", dep.name()))
            }
            Self::Cio(routine) => routine.matches_linked(image, unit, name),
            Self::Hardware(routine) => routine.matches_linked(image, unit, name),
            Self::Memory(routine) => routine.matches_linked(image, unit, name),
            Self::Strings(routine) => routine.matches_linked(image, unit, name),
            Self::Decimal(routine) => routine.matches_linked(image, unit, name),
        }
    }

    pub(crate) fn facts<Id>(
        self,
        mut argument: impl FnMut(usize) -> Option<u16>,
        origin: Option<u16>,
    ) -> CallFacts<Id> {
        let mut byte = || argument(0).and_then(|value| u8::try_from(value).ok());
        match self {
            Self::Graphics(routine) => routine.facts(byte()),
            Self::Cio(_) => CallFacts::default(),
            Self::Hardware(routine) => routine.facts(byte()),
            Self::Memory(routine) => routine.facts(argument(0), origin),
            Self::Strings(routine) => routine.facts(StringArguments::from_arguments(argument), origin),
            Self::Decimal(routine) => routine.facts(argument(0)),
        }
    }

    pub(crate) fn linked_abi_only_aliases(self) -> impl Iterator<Item = &'static str> {
        let peek = (self == Self::Memory(MemoryRoutine::Peek)).then_some("PEEK");
        let decimal = match self {
            Self::Decimal(routine) => routine.linked_aliases(),
            _ => &[],
        };
        peek.into_iter().chain(decimal.iter().map(|routine| routine.name()))
    }

    pub(crate) fn uses_precomputed_writes(self) -> bool {
        matches!(self, Self::Memory(_) | Self::Strings(_) | Self::Decimal(_))
    }
}

fn dependencies(routine: GraphicsRoutine) -> &'static [CioRoutine] {
    match routine {
        GraphicsRoutine::Plot => &[CioRoutine::PutD],
        GraphicsRoutine::Locate => &[CioRoutine::GetD],
        GraphicsRoutine::Graphics => &[CioRoutine::Close, CioRoutine::Open],
        GraphicsRoutine::DrawTo | GraphicsRoutine::Fill => &[CioRoutine::Xio],
        _ => &[],
    }
}
