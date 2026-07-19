use std::collections::{BTreeMap, BTreeSet};

use super::analysis::{
    cfg::NirCfg,
    dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow},
    dominance::NirDominance,
    storage::NirRoutineStorageAnalysis,
    use_def::NirUseDef,
};
use super::facts::{BlockId, NirStorageId, NirType, NirValue, TempId, value_width};
use super::ir::*;
use super::{NirDiagnostic, analyze_program_storage, direct_storage_id, verify_program};

pub(super) fn propagate_program(program: &NirProgram) -> Result<NirProgram, Vec<NirDiagnostic>> {
    verify_program(program)?;
    let analyses = analyze_program_storage(program);
    let mut optimized = program.clone();
    for (routine, analysis) in optimized.routines.iter_mut().zip(&analyses.routines) {
        propagate_routine(routine, analysis);
    }
    verify_program(&optimized)?;
    Ok(optimized)
}

fn propagate_routine(routine: &mut NirRoutine, analysis: &NirRoutineStorageAnalysis) {
    let trackable = analysis
        .homes
        .values()
        .filter(|facts| facts.is_value_trackable())
        .filter_map(|facts| {
            facts
                .direct_access_ty
                .as_ref()
                .and_then(|ty| ty.width)
                .or(facts.width)
                .map(|width| (facts.id, width))
        })
        .collect::<BTreeMap<_, _>>();
    if trackable.is_empty() {
        return;
    }

    let cfg = NirCfg::from_routine(routine);
    let dominance = NirDominance::from_cfg(&cfg);
    let use_def = NirUseDef::from_routine(routine);
    let routine_name = routine.name.clone();
    let result = solve_dataflow(
        &cfg,
        &StorageValueProblem::new(
            routine,
            &cfg,
            &dominance,
            &use_def,
            &trackable,
            &routine_name,
        ),
    );

    for block in &mut routine.blocks {
        let mut facts = result
            .in_state(block.id)
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_default();
        let mut rewritten = Vec::with_capacity(block.ops.len());
        for (op_index, op) in block.ops.drain(..).enumerate() {
            if let Some(op) = transfer_op(
                op,
                block.id,
                op_index,
                &trackable,
                &use_def,
                &dominance,
                &routine_name,
                &mut facts,
            ) {
                rewritten.push(op);
            }
        }
        block.ops = rewritten;
        rewrite_terminator(&mut block.terminator, &facts.replacements);
    }
    routine.temps = collect_temps(&routine.blocks);
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct StorageValueFacts {
    /// Temp aliases introduced by eliminated loads.
    replacements: BTreeMap<TempId, NirValue>,
    /// Last known value of an exact direct storage home.
    storage: BTreeMap<NirStorageId, NirValue>,
}

impl StorageValueFacts {
    fn intersect_with(&mut self, other: &Self) {
        self.replacements
            .retain(|temp, value| other.replacements.get(temp) == Some(value));
        self.storage
            .retain(|storage, value| other.storage.get(storage) == Some(value));
    }
}

struct StorageValueProblem<'a> {
    entry: Option<BlockId>,
    blocks: BTreeMap<BlockId, &'a NirBlock>,
    dominance: &'a NirDominance,
    use_def: &'a NirUseDef,
    trackable: &'a BTreeMap<NirStorageId, u16>,
    routine_name: &'a str,
}

impl<'a> StorageValueProblem<'a> {
    fn new(
        routine: &'a NirRoutine,
        cfg: &NirCfg,
        dominance: &'a NirDominance,
        use_def: &'a NirUseDef,
        trackable: &'a BTreeMap<NirStorageId, u16>,
        routine_name: &'a str,
    ) -> Self {
        Self {
            entry: cfg.entry(),
            blocks: routine
                .blocks
                .iter()
                .map(|block| (block.id, block))
                .collect(),
            dominance,
            use_def,
            trackable,
            routine_name,
        }
    }
}

