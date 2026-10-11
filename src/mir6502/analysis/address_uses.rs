//! Typed address consumers and byte-window proofs, independent of source syntax.
#![allow(dead_code)] // Consumers are enabled in the following selection slice.

use crate::mir6502::ir::{MirAddr, MirMem, MirOp, MirValue, MirWidth};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirAddressBase {
    /// A value already evaluated by the program. Folding arithmetic must retain
    /// that evaluation, including a load preceding an aliasing write or call.
    Captured(MirValue),
    /// A pointer cell read at the consumer; this is not an immutable value.
    Cell(MirMem),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirAddressUse {
    pub base: MirAddressBase,
    pub index: Option<(MirValue, u16)>,
    pub displacement: u16,
    pub width: MirWidth,
}

impl MirAddressUse {
    pub(in crate::mir6502) fn from_op(op: &MirOp) -> Option<Self> {
        let (address, width) = match op {
            MirOp::Load { src, width, .. } => (src, *width),
            MirOp::Store { dst, width, .. } => (dst, *width),
            _ => return None,
        };
        let (base, index, displacement) = match address {
            MirAddr::Deref { ptr, offset } => {
                (MirAddressBase::Captured(ptr.clone()), None, *offset)
            }
            MirAddr::PointerCell { ptr, offset } => {
                (MirAddressBase::Cell(ptr.clone()), None, *offset)
            }
            MirAddr::ComputedIndex {
                base,
                index,
                elem_size,
                offset,
            } => (
                MirAddressBase::Captured(base.clone()),
                Some((index.clone(), *elem_size)),
                *offset,
            ),
            _ => return None,
        };
        Some(Self {
            base,
            index,
            displacement,
            width,
        })
    }
}

/// One complete byte/word access within a 6502 Y-index window. Rebase once when
/// its final byte would exceed 255; in particular, word offset 255 is rebased.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) struct MirIndirectWindow {
    pub base_adjustment: u16,
    pub y_displacement: u8,
}

impl MirIndirectWindow {
    pub(in crate::mir6502) fn constant(displacement: u16, width: MirWidth) -> Self {
        let last = u32::from(displacement) + u32::from(width == MirWidth::Word);
        if last <= 255 {
            Self {
                base_adjustment: 0,
                y_displacement: displacement as u8,
            }
        } else {
            Self {
                base_adjustment: displacement,
                y_displacement: 0,
            }
        }
    }

    /// The caller supplies a proven bound on a byte index, never an array
    /// capacity. Without a narrower bound, use 255 and rebase the field offset.
    /// A word access with index 255 needs the general full-width address path.
    pub(in crate::mir6502) fn byte_index(
        displacement: u16,
        width: MirWidth,
        max: u8,
    ) -> Option<Self> {
        let trailing = u32::from(width == MirWidth::Word);
        if u32::from(max) + trailing > 255 {
            return None;
        }
        if u32::from(displacement) + u32::from(max) + trailing <= 255 {
            Some(Self {
                base_adjustment: 0,
                y_displacement: displacement as u8,
            })
        } else {
            Some(Self {
                base_adjustment: displacement,
                y_displacement: 0,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{MirDef, MirTempId};
    use crate::nir::ParamId;

    #[test]
    fn captured_addresses_and_mutable_pointer_cells_are_distinct() {
        let captured = MirValue::Def(MirDef::VTemp(MirTempId(3)));
        let cell = MirMem::Param {
            id: ParamId(0),
            offset: 0,
        };
        let access = |src| MirOp::Load {
            dst: MirDef::VTemp(MirTempId(4)),
            src,
            width: MirWidth::Word,
        };
        assert_eq!(
            MirAddressUse::from_op(&access(MirAddr::Deref {
                ptr: captured.clone(),
                offset: 255
            }))
            .unwrap()
            .base,
            MirAddressBase::Captured(captured)
        );
        assert_eq!(
            MirAddressUse::from_op(&access(MirAddr::PointerCell {
                ptr: cell.clone(),
                offset: 255
            }))
            .unwrap()
            .base,
            MirAddressBase::Cell(cell)
        );
    }

    #[test]
    fn constant_windows_preserve_every_offset_and_both_lanes_including_wrap() {
        for width in [MirWidth::Byte, MirWidth::Word] {
            for displacement in 0..=u16::MAX {
                let window = MirIndirectWindow::constant(displacement, width);
                for lane in 0..=u16::from(width == MirWidth::Word) {
                    let y = u16::from(window.y_displacement) + lane;
                    assert!(y <= 255);
                    assert_eq!(
                        window.base_adjustment.wrapping_add(y),
                        displacement.wrapping_add(lane)
                    );
                }
            }
        }
    }

    #[test]
    fn dynamic_windows_require_the_complete_access_to_fit() {
        assert_eq!(
            MirIndirectWindow::byte_index(5, MirWidth::Byte, 63),
            Some(MirIndirectWindow {
                base_adjustment: 0,
                y_displacement: 5
            })
        );
        assert_eq!(
            MirIndirectWindow::byte_index(5, MirWidth::Byte, 255),
            Some(MirIndirectWindow {
                base_adjustment: 5,
                y_displacement: 0
            })
        );
        assert_eq!(MirIndirectWindow::byte_index(0, MirWidth::Word, 255), None);
        for displacement in [0u16, 254, 255, 256, 65535] {
            for width in [MirWidth::Byte, MirWidth::Word] {
                for max in [0u8, 63, 254, 255] {
                    if let Some(window) = MirIndirectWindow::byte_index(displacement, width, max) {
                        for index in 0..=max {
                            for lane in 0..=u16::from(width == MirWidth::Word) {
                                let y = u16::from(window.y_displacement) + u16::from(index) + lane;
                                assert!(y <= 255);
                                assert_eq!(
                                    window.base_adjustment.wrapping_add(y),
                                    displacement
                                        .wrapping_add(u16::from(index))
                                        .wrapping_add(lane)
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
