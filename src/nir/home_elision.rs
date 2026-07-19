use std::collections::BTreeSet;

use super::analysis::{
    cfg::NirCfg,
    dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow},
    storage::{NirPromotionBlocker, NirRoutineStorageAnalysis, NirStorageBackingClass},
};
use super::facts::{BlockId, NirStorageId, direct_storage_id, root_storage_id};
use super::ir::*;
use super::{NirDiagnostic, analyze_program_storage, verify_program};

pub(super) fn elide_program(program: &NirProgram) -> Result<NirProgram, Vec<NirDiagnostic>> {
    verify_program(program)?;
    let analyses = analyze_program_storage(program);
    let mut elided = program.clone();
    for (routine, analysis) in elided.routines.iter_mut().zip(&analyses.routines) {
        eliminate_dead_stores(routine, analysis);
    }

    let analyses = analyze_program_storage(&elided);
    for (routine, analysis) in elided.routines.iter_mut().zip(&analyses.routines) {
        eliminate_unused_local_homes(routine, analysis);
    }
    verify_program(&elided)?;
    Ok(elided)
}

fn eliminate_dead_stores(routine: &mut NirRoutine, analysis: &NirRoutineStorageAnalysis) {
    let candidates = analysis
        .homes
        .values()
        .filter(|facts| {
            matches!(facts.id, NirStorageId::Local(_))
                && facts.is_promotable()
                && facts.backing == NirStorageBackingClass::Ordinary
                && facts.storage_class == Some(NirStorageClass::Scalar)
        })
        .map(|facts| facts.id)
        .collect::<BTreeSet<_>>();
    if candidates.is_empty() {
        return;
    }

    let cfg = NirCfg::from_routine(routine);
    let boundary = analysis
        .homes
        .values()
        .filter(|facts| candidates.contains(&facts.id) && facts.value_needed_at_exit)
        .map(|facts| facts.id)
        .collect::<BTreeSet<_>>();
    let result = solve_dataflow(
        &cfg,
        &StorageLivenessProblem {
            routine,
            candidates: &candidates,
            boundary,
        },
    );

    for block in &mut routine.blocks {
        let mut live = result.out_state(block.id).cloned().unwrap_or_default();
        let mut retained = Vec::with_capacity(block.ops.len());
        for op in block.ops.drain(..).rev() {
            let dead_store = matches!(
                &op,
                NirOp::Store { place, .. }
                    if direct_storage_id(place).is_some_and(|id| {
                        candidates.contains(&id) && !live.contains(&id)
                    })
            );
            transfer_op_backwards(&op, &mut live, &candidates, &routine.name);
            if !dead_store {
                retained.push(op);
            }
        }
        retained.reverse();
        block.ops = retained;
    }
    routine.temps = collect_temps(&routine.blocks);
}

fn eliminate_unused_local_homes(routine: &mut NirRoutine, analysis: &NirRoutineStorageAnalysis) {
    let removable = analysis
        .homes
        .values()
        .filter(|facts| matches!(facts.id, NirStorageId::Local(_)))
        .filter(|facts| facts.backing == NirStorageBackingClass::Ordinary)
        .filter(|facts| facts.storage_class == Some(NirStorageClass::Scalar))
        .filter(|facts| facts.direct_loads == 0 && facts.direct_stores == 0)
        .filter(|facts| !facts.address_taken && !facts.machine_visible)
        .filter(|facts| !facts.calls_may_read && !facts.calls_may_write)
        .filter(|facts| !facts.value_needed_at_exit)
        .filter(|facts| facts.blockers == BTreeSet::from([NirPromotionBlocker::NoDirectAccess]))
        .map(|facts| facts.id)
        .collect::<BTreeSet<_>>();

    routine
        .locals
        .retain(|local| !removable.contains(&NirStorageId::Local(local.id)));
}

struct StorageLivenessProblem<'a> {
    routine: &'a NirRoutine,
    candidates: &'a BTreeSet<NirStorageId>,
    boundary: BTreeSet<NirStorageId>,
}

