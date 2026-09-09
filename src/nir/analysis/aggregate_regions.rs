//! Bounded, read-only proofs for aggregate forwarding. No copies are rewritten.
//!
//! Resolving an address does not prove privacy, initialization, or permission to
//! merge homes. Likewise, an unchanged byte interval is only one prerequisite
//! for replacing a snapshot. Keep those obligations separate.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    cfg::NirCfg,
    dominance::NirDominance,
    use_def::{NirUseDef, NirUseKind, NirUseSite},
};
use crate::nir::*;

const ADDRESS_DEPTH_LIMIT: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NirAggregatePoint {
    pub block: BlockId,
    /// Position before an operation; ops.len() denotes the terminator.
    pub op_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NirAggregateProofFailure {
    InvalidPoint,
    CrossBlockInterval,
    UnknownStorage,
    AbsoluteStorage,
    AliasedStorage,
    UnknownExtent,
    OutOfBounds,
    UnknownAddress,
    AddressDepthLimit,
    UnavailableAddress,
    DynamicIndex,
    OverlappingWrite,
    UnknownWrite,
    VolatileBoundary,
    CallBoundary,
    MachineBoundary,
    FaultBoundary,
    UnsupportedEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirAggregateAddressUse {
    NotFormed,
    /// Only bounded, explicitly checked memory consumers of AddrOf temps.
    InternalOnly,
    /// Includes observable identity, stored/returned pointers, relocation
    /// exposure and unsupported flows; not necessarily a proven escape.
    ExposedOrUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirRegionRelation {
    Identical,
    Disjoint,
    PartialOverlap,
}

/// An in-bounds range in ordinary, non-aliased storage. Fields deliberately use
/// byte offsets, not FieldIds: two union members can denote the same bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirExactStorageRegion {
    pub memory: NirMemoryRegion,
    pub identity_domain: NirStorageIdentityDomain,
}

/// Facts borrow an immutable verified program; recreate after any rewrite.
pub struct NirAggregateRegionAnalysis<'a> {
    routines: BTreeMap<RoutineId, NirRoutineAggregateRegions<'a>>,
}

pub struct NirRoutineAggregateRegions<'a> {
    routine: &'a NirRoutine,
    globals: &'a [NirGlobal],
    storage: NirRoutineStorageAnalysis,
    cfg: NirCfg,
    dominance: NirDominance,
    use_def: NirUseDef,
}

pub fn analyze_aggregate_regions(
    program: &NirProgram,
) -> Result<NirAggregateRegionAnalysis<'_>, Vec<NirDiagnostic>> {
    verify_program(program)?;
    let storage = analyze_program_storage(program);
    Ok(NirAggregateRegionAnalysis {
        routines: program
            .routines
            .iter()
            .zip(storage.routines)
            .map(|(routine, storage)| {
                let cfg = NirCfg::from_routine(routine);
                let dominance = NirDominance::from_cfg(&cfg);
                (
                    routine.id,
                    NirRoutineAggregateRegions {
                        routine,
                        globals: &program.globals,
                        storage,
                        cfg,
                        dominance,
                        use_def: NirUseDef::from_routine(routine),
                    },
                )
            })
            .collect(),
    })
}

impl<'a> NirAggregateRegionAnalysis<'a> {
    pub fn routine(&self, id: RoutineId) -> Option<&NirRoutineAggregateRegions<'a>> {
        self.routines.get(&id)
    }
}

