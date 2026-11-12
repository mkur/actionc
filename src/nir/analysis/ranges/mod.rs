//! Bounded unsigned value analysis over an immutable, verified NIR snapshot.
//! Absence of a fact means unknown; `None` dataflow state means unreachable.
use super::{
    cfg::NirCfg,
    dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow},
    dominance::NirDominance,
    use_def::NirUseDef,
};
use crate::nir::*;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

mod summaries;
use summaries::ReturnSummary;

const MAX_BLOCKS: usize = 512;
const MAX_OPS: usize = 4096;
const MAX_EVALUATIONS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NirUnsignedRange {
    min: u32,
    max: u32,
    bits: u8,
}
impl NirUnsignedRange {
    pub fn min(self) -> u32 {
        self.min
    }
    pub fn max(self) -> u32 {
        self.max
    }
    pub fn bits(self) -> u8 {
        self.bits
    }
    fn top(bits: u8) -> Self {
        Self {
            min: 0,
            max: (1u32 << bits) - 1,
            bits,
        }
    }
    fn singleton(value: u32, bits: u8) -> Self {
        Self {
            min: value,
            max: value,
            bits,
        }
    }
    fn hull(self, other: Self) -> Self {
        debug_assert_eq!(self.bits, other.bits);
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
            ..self
        }
    }
    fn intersect(self, min: u32, max: u32) -> Option<Self> {
        let (min, max) = (self.min.max(min), self.max.min(max));
        (min <= max).then_some(Self { min, max, ..self })
    }
}

/// A receipt identifying the exact point and value in the analyzed snapshot.
/// Consumers must preserve these associations or discard and recompute it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NirUnsignedRangeProof {
    routine: RoutineId,
    block: BlockId,
    op_index: usize,
    value: NirValue,
    range: NirUnsignedRange,
}

/// A complete ordinary storage value at an exact point, without an ownership,
/// lifetime, buffer-capacity or universally initialized-global claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NirStorageRangeProof {
    routine: RoutineId,
    block: BlockId,
    op_index: usize,
    storage: NirStorageId,
    range: NirUnsignedRange,
}
impl NirStorageRangeProof {
    pub fn range(&self) -> NirUnsignedRange {
        self.range
    }
    pub fn source(&self) -> (RoutineId, BlockId, usize, NirStorageId) {
        (self.routine, self.block, self.op_index, self.storage)
    }
}
impl NirUnsignedRangeProof {
    pub fn range(&self) -> NirUnsignedRange {
        self.range
    }
    pub fn source(&self) -> (RoutineId, BlockId, usize, &NirValue) {
        (self.routine, self.block, self.op_index, &self.value)
    }
}

