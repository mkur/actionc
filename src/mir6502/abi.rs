use crate::nir::{NirMemoryAccess, NirMemoryRegionKind, NirStorageId};

use super::ir::{MirMemoryEffect, MirMemoryRegion, MirMemoryRegionKind, MirRegisterSet};

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
        NirMemoryAccess::Regions(regions) => MirMemoryEffect::Regions(
            regions
                .iter()
                .map(|region| MirMemoryRegion {
                    kind: match region.kind {
                        NirMemoryRegionKind::Storage(NirStorageId::Local(id)) => {
                            MirMemoryRegionKind::Local(id)
                        }
                        NirMemoryRegionKind::Storage(NirStorageId::Param(id)) => {
                            MirMemoryRegionKind::Param(id)
                        }
                        NirMemoryRegionKind::Storage(NirStorageId::Global(id)) => {
                            MirMemoryRegionKind::Global(id)
                        }
                        NirMemoryRegionKind::Static(id) => MirMemoryRegionKind::Static(id),
                        NirMemoryRegionKind::AbsoluteRange => MirMemoryRegionKind::AbsoluteRange,
                        NirMemoryRegionKind::ZeroPage => MirMemoryRegionKind::ZeroPage,
                    },
                    offset: region.offset,
                    size: region.size,
                })
                .collect(),
        ),
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

    #[test]
    fn nir_memory_regions_retain_identity_offset_and_size_in_mir() {
        let effect = mir_memory_effect(&NirMemoryAccess::Regions(vec![
            crate::nir::NirMemoryRegion {
                kind: NirMemoryRegionKind::Storage(NirStorageId::Param(crate::nir::ParamId(4))),
                offset: 1,
                size: 2,
            },
        ]));

        assert_eq!(
            effect,
            MirMemoryEffect::Regions(vec![MirMemoryRegion {
                kind: MirMemoryRegionKind::Param(crate::nir::ParamId(4)),
                offset: 1,
                size: 2,
            }])
        );
    }
}
