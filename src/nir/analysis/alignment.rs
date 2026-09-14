//! Guaranteed low-zero-bit facts from verified NIR. No target instructions.
use super::{
    cfg::NirCfg,
    dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow},
};
use crate::nir::*;
use std::collections::{BTreeMap, BTreeSet};

/// Borrows the program so facts cannot outlive a mutation of their input.
pub struct NirAlignmentAnalysis<'a> {
    program: &'a NirProgram,
    routines: BTreeMap<RoutineId, BTreeMap<BlockId, State>>,
}

/// Receipt for an analysis result. Consumers must preserve the SSA definition
/// and this routine/block/value association, or discard and recompute the proof.
/// Only the verified analysis can construct a receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NirAlignmentProof {
    routine: RoutineId,
    block: BlockId,
    value: NirValue,
}
impl NirAlignmentProof {
    pub(crate) fn source(&self) -> (RoutineId, BlockId, &NirValue) {
        (self.routine, self.block, &self.value)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    temps: BTreeSet<TempId>,
    homes: BTreeSet<NirStorageId>,
}

pub fn analyze_alignment(
    program: &NirProgram,
) -> Result<NirAlignmentAnalysis<'_>, Vec<NirDiagnostic>> {
    verify_program(program)?;
    let storage = analyze_program_storage(program);
    let mut routines = BTreeMap::new();
    for (routine, storage) in program.routines.iter().zip(&storage.routines) {
        let cfg = NirCfg::from_routine(routine);
        let private = storage
            .homes
            .values()
            .filter(|facts| {
                facts.is_proven_private_to_invocation()
                    && facts.storage_class == Some(NirStorageClass::Scalar)
                    && !facts.calls_may_write
                    && facts.ty.as_ref().is_some_and(|ty| {
                        matches!(ty.width.map(ByteSize::get), Some(1 | 2 | 4))
                            && (ty.kind.integer().is_some()
                                || matches!(ty.kind, NirTypeKind::Pointer { .. }))
                    })
                    && facts.blockers.iter().all(|blocker| {
                        matches!(
                            blocker,
                            NirPromotionBlocker::UnsupportedType
                                | NirPromotionBlocker::InitializedStorage
                                | NirPromotionBlocker::ReadBeforeDefinition
                                | NirPromotionBlocker::NoDirectAccess
                        )
                    })
            })
            .map(|facts| facts.id)
            .collect();
        let problem = AlignmentProblem {
            program,
            routine,
            cfg: &cfg,
            private,
        };
        let result = solve_dataflow(&cfg, &problem);
        routines.insert(
            routine.id,
            routine
                .blocks
                .iter()
                .filter_map(|block| {
                    result
                        .out_state(block.id)
                        .cloned()
                        .flatten()
                        .map(|state| (block.id, state))
                })
                .collect(),
        );
    }
    Ok(NirAlignmentAnalysis { program, routines })
}

impl NirAlignmentAnalysis<'_> {
    pub fn value_proof(
        &self,
        routine: RoutineId,
        block: BlockId,
        value: &NirValue,
    ) -> Option<NirAlignmentProof> {
        (self.value_alignment(routine, block, value).get() == 2).then(|| NirAlignmentProof {
            routine,
            block,
            value: value.clone(),
        })
    }
    /// Scalar SSA values remain captured across later memory effects. A missing
    /// block or value is unknown, including unreachable blocks and parameters.
    pub fn value_alignment(
        &self,
        routine: RoutineId,
        block: BlockId,
        value: &NirValue,
    ) -> ByteSize {
        let even = self
            .routines
            .get(&routine)
            .and_then(|r| r.get(&block))
            .is_some_and(|state| value_even(self.program, state, value));
        ByteSize::new(if even { 2 } else { 1 })
    }
}

struct AlignmentProblem<'a> {
    program: &'a NirProgram,
    routine: &'a NirRoutine,
    cfg: &'a NirCfg,
    private: BTreeSet<NirStorageId>,
}