/// The borrow prevents using analysis tables after mutating their program.
pub struct NirRangeAnalysis<'a> {
    program: &'a NirProgram,
    routines: BTreeMap<RoutineId, RoutineFacts>,
    non_returning: BTreeSet<RoutineId>,
    summaries: BTreeMap<RoutineId, ReturnSummary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Temp(TempId),
    Constant(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Fact {
    range: NirUnsignedRange,
    key: Option<Key>,
    // Only recorded after proving that the subtraction cannot wrap.
    difference: Option<(u32, Key)>,
}
impl Fact {
    fn unknown(bits: u8, key: Option<Key>) -> Self {
        Self {
            range: NirUnsignedRange::top(bits),
            key,
            difference: None,
        }
    }
    fn join(&self, other: &Self) -> Self {
        Self {
            range: self.range.hull(other.range),
            key: (self.key == other.key).then_some(self.key).flatten(),
            difference: (self.difference == other.difference)
                .then_some(self.difference)
                .flatten(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    values: BTreeMap<Key, Fact>,
    cells: BTreeMap<NirStorageId, Fact>,
    // a+b <= limit; both keys identify captured values, never storage names.
    sums: BTreeSet<(Key, Key, u32)>,
}
impl State {
    fn join(&mut self, other: &Self) {
        self.values.retain(|key, fact| {
            let Some(rhs) = other.values.get(key) else {
                return false;
            };
            *fact = fact.join(rhs);
            true
        });
        self.cells.retain(|key, fact| {
            let Some(rhs) = other.cells.get(key) else {
                return false;
            };
            if fact.range.bits != rhs.range.bits {
                return false;
            }
            *fact = fact.join(rhs);
            true
        });
        self.sums.retain(|sum| other.sums.contains(sum));
    }
    fn forget_key(&mut self, key: Key) {
        // A definition in a loop is executed again. Prior-iteration aliases
        // must not acquire the next execution's refinements or certificates.
        for f in self.values.values_mut().chain(self.cells.values_mut()) {
            if f.key == Some(key) {
                f.key = None;
            }
            if f.difference.is_some_and(|(_, k)| k == key) {
                f.difference = None;
            }
        }
        self.sums.retain(|(a, b, _)| *a != key && *b != key);
        self.values.remove(&key);
    }
    fn refine(&mut self, fact: &Fact, min: u32, max: u32) -> Option<()> {
        fact.range.intersect(min, max)?;
        if let Some(key) = fact.key {
            for f in self.values.values_mut().chain(self.cells.values_mut()) {
                if f.key == Some(key) {
                    f.range = f.range.intersect(min, max)?;
                }
            }
        }
        Some(())
    }
}

struct RoutineFacts {
    inputs: BTreeMap<BlockId, State>,
    outputs: BTreeMap<BlockId, State>,
    dominance: NirDominance,
    trackable: BTreeMap<NirStorageId, u8>,
    unique: BTreeSet<TempId>,
    evaluations: usize,
}

pub fn analyze_unsigned_ranges(
    program: &NirProgram,
) -> Result<NirRangeAnalysis<'_>, Vec<NirDiagnostic>> {
    verify_program(program)?;
    let mut resolver = summaries::Resolver::new(program);
    for routine in &program.routines {
        resolver.resolve(routine.id);
    }
    Ok(NirRangeAnalysis {
        program,
        routines: resolver.routines,
        non_returning: resolver.non_returning,
        summaries: resolver.summaries,
    })
}

fn analyze_routine(
    routine: &NirRoutine,
    trackable: &BTreeMap<NirStorageId, u8>,
    non_returning: &BTreeSet<RoutineId>,
    summaries: &BTreeMap<RoutineId, ReturnSummary>,
) -> Option<RoutineFacts> {
    if routine.blocks.len() > MAX_BLOCKS
        || routine.blocks.iter().map(|b| b.ops.len()).sum::<usize>() > MAX_OPS
    {
        return None;
    }
    let cfg = NirCfg::from_routine(routine);
    let uses = NirUseDef::from_routine(routine);
    let unique = uses
        .all_definitions()
        .keys()
        .copied()
        .filter(|id| uses.unique_definition(*id).is_some())
        .collect();
    let problem = Problem {
        routine,
        cfg: &cfg,
        dominance: NirDominance::from_cfg(&cfg),
        uses,
        trackable,
        unique: &unique,
        non_returning,
        summaries,
        evaluations: Cell::new(0),
        exhausted: Cell::new(false),
    };
    let result = solve_dataflow(&cfg, &problem);
    if problem.exhausted.get() {
        return None;
    }
    let inputs = routine
        .blocks
        .iter()
        .filter_map(|b| result.in_state(b.id)?.clone().map(|s| (b.id, s)))
        .collect();
    let outputs = routine
        .blocks
        .iter()
        .filter_map(|b| result.out_state(b.id)?.clone().map(|s| (b.id, s)))
        .collect();
    Some(RoutineFacts {
        inputs,
        outputs,
        dominance: NirDominance::from_cfg(&cfg),
        trackable: trackable.clone(),
        unique,
        evaluations: result.evaluations(),
    })
}

impl NirRangeAnalysis<'_> {
    /// Query immediately before an operation, or before the terminator when
    /// `op_index == block.ops.len()`. Unknown/over-budget routines return None.
    pub fn prove_before(
        &self,
        routine: RoutineId,
        block: BlockId,
        op_index: usize,
        value: &NirValue,
    ) -> Option<NirUnsignedRangeProof> {
        let r = self.program.routines.iter().find(|r| r.id == routine)?;
        let b = r.blocks.iter().find(|b| b.id == block)?;
        if op_index > b.ops.len() {
            return None;
        }
        let facts = self.routines.get(&routine)?;
        if let NirValue::Temp { id, ty } = value {
            let temp = r.temps.iter().find(|t| t.id == *id)?;
            if temp.ty != *ty
                || !facts.unique.contains(id)
                || !facts.dominance.dominates(temp.def.block, block)
                || (temp.def.block == block && temp.def.op_index.is_some_and(|i| i >= op_index))
            {
                return None;
            }
        }
        let mut state = facts.inputs.get(&block)?.clone();
        for op in &b.ops[..op_index] {
            transfer_op(
                r,
                &facts.trackable,
                &facts.unique,
                &self.non_returning,
                &self.summaries,
                &mut state,
                op,
            )?;
        }
        let range = value_fact(r, &state, value)?.range;
        Some(NirUnsignedRangeProof {
            routine,
            block,
            op_index,
            value: value.clone(),
            range,
        })
    }
    pub fn normal_return_range(&self, routine: RoutineId) -> Option<NirUnsignedRange> {
        self.summaries.get(&routine)?.range
    }
    pub fn storage_ranges_at_entry(
        &self,
        routine: RoutineId,
        block: BlockId,
    ) -> Vec<NirStorageRangeProof> {
        let Some(state) = self
            .routines
            .get(&routine)
            .and_then(|r| r.inputs.get(&block))
        else {
            return vec![];
        };
        state
            .cells
            .iter()
            .map(|(storage, f)| NirStorageRangeProof {
                routine,
                block,
                op_index: 0,
                storage: *storage,
                range: f.range,
            })
            .collect()
    }
    pub fn is_non_returning(&self, routine: RoutineId) -> bool {
        self.non_returning.contains(&routine)
    }
    pub fn evaluations(&self, routine: RoutineId) -> Option<usize> {
        Some(self.routines.get(&routine)?.evaluations)
    }
}

fn type_bits(ty: &NirType) -> Option<u8> {
    match ty.kind {
        NirTypeKind::Bool => Some(1),
        NirTypeKind::Integer(t) if !t.signed && matches!(t.bits, 8 | 16) => Some(t.bits),
        NirTypeKind::Pointer { .. } if ty.width == Some(ByteSize::new(2)) => Some(16),
        _ => None,
    }
}

fn value_fact(_routine: &NirRoutine, state: &State, value: &NirValue) -> Option<Fact> {
    let (bits, key, constant) = match value {
        NirValue::IntegerConst { bits: value, ty }
            if matches!(ty.bits, 8 | 16)
                && (!ty.signed || value & (1u64 << (ty.bits - 1)) == 0) =>
        {
            (ty.bits, Key::Constant(*value as u32), Some(*value as u32))
        }
        NirValue::AddressConst { address, ty } if address.value <= 65535 => (
            type_bits(ty)?,
            Key::Constant(address.value as u32),
            Some(address.value as u32),
        ),
        NirValue::Null { ty } => (type_bits(ty)?, Key::Constant(0), Some(0)),
        NirValue::Temp { id, ty } => (type_bits(ty)?, Key::Temp(*id), None),
        _ => return None,
    };
    if let Some(value) = constant {
        return Some(Fact {
            range: NirUnsignedRange::singleton(value, bits),
            key: Some(key),
            difference: None,
        });
    }
    state
        .values
        .get(&key)
        .cloned()
        .or_else(|| Some(Fact::unknown(bits, None)))
}

fn transfer_op(
    routine: &NirRoutine,
    trackable: &BTreeMap<NirStorageId, u8>,
    unique: &BTreeSet<TempId>,
    non_returning: &BTreeSet<RoutineId>,
    summaries: &BTreeMap<RoutineId, ReturnSummary>,
    state: &mut State,
    op: &NirOp,
) -> Option<()> {
    let dest = match op {
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
        NirOp::Real(
            NirRealOp::Compare { result, .. } | NirRealOp::RealToInteger { result, .. },
        ) => Some(*result),
        _ => None,
    };
    if crate::nir::optimizer::binary_may_fault(op) {
        state.cells.clear();
    }
    if let Some(dest) = dest {
        state.forget_key(Key::Temp(dest));
    }
    let mut assigned = None;
    match op {
        NirOp::Load { dest, ty, place } => {
            if let Some(bits) = type_bits(ty) {
                let id = direct_storage_id(place).filter(|id| trackable.get(id) == Some(&bits));
                let mut fact = id
                    .and_then(|id| state.cells.get(&id).cloned())
                    .unwrap_or_else(|| Fact::unknown(bits, None));
                if fact.key.is_none() && unique.contains(dest) {
                    fact.key = Some(Key::Temp(*dest));
                }
                if let Some(id) = id {
                    state.cells.insert(id, fact.clone());
                }
                assigned = Some(fact);
            }
        }
        NirOp::VolatileLoad { .. } => {
            state.cells.clear();
        }
        NirOp::Store { place, src, ty } => {
            if let Some(id) = direct_storage_id(place) {
                state.cells.remove(&id);
                if let Some(bits) = type_bits(ty).filter(|bits| trackable.get(&id) == Some(bits)) {
                    if let Some(fact) =
                        value_fact(routine, state, src).filter(|f| f.range.bits == bits)
                    {
                        state.cells.insert(id, fact);
                    }
                }
            } else {
                state.cells.clear();
            }
        }
        NirOp::Cast { src, to, .. } => {
            // Positive signed literals have a known mathematical value.
            // Signed variables and negative signed constants remain unknown.
            if let (Some(bits), Some(mut fact)) = (type_bits(to), value_fact(routine, state, src)) {
                if fact.range.max <= NirUnsignedRange::top(bits).max {
                    fact.range.bits = bits;
                    assigned = Some(fact);
                }
            }
        }
        NirOp::Unary {
            op: NirUnaryOp::Plus,
            src,
            ..
        } => assigned = value_fact(routine, state, src),
        NirOp::Binary {
            ty,
            op,
            left,
            right,
            ..
        } => {
            if let (Some(bits), Some(a), Some(b)) = (
                type_bits(ty),
                value_fact(routine, state, left),
                value_fact(routine, state, right),
            ) {
                assigned = arithmetic(bits, *op, &a, &b, state);
            }
        }
        NirOp::PointerOffset {
            ty,
            base,
            offset,
            subtract,
            ..
        } => {
            if let (Some(bits), Some(a), Some(b)) = (
                type_bits(ty),
                value_fact(routine, state, base),
                value_fact(routine, state, offset),
            ) {
                assigned = arithmetic(
                    bits,
                    if *subtract {
                        NirBinaryOp::Sub
                    } else {
                        NirBinaryOp::Add
                    },
                    &a,
                    &b,
                    state,
                );
            }
        }
        NirOp::Compare {
            ty,
            operand_ty,
            op,
            left,
            right,
            ..
        } if type_bits(operand_ty).is_some() => {
            if let (Some(a), Some(b), Some(bits)) = (
                value_fact(routine, state, left),
                value_fact(routine, state, right),
                type_bits(ty),
            ) {
                let truth = compare_known(*op, a.range, b.range);
                assigned = Some(Fact {
                    range: truth.map_or(
                        NirUnsignedRange {
                            min: 0,
                            max: 1,
                            bits,
                        },
                        |t| NirUnsignedRange::singleton(u32::from(t), bits),
                    ),
                    key: None,
                    difference: None,
                });
            }
        }
        NirOp::Call {
            callee: NirCallee::Fault(_),
            ..
        } => return None,
        NirOp::Call {
            callee: NirCallee::User { id, .. },
            ..
        } if non_returning.contains(id) => return None,
        NirOp::Call {
            callee: NirCallee::User { id, .. },
            result,
            ..
        } if summaries.contains_key(id) => {
            let summary = &summaries[id];
            summary.apply_writes(routine.id, state);
            if let Some(result) = result {
                if let (Some(bits), Some(range)) = (type_bits(&result.ty), summary.range) {
                    if bits == range.bits {
                        assigned = Some(Fact {
                            range,
                            key: None,
                            difference: None,
                        });
                    }
                }
            }
        }
        NirOp::Call { .. }
        | NirOp::ForeignCode { .. }
        | NirOp::VolatileStore { .. }
        | NirOp::CopyBytes { .. }
        | NirOp::Real(_) => state.cells.clear(),
        _ => {}
    }
    if let Some(dest) = dest {
        if assigned.is_none() {
            assigned = routine
                .temps
                .iter()
                .find(|t| t.id == dest)
                .and_then(|t| type_bits(&t.ty))
                .map(|bits| Fact::unknown(bits, None));
        }
        if let Some(mut fact) = assigned {
            if fact.key.is_none() && unique.contains(&dest) {
                fact.key = Some(Key::Temp(dest));
            }
            state.values.insert(Key::Temp(dest), fact);
        }
    }
    Some(())
}

fn arithmetic(bits: u8, op: NirBinaryOp, a: &Fact, b: &Fact, state: &State) -> Option<Fact> {
    let ceiling = NirUnsignedRange::top(bits).max;
    let (min, max, difference) = match op {
        NirBinaryOp::Add => {
            let min = a.range.min + b.range.min;
            let mut max = a.range.max + b.range.max;
            if let (Some(a), Some(b)) = (a.key, b.key) {
                for &(x, y, limit) in &state.sums {
                    if (a == x && b == y) || (a == y && b == x) {
                        max = max.min(limit);
                    }
                }
            }
            (min, max, None)
        }
        NirBinaryOp::Sub if a.range.min >= b.range.max => (
            a.range.min - b.range.max,
            a.range.max - b.range.min,
            (a.range.min == a.range.max)
                .then(|| b.key.map(|k| (a.range.min, k)))
                .flatten(),
        ),
        _ => return None,
    };
    (min <= max && max <= ceiling).then_some(Fact {
        range: NirUnsignedRange { min, max, bits },
        key: None,
        difference,
    })
}

fn compare_known(op: NirCompareOp, a: NirUnsignedRange, b: NirUnsignedRange) -> Option<bool> {
    match op {
        NirCompareOp::Lt if a.max < b.min => Some(true),
        NirCompareOp::Lt if a.min >= b.max => Some(false),
        NirCompareOp::Le if a.max <= b.min => Some(true),
        NirCompareOp::Le if a.min > b.max => Some(false),
        NirCompareOp::Gt => compare_known(NirCompareOp::Lt, b, a),
        NirCompareOp::Ge => compare_known(NirCompareOp::Le, b, a),
        NirCompareOp::Eq if a.max < b.min || b.max < a.min => Some(false),
        NirCompareOp::Eq if a.min == a.max && a == b => Some(true),
        NirCompareOp::Ne => compare_known(NirCompareOp::Eq, a, b).map(|v| !v),
        _ => None,
    }
}

struct Problem<'a> {
    routine: &'a NirRoutine,
    cfg: &'a NirCfg,
    dominance: NirDominance,
    uses: NirUseDef,
    trackable: &'a BTreeMap<NirStorageId, u8>,
    unique: &'a BTreeSet<TempId>,
    non_returning: &'a BTreeSet<RoutineId>,
    summaries: &'a BTreeMap<RoutineId, ReturnSummary>,
    evaluations: Cell<usize>,
    exhausted: Cell<bool>,
}
impl Problem<'_> {
    fn block(&self, id: BlockId) -> Option<&NirBlock> {
        self.routine.blocks.iter().find(|b| b.id == id)
    }
    fn assume(&self, state: &mut State, block: &NirBlock, taken: bool) -> Option<()> {
        let NirTerminator::Branch { condition, .. } = &block.terminator else {
            return Some(());
        };
        if let Some(f) = value_fact(self.routine, state, condition) {
            if f.range.max == 0 && taken || f.range.min > 0 && !taken {
                return None;
            }
        }
        let NirValue::Temp { id, .. } = condition else {
            return Some(());
        };
        let Some(def) = self.uses.unique_definition(*id) else {
            return Some(());
        };
        if !self.dominance.dominates(def.block, block.id) {
            return Some(());
        }
        let Some(NirOp::Compare {
            op,
            operand_ty,
            left,
            right,
            ..
        }) = def.op_index.and_then(|i| self.block(def.block)?.ops.get(i))
        else {
            return Some(());
        };
        if type_bits(operand_ty).is_none() {
            return Some(());
        }
        let (Some(a), Some(b)) = (
            value_fact(self.routine, state, left),
            value_fact(self.routine, state, right),
        ) else {
            return Some(());
        };
        let op = if taken {
            *op
        } else {
            match op {
                NirCompareOp::Eq => NirCompareOp::Ne,
                NirCompareOp::Ne => NirCompareOp::Eq,
                NirCompareOp::Lt => NirCompareOp::Ge,
                NirCompareOp::Le => NirCompareOp::Gt,
                NirCompareOp::Gt => NirCompareOp::Le,
                NirCompareOp::Ge => NirCompareOp::Lt,
            }
        };
        assume_compare(state, op, &a, &b)
    }
}
impl NirDataflowProblem for Problem<'_> {
    type State = Option<State>;
    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Forward
    }
    fn bottom(&self) -> Self::State {
        None
    }
    fn boundary(&self, block: BlockId) -> Option<Self::State> {
        (self.cfg.entry() == Some(block)).then(|| Some(State::default()))
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        if let Some(other) = other {
            if let Some(into) = into {
                into.join(other);
            } else {
                *into = Some(other.clone());
            }
        }
    }
    fn transfer(&self, block: BlockId, state: &Self::State) -> Self::State {
        self.evaluations.set(self.evaluations.get() + 1);
        if self.evaluations.get() > MAX_EVALUATIONS {
            self.exhausted.set(true);
            return None;
        }
        let mut state = state.clone()?;
        for op in &self.block(block)?.ops {
            transfer_op(
                self.routine,
                self.trackable,
                self.unique,
                self.non_returning,
                self.summaries,
                &mut state,
                op,
            )?;
        }
        Some(state)
    }
    fn transfer_forward_edge(
        &self,
        from: BlockId,
        to: BlockId,
        output: &Self::State,
    ) -> Self::State {
        let block = self.block(from)?;
        let edges = match &block.terminator {
            NirTerminator::Goto(e) => vec![(e, None)],
            NirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => vec![(then_edge, Some(true)), (else_edge, Some(false))],
            _ => return None,
        };
        let mut joined = None;
        for (edge, taken) in edges {
            if edge.target != to {
                continue;
            }
            let Some(mut state) = output.clone() else {
                continue;
            };
            if taken.is_some_and(|t| self.assume(&mut state, block, t).is_none()) {
                continue;
            }
            let args = edge
                .args
                .iter()
                .map(|a| value_fact(self.routine, &state, a))
                .collect::<Vec<_>>();
            for (param, arg) in self.block(to)?.params.iter().zip(args) {
                state.forget_key(Key::Temp(param.dest));
                if let (Some(bits), Some(mut fact)) = (type_bits(&param.ty), arg) {
                    if fact.range.max <= NirUnsignedRange::top(bits).max {
                        fact.range.bits = bits;
                    } else {
                        fact = Fact::unknown(bits, None);
                    }
                    state.values.insert(Key::Temp(param.dest), fact);
                }
            }
            self.join(&mut joined, &Some(state));
        }
        joined
    }
}

