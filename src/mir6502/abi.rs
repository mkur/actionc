use crate::nir::{NirMemoryAccess, NirRegisterSet};

use super::ir::{MirMemoryEffect, MirRegisterSet};

pub(super) fn mir_register_set(registers: NirRegisterSet) -> MirRegisterSet {
    MirRegisterSet {
        a: registers.a,
        x: registers.x,
        y: registers.y,
        flags: registers.flags,
        sp: false,
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
