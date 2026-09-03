use std::collections::{BTreeMap, BTreeSet};

use super::cfg::NirCfg;
use super::dataflow::{
    NirDataflowDirection, NirDataflowProblem, NirDataflowResult, solve_dataflow,
};
use super::storage::NirRoutineStorageAnalysis;
use crate::nir::facts::{NirStorageId, direct_storage_id};
use crate::nir::{
    BlockId, NirBlock, NirCompareOp, NirOp, NirRoutine, NirTerminator, NirValue, TempId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::nir) enum NirPredicateSubject {
    Storage(NirStorageId),
    Temp(TempId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::nir) struct NirPredicate {
    pub subject: NirPredicateSubject,
    pub value: u16,
    pub equal: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NirPredicateFacts {
    facts: BTreeMap<(NirPredicateSubject, u16), bool>,
}

impl NirPredicateFacts {
    fn intersect_with(&mut self, other: &Self) {
        self.facts
            .retain(|key, equal| other.facts.get(key) == Some(equal));
    }

    fn assume(&mut self, predicate: NirPredicate) -> bool {
        let key = (predicate.subject, predicate.value);
        if self
            .facts
            .get(&key)
            .is_some_and(|equal| *equal != predicate.equal)
        {
            return false;
        }
        self.facts.insert(key, predicate.equal);
        true
    }

    fn proves(&self, predicate: NirPredicate) -> Option<bool> {
        self.facts
            .get(&(predicate.subject, predicate.value))
            .map(|equal| *equal == predicate.equal)
    }

    fn kill_storage(&mut self, storage: Option<NirStorageId>) {
        self.facts.retain(|(subject, _), _| match subject {
            NirPredicateSubject::Temp(_) => true,
            NirPredicateSubject::Storage(candidate) => {
                storage.is_some_and(|storage| *candidate != storage)
            }
        });
    }
}

#[derive(Debug, Clone)]
struct NirBranchPredicate {
    primary: NirPredicate,
    assumptions: Vec<NirPredicate>,
}

struct NirPredicateProblem<'a> {
    entry: Option<BlockId>,
    blocks: BTreeMap<BlockId, &'a NirBlock>,
    branches: BTreeMap<BlockId, NirBranchPredicate>,
}

impl<'a> NirPredicateProblem<'a> {
    fn new(routine: &'a NirRoutine, cfg: &NirCfg, storage: &NirRoutineStorageAnalysis) -> Self {
        Self {
            entry: cfg.entry(),
            blocks: routine
                .blocks
                .iter()
                .map(|block| (block.id, block))
                .collect(),
            branches: routine
                .blocks
                .iter()
                .filter_map(|block| {
                    branch_predicate(block, storage).map(|predicate| (block.id, predicate))
                })
                .collect(),
        }
    }

    fn edge_kind(&self, from: BlockId, to: BlockId) -> Option<bool> {
        let block = self.blocks.get(&from)?;
        let NirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } = &block.terminator
        else {
            return None;
        };
        match (then_edge.target == to, else_edge.target == to) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            (false, false) | (true, true) => None,
        }
    }

    fn edge_state(
        &self,
        from: BlockId,
        to: BlockId,
        from_out: &Option<NirPredicateFacts>,
    ) -> Option<NirPredicateFacts> {
        let mut facts = from_out.clone()?;
        let Some(predicate) = self.branches.get(&from) else {
            return Some(facts);
        };
        let Some(taken) = self.edge_kind(from, to) else {
            return Some(facts);
        };
        for assumption in &predicate.assumptions {
            let mut assumption = *assumption;
            if !taken {
                assumption.equal = !assumption.equal;
            }
            if !facts.assume(assumption) {
                return None;
            }
        }
        Some(facts)
    }
}