impl NirDataflowProblem for StorageLivenessProblem<'_> {
    type State = BTreeSet<NirStorageId>;

    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Backward
    }

    fn bottom(&self) -> Self::State {
        BTreeSet::new()
    }

    fn boundary(&self, block: BlockId) -> Option<Self::State> {
        let terminator = self
            .routine
            .blocks
            .iter()
            .find(|candidate| candidate.id == block)
            .map(|candidate| &candidate.terminator)?;
        is_observable_exit(terminator).then(|| self.boundary.clone())
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.extend(other.iter().copied());
    }

    fn transfer(&self, block: BlockId, live_out: &Self::State) -> Self::State {
        let Some(block) = self
            .routine
            .blocks
            .iter()
            .find(|candidate| candidate.id == block)
        else {
            return live_out.clone();
        };
        let mut live = live_out.clone();
        for op in block.ops.iter().rev() {
            transfer_op_backwards(op, &mut live, self.candidates, &self.routine.name);
        }
        live
    }
}

fn transfer_op_backwards(
    op: &NirOp,
    live: &mut BTreeSet<NirStorageId>,
    candidates: &BTreeSet<NirStorageId>,
    routine_name: &str,
) {
    match op {
        NirOp::Load { place, .. } => {
            if let Some(id) = direct_storage_id(place)
                && candidates.contains(&id)
            {
                live.insert(id);
            }
            add_place_dependencies(place, live, candidates);
        }
        NirOp::Store { place, .. } => {
            if let Some(id) = direct_storage_id(place)
                && candidates.contains(&id)
            {
                live.remove(&id);
            } else {
                add_place_dependencies(place, live, candidates);
            }
        }
        NirOp::AddrOf { place, .. } => add_place_dependencies(place, live, candidates),
        NirOp::Call {
            callee, effects, ..
        } => {
            if effects.opaque
                || effects.may_call_os
                || matches!(callee, NirCallee::Indirect { .. })
                || matches!(callee, NirCallee::User(name) if name.eq_ignore_ascii_case(routine_name))
            {
                live.extend(candidates.iter().copied());
            } else {
                apply_effects_backwards(&effects.memory, live, candidates);
            }
        }
        NirOp::MachineBlock { effects, .. } => {
            if effects.opaque {
                live.extend(candidates.iter().copied());
            } else {
                apply_effects_backwards(&effects.memory, live, candidates);
            }
        }
        NirOp::Unsupported { .. }
        | NirOp::Set { .. }
        | NirOp::Assign { .. }
        | NirOp::CompoundAssign { .. } => live.extend(candidates.iter().copied()),
        NirOp::Define { .. }
        | NirOp::Declare { .. }
        | NirOp::Unary { .. }
        | NirOp::Cast { .. }
        | NirOp::Binary { .. }
        | NirOp::Compare { .. }
        | NirOp::Note { .. } => {}
    }
}

fn add_place_dependencies(
    place: &NirPlace,
    live: &mut BTreeSet<NirStorageId>,
    candidates: &BTreeSet<NirStorageId>,
) {
    if matches!(place.kind, NirPlaceKind::Field { .. })
        && let Some(id) = root_storage_id(place)
        && candidates.contains(&id)
    {
        live.insert(id);
    }
}

fn apply_effects_backwards(
    effects: &NirMemoryEffects,
    live: &mut BTreeSet<NirStorageId>,
    candidates: &BTreeSet<NirStorageId>,
) {
    kill_written(&effects.writes, live, candidates);
    add_read(&effects.reads, live, candidates);
}

fn kill_written(
    access: &NirMemoryAccess,
    live: &mut BTreeSet<NirStorageId>,
    candidates: &BTreeSet<NirStorageId>,
) {
    match access {
        NirMemoryAccess::None => {}
        NirMemoryAccess::Regions(regions) => {
            for region in regions {
                if let NirMemoryRegionKind::Storage(id) = region.kind
                    && region.offset == 0
                    && candidates.contains(&id)
                {
                    live.remove(&id);
                }
            }
        }
        NirMemoryAccess::Unknown | NirMemoryAccess::All => live.clear(),
    }
}

