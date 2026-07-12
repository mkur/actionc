#![allow(dead_code)]

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct VirtualTempId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VirtualTempWidth {
    Byte,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VirtualTempPurpose {
    Expression,
    Address,
    LocalScalar,
    CallResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VirtualTempHome {
    Unassigned,
    Register(RegisterName),
    ZeroPage {
        slot: StorageSlot,
        volatility: ZeroPageTempVolatility,
    },
    RoutineStorage(StorageSlot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ZeroPageTempVolatility {
    ClobberedByAnyCall,
    PreservedAcrossKnownCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VirtualTemp {
    pub(super) id: VirtualTempId,
    pub(super) width: VirtualTempWidth,
    pub(super) purpose: VirtualTempPurpose,
    pub(super) home: VirtualTempHome,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct VirtualTempAllocator {
    next_id: u32,
    temps: Vec<VirtualTemp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ZeroPageTempCandidate {
    pub(super) slot: StorageSlot,
    pub(super) volatility: ZeroPageTempVolatility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ZeroPageTempRange {
    pub(super) start: ZeroPage,
    pub(super) len: u8,
    pub(super) sliding: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ZeroPageTempPool {
    pub(super) ranges: Vec<ZeroPageTempRange>,
    pub(super) reserved: Vec<ZeroPageTempRange>,
    pub(super) volatility: ZeroPageTempVolatility,
}

impl VirtualTempWidth {
    pub(super) fn bytes(self) -> u16 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
        }
    }
}

impl VirtualTempAllocator {
    pub(super) fn create(
        &mut self,
        width: VirtualTempWidth,
        purpose: VirtualTempPurpose,
    ) -> VirtualTempId {
        let id = VirtualTempId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.temps.push(VirtualTemp {
            id,
            width,
            purpose,
            home: VirtualTempHome::Unassigned,
        });
        id
    }

    pub(super) fn get(&self, id: VirtualTempId) -> Option<&VirtualTemp> {
        self.temps.iter().find(|temp| temp.id == id)
    }

    pub(super) fn assign_home(&mut self, id: VirtualTempId, home: VirtualTempHome) -> bool {
        let Some(temp) = self.temps.iter_mut().find(|temp| temp.id == id) else {
            return false;
        };
        temp.home = home;
        true
    }

    pub(super) fn allocate_zero_page(
        &mut self,
        width: VirtualTempWidth,
        purpose: VirtualTempPurpose,
    ) -> Option<VirtualTempId> {
        self.allocate_zero_page_from_pool(
            width,
            purpose,
            &ZeroPageTempPool::default_action_modern(),
        )
    }

    pub(super) fn allocate_zero_page_from_pool(
        &mut self,
        width: VirtualTempWidth,
        purpose: VirtualTempPurpose,
        pool: &ZeroPageTempPool,
    ) -> Option<VirtualTempId> {
        let candidate = pool
            .candidates(width)
            .into_iter()
            .find(|candidate| self.zero_page_candidate_is_free(*candidate))?;
        let id = self.create(width, purpose);
        self.assign_home(
            id,
            VirtualTempHome::ZeroPage {
                slot: candidate.slot,
                volatility: candidate.volatility,
            },
        );
        Some(id)
    }

    fn zero_page_candidate_is_free(&self, candidate: ZeroPageTempCandidate) -> bool {
        !self.temps.iter().any(|temp| match temp.home {
            VirtualTempHome::ZeroPage { slot, .. } => storage_slots_overlap(slot, candidate.slot),
            _ => false,
        })
    }
}

impl ZeroPageTempRange {
    pub(super) fn fixed(start: ZeroPage, len: u8) -> Self {
        Self {
            start,
            len,
            sliding: false,
        }
    }

    pub(super) fn sliding(start: ZeroPage, len: u8) -> Self {
        Self {
            start,
            len,
            sliding: true,
        }
    }

    fn end_exclusive(self) -> u16 {
        u16::from(self.start.address()).saturating_add(u16::from(self.len))
    }

    fn contains_byte(self, address: u8) -> bool {
        let address = u16::from(address);
        address >= u16::from(self.start.address()) && address < self.end_exclusive().min(0x100)
    }
}

impl ZeroPageTempPool {
    pub(super) fn default_action_modern() -> Self {
        Self {
            ranges: vec![
                ZeroPageTempRange::fixed(runtime_zp::VALUE_TEMP, 1),
                ZeroPageTempRange::fixed(runtime_zp::ELEMENT_ADDR, 2),
                ZeroPageTempRange::fixed(runtime_zp::ARRAY_ADDR, 2),
                ZeroPageTempRange::fixed(runtime_zp::ADDR, 2),
            ],
            reserved: Vec::new(),
            volatility: ZeroPageTempVolatility::ClobberedByAnyCall,
        }
    }

    pub(super) fn with_ranges(ranges: Vec<ZeroPageTempRange>) -> Self {
        Self {
            ranges,
            reserved: Vec::new(),
            volatility: ZeroPageTempVolatility::ClobberedByAnyCall,
        }
    }

    pub(super) fn with_reserved(mut self, reserved: Vec<ZeroPageTempRange>) -> Self {
        self.reserved = reserved;
        self
    }

    pub(super) fn candidates(&self, width: VirtualTempWidth) -> Vec<ZeroPageTempCandidate> {
        let mut candidates = Vec::new();
        let size = width.bytes();
        for range in &self.ranges {
            candidates.extend(self.range_candidates(*range, size));
        }
        candidates
    }

    fn range_candidates(&self, range: ZeroPageTempRange, size: u16) -> Vec<ZeroPageTempCandidate> {
        if size == 0 || u16::from(range.len) < size {
            return Vec::new();
        }

        let first = range.start.address();
        let last = range.end_exclusive().min(0x100).saturating_sub(size) as u8;
        let starts: Vec<u8> = if range.sliding {
            (first..=last).collect()
        } else {
            vec![first]
        };

        starts
            .into_iter()
            .map(|start| ZeroPageTempCandidate {
                slot: StorageSlot::zero_page(start, size),
                volatility: self.volatility,
            })
            .filter(|candidate| !self.is_reserved(candidate.slot))
            .collect()
    }

    fn is_reserved(&self, slot: StorageSlot) -> bool {
        self.reserved.iter().any(|reserved| {
            (0..slot.size)
                .any(|byte_index| reserved.contains_byte(slot.zero_page_byte(byte_index).address()))
        })
    }
}

pub(super) fn zero_page_temp_candidates(width: VirtualTempWidth) -> Vec<ZeroPageTempCandidate> {
    ZeroPageTempPool::default_action_modern().candidates(width)
}

pub(super) fn zero_page_temp_survives_effects(
    home: VirtualTempHome,
    effects: RoutineEffects,
) -> bool {
    let VirtualTempHome::ZeroPage { slot, volatility } = home else {
        return true;
    };
    match volatility {
        ZeroPageTempVolatility::ClobberedByAnyCall => false,
        ZeroPageTempVolatility::PreservedAcrossKnownCall => {
            effects.known
                && (0..slot.size)
                    .all(|byte_index| !effects.writes_zero_page(slot.zero_page_byte(byte_index)))
        }
    }
}

pub(super) fn storage_slots_overlap(left: StorageSlot, right: StorageSlot) -> bool {
    if left.space != right.space {
        return false;
    }
    if !matches!(left.space, AddressSpace::Absolute | AddressSpace::ZeroPage) {
        return false;
    }
    let left_start = left.address;
    let left_end = left.address.saturating_add(left.size.max(1));
    let right_start = right.address;
    let right_end = right.address.saturating_add(right.size.max(1));
    left_start < right_end && right_start < left_end
}