impl NirDataflowProblem for NirPredicateProblem<'_> {
    type State = Option<NirPredicateFacts>;

    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Forward
    }

    fn bottom(&self) -> Self::State {
        None
    }

    fn boundary(&self, block: BlockId) -> Option<Self::State> {
        (Some(block) == self.entry).then(|| Some(NirPredicateFacts::default()))
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
        let block = self.blocks.get(&block)?;
        for op in &block.ops {
            match op {
                NirOp::Store { place, .. } => {
                    facts.kill_storage(direct_storage_id(place));
                }
                NirOp::VolatileStore { .. } | NirOp::VolatileLoad { .. } => {
                    facts.kill_storage(None);
                }
                NirOp::CopyBytes {
                    destination,
                    destination_volatile,
                    source_volatile,
                    ..
                } => {
                    facts.kill_storage(if *destination_volatile || *source_volatile {
                        None
                    } else {
                        direct_storage_id(destination)
                    });
                }
                NirOp::Call { .. }
                | NirOp::Real(_)
                | NirOp::MachineBlock { .. }
                | NirOp::InlineAsm { .. }
                | NirOp::Unsupported { .. } => facts.kill_storage(None),
                NirOp::RuntimeHelperOverride { .. }
                | NirOp::Load { .. }
                | NirOp::AddrOf { .. }
                | NirOp::Unary { .. }
                | NirOp::Cast { .. }
                | NirOp::Binary { .. }
                | NirOp::Compare { .. } => {}
            }
        }
        Some(facts)
    }

    fn forward_edge_is_executable(
        &self,
        from: BlockId,
        to: BlockId,
        from_out: &Self::State,
    ) -> bool {
        let Some(facts) = from_out else {
            return false;
        };
        let Some(predicate) = self.branches.get(&from) else {
            return true;
        };
        let Some(taken) = self.edge_kind(from, to) else {
            return true;
        };
        match facts.proves(predicate.primary) {
            Some(result) => result == taken,
            None => true,
        }
    }

    fn transfer_forward_edge(
        &self,
        from: BlockId,
        to: BlockId,
        from_out: &Self::State,
    ) -> Self::State {
        self.edge_state(from, to, from_out)
    }
}

pub(in crate::nir) struct NirPredicateAnalysis {
    edge_facts: BTreeMap<(BlockId, BlockId), NirPredicateFacts>,
    branches: BTreeMap<BlockId, NirPredicate>,
}

impl NirPredicateAnalysis {
    pub(in crate::nir) fn analyze(
        routine: &NirRoutine,
        storage: &NirRoutineStorageAnalysis,
    ) -> Self {
        let cfg = NirCfg::from_routine(routine);
        let problem = NirPredicateProblem::new(routine, &cfg, storage);
        let result: NirDataflowResult<Option<NirPredicateFacts>> = solve_dataflow(&cfg, &problem);
        let mut edge_facts = BTreeMap::new();
        for from in cfg.reachable() {
            let Some(from_out) = result.out_state(*from) else {
                continue;
            };
            for to in cfg.successors(*from) {
                if !problem.forward_edge_is_executable(*from, *to, from_out) {
                    continue;
                }
                if let Some(facts) = problem.edge_state(*from, *to, from_out) {
                    edge_facts.insert((*from, *to), facts);
                }
            }
        }
        Self {
            edge_facts,
            branches: problem
                .branches
                .into_iter()
                .map(|(block, predicate)| (block, predicate.primary))
                .collect(),
        }
    }

    pub(in crate::nir) fn branch_predicate(&self, block: BlockId) -> Option<NirPredicate> {
        self.branches.get(&block).copied()
    }

    pub(in crate::nir) fn edge_proves(
        &self,
        from: BlockId,
        to: BlockId,
        predicate: NirPredicate,
    ) -> Option<bool> {
        self.edge_facts.get(&(from, to))?.proves(predicate)
    }
}

fn branch_predicate(
    block: &NirBlock,
    storage: &NirRoutineStorageAnalysis,
) -> Option<NirBranchPredicate> {
    let NirTerminator::Branch {
        condition: NirValue::Temp { id: condition, .. },
        ..
    } = &block.terminator
    else {
        return None;
    };
    let (compare_index, op, left, right) =
        block
            .ops
            .iter()
            .enumerate()
            .find_map(|(index, op)| match op {
                NirOp::Compare {
                    dest,
                    op,
                    left,
                    right,
                    ..
                } if dest == condition => Some((index, *op, left, right)),
                _ => None,
            })?;
    let (operand, value) = match (left, right) {
        (operand @ NirValue::Temp { .. }, NirValue::ConstU8(value)) => (operand, u16::from(*value)),
        (operand @ NirValue::Temp { .. }, NirValue::ConstU16(value)) => (operand, *value),
        (NirValue::ConstU8(value), operand @ NirValue::Temp { .. }) => (operand, u16::from(*value)),
        (NirValue::ConstU16(value), operand @ NirValue::Temp { .. }) => (operand, *value),
        _ => return None,
    };
    let equal = match op {
        NirCompareOp::Eq => true,
        NirCompareOp::Ne => false,
        NirCompareOp::Lt | NirCompareOp::Le | NirCompareOp::Gt | NirCompareOp::Ge => return None,
    };
    let NirValue::Temp { id: operand, .. } = operand else {
        unreachable!()
    };
    let temp_predicate = NirPredicate {
        subject: NirPredicateSubject::Temp(*operand),
        value,
        equal,
    };
    let mut assumptions = vec![temp_predicate];
    let storage_predicate =
        storage_subject_for_temp(block, compare_index, *operand, storage).map(|subject| {
            NirPredicate {
                subject,
                value,
                equal,
            }
        });
    if let Some(predicate) = storage_predicate {
        assumptions.push(predicate);
    }
    Some(NirBranchPredicate {
        primary: storage_predicate.unwrap_or(temp_predicate),
        assumptions,
    })
}