fn add_read(
    access: &NirMemoryAccess,
    live: &mut BTreeSet<NirStorageId>,
    candidates: &BTreeSet<NirStorageId>,
) {
    match access {
        NirMemoryAccess::None => {}
        NirMemoryAccess::Regions(regions) => {
            for region in regions {
                if let NirMemoryRegionKind::Storage(id) = region.kind
                    && candidates.contains(&id)
                {
                    live.insert(id);
                }
            }
        }
        NirMemoryAccess::Unknown | NirMemoryAccess::All => live.extend(candidates.iter().copied()),
    }
}

fn is_observable_exit(terminator: &NirTerminator) -> bool {
    matches!(
        terminator,
        NirTerminator::Return(_) | NirTerminator::Exit | NirTerminator::Fallthrough
    )
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
            let result = match op {
                NirOp::Load { dest, ty, .. }
                | NirOp::AddrOf { dest, ty, .. }
                | NirOp::Unary { dest, ty, .. }
                | NirOp::Binary { dest, ty, .. }
                | NirOp::Compare { dest, ty, .. } => Some((*dest, ty)),
                NirOp::Cast { dest, to, .. } => Some((*dest, to)),
                NirOp::Call {
                    result: Some(result),
                    ..
                } => Some((result.dest, &result.ty)),
                _ => None,
            };
            if let Some((id, ty)) = result {
                temps.push(NirTemp {
                    id,
                    ty: ty.clone(),
                    def: NirTempDef {
                        block: block.id,
                        op_index: Some(op_index),
                    },
                });
            }
        }
    }
    temps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{LocalId, NirLocalBacking, NirType, NirTypeKind, NirValue, TempId};

    fn byte_type() -> NirType {
        NirType {
            kind: NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(1),
            pointer: false,
        }
    }

    fn place() -> NirPlace {
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
            place: place(),
            src: NirValue::ConstU8(value),
            ty: byte_type(),
        }
    }

    fn load() -> NirOp {
        NirOp::Load {
            dest: TempId(0),
            ty: byte_type(),
            place: place(),
        }
    }

    fn program(ops: Vec<NirOp>) -> NirProgram {
        let temps = if ops.iter().any(|op| matches!(op, NirOp::Load { .. })) {
            vec![NirTemp {
                id: TempId(0),
                ty: byte_type(),
                def: NirTempDef {
                    block: BlockId(0),
                    op_index: ops.iter().position(|op| matches!(op, NirOp::Load { .. })),
                },
            }]
        } else {
            Vec::new()
        };
        NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
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
                temps,
                notes: Vec::new(),
                blocks: vec![NirBlock {
                    id: BlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops,
                    terminator: NirTerminator::Return(None),
                }],
            }],
        }
    }

    #[test]
    fn removes_a_store_overwritten_before_the_next_read() {
        let elided = elide_program(&program(vec![store(1), store(2), load()]))
            .expect("elide overwritten store");
        let ops = &elided.routines[0].blocks[0].ops;
        assert_eq!(ops.len(), 2);
        assert!(matches!(
            &ops[0],
            NirOp::Store {
                src: NirValue::ConstU8(2),
                ..
            }
        ));
    }

    #[test]
    fn removes_an_unobserved_store_and_its_backing_home() {
        let elided = elide_program(&program(vec![store(1)])).expect("elide unused home");
        assert!(elided.routines[0].blocks[0].ops.is_empty());
        assert!(elided.routines[0].locals.is_empty());
    }

    #[test]
    fn structured_call_read_keeps_the_reaching_store_and_home() {
        let effects = NirCallEffects {
            memory: NirMemoryEffects {
                reads: NirMemoryAccess::Regions(vec![NirMemoryRegion {
                    kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(0))),
                    offset: 0,
                    size: 1,
                }]),
                writes: NirMemoryAccess::None,
            },
            may_call_os: false,
            opaque: false,
        };
        let elided = elide_program(&program(vec![
            store(1),
            NirOp::Call {
                callee: NirCallee::User("Observe".to_string()),
                args: Vec::new(),
                result: None,
                signature: None,
                effects,
            },
        ]))
        .expect("preserve call-visible store");

        assert!(matches!(
            elided.routines[0].blocks[0].ops.first(),
            Some(NirOp::Store { .. })
        ));
        assert_eq!(elided.routines[0].locals.len(), 1);
    }
}