impl NirDataflowProblem for AlignmentProblem<'_> {
    // None is an unvisited/unreachable edge, NOT a reachable unknown value.
    type State = Option<State>;
    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Forward
    }
    fn bottom(&self) -> Self::State {
        None
    }
    fn boundary(&self, block: BlockId) -> Option<Self::State> {
        (Some(block) == self.cfg.entry()).then(|| Some(State::default()))
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        let Some(other) = other else { return };
        if let Some(into) = into {
            into.temps.retain(|id| other.temps.contains(id));
            into.homes.retain(|id| other.homes.contains(id));
        } else {
            *into = Some(other.clone());
        }
    }
    fn transfer(&self, block: BlockId, state: &Self::State) -> Self::State {
        let mut state = state.clone()?;
        let block = &self.routine.blocks[self.cfg.block_index(block)?];
        for op in &block.ops {
            let known = match op {
                NirOp::AddrOf { dest, place, .. } => Some((*dest, self.place_even(&state, place))),
                NirOp::Load { dest, place, .. } => Some((
                    *dest,
                    direct_storage_id(place).is_some_and(|id| state.homes.contains(&id)),
                )),
                NirOp::Unary { dest, src, .. } => {
                    Some((*dest, value_even(self.program, &state, src)))
                }
                NirOp::Cast {
                    dest,
                    src,
                    from,
                    to,
                    ..
                } => Some((
                    *dest,
                    // Keep narrowing conservative; widening preserves bit zero.
                    from.width
                        .zip(to.width)
                        .is_some_and(|(from, to)| from <= to)
                        && value_even(self.program, &state, src),
                )),
                NirOp::PointerOffset {
                    dest, base, offset, ..
                } => Some((
                    *dest,
                    value_even(self.program, &state, base)
                        && value_even(self.program, &state, offset),
                )),
                NirOp::Binary {
                    dest,
                    op,
                    left,
                    right,
                    ty,
                } => {
                    let a = value_even(self.program, &state, left);
                    let b = value_even(self.program, &state, right);
                    let even = match op {
                        NirBinaryOp::Add
                        | NirBinaryOp::Sub
                        | NirBinaryOp::Or
                        | NirBinaryOp::Xor => a && b,
                        NirBinaryOp::Mul | NirBinaryOp::And => a || b,
                        NirBinaryOp::Lsh => right.as_integer_const().is_some_and(|(count, _)| {
                            count == 0 && a
                                || count > 0
                                    && ty.kind.integer().is_some_and(|i| count < u64::from(i.bits))
                        }),
                        _ => false,
                    };
                    Some((*dest, even))
                }
                NirOp::Store { place, src, .. } => {
                    if let Some(id) =
                        direct_storage_id(place).filter(|id| self.private.contains(id))
                    {
                        if value_even(self.program, &state, src) {
                            state.homes.insert(id);
                        } else {
                            state.homes.remove(&id);
                        }
                    }
                    None
                }
                NirOp::VolatileLoad { dest, .. } | NirOp::Compare { dest, .. } => {
                    Some((*dest, false))
                }
                NirOp::Call { result, .. } => result.as_ref().map(|r| (r.dest, false)),
                NirOp::Real(_) | NirOp::ForeignCode { .. } | NirOp::Unsupported { .. } => {
                    state.homes.clear();
                    state.temps.clear();
                    None
                }
                // Aliased/volatile/copy-visible cells were excluded from private.
                NirOp::VolatileStore { .. } | NirOp::CopyBytes { .. } => None,
            };
            if let Some((id, even)) = known {
                if even {
                    state.temps.insert(id);
                } else {
                    state.temps.remove(&id);
                }
            }
        }
        Some(state)
    }
    fn transfer_forward_edge(
        &self,
        from: BlockId,
        to: BlockId,
        state: &Self::State,
    ) -> Self::State {
        let state = state.as_ref()?;
        let target = &self.routine.blocks[self.cfg.block_index(to)?];
        let from = &self.routine.blocks[self.cfg.block_index(from)?];
        let edges: Vec<_> = match &from.terminator {
            NirTerminator::Goto(edge) => vec![edge],
            NirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => vec![then_edge, else_edge],
            _ => Vec::new(),
        };
        // Multiple edges between the same blocks can carry different arguments.
        let edges: Vec<_> = edges.into_iter().filter(|edge| edge.target == to).collect();
        let mut next = state.clone();
        for (index, param) in target.params.iter().enumerate() {
            if !edges.is_empty()
                && edges
                    .iter()
                    .all(|edge| value_even(self.program, state, &edge.args[index]))
            {
                next.temps.insert(param.dest);
            } else {
                next.temps.remove(&param.dest);
            }
        }
        Some(next)
    }
}

