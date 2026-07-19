use crate::nir::NirMemoryAccess;

use super::ir::{MirMemoryEffect, MirRegisterSet};

/// Registers that the 6502 Action calling convention leaves volatile.
pub(super) const fn action_call_clobbers() -> MirRegisterSet {
    MirRegisterSet {
        a: true,
        x: true,
        y: true,
        flags: true,
        sp: false,
    }
}

/// An opaque inline machine block can alter any 6502 processor state.
pub(super) const fn opaque_machine_clobbers() -> MirRegisterSet {
    MirRegisterSet {
        a: true,
        x: true,
        y: true,
        flags: true,
        sp: true,
    }
}

pub(super) fn mir_memory_effect(effect: &NirMemoryAccess) -> MirMemoryEffect {
    match effect {
        NirMemoryAccess::None => MirMemoryEffect::None,
        NirMemoryAccess::Known { regions } => {
            MirMemoryEffect::Regions(Vec::with_capacity(*regions))
        }
        NirMemoryAccess::Unknown => MirMemoryEffect::Unknown,
        NirMemoryAccess::All => MirMemoryEffect::All,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_calls_clobber_volatile_6502_state_but_preserve_stack_pointer() {
        assert_eq!(
            action_call_clobbers(),
            MirRegisterSet {
                a: true,
                x: true,
                y: true,
                flags: true,
                sp: false,
            }
        );
    }

    #[test]
    fn opaque_machine_blocks_clobber_all_6502_processor_state() {
        assert_eq!(
            opaque_machine_clobbers(),
            MirRegisterSet {
                a: true,
                x: true,
                y: true,
                flags: true,
                sp: true,
            }
        );
    }
}
