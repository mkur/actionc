//! Constants in exact byte cells of private aggregate captures. No source
//! constructor, field name, validity assumption, or static initializer is used.
use std::collections::{BTreeMap, BTreeSet};

use super::analysis::{
    cfg::NirCfg,
    dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow},
};
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Cell {
    local: LocalId,
    offset: ByteOffset,
}

impl Cell {
    fn memory(self) -> NirMemoryRegion {
        NirMemoryRegion {
            kind: NirMemoryRegionKind::Storage(NirStorageId::Local(self.local)),
            offset: self.offset,
            size: ByteSize::ONE,
        }
    }
}

type Facts = BTreeMap<Cell, u8>;

/// An immutable proof census. Unknown reads are barriers as well as unknown
/// writes: absolute memory and opaque pointer accesses can observe hardware.
#[derive(Debug, Clone)]
enum Effect {
    Preserve,
    Barrier,
    Load {
        dest: TempId,
        cell: Option<Cell>,
    },
    Store {
        region: NirMemoryRegion,
        value: Option<(Cell, u8)>,
    },
}

fn eligible_locals(
    routine: &NirRoutine,
    regions: &NirRoutineAggregateRegions<'_>,
) -> BTreeSet<LocalId> {
    routine.locals.iter().filter(|local| {
        local.purpose == NirLocalPurpose::AggregateCapture
            && matches!(local.backing, NirLocalBacking::Ordinary)
            && regions.address_use(NirStorageId::Local(local.id)) != NirAggregateAddressUse::ExposedOrUnknown
            && regions.storage().homes.get(&NirStorageId::Local(local.id)).is_some_and(|home| {
                matches!(home.identity_domain,
                    NirStorageIdentityDomain::Routine(id) | NirStorageIdentityDomain::Invocation(id) if id == routine.id)
            })
    }).map(|local| local.id).collect()
}

fn cell(
    region: &NirExactStorageRegion,
    ty: &NirType,
    eligible: &BTreeSet<LocalId>,
) -> Option<Cell> {
    if ty.kind != NirTypeKind::Integer(NirIntegerType::U8) || ty.width != Some(ByteSize::ONE) {
        return None;
    }
    match region.memory.kind {
        NirMemoryRegionKind::Storage(NirStorageId::Local(local)) if eligible.contains(&local) => {
            Some(Cell {
                local,
                offset: region.memory.offset,
            })
        }
        _ => None,
    }
}

fn classify(
    op: &NirOp,
    at: NirAggregatePoint,
    regions: &NirRoutineAggregateRegions<'_>,
    eligible: &BTreeSet<LocalId>,
) -> Effect {
    match op {
        NirOp::Load { dest, ty, place } => {
            match ty
                .width
                .and_then(|width| regions.region(place, width, at).ok())
            {
                Some(region) => Effect::Load {
                    dest: *dest,
                    cell: cell(&region, ty, eligible),
                },
                None => Effect::Barrier,
            }
        }
        NirOp::Store { place, ty, src } => {
            match ty
                .width
                .and_then(|width| regions.region(place, width, at).ok())
            {
                Some(region) => {
                    let value = cell(&region, ty, eligible).and_then(|cell| match src {
                        NirValue::IntegerConst {
                            bits,
                            ty: NirIntegerType::U8,
                        } => Some((cell, *bits as u8)),
                        _ => None,
                    });
                    Effect::Store {
                        region: region.memory,
                        value,
                    }
                }
                None => Effect::Barrier,
            }
        }
        NirOp::CopyBytes {
            destination,
            source,
            size,
            destination_volatile: false,
            source_volatile: false,
        } => {
            match (
                regions.region(source, *size, at),
                regions.region(destination, *size, at),
            ) {
                (Ok(_), Ok(destination)) => Effect::Store {
                    region: destination.memory,
                    value: None,
                },
                _ => Effect::Barrier,
            }
        }
        NirOp::VolatileLoad { .. }
        | NirOp::VolatileStore { .. }
        | NirOp::CopyBytes { .. }
        | NirOp::Call { .. }
        | NirOp::ForeignCode { .. }
        | NirOp::Real(_)
        | NirOp::Unsupported { .. } => Effect::Barrier,
        NirOp::Binary { .. } if super::optimizer::binary_may_fault(op) => Effect::Barrier,
        NirOp::AddrOf { .. }
        | NirOp::Unary { .. }
        | NirOp::Cast { .. }
        | NirOp::PointerOffset { .. }
        | NirOp::Binary { .. }
        | NirOp::Compare { .. } => Effect::Preserve,
    }
}

