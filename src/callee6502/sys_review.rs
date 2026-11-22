//! Dispatch reviewed SYS facts after resolving the typed public binding.
use super::CallFacts;
use super::sys_cio::CioRoutine;
use super::sys_graphics::GraphicsRoutine;
use crate::runtime_source::RuntimeImage;
use crate::semantic::ir::SemRoutineSignature;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SysReview {
    Graphics(GraphicsRoutine),
    Cio(CioRoutine),
}

impl SysReview {
    pub(crate) fn for_interface(name: &str) -> Option<Self> {
        GraphicsRoutine::for_interface(name)
            .map(Self::Graphics)
            .or_else(|| CioRoutine::for_interface(name).map(Self::Cio))
    }

    pub(crate) fn matches_signature(self, signature: &SemRoutineSignature) -> bool {
        match self {
            Self::Graphics(routine) => routine.matches_signature(signature),
            Self::Cio(routine) => routine.matches_signature(signature),
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
        }
    }

    pub(crate) fn facts<Id>(self, first_argument: Option<u8>) -> CallFacts<Id> {
        match self {
            Self::Graphics(routine) => routine.facts(first_argument),
            Self::Cio(_) => CallFacts::default(),
        }
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