fn storage_subject_for_temp(
    block: &NirBlock,
    compare_index: usize,
    temp: TempId,
    storage: &NirRoutineStorageAnalysis,
) -> Option<NirPredicateSubject> {
    let (load_index, storage_id) =
        block
            .ops
            .iter()
            .enumerate()
            .take(compare_index)
            .find_map(|(index, op)| match op {
                NirOp::Load { dest, place, .. } if *dest == temp => {
                    direct_storage_id(place).map(|storage| (index, storage))
                }
                _ => None,
            })?;
    if !storage
        .homes
        .get(&storage_id)
        .is_some_and(|facts| facts.is_promotable())
    {
        return None;
    }
    if block.ops[load_index + 1..compare_index]
        .iter()
        .any(|op| invalidates_storage(op, storage_id))
    {
        return None;
    }
    Some(NirPredicateSubject::Storage(storage_id))
}

fn invalidates_storage(op: &NirOp, storage: NirStorageId) -> bool {
    match op {
        NirOp::Store { place, .. } => direct_storage_id(place).is_none_or(|id| id == storage),
        NirOp::VolatileLoad { .. } | NirOp::VolatileStore { .. } => true,
        NirOp::CopyBytes { destination, .. } => {
            direct_storage_id(destination).is_none_or(|id| id == storage)
        }
        NirOp::Call { .. }
        | NirOp::Real(_)
        | NirOp::MachineBlock { .. }
        | NirOp::InlineAsm { .. }
        | NirOp::Unsupported { .. } => true,
        NirOp::RuntimeHelperOverride { .. }
        | NirOp::Load { .. }
        | NirOp::AddrOf { .. }
        | NirOp::Unary { .. }
        | NirOp::Cast { .. }
        | NirOp::Binary { .. }
        | NirOp::Compare { .. } => false,
    }
}

