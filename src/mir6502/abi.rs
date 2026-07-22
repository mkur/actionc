use crate::nir::{NirMemoryAccess, NirMemoryRegionKind, NirStorageId};

use super::ir::{
    MirArgHome, MirFixedZpSlot, MirMemoryEffect, MirMemoryRegion, MirMemoryRegionKind, MirReg,
    MirRegisterSet, MirWidth,
};

/// Returns the one canonical Action ABI home for an argument at `offset`.
///
/// Argument bytes zero through two live only in A, X, and Y. The fixed
/// zero-page argument area starts at byte offset three in $A3; there are no
/// caller-side $A0-$A2 mirrors, including for current-location routines.
pub(super) fn action_arg_home(offset: u16, width: MirWidth) -> MirArgHome {
    match width {
        MirWidth::Byte => action_arg_byte_home(offset),
        MirWidth::Word => {
            let lo = action_arg_byte_home(offset);
            let hi = action_arg_byte_home(offset.saturating_add(1));
            match (&lo, &hi) {
                (MirArgHome::Reg(lo), MirArgHome::Reg(hi)) => {
                    MirArgHome::RegisterPair { lo: *lo, hi: *hi }
                }
                _ => MirArgHome::BytePair {
                    lo: Box::new(lo),
                    hi: Box::new(hi),
                },
            }
        }
    }
}

pub(super) const fn action_arg_width_bytes(width: MirWidth) -> u16 {
    match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    }
}

fn action_arg_byte_home(offset: u16) -> MirArgHome {
    match offset {
        0 => MirArgHome::Reg(MirReg::A),
        1 => MirArgHome::Reg(MirReg::X),
        2 => MirArgHome::Reg(MirReg::Y),
        _ => MirArgHome::FixedZeroPage(MirFixedZpSlot(
            u8::try_from(0x00A0u16.saturating_add(offset)).unwrap_or(u8::MAX),
        )),
    }
}

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
    fn action_argument_homes_use_registers_then_a3_and_later() {
        assert_eq!(
            action_arg_home(0, MirWidth::Byte),
            MirArgHome::Reg(MirReg::A)
        );
        assert_eq!(
            action_arg_home(1, MirWidth::Byte),
            MirArgHome::Reg(MirReg::X)
        );
        assert_eq!(
            action_arg_home(2, MirWidth::Byte),
            MirArgHome::Reg(MirReg::Y)
        );
        assert_eq!(
            action_arg_home(3, MirWidth::Byte),
            MirArgHome::FixedZeroPage(MirFixedZpSlot(0xa3))
        );
        assert_eq!(
            action_arg_home(0, MirWidth::Word),
            MirArgHome::RegisterPair {
                lo: MirReg::A,
                hi: MirReg::X,
            }
        );
        assert_eq!(
            action_arg_home(2, MirWidth::Word),
            MirArgHome::BytePair {
                lo: Box::new(MirArgHome::Reg(MirReg::Y)),
                hi: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(0xa3))),
            }
        );
    }

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