impl AlignmentProblem<'_> {
    fn place_even(&self, state: &State, place: &NirPlace) -> bool {
        match &place.kind {
            NirPlaceKind::Param { id, .. } => self
                .routine
                .params
                .iter()
                .find(|p| p.id == *id)
                .is_some_and(|p| p.layout.alignment.get() >= 2),
            NirPlaceKind::Local { id, .. } => self.local_even(*id, 0),
            NirPlaceKind::Global { id, .. } => global_even(self.program, *id, 0),
            NirPlaceKind::Absolute(address) => address.value & 1 == 0,
            NirPlaceKind::Deref { addr } => value_even(self.program, state, addr),
            NirPlaceKind::Index {
                base_addr,
                index,
                elem_size,
                ..
            } => {
                value_even(self.program, state, base_addr)
                    && (elem_size.get() & 1 == 0 || value_even(self.program, state, index))
            }
            NirPlaceKind::Field { base, offset, .. } => {
                offset.get() & 1 == 0 && self.place_even(state, base)
            }
        }
    }
    fn local_even(&self, id: LocalId, depth: usize) -> bool {
        if depth > self.routine.locals.len() {
            return false;
        }
        let Some(local) = self.routine.locals.iter().find(|l| l.id == id) else {
            return false;
        };
        match local.backing {
            NirLocalBacking::Ordinary => local.layout.alignment.get() >= 2,
            NirLocalBacking::Absolute(address) => address.value & 1 == 0,
            NirLocalBacking::Alias { target, offset, .. } => {
                offset.get() & 1 == 0 && self.local_even(target, depth + 1)
            }
            NirLocalBacking::GlobalAlias { target, offset, .. } => {
                offset.get() & 1 == 0 && global_even(self.program, target, 0)
            }
        }
    }
}

fn value_even(program: &NirProgram, state: &State, value: &NirValue) -> bool {
    match value {
        NirValue::IntegerConst { bits, .. } => bits & 1 == 0,
        NirValue::Null { .. } => true,
        NirValue::AddressConst { address, .. } => address.value & 1 == 0,
        NirValue::Temp { id, .. } => state.temps.contains(id),
        NirValue::StaticAddr { id, .. } => program
            .statics
            .iter()
            .find(|s| s.id == *id)
            .is_some_and(|s| s.alignment.get() >= 2),
        NirValue::GlobalAddr(id) => global_even(program, *id, 0),
        NirValue::Param(_) | NirValue::RoutineAddr { .. } | NirValue::Aggregate { .. } => false,
    }
}