fn assume_compare(state: &mut State, op: NirCompareOp, a: &Fact, b: &Fact) -> Option<()> {
    match op {
        NirCompareOp::Gt => return assume_compare(state, NirCompareOp::Lt, b, a),
        NirCompareOp::Ge => return assume_compare(state, NirCompareOp::Le, b, a),
        NirCompareOp::Lt | NirCompareOp::Le => {
            let strict = u32::from(op == NirCompareOp::Lt);
            state.refine(a, 0, b.range.max.checked_sub(strict)?)?;
            state.refine(b, a.range.min.checked_add(strict)?, u32::MAX)?;
            if let (Some(key), Some((limit, other))) = (a.key, b.difference) {
                state.sums.insert((key, other, limit.checked_sub(strict)?));
            }
        }
        NirCompareOp::Eq => {
            state.refine(a, b.range.min, b.range.max)?;
            state.refine(b, a.range.min, a.range.max)?;
        }
        NirCompareOp::Ne => {
            if a.range.min == a.range.max
                && b.range.min == b.range.max
                && a.range.min == b.range.min
            {
                return None;
            }
        }
    }
    Some(())
}

fn non_returning_routines(program: &NirProgram) -> BTreeSet<RoutineId> {
    let mut proved = BTreeSet::new();
    // Acyclic proof dependencies only. An unresolved recursive cycle cannot
    // manufacture its own non-return certificate.
    for _ in 0..16 {
        let previous = proved.clone();
        for r in &program.routines {
            if r.entry.external
                || r.entry.external_symbol.is_some()
                || matches!(r.entry.placement, NirRoutinePlacement::Absolute(_))
                || r.blocks.is_empty()
                || r.blocks.len() > MAX_BLOCKS
                || r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() > MAX_OPS
            {
                continue;
            }
            let mut todo = vec![r.blocks[0].id];
            let mut seen = BTreeSet::new();
            let mut safe = true;
            while let Some(id) = todo.pop() {
                if !seen.insert(id) {
                    continue;
                }
                let Some(block) = r.blocks.iter().find(|b| b.id == id) else {
                    safe = false;
                    break;
                };
                let mut sink = false;
                for op in &block.ops {
                    if crate::nir::optimizer::binary_may_fault(op) {
                        safe = false;
                        break;
                    }
                    match op {
                        NirOp::Call {
                            callee: NirCallee::Fault(_),
                            ..
                        } => {
                            sink = true;
                            break;
                        }
                        NirOp::Call {
                            callee: NirCallee::User { id, .. },
                            ..
                        } if previous.contains(id) => {
                            sink = true;
                            break;
                        }
                        NirOp::ForeignCode { .. }
                        | NirOp::Unsupported { .. }
                        | NirOp::VolatileStore { .. }
                        | NirOp::CopyBytes { .. }
                        | NirOp::Real(_)
                        | NirOp::Call { .. } => {
                            safe = false;
                            break;
                        }
                        NirOp::Store { place, .. } if !ordinary_store(program, r, place) => {
                            // An absolute/indirect write can change executed
                            // code or control storage. It cannot certify a sink.
                            safe = false;
                            break;
                        }
                        _ => {}
                    }
                }
                if !safe {
                    break;
                }
                if sink {
                    continue;
                }
                match &block.terminator {
                    NirTerminator::Goto(e) => todo.push(e.target),
                    NirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => todo.extend([then_edge.target, else_edge.target]),
                    _ => {
                        safe = false;
                        break;
                    }
                }
            }
            if safe {
                proved.insert(r.id);
            }
        }
        if proved == previous {
            break;
        }
    }
    proved
}

fn ordinary_store(program: &NirProgram, routine: &NirRoutine, place: &NirPlace) -> bool {
    match place.kind {
        NirPlaceKind::Global { id, .. } => program
            .globals
            .iter()
            .any(|g| g.id == id && g.backing == NirGlobalBacking::Ordinary),
        NirPlaceKind::Local { id, .. } => routine
            .locals
            .iter()
            .any(|l| l.id == id && l.backing == NirLocalBacking::Ordinary),
        NirPlaceKind::Param { id, .. } => routine.params.iter().any(|p| p.id == id),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