impl NirDataflowProblem for StorageValueProblem<'_> {
    type State = Option<StorageValueFacts>;

    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Forward
    }

    fn bottom(&self) -> Self::State {
        None
    }

    fn boundary(&self, block: BlockId) -> Option<Self::State> {
        (Some(block) == self.entry).then(|| Some(StorageValueFacts::default()))
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        let Some(other) = other else {
            return;
        };
        if let Some(into) = into {
            into.intersect_with(other);
        } else {
            *into = Some(other.clone());
        }
    }

    fn transfer(&self, block: BlockId, input: &Self::State) -> Self::State {
        let mut facts = input.clone()?;
        for (op_index, op) in self.blocks.get(&block)?.ops.iter().cloned().enumerate() {
            transfer_op(
                op,
                block,
                op_index,
                self.trackable,
                self.use_def,
                self.dominance,
                self.routine_name,
                &mut facts,
            );
        }
        Some(facts)
    }
}

fn transfer_op(
    mut op: NirOp,
    block: BlockId,
    op_index: usize,
    trackable: &BTreeMap<NirStorageId, u16>,
    use_def: &NirUseDef,
    dominance: &NirDominance,
    routine_name: &str,
    facts: &mut StorageValueFacts,
) -> Option<NirOp> {
    rewrite_op_values(&mut op, &facts.replacements);
    match &op {
        NirOp::Load {
            dest, ty, place, ..
        } => {
            let direct = direct_storage_id(place).filter(|id| trackable.contains_key(id));
            if let Some(id) = direct
                && let Some(value) = facts.storage.get(&id)
            {
                let value = resolve_value(value, &facts.replacements);
                if value_width(&value) == ty.width
                    && value_available(&value, block, op_index, use_def, dominance)
                {
                    facts.replacements.insert(*dest, value);
                    return None;
                }
            }
            facts.replacements.remove(dest);
            retain_available_storage_values(facts, block, op_index, use_def);
            if let Some(id) = direct {
                facts.storage.insert(
                    id,
                    NirValue::Temp {
                        id: *dest,
                        ty: ty.clone(),
                    },
                );
            }
        }
        NirOp::Store { place, src, ty } => {
            if let Some(id) = direct_storage_id(place) {
                let value = resolve_value(src, &facts.replacements);
                let value = if trackable.contains_key(&id) {
                    value_for_storage(value, ty)
                } else {
                    None
                };
                if let Some(value) = value {
                    facts.storage.insert(id, value);
                } else {
                    facts.storage.remove(&id);
                }
            } else {
                // Until Phase 3 carries exact effect regions, an indirect,
                // indexed, field, or absolute write may overlap any tracked
                // home.
                facts.storage.clear();
            }
        }
        NirOp::Call {
            callee,
            result,
            effects,
            ..
        } => {
            if let Some(result) = result {
                facts.replacements.remove(&result.dest);
            }
            retain_available_storage_values(facts, block, op_index, use_def);
            apply_call_barrier(facts, callee, effects, trackable, routine_name);
        }
        NirOp::MachineBlock { .. }
        | NirOp::Unsupported { .. }
        | NirOp::Set { .. }
        | NirOp::Assign { .. }
        | NirOp::CompoundAssign { .. } => {
            facts.storage.clear();
        }
        NirOp::AddrOf { dest, .. }
        | NirOp::Unary { dest, .. }
        | NirOp::Binary { dest, .. }
        | NirOp::Compare { dest, .. } => {
            facts.replacements.remove(dest);
            retain_available_storage_values(facts, block, op_index, use_def);
        }
        NirOp::Cast { dest, .. } => {
            facts.replacements.remove(dest);
            retain_available_storage_values(facts, block, op_index, use_def);
        }
        NirOp::Define { .. } | NirOp::Declare { .. } | NirOp::Note { .. } => {}
    }
    Some(op)
}