impl NirRoutineAggregateRegions<'_> {
    pub fn storage(&self) -> &NirRoutineStorageAnalysis {
        &self.storage
    }

    fn block(&self, point: NirAggregatePoint) -> Result<&NirBlock, NirAggregateProofFailure> {
        self.routine
            .blocks
            .iter()
            .find(|block| {
                block.id == point.block
                    && self.cfg.reachable().contains(&block.id)
                    && point.op_index <= block.ops.len()
            })
            .ok_or(NirAggregateProofFailure::InvalidPoint)
    }

    pub fn region(
        &self,
        place: &NirPlace,
        size: ByteSize,
        at: NirAggregatePoint,
    ) -> Result<NirExactStorageRegion, NirAggregateProofFailure> {
        self.block(at)?;
        let (id, offset) = self.place_address(place, at, ADDRESS_DEPTH_LIMIT)?;
        let facts = self
            .storage
            .homes
            .get(&id)
            .ok_or(NirAggregateProofFailure::UnknownStorage)?;
        match facts.backing {
            NirStorageBackingClass::Absolute => {
                return Err(NirAggregateProofFailure::AbsoluteStorage);
            }
            NirStorageBackingClass::Alias => return Err(NirAggregateProofFailure::AliasedStorage),
            NirStorageBackingClass::Ordinary => {}
        }
        if facts
            .blockers
            .contains(&NirPromotionBlocker::AliasedStorage)
        {
            return Err(NirAggregateProofFailure::AliasedStorage);
        }
        let extent = match id {
            NirStorageId::Global(id) => self
                .globals
                .iter()
                .find(|global| global.id == id)
                .map(|global| global.storage_size),
            _ => facts.layout.map(|layout| layout.size),
        }
        .ok_or(NirAggregateProofFailure::UnknownExtent)?;
        if size.is_zero() || u64::from(offset) + u64::from(size.get()) > u64::from(extent.get()) {
            return Err(NirAggregateProofFailure::OutOfBounds);
        }
        Ok(NirExactStorageRegion {
            memory: NirMemoryRegion {
                kind: NirMemoryRegionKind::Storage(id),
                offset: ByteOffset::new(offset),
                size,
            },
            identity_domain: facts.identity_domain,
        })
    }

    /// A relation only exists when BOTH endpoints resolve to ordinary exact
    /// ranges. Failure is unknown, never evidence of disjointness.
    pub fn relation(
        &self,
        left: &NirPlace,
        left_size: ByteSize,
        right: &NirPlace,
        right_size: ByteSize,
        at: NirAggregatePoint,
    ) -> Result<NirRegionRelation, NirAggregateProofFailure> {
        let left = self.region(left, left_size, at)?;
        let right = self.region(right, right_size, at)?;
        Ok(if left == right {
            NirRegionRelation::Identical
        } else if left.memory.overlaps(&right.memory) {
            NirRegionRelation::PartialOverlap
        } else {
            NirRegionRelation::Disjoint
        })
    }

    /// Prove that the bytes selected at `from` are not written by operations
    /// [from, to). This does NOT prove initialization, deadness, privacy, safe
    /// destination publication, or elimination of an entire copy.
    pub fn unchanged_between(
        &self,
        place: &NirPlace,
        size: ByteSize,
        from: NirAggregatePoint,
        to: NirAggregatePoint,
    ) -> Result<(), NirAggregateProofFailure> {
        let block = self.block(from)?;
        self.block(to)?;
        if from.block != to.block {
            return Err(NirAggregateProofFailure::CrossBlockInterval);
        }
        if from.op_index > to.op_index {
            return Err(NirAggregateProofFailure::InvalidPoint);
        }
        let source = self.region(place, size, from)?;
        for (index, op) in block
            .ops
            .iter()
            .enumerate()
            .take(to.op_index)
            .skip(from.op_index)
        {
            let at = NirAggregatePoint {
                block: block.id,
                op_index: index,
            };
            match op {
                NirOp::VolatileLoad { .. } | NirOp::VolatileStore { .. } => {
                    return Err(NirAggregateProofFailure::VolatileBoundary);
                }
                NirOp::CopyBytes {
                    destination_volatile: true,
                    ..
                }
                | NirOp::CopyBytes {
                    source_volatile: true,
                    ..
                } => return Err(NirAggregateProofFailure::VolatileBoundary),
                NirOp::Store { place, ty, .. } => {
                    let size = ty.width.ok_or(NirAggregateProofFailure::UnknownWrite)?;
                    self.check_write(&source, place, size, at)?;
                }
                NirOp::CopyBytes {
                    destination, size, ..
                } => self.check_write(&source, destination, *size, at)?,
                NirOp::Call { .. } => return Err(NirAggregateProofFailure::CallBoundary),
                NirOp::ForeignCode { .. } => return Err(NirAggregateProofFailure::MachineBoundary),
                NirOp::Real(_) | NirOp::Unsupported { .. } => {
                    return Err(NirAggregateProofFailure::UnsupportedEffect);
                }
                NirOp::Binary { .. } if crate::nir::optimizer::binary_may_fault(op) => {
                    return Err(NirAggregateProofFailure::FaultBoundary);
                }
                NirOp::Load { .. }
                | NirOp::AddrOf { .. }
                | NirOp::Unary { .. }
                | NirOp::Cast { .. }
                | NirOp::PointerOffset { .. }
                | NirOp::Binary { .. }
                | NirOp::Compare { .. } => {}
            }
        }
        Ok(())
    }

    fn check_write(
        &self,
        source: &NirExactStorageRegion,
        destination: &NirPlace,
        size: ByteSize,
        at: NirAggregatePoint,
    ) -> Result<(), NirAggregateProofFailure> {
        let destination = self
            .region(destination, size, at)
            .map_err(|_| NirAggregateProofFailure::UnknownWrite)?;
        if source.memory.overlaps(&destination.memory) {
            Err(NirAggregateProofFailure::OverlappingWrite)
        } else {
            Ok(())
        }
    }

    /// Classify explicit address uses without weakening scalar promotion or
    /// claiming automatic lifetime for Atari routine-static storage. Logical
    /// by-value calls/returns still require their own ABI/lifetime proof.
    pub fn address_use(&self, id: NirStorageId) -> NirAggregateAddressUse {
        let Some(facts) = self.storage.homes.get(&id) else {
            return NirAggregateAddressUse::ExposedOrUnknown;
        };
        if facts.backing != NirStorageBackingClass::Ordinary
            || facts.address_in_data
            || facts.machine_visible
            || facts
                .blockers
                .contains(&NirPromotionBlocker::AliasedStorage)
        {
            return NirAggregateAddressUse::ExposedOrUnknown;
        }
        let mut formed = false;
        for block in &self.routine.blocks {
            if !self.cfg.reachable().contains(&block.id) {
                continue;
            }
            for (op_index, op) in block.ops.iter().enumerate() {
                if let NirOp::AddrOf { dest, place, .. } = op {
                    let at = NirAggregatePoint {
                        block: block.id,
                        op_index,
                    };
                    if self
                        .place_address(place, at, ADDRESS_DEPTH_LIMIT)
                        .is_ok_and(|(root, _)| root == id)
                    {
                        formed = true;
                        if !self.internal_address_uses(*dest, &mut BTreeSet::new()) {
                            return NirAggregateAddressUse::ExposedOrUnknown;
                        }
                    }
                }
            }
        }
        match (formed, facts.address_taken) {
            (true, _) => NirAggregateAddressUse::InternalOnly,
            (false, false) => NirAggregateAddressUse::NotFormed,
            (false, true) => NirAggregateAddressUse::ExposedOrUnknown,
        }
    }

    fn internal_address_uses(&self, temp: TempId, visited: &mut BTreeSet<TempId>) -> bool {
        if visited.contains(&temp) {
            return true;
        }
        if visited.len() >= ADDRESS_DEPTH_LIMIT {
            return false;
        }
        visited.insert(temp);
        self.use_def.uses(temp).iter().all(|site| {
            let NirUseSite::Op {
                block,
                op_index,
                kind,
            } = *site
            else {
                return false;
            };
            let at = NirAggregatePoint { block, op_index };
            let Ok(block) = self.block(at) else {
                return false;
            };
            match (&block.ops[op_index], kind) {
                (NirOp::Load { place, ty, .. }, NirUseKind::LoadPlace)
                | (NirOp::Store { place, ty, .. }, NirUseKind::StorePlace) => ty
                    .width
                    .is_some_and(|size| self.region(place, size, at).is_ok()),
                (
                    NirOp::CopyBytes {
                        source,
                        size,
                        source_volatile: false,
                        destination_volatile: false,
                        ..
                    },
                    NirUseKind::LoadPlace,
                ) => self.region(source, *size, at).is_ok(),
                (
                    NirOp::CopyBytes {
                        destination,
                        size,
                        source_volatile: false,
                        destination_volatile: false,
                        ..
                    },
                    NirUseKind::StorePlace,
                ) => self.region(destination, *size, at).is_ok(),
                (NirOp::AddrOf { dest, place, .. }, NirUseKind::AddressPlace) => {
                    self.place_address(place, at, ADDRESS_DEPTH_LIMIT).is_ok()
                        && self.internal_address_uses(*dest, visited)
                }
                // Arithmetic/casts, mutable pointer relays, calls, identity
                // observations and cross-edge values are deliberately not
                // granted a no-escape proof in the first bounded slice.
                _ => false,
            }
        })
    }

    fn place_address(
        &self,
        place: &NirPlace,
        at: NirAggregatePoint,
        budget: usize,
    ) -> Result<(NirStorageId, u32), NirAggregateProofFailure> {
        if budget == 0 {
            return Err(NirAggregateProofFailure::AddressDepthLimit);
        }
        match &place.kind {
            NirPlaceKind::Local { id, .. } => Ok((NirStorageId::Local(*id), 0)),
            NirPlaceKind::Param { id, .. } => Ok((NirStorageId::Param(*id), 0)),
            NirPlaceKind::Global { id, .. } => Ok((NirStorageId::Global(*id), 0)),
            NirPlaceKind::Absolute(_) => Err(NirAggregateProofFailure::AbsoluteStorage),
            NirPlaceKind::Field { base, offset, .. } => {
                let (id, base) = self.place_address(base, at, budget - 1)?;
                Ok((
                    id,
                    base.checked_add(offset.get())
                        .ok_or(NirAggregateProofFailure::OutOfBounds)?,
                ))
            }
            NirPlaceKind::Deref { addr } => self.value_address(addr, at, budget - 1),
            NirPlaceKind::Index {
                base_addr,
                index,
                elem_size,
                ..
            } => {
                let (bits, ty) = index
                    .as_integer_const()
                    .ok_or(NirAggregateProofFailure::DynamicIndex)?;
                if ty.signed && bits & (1u64 << (ty.bits - 1)) != 0 {
                    return Err(NirAggregateProofFailure::OutOfBounds);
                }
                let offset = bits
                    .checked_mul(u64::from(elem_size.get()))
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or(NirAggregateProofFailure::OutOfBounds)?;
                let (id, base) = self.value_address(base_addr, at, budget - 1)?;
                Ok((
                    id,
                    base.checked_add(offset)
                        .ok_or(NirAggregateProofFailure::OutOfBounds)?,
                ))
            }
        }
    }

    fn value_address(
        &self,
        value: &NirValue,
        at: NirAggregatePoint,
        budget: usize,
    ) -> Result<(NirStorageId, u32), NirAggregateProofFailure> {
        if budget == 0 {
            return Err(NirAggregateProofFailure::AddressDepthLimit);
        }
        // Only exact SSA AddrOf origins. Loading a pointer from an ordinary
        // descriptor does not identify the object to which it currently points.
        let temp = value
            .temp()
            .ok_or(NirAggregateProofFailure::UnknownAddress)?;
        let definition = self
            .use_def
            .unique_definition(temp)
            .ok_or(NirAggregateProofFailure::UnknownAddress)?;
        let index = definition
            .op_index
            .ok_or(NirAggregateProofFailure::UnknownAddress)?;
        if (definition.block == at.block && index >= at.op_index)
            || !self.dominance.dominates(definition.block, at.block)
        {
            return Err(NirAggregateProofFailure::UnavailableAddress);
        }
        let definition = NirAggregatePoint {
            block: definition.block,
            op_index: index,
        };
        match &self.block(definition)?.ops[index] {
            NirOp::AddrOf { place, .. } => self.place_address(place, definition, budget - 1),
            _ => Err(NirAggregateProofFailure::UnknownAddress),
        }
    }
}