pub(in crate::nir) fn predicate_threading_candidates(
    routine: &NirRoutine,
    analysis: &NirPredicateAnalysis,
    storage: &NirRoutineStorageAnalysis,
) -> BTreeSet<BlockId> {
    routine
        .blocks
        .iter()
        .filter(|block| {
            block.params.is_empty()
                && analysis.branch_predicate(block.id).is_some()
                && matches!(
                    block.terminator,
                    NirTerminator::Branch {
                        ref then_edge,
                        ref else_edge,
                        ..
                    } if then_edge.args.is_empty() && else_edge.args.is_empty()
                )
                && block.ops.iter().all(|op| {
                    matches!(
                        op,
                        NirOp::Load { place, .. }
                            if direct_storage_id(place).is_some_and(|id| {
                                storage
                                    .homes
                                    .get(&id)
                                    .is_some_and(|facts| facts.is_promotable())
                            })
                    ) || matches!(op, NirOp::Compare { .. })
                })
        })
        .map(|block| block.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{
        NirBlockParam, NirCallEffects, NirCallee, NirEdge, NirMachineEffects, NirMachineItem,
        NirMemoryAccess, NirMemoryEffects, NirParam, NirPlace, NirPlaceKind, NirProgram,
        NirStorageClass, NirTemp, NirTempDef, NirType, NirTypeKind, ParamId,
    };

    fn byte_type() -> NirType {
        NirType {
            kind: NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(crate::target::ByteSize::ONE),
            pointer: false,
        }
    }

    fn condition_type() -> NirType {
        NirType {
            kind: NirTypeKind::Bool,
            summary: "condition".to_string(),
            width: Some(crate::target::ByteSize::ONE),
            pointer: false,
        }
    }

    fn edge(target: u32) -> NirEdge {
        NirEdge {
            target: BlockId(target),
            args: Vec::new(),
        }
    }

    fn param_place() -> NirPlace {
        NirPlace {
            kind: NirPlaceKind::Param {
                id: ParamId(0),
                name: "value".to_string(),
            },
            ty: Some(byte_type()),
        }
    }

    fn predicate_block(
        id: u32,
        load: u32,
        condition: u32,
        op: NirCompareOp,
        then_target: u32,
        else_target: u32,
    ) -> NirBlock {
        NirBlock {
            id: BlockId(id),
            label: format!("bb{id}"),
            params: Vec::new(),
            ops: vec![
                NirOp::Load {
                    dest: TempId(load),
                    ty: byte_type(),
                    place: param_place(),
                },
                NirOp::Compare {
                    dest: TempId(condition),
                    ty: condition_type(),
                    operand_ty: byte_type(),
                    op,
                    left: NirValue::Temp {
                        id: TempId(load),
                        ty: byte_type(),
                    },
                    right: NirValue::ConstU8(0),
                },
            ],
            terminator: NirTerminator::Branch {
                condition: NirValue::Temp {
                    id: TempId(condition),
                    ty: condition_type(),
                },
                then_edge: edge(then_target),
                else_edge: edge(else_target),
            },
        }
    }

    fn predicate_program(left_ops: Vec<NirOp>) -> NirProgram {
        NirProgram {
            target_layout: crate::target::TargetLayout::atari_6502(),
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![NirRoutine {
                name: "Test".to_string(),
                params: vec![NirParam {
                    id: ParamId(0),
                    name: "value".to_string(),
                    storage: NirStorageClass::Scalar,
                    ty: byte_type(),
                }],
                locals: Vec::new(),
                temps: vec![
                    NirTemp {
                        id: TempId(0),
                        ty: byte_type(),
                        def: NirTempDef {
                            block: BlockId(0),
                            op_index: Some(0),
                        },
                    },
                    NirTemp {
                        id: TempId(1),
                        ty: condition_type(),
                        def: NirTempDef {
                            block: BlockId(0),
                            op_index: Some(1),
                        },
                    },
                    NirTemp {
                        id: TempId(2),
                        ty: byte_type(),
                        def: NirTempDef {
                            block: BlockId(3),
                            op_index: Some(0),
                        },
                    },
                    NirTemp {
                        id: TempId(3),
                        ty: condition_type(),
                        def: NirTempDef {
                            block: BlockId(3),
                            op_index: Some(1),
                        },
                    },
                ],
                notes: Vec::new(),
                blocks: vec![
                    predicate_block(0, 0, 1, NirCompareOp::Eq, 1, 2),
                    NirBlock {
                        id: BlockId(1),
                        label: "left".to_string(),
                        params: Vec::new(),
                        ops: left_ops,
                        terminator: NirTerminator::Goto(edge(3)),
                    },
                    NirBlock {
                        id: BlockId(2),
                        label: "right".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: NirTerminator::Goto(edge(3)),
                    },
                    predicate_block(3, 2, 3, NirCompareOp::Eq, 4, 5),
                    NirBlock {
                        id: BlockId(4),
                        label: "equal".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: NirTerminator::Return(None),
                    },
                    NirBlock {
                        id: BlockId(5),
                        label: "not_equal".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: NirTerminator::Return(None),
                    },
                ],
            }],
        }
    }

    #[test]
    fn edge_facts_preserve_opposite_predicate_results_through_empty_blocks() {
        let program = predicate_program(Vec::new());
        let storage = super::super::storage::analyze_program_storage(&program);
        let routine = &program.routines[0];
        let analysis = NirPredicateAnalysis::analyze(routine, &storage.routines[0]);
        let predicate = analysis.branch_predicate(BlockId(3)).unwrap();

        assert_eq!(
            analysis.edge_proves(BlockId(1), BlockId(3), predicate),
            Some(true)
        );
        assert_eq!(
            analysis.edge_proves(BlockId(2), BlockId(3), predicate),
            Some(false)
        );
    }

    #[test]
    fn call_barrier_kills_storage_predicates() {
        let call = NirOp::Call {
            callee: NirCallee::Builtin("Touch".to_string()),
            args: Vec::new(),
            result: None,
            signature: None,
            effects: NirCallEffects {
                memory: NirMemoryEffects {
                    reads: NirMemoryAccess::Unknown,
                    writes: NirMemoryAccess::Unknown,
                },
                may_call_os: false,
                opaque: true,
            },
        };
        let program = predicate_program(vec![call]);
        let storage = super::super::storage::analyze_program_storage(&program);
        let routine = &program.routines[0];
        let analysis = NirPredicateAnalysis::analyze(routine, &storage.routines[0]);
        let predicate = analysis.branch_predicate(BlockId(3)).unwrap();

        assert_eq!(
            analysis.edge_proves(BlockId(1), BlockId(3), predicate),
            None
        );
        assert_eq!(
            analysis.edge_proves(BlockId(2), BlockId(3), predicate),
            Some(false)
        );
    }

    #[test]
    fn unknown_store_and_machine_barrier_kill_storage_predicates() {
        let barriers = [
            NirOp::Store {
                place: NirPlace {
                    kind: NirPlaceKind::Deref {
                        addr: NirValue::ConstU16(0x4000),
                    },
                    ty: Some(byte_type()),
                },
                src: NirValue::ConstU8(1),
                ty: byte_type(),
            },
            NirOp::MachineBlock {
                items: vec![NirMachineItem::Byte(0xea)],
                effects: NirMachineEffects {
                    memory: NirMemoryEffects {
                        reads: NirMemoryAccess::Unknown,
                        writes: NirMemoryAccess::Unknown,
                    },
                    may_call_os: false,
                    opaque: true,
                },
            },
        ];

        for barrier in barriers {
            let program = predicate_program(vec![barrier]);
            let storage = super::super::storage::analyze_program_storage(&program);
            let routine = &program.routines[0];
            let analysis = NirPredicateAnalysis::analyze(routine, &storage.routines[0]);
            let predicate = analysis.branch_predicate(BlockId(3)).unwrap();

            assert_eq!(
                analysis.edge_proves(BlockId(1), BlockId(3), predicate),
                None
            );
        }
    }

    #[test]
    fn direct_store_kills_only_its_storage_predicate() {
        let store = NirOp::Store {
            place: param_place(),
            src: NirValue::ConstU8(1),
            ty: byte_type(),
        };
        let program = predicate_program(vec![store]);
        let storage = super::super::storage::analyze_program_storage(&program);
        let routine = &program.routines[0];
        let analysis = NirPredicateAnalysis::analyze(routine, &storage.routines[0]);
        let predicate = analysis.branch_predicate(BlockId(3)).unwrap();

        assert_eq!(
            analysis.edge_proves(BlockId(1), BlockId(3), predicate),
            None
        );
    }

    #[test]
    fn threading_rejects_block_parameters_and_nonpromotable_loads() {
        let mut with_param = predicate_program(Vec::new());
        with_param.routines[0].blocks[3].params.push(NirBlockParam {
            dest: TempId(9),
            ty: byte_type(),
        });
        let storage = super::super::storage::analyze_program_storage(&with_param);
        let analysis = NirPredicateAnalysis::analyze(&with_param.routines[0], &storage.routines[0]);
        assert!(
            !predicate_threading_candidates(
                &with_param.routines[0],
                &analysis,
                &storage.routines[0]
            )
            .contains(&BlockId(3))
        );

        let address_taken = NirOp::AddrOf {
            dest: TempId(9),
            ty: NirType {
                kind: NirTypeKind::Ptr16 {
                    pointee: Some(Box::new(NirTypeKind::U8)),
                },
                summary: "Byte*".to_string(),
                width: Some(crate::target::ByteSize::new(2)),
                pointer: true,
            },
            place: param_place(),
        };
        let nonpromotable = predicate_program(vec![address_taken]);
        let storage = super::super::storage::analyze_program_storage(&nonpromotable);
        let analysis =
            NirPredicateAnalysis::analyze(&nonpromotable.routines[0], &storage.routines[0]);
        assert!(
            !predicate_threading_candidates(
                &nonpromotable.routines[0],
                &analysis,
                &storage.routines[0]
            )
            .contains(&BlockId(3))
        );
    }
}
