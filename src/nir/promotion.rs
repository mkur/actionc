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

// Promotion exposes long-lived values to the target allocator. Until MIR6502
// has routine-wide home coloring, keep automatic promotion to hot byte homes
// with a small definition set; colder and wider homes can otherwise replace
// direct storage traffic one-for-one with spills.
const MIN_HOT_HOME_LOADS: usize = 7;
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
        .filter(|facts| {
            facts
                .direct_access_ty
                .as_ref()
                .is_some_and(|ty| ty.width == Some(1))
        })
        .filter(|facts| facts.direct_loads >= MIN_HOT_HOME_LOADS)
        .filter(|facts| facts.store_blocks.len() <= MAX_HOT_HOME_STORE_BLOCKS)
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
                if ty.width != context.ty.width || value_width(src) != context.ty.width {
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
            NirOp::MachineBlock { effects, .. } => {
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
            NirOp::Unsupported { .. }
            | NirOp::Set { .. }
            | NirOp::Assign { .. }
            | NirOp::CompoundAssign { .. } => {
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
        | NirTerminator::Exit
        | NirTerminator::Unknown(_) => true,
    }
}

fn coerce_to_home_type(
    value: NirValue,
    rewritten: &mut Vec<NirOp>,
    context: &mut RenameContext<'_>,
) -> Option<NirValue> {
    let actual = match &value {
        NirValue::ConstU8(_) | NirValue::ConstU16(_) => return Some(value),
        NirValue::StaticAddr { ty, .. } | NirValue::Temp { ty, .. } => ty.clone(),
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
    });
    Some(NirValue::Temp {
        id: dest,
        ty: context.ty.clone(),
    })
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
                    NirOp::MachineBlock { effects, .. } => {
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
                    NirOp::Unsupported { .. }
                    | NirOp::Set { .. }
                    | NirOp::Assign { .. }
                    | NirOp::CompoundAssign { .. } => {
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
    width: Option<u16>,
    routine_name: &str,
) -> (bool, bool) {
    if effects.opaque
        || effects.may_call_os
        || matches!(callee, NirCallee::Indirect { .. })
        || matches!(callee, NirCallee::User(name) if name.eq_ignore_ascii_case(routine_name))
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
    width: Option<u16>,
) -> bool {
    match access {
        NirMemoryAccess::None => false,
        NirMemoryAccess::Regions(regions) => {
            let Some(width) = width else {
                return true;
            };
            let storage = NirMemoryRegion {
                kind: NirMemoryRegionKind::Storage(storage),
                offset: 0,
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
        | NirOp::AddrOf { dest, .. }
        | NirOp::Unary { dest, .. }
        | NirOp::Cast { dest, .. }
        | NirOp::Binary { dest, .. }
        | NirOp::Compare { dest, .. } => Some(*dest),
        NirOp::Call {
            result: Some(result),
            ..
        } => Some(result.dest),
        _ => None,
    }
}

fn rewrite_op_values(op: &mut NirOp, replacements: &BTreeMap<TempId, NirValue>) {
    match op {
        NirOp::Store { place, src, .. } => {
            rewrite_place_values(place, replacements);
            rewrite_value(src, replacements);
        }
        NirOp::Load { place, .. } | NirOp::AddrOf { place, .. } => {
            rewrite_place_values(place, replacements);
        }
        NirOp::Unary { src, .. } | NirOp::Cast { src, .. } => rewrite_value(src, replacements),
        NirOp::Binary { left, right, .. } | NirOp::Compare { left, right, .. } => {
            rewrite_value(left, replacements);
            rewrite_value(right, replacements);
        }
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
                | NirOp::AddrOf { ty, .. }
                | NirOp::Unary { ty, .. }
                | NirOp::Binary { ty, .. }
                | NirOp::Compare { ty, .. } => ty,
                NirOp::Cast { to, .. } => to,
                NirOp::Call {
                    result: Some(result),
                    ..
                } => &result.ty,
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
            width: Some(1),
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
}