fn value_for_storage(value: NirValue, ty: &NirType) -> Option<NirValue> {
    match (value, ty.width) {
        (NirValue::ConstU8(value), Some(1)) => Some(NirValue::ConstU8(value)),
        (NirValue::ConstU8(value), Some(2)) => Some(NirValue::ConstU16(u16::from(value))),
        (NirValue::ConstU16(value), Some(1)) => Some(NirValue::ConstU8(value as u8)),
        (NirValue::ConstU16(value), Some(2)) => Some(NirValue::ConstU16(value)),
        (value, width) if value_width(&value) == width => Some(value),
        _ => None,
    }
}

fn retain_available_storage_values(
    facts: &mut StorageValueFacts,
    block: BlockId,
    op_index: usize,
    use_def: &NirUseDef,
) {
    facts.storage.retain(|_, value| {
        let NirValue::Temp { id, .. } = value else {
            return true;
        };
        use_def.uses(*id).iter().any(|site| {
            site.block() == block && site.op_index().is_none_or(|index| index > op_index)
        })
    });
}

fn apply_call_barrier(
    facts: &mut StorageValueFacts,
    callee: &NirCallee,
    effects: &NirCallEffects,
    trackable: &BTreeMap<NirStorageId, u16>,
    routine_name: &str,
) {
    if effects.opaque || effects.may_call_os || matches!(callee, NirCallee::Indirect { .. }) {
        facts.storage.clear();
        return;
    }

    if matches!(callee, NirCallee::User(name) if name.eq_ignore_ascii_case(routine_name)) {
        facts.storage.clear();
        return;
    }

    match &effects.memory.writes {
        NirMemoryAccess::None => {
            // The current source pipeline has not yet annotated every direct
            // call with a trustworthy summary. Keep the Phase 2 transitional
            // rule for globals while allowing private caller storage to flow.
            facts
                .storage
                .retain(|id, _| !matches!(id, NirStorageId::Global(_)));
        }
        NirMemoryAccess::Regions(regions) => facts.storage.retain(|id, _| {
            let Some(size) = trackable.get(id) else {
                return false;
            };
            let storage = NirMemoryRegion {
                kind: NirMemoryRegionKind::Storage(*id),
                offset: 0,
                size: *size,
            };
            !regions.iter().any(|region| region.overlaps(&storage))
        }),
        NirMemoryAccess::Unknown | NirMemoryAccess::All => facts.storage.clear(),
    }
}

fn value_available(
    value: &NirValue,
    block: BlockId,
    op_index: usize,
    use_def: &NirUseDef,
    dominance: &NirDominance,
) -> bool {
    let NirValue::Temp { id, .. } = value else {
        return !matches!(value, NirValue::Param(_) | NirValue::GlobalAddr(_));
    };
    let Some(definition) = use_def.unique_definition(*id) else {
        return false;
    };
    if definition.block == block {
        definition.op_index < op_index
    } else {
        // Do not create a new cross-block temp live range merely to remove a
        // source-home load. Until typed block arguments exist, that exchange
        // can turn a cheap home reload into a spill. Constants remain freely
        // propagatable, and an already-live temp may be reused safely.
        dominance.dominates(definition.block, block)
            && use_def.uses(*id).iter().any(|site| site.block() == block)
    }
}

fn resolve_value(value: &NirValue, replacements: &BTreeMap<TempId, NirValue>) -> NirValue {
    let mut value = value.clone();
    let mut visited = BTreeSet::new();
    while let NirValue::Temp { id, .. } = &value {
        if !visited.insert(*id) {
            break;
        }
        let Some(replacement) = replacements.get(id) else {
            break;
        };
        if value_width(replacement) != value_width(&value) {
            break;
        }
        value = replacement.clone();
    }
    value
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
        NirOp::Unary { src, .. } | NirOp::Cast { src, .. } => {
            rewrite_value(src, replacements);
        }
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
        NirOp::Define { .. }
        | NirOp::Set { .. }
        | NirOp::Declare { .. }
        | NirOp::Assign { .. }
        | NirOp::CompoundAssign { .. }
        | NirOp::MachineBlock { .. }
        | NirOp::Unsupported { .. }
        | NirOp::Note { .. } => {}
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
        NirPlaceKind::Symbol(_)
        | NirPlaceKind::Param { .. }
        | NirPlaceKind::Local { .. }
        | NirPlaceKind::Global { .. }
        | NirPlaceKind::Absolute(_)
        | NirPlaceKind::UnresolvedName(_) => {}
    }
}

