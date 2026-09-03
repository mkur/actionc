use std::collections::{BTreeMap, BTreeSet};

use super::analysis::{
    cfg::NirCfg,
    dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow},
    dominance::NirDominance,
    storage::{NirRoutineStorageAnalysis, NirStorageFacts},
};
use super::facts::{BlockId, NirStorageId, NirType, NirValue, TempId, value_width};
use super::ir::*;
use super::{NirDiagnostic, analyze_program_storage, direct_storage_id, verify_program};
use crate::target::{ByteOffset, ByteSize};

// Promotion exposes long-lived values to the target allocator. Keep the
// general automatic tier to hot byte homes with a small definition set;
// colder and wider homes can otherwise replace direct storage traffic
// one-for-one with spills. A separate word tier recognizes loop induction
// values used in indexed addresses. Those have enough dynamic traffic to
// justify promotion and give target backends an explicit loop-carried value
// to cache in scarce fast storage.
const MIN_HOT_HOME_LOADS: usize = 7;
const MIN_INDUCTION_ADDRESS_LOADS: usize = 3;
const MAX_HOT_HOME_STORE_BLOCKS: usize = 2;

pub(super) fn promote_program(program: &NirProgram) -> Result<NirProgram, Vec<NirDiagnostic>> {
    verify_program(program)?;
    let analyses = analyze_program_storage(program);
    let mut promoted = program.clone();
    for (routine, analysis) in promoted.routines.iter_mut().zip(&analyses.routines) {
        promote_routine(routine, analysis);
    }
    verify_program(&promoted)?;
    Ok(promoted)
}

fn promote_routine(routine: &mut NirRoutine, analysis: &NirRoutineStorageAnalysis) {
    let cfg = NirCfg::from_routine(routine);
    let Some(entry) = cfg.entry() else {
        return;
    };
    if !cfg.predecessors(entry).is_empty() {
        return;
    }
    let induction_address_homes = induction_address_homes(routine, &cfg);

    let mut next_temp = routine
        .temps
        .iter()
        .map(|temp| temp.id.0)
        .max()
        .unwrap_or(0)
        .saturating_add(u32::from(!routine.temps.is_empty()));
    let homes = analysis
        .homes
        .values()
        .filter(|facts| facts.is_promotable())
        .filter(|facts| matches!(facts.id, NirStorageId::Local(_)))
        .filter(|facts| facts.store_blocks.len() <= MAX_HOT_HOME_STORE_BLOCKS)
        .filter(|facts| {
            let width = facts.direct_access_ty.as_ref().and_then(|ty| ty.width);
            width == Some(ByteSize::ONE) && facts.direct_loads >= MIN_HOT_HOME_LOADS
                || width == Some(ByteSize::new(2))
                    && facts.direct_loads >= MIN_INDUCTION_ADDRESS_LOADS
                    && induction_address_homes.contains(&facts.id)
        })
        .cloned()
        .collect::<Vec<_>>();

    for facts in homes {
        let mut candidate = routine.clone();
        if promote_home(&mut candidate, &facts, &mut next_temp) {
            *routine = candidate;
        }
    }
    routine.temps = collect_temps(&routine.blocks);
}