fn transfer(effect: &Effect, facts: &mut Facts) -> Option<(TempId, NirValue)> {
    match effect {
        Effect::Barrier => facts.clear(),
        Effect::Store { region, value } => {
            facts.retain(|cell, _| !cell.memory().overlaps(region));
            if let Some((cell, value)) = value {
                facts.insert(*cell, *value);
            }
        }
        Effect::Load {
            dest,
            cell: Some(cell),
        } => {
            return facts
                .get(cell)
                .map(|value| (*dest, NirValue::ConstU8(*value)));
        }
        Effect::Preserve | Effect::Load { cell: None, .. } => {}
    }
    None
}

/// None is unreachable/unprocessed; Some(empty) is reachable but unknown.
/// A backedge cannot invent initialization on the entry path.
struct ByteProblem<'a> {
    entry: Option<BlockId>,
    effects: &'a BTreeMap<BlockId, Vec<Effect>>,
}

impl NirDataflowProblem for ByteProblem<'_> {
    type State = Option<Facts>;
    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Forward
    }
    fn bottom(&self) -> Self::State {
        None
    }
    fn boundary(&self, block: BlockId) -> Option<Self::State> {
        (Some(block) == self.entry).then(|| Some(Facts::new()))
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        let Some(other) = other else {
            return;
        };
        match into {
            Some(into) => into.retain(|cell, value| other.get(cell) == Some(value)),
            None => *into = Some(other.clone()),
        }
    }
    fn transfer(&self, block: BlockId, state: &Self::State) -> Self::State {
        let mut facts = state.clone()?;
        for effect in &self.effects[&block] {
            transfer(effect, &mut facts);
        }
        Some(facts)
    }
}

pub(super) fn propagate_program(program: &NirProgram) -> Result<NirProgram, Vec<NirDiagnostic>> {
    // Region analysis verifies the input and borrows this exact generation.
    let analyses = analyze_aggregate_regions(program)?;
    let mut optimized = program.clone();
    for (routine, output) in program.routines.iter().zip(&mut optimized.routines) {
        let regions = analyses.routine(routine.id).expect("analyzed routine");
        let eligible = eligible_locals(routine, regions);
        if eligible.is_empty() {
            continue;
        }
        let cfg = NirCfg::from_routine(routine);
        let effects: BTreeMap<_, Vec<_>> = routine
            .blocks
            .iter()
            .map(|block| {
                (
                    block.id,
                    block
                        .ops
                        .iter()
                        .enumerate()
                        .map(|(op_index, op)| {
                            classify(
                                op,
                                NirAggregatePoint {
                                    block: block.id,
                                    op_index,
                                },
                                regions,
                                &eligible,
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        let result = solve_dataflow(
            &cfg,
            &ByteProblem {
                entry: cfg.entry(),
                effects: &effects,
            },
        );
        let mut replacements = BTreeMap::new();
        for block in &routine.blocks {
            let Some(mut facts) = result.in_state(block.id).and_then(Option::as_ref).cloned()
            else {
                continue;
            };
            for effect in &effects[&block.id] {
                if let Some((dest, value)) = transfer(effect, &mut facts) {
                    replacements.insert(dest, value);
                }
            }
        }
        if replacements.is_empty() {
            continue;
        }
        for block in &mut output.blocks {
            block.ops.retain(
                |op| !matches!(op, NirOp::Load { dest, .. } if replacements.contains_key(dest)),
            );
            for op in &mut block.ops {
                super::optimizer::rewrite_op_values(op, &replacements);
            }
            super::optimizer::rewrite_terminator_values(&mut block.terminator, &replacements);
        }
        output.temps = super::optimizer::collect_temps(&output.blocks);
    }
    verify_program(&optimized)?;
    Ok(optimized)
}

/// Every successful round removes at least one load; ordinary value cleanup
/// cannot introduce loads. The budget therefore permits a complete fixed point
/// without repeating scalar promotion or ABI work.
pub(super) fn stabilize_program(program: &NirProgram) -> Result<NirProgram, Vec<NirDiagnostic>> {
    let load_count = program
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .filter(|op| matches!(op, NirOp::Load { .. }))
        .count();
    let mut optimized = program.clone();
    for _ in 0..=load_count {
        let next = propagate_program(&optimized)?;
        if next == optimized {
            break;
        }
        optimized = super::optimizer::optimize_program(&next)?;
    }
    Ok(optimized)
}