fn rewrite_value(value: &mut NirValue, replacements: &BTreeMap<TempId, NirValue>) {
    *value = resolve_value(value, replacements);
}

fn rewrite_terminator(terminator: &mut NirTerminator, replacements: &BTreeMap<TempId, NirValue>) {
    match terminator {
        NirTerminator::Branch { condition, .. } | NirTerminator::Return(Some(condition)) => {
            rewrite_value(condition, replacements);
        }
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Goto(_)
        | NirTerminator::Return(None)
        | NirTerminator::Exit
        | NirTerminator::Unknown(_) => {}
    }
}

fn op_definition(op: &NirOp) -> Option<(TempId, &NirType)> {
    match op {
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
        NirOp::Define { .. }
        | NirOp::Set { .. }
        | NirOp::Declare { .. }
        | NirOp::Assign { .. }
        | NirOp::CompoundAssign { .. }
        | NirOp::Store { .. }
        | NirOp::Call { result: None, .. }
        | NirOp::MachineBlock { .. }
        | NirOp::Unsupported { .. }
        | NirOp::Note { .. } => None,
    }
}

fn collect_temps(blocks: &[NirBlock]) -> Vec<NirTemp> {
    let mut temps = Vec::new();
    for block in blocks {
        for (op_index, op) in block.ops.iter().enumerate() {
            if let Some((id, ty)) = op_definition(op) {
                temps.push(NirTemp {
                    id,
                    ty: ty.clone(),
                    def: NirTempDef {
                        block: block.id,
                        op_index,
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
    use crate::nir::{LocalId, NirLocal, NirLocalBacking, NirStorageClass, NirTypeKind};

    fn byte_type() -> NirType {
        NirType {
            kind: NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(1),
            pointer: false,
        }
    }

    fn local(id: u32, name: &str) -> NirLocal {
        NirLocal {
            id: LocalId(id),
            name: name.to_string(),
            kind: "Byte".to_string(),
            storage: NirStorageClass::Scalar,
            ty: byte_type(),
            backing: NirLocalBacking::Ordinary,
            init: None,
        }
    }

    fn place(id: u32, name: &str) -> NirPlace {
        NirPlace {
            kind: NirPlaceKind::Local {
                id: LocalId(id),
                name: name.to_string(),
            },
            ty: Some(byte_type()),
        }
    }

    fn store(id: u32, name: &str, src: NirValue) -> NirOp {
        NirOp::Store {
            place: place(id, name),
            src,
            ty: byte_type(),
        }
    }

    fn load(dest: u32, id: u32, name: &str) -> NirOp {
        NirOp::Load {
            dest: TempId(dest),
            ty: byte_type(),
            place: place(id, name),
        }
    }

    fn block(id: u32, label: &str, ops: Vec<NirOp>, terminator: NirTerminator) -> NirBlock {
        NirBlock {
            id: BlockId(id),
            label: label.to_string(),
            ops,
            terminator,
        }
    }

    fn program(locals: Vec<NirLocal>, blocks: Vec<NirBlock>) -> NirProgram {
        let mut routine = NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals,
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

    fn optimized(program: NirProgram) -> NirRoutine {
        propagate_program(&program).unwrap().routines.remove(0)
    }

    fn loads(routine: &NirRoutine) -> usize {
        routine
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .filter(|op| matches!(op, NirOp::Load { .. }))
            .count()
    }

    #[test]
    fn forwards_store_load_and_repeated_load_in_one_block() {
        let routine = optimized(program(
            vec![local(0, "x"), local(1, "out")],
            vec![block(
                0,
                "entry",
                vec![
                    store(0, "x", NirValue::ConstU8(7)),
                    load(0, 0, "x"),
                    load(1, 0, "x"),
                    store(
                        1,
                        "out",
                        NirValue::Temp {
                            id: TempId(1),
                            ty: byte_type(),
                        },
                    ),
                ],
                NirTerminator::Return(None),
            )],
        ));

        assert_eq!(loads(&routine), 0);
        assert!(matches!(
            routine.blocks[0].ops.last(),
            Some(NirOp::Store {
                src: NirValue::ConstU8(7),
                ..
            })
        ));
    }

    #[test]
    fn first_load_can_seed_a_persistent_home_fact() {
        let routine = optimized(program(
            vec![local(0, "x"), local(1, "out")],
            vec![block(
                0,
                "entry",
                vec![
                    load(0, 0, "x"),
                    load(1, 0, "x"),
                    store(
                        1,
                        "out",
                        NirValue::Temp {
                            id: TempId(1),
                            ty: byte_type(),
                        },
                    ),
                ],
                NirTerminator::Return(None),
            )],
        ));

        assert_eq!(loads(&routine), 1);
        assert!(matches!(
            routine.blocks[0].ops.last(),
            Some(NirOp::Store {
                src: NirValue::Temp { id: TempId(0), .. },
                ..
            })
        ));
    }

    #[test]
    fn join_forwards_only_equal_incoming_storage_values() {
        let equal = optimized(diamond(4, 4));
        assert_eq!(loads(&equal), 0);

        let different = optimized(diamond(4, 5));
        assert_eq!(loads(&different), 1);
    }

    #[test]
    fn join_forwards_a_common_dominating_temp() {
        let routine = optimized(program(
            vec![local(0, "x"), local(1, "out"), local(2, "existing_use")],
            vec![
                block(
                    0,
                    "entry",
                    vec![NirOp::Binary {
                        dest: TempId(0),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: NirValue::ConstU8(1),
                        right: NirValue::ConstU8(2),
                    }],
                    NirTerminator::Branch {
                        condition: NirValue::ConstU8(1),
                        then_label: "left".to_string(),
                        else_label: "right".to_string(),
                    },
                ),
                block(
                    1,
                    "left",
                    vec![store(
                        0,
                        "x",
                        NirValue::Temp {
                            id: TempId(0),
                            ty: byte_type(),
                        },
                    )],
                    NirTerminator::Goto("join".to_string()),
                ),
                block(
                    2,
                    "right",
                    vec![store(
                        0,
                        "x",
                        NirValue::Temp {
                            id: TempId(0),
                            ty: byte_type(),
                        },
                    )],
                    NirTerminator::Goto("join".to_string()),
                ),
                block(
                    3,
                    "join",
                    vec![
                        store(
                            2,
                            "existing_use",
                            NirValue::Temp {
                                id: TempId(0),
                                ty: byte_type(),
                            },
                        ),
                        load(1, 0, "x"),
                        store(
                            1,
                            "out",
                            NirValue::Temp {
                                id: TempId(1),
                                ty: byte_type(),
                            },
                        ),
                    ],
                    NirTerminator::Return(None),
                ),
            ],
        ));

        assert_eq!(loads(&routine), 0);
        assert!(matches!(
            routine.blocks[3].ops.last(),
            Some(NirOp::Store {
                src: NirValue::Temp { id: TempId(0), .. },
                ..
            })
        ));
    }

    #[test]
    fn does_not_create_a_new_cross_block_temp_live_range() {
        let routine = optimized(program(
            vec![local(0, "x"), local(1, "out")],
            vec![
                block(
                    0,
                    "entry",
                    vec![
                        NirOp::Binary {
                            dest: TempId(0),
                            ty: byte_type(),
                            op: NirBinaryOp::Add,
                            left: NirValue::ConstU8(1),
                            right: NirValue::ConstU8(2),
                        },
                        store(
                            0,
                            "x",
                            NirValue::Temp {
                                id: TempId(0),
                                ty: byte_type(),
                            },
                        ),
                    ],
                    NirTerminator::Goto("next".to_string()),
                ),
                block(
                    1,
                    "next",
                    vec![
                        load(1, 0, "x"),
                        store(
                            1,
                            "out",
                            NirValue::Temp {
                                id: TempId(1),
                                ty: byte_type(),
                            },
                        ),
                    ],
                    NirTerminator::Return(None),
                ),
            ],
        ));

        assert_eq!(loads(&routine), 1);
    }

    fn diamond(left: u8, right: u8) -> NirProgram {
        program(
            vec![local(0, "x"), local(1, "out")],
            vec![
                block(
                    0,
                    "entry",
                    Vec::new(),
                    NirTerminator::Branch {
                        condition: NirValue::ConstU8(1),
                        then_label: "left".to_string(),
                        else_label: "right".to_string(),
                    },
                ),
                block(
                    1,
                    "left",
                    vec![store(0, "x", NirValue::ConstU8(left))],
                    NirTerminator::Goto("join".to_string()),
                ),
                block(
                    2,
                    "right",
                    vec![store(0, "x", NirValue::ConstU8(right))],
                    NirTerminator::Goto("join".to_string()),
                ),
                block(
                    3,
                    "join",
                    vec![
                        load(0, 0, "x"),
                        store(
                            1,
                            "out",
                            NirValue::Temp {
                                id: TempId(0),
                                ty: byte_type(),
                            },
                        ),
                    ],
                    NirTerminator::Return(None),
                ),
            ],
        )
    }

    #[test]
    fn loop_backedge_prevents_stale_entry_propagation() {
        let routine = optimized(program(
            vec![local(0, "x"), local(1, "out")],
            vec![
                block(
                    0,
                    "entry",
                    vec![store(0, "x", NirValue::ConstU8(0))],
                    NirTerminator::Goto("header".to_string()),
                ),
                block(
                    1,
                    "header",
                    vec![
                        load(0, 0, "x"),
                        store(
                            1,
                            "out",
                            NirValue::Temp {
                                id: TempId(0),
                                ty: byte_type(),
                            },
                        ),
                    ],
                    NirTerminator::Branch {
                        condition: NirValue::ConstU8(1),
                        then_label: "body".to_string(),
                        else_label: "exit".to_string(),
                    },
                ),
                block(
                    2,
                    "body",
                    vec![store(0, "x", NirValue::ConstU8(1))],
                    NirTerminator::Goto("header".to_string()),
                ),
                block(3, "exit", Vec::new(), NirTerminator::Return(None)),
            ],
        ));

        assert_eq!(loads(&routine), 1);
    }

    #[test]
    fn direct_pure_calls_preserve_private_facts_but_unknown_writes_kill_them() {
        let no_effects = NirCallEffects {
            memory: NirMemoryEffects {
                reads: NirMemoryAccess::None,
                writes: NirMemoryAccess::None,
            },
            may_call_os: false,
            opaque: false,
        };
        let unknown_writes = NirCallEffects {
            memory: NirMemoryEffects {
                reads: NirMemoryAccess::Unknown,
                writes: NirMemoryAccess::Unknown,
            },
            may_call_os: false,
            opaque: true,
        };
        let routine = optimized(program(
            vec![local(0, "x"), local(1, "out")],
            vec![block(
                0,
                "entry",
                vec![
                    store(0, "x", NirValue::ConstU8(3)),
                    NirOp::Call {
                        callee: NirCallee::Builtin("Pure".to_string()),
                        args: Vec::new(),
                        result: None,
                        signature: None,
                        effects: no_effects,
                    },
                    load(0, 0, "x"),
                    store(0, "x", NirValue::ConstU8(4)),
                    NirOp::Call {
                        callee: NirCallee::Builtin("Unknown".to_string()),
                        args: Vec::new(),
                        result: None,
                        signature: None,
                        effects: unknown_writes,
                    },
                    load(1, 0, "x"),
                    store(
                        1,
                        "out",
                        NirValue::Temp {
                            id: TempId(1),
                            ty: byte_type(),
                        },
                    ),
                ],
                NirTerminator::Return(None),
            )],
        ));

        assert_eq!(loads(&routine), 1);
    }

    #[test]
    fn structured_call_writes_kill_only_the_overlapping_storage_fact() {
        let routine = optimized(program(
            vec![local(0, "x"), local(1, "y"), local(2, "out")],
            vec![block(
                0,
                "entry",
                vec![
                    store(0, "x", NirValue::ConstU8(3)),
                    store(1, "y", NirValue::ConstU8(4)),
                    NirOp::Call {
                        callee: NirCallee::Builtin("WritesX".to_string()),
                        args: Vec::new(),
                        result: None,
                        signature: None,
                        effects: NirCallEffects {
                            memory: NirMemoryEffects {
                                reads: NirMemoryAccess::None,
                                writes: NirMemoryAccess::Regions(vec![NirMemoryRegion {
                                    kind: NirMemoryRegionKind::Storage(NirStorageId::Local(
                                        LocalId(0),
                                    )),
                                    offset: 0,
                                    size: 1,
                                }]),
                            },
                            may_call_os: false,
                            opaque: false,
                        },
                    },
                    load(0, 0, "x"),
                    load(1, 1, "y"),
                    store(
                        2,
                        "out",
                        NirValue::Temp {
                            id: TempId(1),
                            ty: byte_type(),
                        },
                    ),
                ],
                NirTerminator::Return(None),
            )],
        ));

        assert_eq!(loads(&routine), 1);
        assert!(matches!(
            routine.blocks[0].ops.last(),
            Some(NirOp::Store {
                src: NirValue::ConstU8(4),
                ..
            })
        ));
    }

    #[test]
    fn indirect_absolute_writes_and_machine_blocks_kill_storage_facts() {
        let routine = optimized(program(
            vec![local(0, "x"), local(1, "out")],
            vec![block(
                0,
                "entry",
                vec![
                    store(0, "x", NirValue::ConstU8(3)),
                    NirOp::Store {
                        place: NirPlace {
                            kind: NirPlaceKind::Absolute(0xD000),
                            ty: Some(byte_type()),
                        },
                        src: NirValue::ConstU8(0),
                        ty: byte_type(),
                    },
                    load(0, 0, "x"),
                    NirOp::MachineBlock {
                        items: vec![NirMachineItem::Byte(0x60)],
                        effects: NirMachineEffects {
                            memory: NirMemoryEffects {
                                reads: NirMemoryAccess::Unknown,
                                writes: NirMemoryAccess::Unknown,
                            },
                            may_call_os: false,
                            opaque: true,
                        },
                    },
                    load(1, 0, "x"),
                    store(0, "x", NirValue::ConstU8(4)),
                    NirOp::Store {
                        place: NirPlace {
                            kind: NirPlaceKind::Deref {
                                addr: NirValue::ConstU16(0x2000),
                            },
                            ty: Some(byte_type()),
                        },
                        src: NirValue::ConstU8(0),
                        ty: byte_type(),
                    },
                    load(2, 0, "x"),
                    store(
                        1,
                        "out",
                        NirValue::Temp {
                            id: TempId(2),
                            ty: byte_type(),
                        },
                    ),
                ],
                NirTerminator::Return(None),
            )],
        ));

        assert_eq!(loads(&routine), 3);
    }
}