/// Finds word locals which are updated by an add/sub recurrence and used as
/// an array index in the same natural loop. This is deliberately structural:
/// the target-independent pass identifies the loop-carried computation, but
/// does not choose registers, zero-page addresses, or an addressing mode.
fn induction_address_homes(routine: &NirRoutine, cfg: &NirCfg) -> BTreeSet<NirStorageId> {
    let dominance = NirDominance::from_cfg(cfg);
    let loops = natural_loops(cfg, &dominance);
    if loops.is_empty() {
        return BTreeSet::new();
    }

    let mut indexed_uses = BTreeMap::<NirStorageId, BTreeSet<BlockId>>::new();
    let mut recurrence_updates = BTreeMap::<NirStorageId, BTreeSet<BlockId>>::new();
    for block in &routine.blocks {
        if !cfg.reachable().contains(&block.id) {
            continue;
        }
        let direct_loads = block
            .ops
            .iter()
            .filter_map(|op| match op {
                NirOp::Load { dest, place, .. } => {
                    direct_storage_id(place).map(|storage| (*dest, storage))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let binary_updates = block
            .ops
            .iter()
            .filter_map(|op| match op {
                NirOp::Binary {
                    dest,
                    op: NirBinaryOp::Add | NirBinaryOp::Sub,
                    left,
                    right,
                    ..
                } => Some((*dest, (left, right))),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();

        for op in &block.ops {
            match op {
                NirOp::Load { place, .. } | NirOp::Store { place, .. } => {
                    for index in place_index_values(place) {
                        let NirValue::Temp { id, .. } = index else {
                            continue;
                        };
                        let Some(storage) = direct_loads.get(id) else {
                            continue;
                        };
                        indexed_uses.entry(*storage).or_default().insert(block.id);
                    }
                }
                _ => {}
            }

            let NirOp::Store { place, src, ty } = op else {
                continue;
            };
            let Some(storage) = direct_storage_id(place) else {
                continue;
            };
            if ty.width != Some(ByteSize::new(2)) {
                continue;
            }
            let NirValue::Temp { id: src, .. } = src else {
                continue;
            };
            let Some((left, right)) = binary_updates.get(src) else {
                continue;
            };
            if value_is_direct_load_of(left, storage, &direct_loads)
                || value_is_direct_load_of(right, storage, &direct_loads)
            {
                recurrence_updates
                    .entry(storage)
                    .or_default()
                    .insert(block.id);
            }
        }
    }

    indexed_uses
        .into_iter()
        .filter_map(|(storage, uses)| {
            let updates = recurrence_updates.get(&storage)?;
            loops
                .iter()
                .any(|loop_blocks| {
                    uses.iter().any(|block| loop_blocks.contains(block))
                        && updates.iter().any(|block| loop_blocks.contains(block))
                })
                .then_some(storage)
        })
        .collect()
}

fn value_is_direct_load_of(
    value: &NirValue,
    storage: NirStorageId,
    direct_loads: &BTreeMap<TempId, NirStorageId>,
) -> bool {
    matches!(value, NirValue::Temp { id, .. } if direct_loads.get(id) == Some(&storage))
}

fn place_index_values(place: &NirPlace) -> Vec<&NirValue> {
    match &place.kind {
        NirPlaceKind::Index { index, .. } => vec![index],
        NirPlaceKind::Field { base, .. } => place_index_values(base),
        _ => Vec::new(),
    }
}

fn natural_loops(cfg: &NirCfg, dominance: &NirDominance) -> Vec<BTreeSet<BlockId>> {
    let mut loops = Vec::new();
    for source in cfg.reachable() {
        for header in cfg.successors(*source) {
            if !dominance.is_backedge(*source, *header) {
                continue;
            }
            let mut blocks = BTreeSet::from([*header, *source]);
            let mut work = vec![*source];
            while let Some(block) = work.pop() {
                for predecessor in cfg.predecessors(block) {
                    if *predecessor != *header && blocks.insert(*predecessor) {
                        work.push(*predecessor);
                    }
                }
            }
            if !loops.contains(&blocks) {
                loops.push(blocks);
            }
        }
    }
    loops
}

fn promote_home(routine: &mut NirRoutine, facts: &NirStorageFacts, next_temp: &mut u32) -> bool {
    let Some(ty) = facts.direct_access_ty.clone().or_else(|| facts.ty.clone()) else {
        return false;
    };
    let Some(place) = home_place(routine, facts.id, &ty) else {
        return false;
    };
    let cfg = NirCfg::from_routine(routine);
    let Some(entry) = cfg.entry() else {
        return false;
    };
    let access = HomeAccess::analyze(routine, &cfg, facts, &ty);
    let needs_entry_value = matches!(facts.id, NirStorageId::Param(_))
        || access.live_in.get(&entry).copied().unwrap_or(false);
    let mut definitions = access.definition_blocks;
    let seed = needs_entry_value.then(|| fresh_temp(next_temp));
    if seed.is_some() {
        definitions.insert(entry);
    }
    let live_in = access
        .live_in
        .iter()
        .filter_map(|(block, live)| (*live).then_some(*block))
        .collect::<BTreeSet<_>>();
    let dominance = NirDominance::from_cfg(&cfg);
    let mut phi_blocks = dominance.pruned_iterated_frontier(&definitions, &live_in);
    phi_blocks.remove(&entry);

    let mut phi_temps = BTreeMap::new();
    for block in &mut routine.blocks {
        if !phi_blocks.contains(&block.id) {
            continue;
        }
        let dest = fresh_temp(next_temp);
        block.params.push(NirBlockParam {
            dest,
            ty: ty.clone(),
        });
        phi_temps.insert(block.id, dest);
    }

    let seed_value = seed.map(|id| NirValue::Temp { id, ty: ty.clone() });
    let mut context = RenameContext {
        routine_name: routine.name.clone(),
        storage: facts.id,
        ty,
        place,
        value_needed_at_exit: facts.value_needed_at_exit,
        phi_temps,
        dominance,
        next_temp,
    };
    rename_block(
        routine,
        entry,
        seed_value,
        BTreeMap::new(),
        seed,
        &mut context,
    )
}

struct RenameContext<'a> {
    routine_name: String,
    storage: NirStorageId,
    ty: NirType,
    place: NirPlace,
    value_needed_at_exit: bool,
    phi_temps: BTreeMap<BlockId, TempId>,
    dominance: NirDominance,
    next_temp: &'a mut u32,
}

fn rename_block(
    routine: &mut NirRoutine,
    block_id: BlockId,
    inherited: Option<NirValue>,
    mut replacements: BTreeMap<TempId, NirValue>,
    seed: Option<TempId>,
    context: &mut RenameContext<'_>,
) -> bool {
    let Some(block_index) = routine.blocks.iter().position(|block| block.id == block_id) else {
        return false;
    };
    let mut current = context
        .phi_temps
        .get(&block_id)
        .map(|id| NirValue::Temp {
            id: *id,
            ty: context.ty.clone(),
        })
        .or(inherited);

    let original_ops = std::mem::take(&mut routine.blocks[block_index].ops);
    let mut rewritten = Vec::with_capacity(original_ops.len().saturating_add(2));
    if seed.is_some() && block_id == context.dominance_root() {
        rewritten.push(NirOp::Load {
            dest: seed.expect("entry seed"),
            ty: context.ty.clone(),
            place: context.place.clone(),
        });
    }

    for mut op in original_ops {
        rewrite_op_values(&mut op, &replacements);
        match &op {
            NirOp::Load { dest, place, .. }
                if direct_storage_id(place) == Some(context.storage) =>
            {
                let Some(value) = current.clone() else {
                    return false;
                };
                replacements.insert(*dest, value);
            }
            NirOp::Store { place, src, ty, .. }
                if direct_storage_id(place) == Some(context.storage) =>
            {
                if ty.width != context.ty.width || !store_value_fits_home(src, &context.ty) {
                    return false;
                }
                current = coerce_to_home_type(src.clone(), &mut rewritten, context);
                if current.is_none() {
                    return false;
                }
            }
            NirOp::Call {
                callee, effects, ..
            } => {
                let (reads, writes) = call_access(
                    callee,
                    effects,
                    context.storage,
                    context.ty.width,
                    &context.routine_name,
                );
                if reads {
                    let Some(value) = current.clone() else {
                        return false;
                    };
                    rewritten.push(sync_store(context, value));
                }
                let result = op_result(&op);
                rewritten.push(op);
                if let Some(result) = result {
                    replacements.remove(&result);
                }
                if writes {
                    let dest = fresh_temp(context.next_temp);
                    rewritten.push(reload(context, dest));
                    current = Some(NirValue::Temp {
                        id: dest,
                        ty: context.ty.clone(),
                    });
                }
            }
            NirOp::ForeignCode { effects, .. } => {
                let reads = effects.opaque
                    || memory_accesses_storage(
                        &effects.memory.reads,
                        context.storage,
                        context.ty.width,
                    );
                let writes = effects.opaque
                    || memory_accesses_storage(
                        &effects.memory.writes,
                        context.storage,
                        context.ty.width,
                    );
                if reads {
                    let Some(value) = current.clone() else {
                        return false;
                    };
                    rewritten.push(sync_store(context, value));
                }
                rewritten.push(op);
                if writes {
                    let dest = fresh_temp(context.next_temp);
                    rewritten.push(reload(context, dest));
                    current = Some(NirValue::Temp {
                        id: dest,
                        ty: context.ty.clone(),
                    });
                }
            }
            NirOp::Unsupported { .. } => {
                let Some(value) = current.clone() else {
                    return false;
                };
                rewritten.push(sync_store(context, value));
                rewritten.push(op);
                let dest = fresh_temp(context.next_temp);
                rewritten.push(reload(context, dest));
                current = Some(NirValue::Temp {
                    id: dest,
                    ty: context.ty.clone(),
                });
            }
            _ => {
                if let Some(result) = op_result(&op) {
                    replacements.remove(&result);
                }
                rewritten.push(op);
            }
        }
    }

    rewrite_terminator_values(&mut routine.blocks[block_index].terminator, &replacements);
    if context.value_needed_at_exit && is_observable_exit(&routine.blocks[block_index].terminator) {
        let Some(value) = current.clone() else {
            return false;
        };
        rewritten.push(sync_store(context, value));
    }
    if !append_phi_arguments(
        &mut routine.blocks[block_index].terminator,
        current.as_ref(),
        &context.phi_temps,
    ) {
        return false;
    }
    routine.blocks[block_index].ops = rewritten;

    let children = context.dominance.children(block_id).to_vec();
    for child in children {
        if !rename_block(
            routine,
            child,
            current.clone(),
            replacements.clone(),
            seed,
            context,
        ) {
            return false;
        }
    }
    true
}

impl RenameContext<'_> {
    fn dominance_root(&self) -> BlockId {
        self.dominance
            .root()
            .expect("promotion dominance tree has an entry")
    }
}

fn append_phi_arguments(
    terminator: &mut NirTerminator,
    current: Option<&NirValue>,
    phi_temps: &BTreeMap<BlockId, TempId>,
) -> bool {
    let append = |edge: &mut NirEdge| {
        if phi_temps.contains_key(&edge.target) {
            let Some(value) = current else {
                return false;
            };
            edge.args.push(value.clone());
        }
        true
    };
    match terminator {
        NirTerminator::Goto(edge) => append(edge),
        NirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => append(then_edge) && append(else_edge),
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Return(_)
        | NirTerminator::Exit => true,
    }
}

fn coerce_to_home_type(
    value: NirValue,
    rewritten: &mut Vec<NirOp>,
    context: &mut RenameContext<'_>,
) -> Option<NirValue> {
    let actual = match &value {
        NirValue::ConstU8(value) if context.ty.width == Some(ByteSize::new(2)) => {
            return Some(NirValue::ConstU16(u16::from(*value)));
        }
        NirValue::ConstU16(value) if context.ty.width == Some(ByteSize::ONE) => {
            return u8::try_from(*value).ok().map(NirValue::ConstU8);
        }
        NirValue::ConstU8(_) | NirValue::ConstU16(_) => return Some(value),
        NirValue::Null { ty }
        | NirValue::AddressConst { ty, .. }
        | NirValue::StaticAddr { ty, .. }
        | NirValue::Temp { ty, .. }
        | NirValue::RoutineAddr { ty, .. } => ty.clone(),
        NirValue::Param(_) | NirValue::GlobalAddr(_) => return None,
    };
    if actual == context.ty {
        return Some(value);
    }
    if actual.width != context.ty.width {
        return None;
    }
    let dest = fresh_temp(context.next_temp);
    rewritten.push(NirOp::Cast {
        dest,
        src: value,
        from: actual,
        to: context.ty.clone(),
        kind: NirCastKind::Integer,
    });
    Some(NirValue::Temp {
        id: dest,
        ty: context.ty.clone(),
    })
}

fn store_value_fits_home(value: &NirValue, home_ty: &NirType) -> bool {
    match value {
        NirValue::ConstU8(_) => matches!(home_ty.width.map(ByteSize::get), Some(1 | 2)),
        NirValue::ConstU16(value) => {
            home_ty.width == Some(ByteSize::new(2))
                || home_ty.width == Some(ByteSize::ONE) && *value <= u16::from(u8::MAX)
        }
        _ => value_width(value) == home_ty.width,
    }
}

fn sync_store(context: &RenameContext<'_>, src: NirValue) -> NirOp {
    NirOp::Store {
        place: context.place.clone(),
        src,
        ty: context.ty.clone(),
    }
}

fn reload(context: &RenameContext<'_>, dest: TempId) -> NirOp {
    NirOp::Load {
        dest,
        ty: context.ty.clone(),
        place: context.place.clone(),
    }
}

fn home_place(routine: &NirRoutine, storage: NirStorageId, ty: &NirType) -> Option<NirPlace> {
    let kind = match storage {
        NirStorageId::Local(id) => {
            let local = routine.locals.iter().find(|local| local.id == id)?;
            NirPlaceKind::Local {
                id,
                name: local.name.clone(),
            }
        }
        NirStorageId::Param(id) => {
            let param = routine.params.iter().find(|param| param.id == id)?;
            NirPlaceKind::Param {
                id,
                name: param.name.clone(),
            }
        }
        NirStorageId::Global(_) => return None,
    };
    Some(NirPlace {
        kind,
        ty: Some(ty.clone()),
    })
}

struct HomeAccess {
    definition_blocks: BTreeSet<BlockId>,
    live_in: BTreeMap<BlockId, bool>,
}

impl HomeAccess {
    fn analyze(routine: &NirRoutine, cfg: &NirCfg, facts: &NirStorageFacts, ty: &NirType) -> Self {
        let mut blocks = BTreeMap::new();
        let mut definition_blocks = BTreeSet::new();
        for block in &routine.blocks {
            if !cfg.reachable().contains(&block.id) {
                continue;
            }
            let mut uses_before_definition = false;
            let mut defines = false;
            for op in &block.ops {
                match op {
                    NirOp::Load { place, .. } if direct_storage_id(place) == Some(facts.id) => {
                        uses_before_definition |= !defines;
                    }
                    NirOp::Store { place, .. } if direct_storage_id(place) == Some(facts.id) => {
                        defines = true;
                        definition_blocks.insert(block.id);
                    }
                    NirOp::Call {
                        callee, effects, ..
                    } => {
                        let (reads, writes) =
                            call_access(callee, effects, facts.id, ty.width, &routine.name);
                        uses_before_definition |= reads && !defines;
                        if writes {
                            defines = true;
                            definition_blocks.insert(block.id);
                        }
                    }
                    NirOp::ForeignCode { effects, .. } => {
                        let reads = effects.opaque
                            || memory_accesses_storage(&effects.memory.reads, facts.id, ty.width);
                        let writes = effects.opaque
                            || memory_accesses_storage(&effects.memory.writes, facts.id, ty.width);
                        uses_before_definition |= reads && !defines;
                        if writes {
                            defines = true;
                            definition_blocks.insert(block.id);
                        }
                    }
                    NirOp::Unsupported { .. } => {
                        uses_before_definition |= !defines;
                        defines = true;
                        definition_blocks.insert(block.id);
                    }
                    _ => {}
                }
            }
            if facts.value_needed_at_exit && is_observable_exit(&block.terminator) && !defines {
                uses_before_definition = true;
            }
            blocks.insert(
                block.id,
                HomeBlockAccess {
                    uses_before_definition,
                    defines,
                },
            );
        }
        let result = solve_dataflow(cfg, &StorageLivenessProblem { blocks });
        let live_in = cfg
            .reachable()
            .iter()
            .copied()
            .map(|block| (block, result.in_state(block).copied().unwrap_or(false)))
            .collect();
        Self {
            definition_blocks,
            live_in,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HomeBlockAccess {
    uses_before_definition: bool,
    defines: bool,
}

struct StorageLivenessProblem {
    blocks: BTreeMap<BlockId, HomeBlockAccess>,
}

impl NirDataflowProblem for StorageLivenessProblem {
    type State = bool;

    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Backward
    }

    fn bottom(&self) -> Self::State {
        false
    }

    fn boundary(&self, _block: BlockId) -> Option<Self::State> {
        None
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        *into |= *other;
    }

    fn transfer(&self, block: BlockId, live_out: &Self::State) -> Self::State {
        let Some(access) = self.blocks.get(&block) else {
            return *live_out;
        };
        access.uses_before_definition || *live_out && !access.defines
    }
}

fn call_access(
    callee: &NirCallee,
    effects: &NirCallEffects,
    storage: NirStorageId,
    width: Option<ByteSize>,
    routine_name: &str,
) -> (bool, bool) {
    if effects.opaque
        || effects.may_call_external
        || matches!(callee, NirCallee::Indirect { .. })
        || matches!(callee, NirCallee::User { name, .. } if name.eq_ignore_ascii_case(routine_name))
    {
        return (true, true);
    }
    (
        memory_accesses_storage(&effects.memory.reads, storage, width),
        memory_accesses_storage(&effects.memory.writes, storage, width),
    )
}

fn memory_accesses_storage(
    access: &NirMemoryAccess,
    storage: NirStorageId,
    width: Option<ByteSize>,
) -> bool {
    match access {
        NirMemoryAccess::None => false,
        NirMemoryAccess::Regions(regions) => {
            let Some(width) = width else {
                return true;
            };
            let storage = NirMemoryRegion {
                kind: NirMemoryRegionKind::Storage(storage),
                offset: ByteOffset::ZERO,
                size: width,
            };
            regions.iter().any(|region| region.overlaps(&storage))
        }
        NirMemoryAccess::Unknown | NirMemoryAccess::All => true,
    }
}

fn is_observable_exit(terminator: &NirTerminator) -> bool {
    matches!(
        terminator,
        NirTerminator::Return(_) | NirTerminator::Exit | NirTerminator::Fallthrough
    )
}

fn fresh_temp(next_temp: &mut u32) -> TempId {
    let id = TempId(*next_temp);
    *next_temp = next_temp.saturating_add(1);
    id
}

fn op_result(op: &NirOp) -> Option<TempId> {
    match op {
        NirOp::Load { dest, .. }
        | NirOp::VolatileLoad { dest, .. }
        | NirOp::AddrOf { dest, .. }
        | NirOp::Unary { dest, .. }
        | NirOp::Cast { dest, .. }
        | NirOp::PointerOffset { dest, .. }
        | NirOp::Binary { dest, .. }
        | NirOp::Compare { dest, .. } => Some(*dest),
        NirOp::Call {
            result: Some(result),
            ..
        } => Some(result.dest),
        NirOp::Real(NirRealOp::Compare { result, .. })
        | NirOp::Real(NirRealOp::RealToInteger { result, .. }) => Some(*result),
        _ => None,
    }
}

fn rewrite_op_values(op: &mut NirOp, replacements: &BTreeMap<TempId, NirValue>) {
    match op {
        NirOp::Store { place, src, .. } | NirOp::VolatileStore { place, src, .. } => {
            rewrite_place_values(place, replacements);
            rewrite_value(src, replacements);
        }
        NirOp::Load { place, .. }
        | NirOp::VolatileLoad { place, .. }
        | NirOp::AddrOf { place, .. } => {
            rewrite_place_values(place, replacements);
        }
        NirOp::Unary { src, .. } | NirOp::Cast { src, .. } => rewrite_value(src, replacements),
        NirOp::Binary { left, right, .. } | NirOp::Compare { left, right, .. } => {
            rewrite_value(left, replacements);
            rewrite_value(right, replacements);
        }
        NirOp::PointerOffset { base, offset, .. } => {
            rewrite_value(base, replacements);
            rewrite_value(offset, replacements);
        }
        NirOp::Real(real) => rewrite_real_op_values(real, replacements),
        NirOp::Call { callee, args, .. } => {
            if let NirCallee::Indirect { target, .. } = callee {
                rewrite_value(target, replacements);
            }
            for arg in args {
                rewrite_value(arg, replacements);
            }
        }
        _ => {}
    }
}

fn rewrite_real_op_values(op: &mut NirRealOp, replacements: &BTreeMap<TempId, NirValue>) {
    match op {
        NirRealOp::Copy {
            destination,
            source,
        } => {
            rewrite_place_values(destination, replacements);
            if let NirRealSource::Place(source) = source {
                rewrite_place_values(source, replacements);
            }
        }
        NirRealOp::Unary {
            destination,
            operand,
            ..
        } => {
            rewrite_place_values(destination, replacements);
            rewrite_real_source_values(operand, replacements);
        }
        NirRealOp::Binary {
            destination,
            left,
            right,
            ..
        } => {
            rewrite_place_values(destination, replacements);
            rewrite_real_source_values(left, replacements);
            rewrite_real_source_values(right, replacements);
        }
        NirRealOp::Compare { left, right, .. } => {
            rewrite_real_source_values(left, replacements);
            rewrite_real_source_values(right, replacements);
        }
        NirRealOp::IntegerToReal {
            destination,
            source,
            ..
        } => {
            rewrite_place_values(destination, replacements);
            rewrite_value(source, replacements);
        }
        NirRealOp::RealToInteger { source, .. } => {
            rewrite_place_values(source, replacements);
        }
    }
}

fn rewrite_place_values(place: &mut NirPlace, replacements: &BTreeMap<TempId, NirValue>) {
    match &mut place.kind {
        NirPlaceKind::Deref { addr } => rewrite_value(addr, replacements),
        NirPlaceKind::Index {
            base_addr, index, ..
        } => {
            rewrite_value(base_addr, replacements);
            rewrite_value(index, replacements);
        }
        NirPlaceKind::Field { base, .. } => rewrite_place_values(base, replacements),
        _ => {}
    }
}

fn rewrite_real_source_values(
    source: &mut NirRealSource,
    replacements: &BTreeMap<TempId, NirValue>,
) {
    if let NirRealSource::Place(place) = source {
        rewrite_place_values(place, replacements);
    }
}

fn rewrite_value(value: &mut NirValue, replacements: &BTreeMap<TempId, NirValue>) {
    let mut visited = BTreeSet::new();
    while let NirValue::Temp { id, .. } = value {
        if !visited.insert(*id) {
            break;
        }
        let Some(replacement) = replacements.get(id) else {
            break;
        };
        if value_width(replacement) != value_width(value) {
            break;
        }
        *value = replacement.clone();
    }
}

fn rewrite_terminator_values(
    terminator: &mut NirTerminator,
    replacements: &BTreeMap<TempId, NirValue>,
) {
    match terminator {
        NirTerminator::Goto(edge) => {
            for arg in &mut edge.args {
                rewrite_value(arg, replacements);
            }
        }
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            rewrite_value(condition, replacements);
            for arg in then_edge.args.iter_mut().chain(&mut else_edge.args) {
                rewrite_value(arg, replacements);
            }
        }
        NirTerminator::Return(Some(value)) => rewrite_value(value, replacements),
        _ => {}
    }
}

fn collect_temps(blocks: &[NirBlock]) -> Vec<NirTemp> {
    let mut temps = Vec::new();
    for block in blocks {
        temps.extend(block.params.iter().map(|param| NirTemp {
            id: param.dest,
            ty: param.ty.clone(),
            def: NirTempDef {
                block: block.id,
                op_index: None,
            },
        }));
        for (op_index, op) in block.ops.iter().enumerate() {
            let Some(dest) = op_result(op) else {
                continue;
            };
            let ty = match op {
                NirOp::Load { ty, .. }
                | NirOp::VolatileLoad { ty, .. }
                | NirOp::AddrOf { ty, .. }
                | NirOp::Unary { ty, .. }
                | NirOp::PointerOffset { ty, .. }
                | NirOp::Binary { ty, .. }
                | NirOp::Compare { ty, .. } => ty,
                NirOp::Cast { to, .. } => to,
                NirOp::Call {
                    result: Some(result),
                    ..
                } => &result.ty,
                NirOp::Real(NirRealOp::Compare { result_type, .. })
                | NirOp::Real(NirRealOp::RealToInteger { result_type, .. }) => result_type,
                _ => continue,
            };
            temps.push(NirTemp {
                id: dest,
                ty: ty.clone(),
                def: NirTempDef {
                    block: block.id,
                    op_index: Some(op_index),
                },
            });
        }
    }
    temps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{LocalId, NirLocalBacking, NirStorageClass, NirTypeKind, direct_storage_id};

    fn byte_type() -> NirType {
        NirType {
            kind: NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(crate::target::ByteSize::ONE),
            pointer: false,
        }
    }

    fn card_type() -> NirType {
        NirType {
            kind: NirTypeKind::U16,
            summary: "Card".to_string(),
            width: Some(crate::target::ByteSize::new(2)),
            pointer: false,
        }
    }

    fn local_place() -> NirPlace {
        NirPlace {
            kind: NirPlaceKind::Local {
                id: LocalId(0),
                name: "value".to_string(),
            },
            ty: Some(byte_type()),
        }
    }

    fn store(value: u8) -> NirOp {
        NirOp::Store {
            place: local_place(),
            src: NirValue::ConstU8(value),
            ty: byte_type(),
        }
    }

    fn load(dest: u32) -> NirOp {
        NirOp::Load {
            dest: TempId(dest),
            ty: byte_type(),
            place: local_place(),
        }
    }

    fn edge(target: u32) -> NirEdge {
        NirEdge {
            target: BlockId(target),
            args: Vec::new(),
        }
    }

    fn block(id: u32, ops: Vec<NirOp>, terminator: NirTerminator) -> NirBlock {
        NirBlock {
            id: BlockId(id),
            label: format!("bb{id}"),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn program(blocks: Vec<NirBlock>) -> NirProgram {
        let mut routine = NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![NirLocal {
                id: LocalId(0),
                name: "value".to_string(),
                kind: "Byte".to_string(),
                purpose: NirLocalPurpose::Storage,
                storage: NirStorageClass::Scalar,
                ty: byte_type(),
                backing: NirLocalBacking::Ordinary,
                init: None,
            }],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks,
        };
        routine.temps = collect_temps(&routine.blocks);
        NirProgram {
            target_layout: crate::target::TargetLayout::atari_6502(),
            runtime_bindings: Vec::new(),
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![routine],
        }
    }

    #[test]
    fn promotes_a_hot_loop_home_with_one_pruned_block_parameter() {
        let program = program(vec![
            block(0, vec![store(0)], NirTerminator::Goto(edge(1))),
            block(
                1,
                (0..MIN_HOT_HOME_LOADS as u32).map(load).collect(),
                NirTerminator::Branch {
                    condition: NirValue::ConstU8(1),
                    then_edge: edge(2),
                    else_edge: edge(3),
                },
            ),
            block(2, vec![store(1)], NirTerminator::Goto(edge(1))),
            block(3, Vec::new(), NirTerminator::Return(None)),
        ]);

        let promoted = promote_program(&program).expect("promote loop home");
        let routine = &promoted.routines[0];
        assert_eq!(routine.blocks[1].params.len(), 1);
        assert!(matches!(
            &routine.blocks[0].terminator,
            NirTerminator::Goto(NirEdge { args, .. }) if args.len() == 1
        ));
        assert!(matches!(
            &routine.blocks[2].terminator,
            NirTerminator::Goto(NirEdge { args, .. }) if args.len() == 1
        ));
        assert!(routine.blocks.iter().flat_map(|block| &block.ops).all(
            |op| !matches!(op, NirOp::Load { place, .. } | NirOp::Store { place, .. }
                if direct_storage_id(place) == Some(NirStorageId::Local(LocalId(0))))
        ));
    }

    #[test]
    fn pressure_guard_leaves_a_cold_home_in_storage_form() {
        let program = program(vec![block(
            0,
            vec![store(3), load(0)],
            NirTerminator::Return(None),
        )]);

        let promoted = promote_program(&program).expect("retain cold home");
        assert_eq!(promoted, program);
    }

    #[test]
    fn promotes_a_word_induction_home_used_by_an_indexed_address() {
        let word = card_type();
        let word_place = NirPlace {
            kind: NirPlaceKind::Local {
                id: LocalId(0),
                name: "index".to_string(),
            },
            ty: Some(word.clone()),
        };
        let load_word = |dest| NirOp::Load {
            dest: TempId(dest),
            ty: word.clone(),
            place: word_place.clone(),
        };
        let mut routine = NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: vec![NirLocal {
                id: LocalId(0),
                name: "index".to_string(),
                kind: "Card".to_string(),
                purpose: NirLocalPurpose::Storage,
                storage: NirStorageClass::Scalar,
                ty: word.clone(),
                backing: NirLocalBacking::Ordinary,
                init: None,
            }],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
                block(
                    0,
                    vec![NirOp::Store {
                        place: word_place.clone(),
                        src: NirValue::ConstU8(0),
                        ty: word.clone(),
                    }],
                    NirTerminator::Goto(edge(1)),
                ),
                block(
                    1,
                    vec![
                        load_word(0),
                        NirOp::Load {
                            dest: TempId(1),
                            ty: byte_type(),
                            place: NirPlace {
                                kind: NirPlaceKind::Index {
                                    base_addr: NirValue::ConstU16(0x4000),
                                    index: NirValue::Temp {
                                        id: TempId(0),
                                        ty: word.clone(),
                                    },
                                    elem_ty: byte_type(),
                                    elem_size: ByteSize::ONE,
                                },
                                ty: Some(byte_type()),
                            },
                        },
                        load_word(2),
                    ],
                    NirTerminator::Branch {
                        condition: NirValue::ConstU8(1),
                        then_edge: edge(2),
                        else_edge: edge(3),
                    },
                ),
                block(
                    2,
                    vec![
                        load_word(3),
                        NirOp::Binary {
                            dest: TempId(4),
                            ty: word.clone(),
                            op: NirBinaryOp::Add,
                            left: NirValue::Temp {
                                id: TempId(3),
                                ty: word.clone(),
                            },
                            right: NirValue::ConstU8(1),
                        },
                        NirOp::Store {
                            place: word_place.clone(),
                            src: NirValue::Temp {
                                id: TempId(4),
                                ty: word.clone(),
                            },
                            ty: word.clone(),
                        },
                    ],
                    NirTerminator::Goto(edge(1)),
                ),
                block(3, Vec::new(), NirTerminator::Return(None)),
            ],
        };
        routine.temps = collect_temps(&routine.blocks);
        let program = NirProgram {
            target_layout: crate::target::TargetLayout::atari_6502(),
            runtime_bindings: Vec::new(),
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![routine],
        };

        let promoted = promote_program(&program).expect("promote indexed induction home");
        let routine = &promoted.routines[0];
        assert_eq!(routine.blocks[1].params[0].ty, word);
        assert!(matches!(
            &routine.blocks[0].terminator,
            NirTerminator::Goto(NirEdge { args, .. })
                if args == &[NirValue::ConstU16(0)]
        ));
        assert!(routine.blocks.iter().flat_map(|block| &block.ops).all(
            |op| !matches!(op, NirOp::Load { place, .. } | NirOp::Store { place, .. }
                if direct_storage_id(place) == Some(NirStorageId::Local(LocalId(0))))
        ));
    }
}
