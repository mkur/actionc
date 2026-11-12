//! NIR range receipts bound to one completed materialization, not entry guesses.
//! Projection is deliberately limited to unchanged CFGs and unique user calls.
use super::{
    cfg::MirCfg,
    sites::{MirProgramPoint, MirRoutineGeneration, MirSite},
};
use crate::{mir6502::ir::*, nir};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mir6502) enum AbiByte {
    Register(MirReg),
    Cell(u8),
}

/// An indivisible numeric word. The two lane locations come from one logical
/// argument's verified ABI placement, never independent byte intervals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct CoherentWordRange {
    pub lo: AbiByte,
    pub hi: AbiByte,
    pub min: u16,
    pub max: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct NumericHomeRange {
    pub lo: MirMem,
    pub hi: MirMem,
    pub min: u16,
    pub max: u16,
}

#[derive(Clone, Debug)]
struct CallProjection {
    caller: Arc<MirRoutine>,
    point: MirProgramPoint,
    source_block: nir::BlockId,
    source_op: usize,
    receipts: Vec<nir::NirUnsignedRangeProof>,
    words: Vec<CoherentWordRange>,
    home_receipts: Vec<nir::NirStorageRangeProof>,
    homes: Vec<NumericHomeRange>,
}

pub(in crate::mir6502) struct MirAddressRanges<'a> {
    // Retain the immutable source generation through selection and final replay.
    _source: &'a nir::NirProgram,
    calls: BTreeMap<(RoutineId, RoutineId), CallProjection>,
}
impl<'a> MirAddressRanges<'a> {
    pub fn project(
        source: &'a nir::NirProgram,
        lowered: &MirProgram,
        materialized: &MirProgram,
    ) -> Result<Self, Vec<crate::mir6502::MirDiagnostic>> {
        let ranges = nir::analyze_unsigned_ranges(source).map_err(|errors| {
            errors
                .into_iter()
                .map(|e| crate::mir6502::MirDiagnostic {
                    routine: e.routine,
                    block: e.block,
                    message: e.message,
                })
                .collect::<Vec<_>>()
        })?;
        let source_ids = source
            .routines
            .iter()
            .map(|r| RoutineId(r.id.0))
            .collect::<BTreeSet<_>>();
        let mut calls = BTreeMap::new();
        for nr in &source.routines {
            let id = RoutineId(nr.id.0);
            let Some(old) = lowered.routines.iter().find(|r| r.id == id) else {
                continue;
            };
            let Some(new) = materialized.routines.iter().find(|r| r.id == id) else {
                continue;
            };
            let (Some(old_cfg), Some(new_cfg)) = (cfg_shape(old), cfg_shape(new)) else {
                continue;
            };
            if old.blocks.len() != nr.blocks.len()
                || new.blocks.iter().map(|b| b.ops.len()).sum::<usize>() > 4096
                || old_cfg != new_cfg
            {
                continue;
            }
            let mut frozen = None;
            let source_counts = user_counts(old, &source_ids);
            let final_counts = user_counts(new, &source_ids);
            for (block_index, nb) in nr.blocks.iter().enumerate() {
                let block_id = MirBlockId(block_index as u32);
                let (Some(ob), Some(mb)) = (
                    old.blocks.iter().find(|b| b.id == block_id),
                    new.blocks.iter().find(|b| b.id == block_id),
                ) else {
                    continue;
                };
                let Some(ncalls) = nb
                    .ops
                    .iter()
                    .enumerate()
                    .filter_map(|(i, op)| {
                        if let nir::NirOp::Call { callee, args, .. } = op {
                            Some(if let nir::NirCallee::User { id, .. } = callee {
                                Some((i, RoutineId(id.0), args))
                            } else {
                                None
                            })
                        } else {
                            None
                        }
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let before = user_calls(ob, &source_ids);
                let after = user_calls(mb, &source_ids);
                let targets = ncalls.iter().map(|(_, id, _)| *id).collect::<Vec<_>>();
                if before.iter().map(|(_, id, _)| *id).collect::<Vec<_>>() != targets
                    || after.iter().map(|(_, id, _)| *id).collect::<Vec<_>>() != targets
                {
                    continue;
                }
                for (((source_op, callee, nargs), (_, _, old_args)), (op_index, _, new_args)) in
                    ncalls.into_iter().zip(before).zip(after)
                {
                    // No census-based entry restriction. Uniqueness is solely
                    // a conservative association between two compiler phases.
                    if source_counts.get(&callee) != Some(&1)
                        || final_counts.get(&callee) != Some(&1)
                        || nargs.len() != old_args.len()
                    {
                        continue;
                    }
                    if !materialized
                        .routines
                        .iter()
                        .find(|r| r.id == callee)
                        .is_some_and(|r| {
                            super::known_callees::BoundedMachine::analyze(r, materialized).is_some()
                        })
                    {
                        continue;
                    }
                    let (Some(old_homes), Some(new_homes)) =
                        (argument_bytes(old_args), argument_bytes(new_args))
                    else {
                        continue;
                    };
                    if old_homes != new_homes {
                        continue;
                    }
                    let mut words = Vec::new();
                    let mut receipts = Vec::new();
                    let mut offset = 0;
                    for (value, arg) in nargs.iter().zip(old_args) {
                        if arg.width == MirWidth::Word {
                            if let Some(proof) = ranges.prove_before(nr.id, nb.id, source_op, value)
                            {
                                let range = proof.range();
                                if range.bits() == 16
                                    && range.max() <= 65535
                                    && (range.min() != 0 || range.max() != 65535)
                                {
                                    words.push(CoherentWordRange {
                                        lo: old_homes[offset],
                                        hi: old_homes[offset + 1],
                                        min: range.min() as u16,
                                        max: range.max() as u16,
                                    });
                                    receipts.push(proof);
                                }
                            }
                        }
                        offset += if arg.width == MirWidth::Word { 2 } else { 1 };
                    }
                    if words.is_empty() {
                        continue;
                    }
                    let mut homes = Vec::new();
                    let mut home_receipts = Vec::new();
                    for receipt in ranges.storage_ranges_before(nr.id, nb.id, source_op) {
                        let range = receipt.range();
                        let (_, _, _, storage) = receipt.source();
                        if range.bits() != 16 || range.max() > 65535 {
                            continue;
                        }
                        if let Some((lo, hi)) = ordinary_home(storage, new, &materialized.globals) {
                            homes.push(NumericHomeRange {
                                lo,
                                hi,
                                min: range.min() as u16,
                                max: range.max() as u16,
                            });
                            home_receipts.push(receipt);
                        }
                    }
                    calls.insert(
                        (id, callee),
                        CallProjection {
                            caller: frozen.get_or_insert_with(|| Arc::new(new.clone())).clone(),
                            point: MirProgramPoint {
                                generation: MirRoutineGeneration::initial(),
                                site: MirSite::Op {
                                    block: block_id,
                                    op_index,
                                },
                            },
                            source_block: nb.id,
                            source_op,
                            receipts,
                            words,
                            homes,
                            home_receipts,
                        },
                    );
                }
            }
        }
        Ok(Self {
            _source: source,
            calls,
        })
    }

    pub fn for_call(
        &self,
        caller: &MirRoutine,
        callee: RoutineId,
        generation: MirRoutineGeneration,
    ) -> Option<(MirProgramPoint, &[CoherentWordRange])> {
        let p = self.calls.get(&(caller.id, callee))?;
        if p.caller.as_ref() != caller
            || p.point.generation != generation
            || p.words.len() != p.receipts.len()
        {
            return None;
        }
        for (word, receipt) in p.words.iter().zip(&p.receipts) {
            let (owner, block, index, _) = receipt.source();
            let range = receipt.range();
            if owner.0 != caller.id.0
                || block != p.source_block
                || index != p.source_op
                || range.bits() != 16
                || range.min() != u32::from(word.min)
                || range.max() != u32::from(word.max)
            {
                return None;
            }
        }
        Some((p.point, &p.words))
    }

    pub fn homes_for_call(
        &self,
        caller: &MirRoutine,
        callee: RoutineId,
        generation: MirRoutineGeneration,
    ) -> Option<&[NumericHomeRange]> {
        self.for_call(caller, callee, generation)?;
        let p = self.calls.get(&(caller.id, callee))?;
        if p.homes.len() != p.home_receipts.len() {
            return None;
        }
        for (home, receipt) in p.homes.iter().zip(&p.home_receipts) {
            let (owner, block, index, _) = receipt.source();
            let range = receipt.range();
            if owner.0 != caller.id.0
                || block != p.source_block
                || index != p.source_op
                || range.bits() != 16
                || range.min() != u32::from(home.min)
                || range.max() != u32::from(home.max)
            {
                return None;
            }
        }
        Some(&p.homes)
    }

    pub fn validated_homes(
        &self,
        baseline: &MirProgram,
    ) -> BTreeMap<(RoutineId, RoutineId), Vec<NumericHomeRange>> {
        self.calls
            .keys()
            .filter_map(|&(owner, callee)| {
                let r = baseline.routines.iter().find(|r| r.id == owner)?;
                Some((
                    (owner, callee),
                    self.homes_for_call(r, callee, MirRoutineGeneration::initial())?
                        .to_vec(),
                ))
            })
            .collect()
    }

    pub fn validated_inputs(
        &self,
        baseline: &MirProgram,
    ) -> BTreeMap<(RoutineId, RoutineId), Vec<CoherentWordRange>> {
        self.calls
            .keys()
            .filter_map(|&(owner, callee)| {
                let r = baseline.routines.iter().find(|r| r.id == owner)?;
                let (_, words) = self.for_call(r, callee, MirRoutineGeneration::initial())?;
                Some(((owner, callee), words.to_vec()))
            })
            .collect()
    }
}

fn ordinary_home(
    id: nir::NirStorageId,
    r: &MirRoutine,
    globals: &[MirGlobal],
) -> Option<(MirMem, MirMem)> {
    match id {
        nir::NirStorageId::Param(id)
            if r.frame.params.iter().any(|s| {
                s.base == MirStorageBase::Param(id)
                    && s.scalar_width == Some(MirWidth::Word)
                    && s.storage_size >= 2
            }) =>
        {
            Some((
                MirMem::Param { id, offset: 0 },
                MirMem::Param { id, offset: 1 },
            ))
        }
        nir::NirStorageId::Local(id)
            if r.frame.locals.iter().any(|s| {
                s.base == MirStorageBase::Local(id)
                    && s.scalar_width == Some(MirWidth::Word)
                    && s.storage_size >= 2
            }) =>
        {
            Some((
                MirMem::Local { id, offset: 0 },
                MirMem::Local { id, offset: 1 },
            ))
        }
        nir::NirStorageId::Global(id)
            if globals.iter().any(|g| {
                g.id == id
                    && g.width == Some(MirWidth::Word)
                    && g.storage_size >= 2
                    && matches!(g.backing, MirGlobalBacking::Ordinary { .. })
            }) =>
        {
            Some((
                MirMem::Global { id, offset: 0 },
                MirMem::Global { id, offset: 1 },
            ))
        }
        _ => None,
    }
}

fn cfg_shape(r: &MirRoutine) -> Option<(Option<MirBlockId>, Vec<(MirBlockId, Vec<MirBlockId>)>)> {
    let cfg = MirCfg::from_routine(r).ok()?;
    Some((
        cfg.entry(),
        cfg.reachable()
            .iter()
            .map(|&id| (id, cfg.successors(id).iter().copied().collect()))
            .collect(),
    ))
}

fn user_calls<'a>(
    b: &'a MirBlock,
    ids: &BTreeSet<RoutineId>,
) -> Vec<(usize, RoutineId, &'a [MirCallArg])> {
    b.ops
        .iter()
        .enumerate()
        .filter_map(|(i, op)| match op {
            MirOp::Call {
                target: MirCallTarget::Routine(id),
                args,
                ..
            } if ids.contains(id) => Some((i, *id, args.as_slice())),
            _ => None,
        })
        .collect()
}
fn user_counts(r: &MirRoutine, ids: &BTreeSet<RoutineId>) -> BTreeMap<RoutineId, usize> {
    let mut counts = BTreeMap::new();
    for (_, id, _) in r.blocks.iter().flat_map(|b| user_calls(b, ids)) {
        *counts.entry(id).or_default() += 1;
    }
    counts
}

fn argument_bytes(args: &[MirCallArg]) -> Option<Vec<AbiByte>> {
    let mut bytes = Vec::new();
    for arg in args {
        home_bytes(&arg.home, arg.width, &mut bytes)?;
    }
    if bytes.len() > 16
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| bytes[..i].contains(b))
    {
        return None;
    }
    Some(bytes)
}
fn home_bytes(home: &MirArgHome, width: MirWidth, out: &mut Vec<AbiByte>) -> Option<()> {
    match (width, home) {
        (MirWidth::Byte, MirArgHome::Reg(r)) => out.push(AbiByte::Register(*r)),
        (MirWidth::Byte, MirArgHome::FixedZeroPage(z)) => out.push(AbiByte::Cell(z.0)),
        (MirWidth::Byte, MirArgHome::Absolute(a)) if *a < 256 => out.push(AbiByte::Cell(*a as u8)),
        (MirWidth::Word, MirArgHome::RegisterPair { lo, hi }) => {
            out.extend([AbiByte::Register(*lo), AbiByte::Register(*hi)])
        }
        (MirWidth::Word, MirArgHome::BytePair { lo, hi }) => {
            home_bytes(lo, MirWidth::Byte, out)?;
            home_bytes(hi, MirWidth::Byte, out)?;
        }
        (MirWidth::Word, MirArgHome::FixedZeroPage(z)) => {
            out.extend([AbiByte::Cell(z.0), AbiByte::Cell(z.0.checked_add(1)?)])
        }
        _ => return None,
    }
    Some(())
}

#[cfg(test)]
mod tests;