fn global_even(program: &NirProgram, id: SymbolId, depth: usize) -> bool {
    if depth > program.globals.len() {
        return false;
    }
    let Some(global) = program.globals.iter().find(|g| g.id == id) else {
        return false;
    };
    match global.backing {
        NirGlobalBacking::Absolute(address) => address.value & 1 == 0,
        NirGlobalBacking::Alias { target, offset } => {
            offset.get() & 1 == 0 && global_even(program, target, depth + 1)
        }
        NirGlobalBacking::Ordinary => {
            // These are guarantees of the target object layout, never of a
            // pointer loaded from that object (including array descriptors).
            let natural = program.target_layout.natural_word_alignment_bytes >= 2;
            if let Some(array) = &global.array {
                return natural && array.elem_size.get() >= 2;
            }
            global.ty.as_ref().is_some_and(|ty| match &ty.kind {
                NirTypeKind::Integer(i) => natural && i.storage_width().get() >= 2,
                NirTypeKind::Pointer { .. } => {
                    program.target_layout.data_pointer.alignment_bytes.get() >= 2
                }
                NirTypeKind::Callable { .. } => {
                    program.target_layout.code_pointer.alignment_bytes.get() >= 2
                }
                NirTypeKind::Record { .. } => {
                    natural
                        && program.target_layout.record_layout
                            == crate::target::RecordLayoutPolicy::Natural
                }
                _ => false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lower(source: &str) -> NirProgram {
        let tokens = crate::lexer::tokenize(source).unwrap();
        let ast = crate::parser::parse(&tokens).unwrap();
        let model = crate::semantic::analyze_with_options(
            &ast,
            crate::semantic::SemanticOptions::modern()
                .with_target(crate::target::TargetId::Motorola68000),
        )
        .unwrap();
        lower_program(&crate::semantic::ir::lower_program(&ast, &model))
    }
    const SOURCE: &str = "\
LONGINT ARRAY data(16)\n\
LONGINT ARRAY descriptor(2)=[1 2]\n\
BYTE flag\n\
LONGINT result\n\
PROC Touch() RETURN\n\
PROC Main()\n\
  LONGINT POINTER p\n\
  CARD i\n\
  p=data\n\
  FOR i=0 TO 3 DO result=p^ p==+4 OD\n\
  Touch() result=p^\n\
  p=descriptor result=p^\n\
  IF flag THEN p=data ELSE p=data p==+1 FI\n\
  result=p^\n\
RETURN\n";

    fn dereferences(program: &NirProgram) -> Vec<u32> {
        let analysis = analyze_alignment(program).unwrap();
        let routine = program.routines.iter().find(|r| r.name == "Main").unwrap();
        routine
            .blocks
            .iter()
            .flat_map(|block| {
                block.ops.iter().filter_map(|op| {
                    if let NirOp::Load {
                        place:
                            NirPlace {
                                kind: NirPlaceKind::Deref { addr },
                                ..
                            },
                        ..
                    } = op
                    {
                        Some(analysis.value_alignment(routine.id, block.id, addr).get())
                    } else {
                        None
                    }
                })
            })
            .collect()
    }

    #[test]
    fn loops_preserve_even_offsets_but_descriptors_and_mixed_joins_are_unknown() {
        let raw = lower(SOURCE);
        assert_eq!(dereferences(&raw), [2, 2, 1, 1]);
        assert_eq!(dereferences(&optimize_program(&raw).unwrap()), [2, 2, 1, 1]);
        let odd_stride = lower(&SOURCE.replace("p==+4", "p==+1"));
        assert_eq!(dereferences(&odd_stride), [1, 1, 1, 1]);
    }

    #[test]
    fn explicit_call_write_effects_prevent_tracking_a_private_pointer_cell() {
        let mut program = lower(SOURCE);
        let routine = program
            .routines
            .iter_mut()
            .find(|r| r.name == "Main")
            .unwrap();
        let pointer = routine.locals.iter().find(|l| l.name == "p").unwrap().id;
        for op in routine.blocks.iter_mut().flat_map(|b| &mut b.ops) {
            if let NirOp::Call { effects, .. } = op {
                effects.memory.writes = NirMemoryAccess::Regions(vec![NirMemoryRegion {
                    kind: NirMemoryRegionKind::Storage(NirStorageId::Local(pointer)),
                    offset: ByteOffset::ZERO,
                    size: ByteSize::new(4),
                }]);
            }
        }
        assert_eq!(dereferences(&program), [1, 1, 1, 1]);
    }

    #[test]
    fn parallel_edges_meet_arguments_and_uninitialized_loops_cannot_prove_themselves() {
        let mut program = lower(SOURCE);
        let routine = program
            .routines
            .iter_mut()
            .find(|r| r.name == "Main")
            .unwrap();
        let ty = routine
            .locals
            .iter()
            .find(|l| l.name == "p")
            .unwrap()
            .ty
            .clone();
        let value = NirValue::Temp {
            id: TempId(0),
            ty: ty.clone(),
        };
        let edge = |address| NirEdge {
            target: BlockId(1),
            args: vec![NirValue::AddressConst {
                address: AddressValue::data(address),
                ty: ty.clone(),
            }],
        };
        routine.blocks = vec![
            NirBlock {
                id: BlockId(0),
                label: "entry".into(),
                params: vec![],
                ops: vec![],
                terminator: NirTerminator::Branch {
                    condition: NirValue::ConstU8(1),
                    then_edge: edge(0x4000),
                    else_edge: edge(0x4001),
                },
            },
            NirBlock {
                id: BlockId(1),
                label: "join".into(),
                params: vec![NirBlockParam {
                    dest: TempId(0),
                    ty: ty.clone(),
                }],
                ops: vec![],
                terminator: NirTerminator::Return(None),
            },
        ];
        routine.temps = vec![NirTemp {
            id: TempId(0),
            ty,
            def: NirTempDef {
                block: BlockId(1),
                op_index: None,
            },
        }];
        let id = routine.id;
        assert_eq!(
            analyze_alignment(&program)
                .unwrap()
                .value_alignment(id, BlockId(1), &value)
                .get(),
            1
        );
        if let NirTerminator::Branch { else_edge, .. } = &mut program
            .routines
            .iter_mut()
            .find(|r| r.id == id)
            .unwrap()
            .blocks[0]
            .terminator
        {
            if let NirValue::AddressConst { address, .. } = &mut else_edge.args[0] {
                address.value = 0x4002;
            }
        }
        assert_eq!(
            analyze_alignment(&program)
                .unwrap()
                .value_alignment(id, BlockId(1), &value)
                .get(),
            2
        );
        let unknown_entry = lower(&SOURCE.replacen("p=data\n", "", 1));
        assert_eq!(dereferences(&unknown_entry)[..2], [1, 1]);
    }
}
