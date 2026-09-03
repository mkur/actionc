use std::collections::{BTreeMap, BTreeSet};

use super::analysis::{
    cfg::NirCfg,
    dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow},
    dominance::NirDominance,
    liveness::NirTempLiveness,
    predicates::{NirPredicateAnalysis, predicate_threading_candidates},
    storage::{NirRoutineStorageAnalysis, analyze_program_storage},
    use_def::NirUseDef,
};
use super::facts::{BlockId, NirType, NirTypeKind, NirValue, TempId, value_width};
use super::ir::*;
use super::verifier::{NirDiagnostic, verify_program};
use crate::target::ByteSize;

pub(super) fn optimize_program(program: &NirProgram) -> Result<NirProgram, Vec<NirDiagnostic>> {
    verify_program(program)?;
    let mut optimized = program.clone();
    let storage = analyze_program_storage(&optimized);
    for (routine, storage) in optimized.routines.iter_mut().zip(&storage.routines) {
        optimize_routine(routine, storage);
    }
    verify_program(&optimized)?;
    Ok(optimized)
}

fn optimize_routine(routine: &mut NirRoutine, storage: &NirRoutineStorageAnalysis) {
    remove_unreachable_blocks(routine);
    loop {
        let before = routine.blocks.clone();
        fold_uniform_block_parameters(routine);
        optimize_values_in_routine(routine);
        simplify_constant_branches(routine);
        thread_predicate_branches(routine, storage);
        eliminate_dominated_pure_redundancy(routine);
        eliminate_dead_pure_temps(routine);
        routine.temps = collect_temps(&routine.blocks);
        if routine.blocks == before {
            break;
        }
    }
}

fn thread_predicate_branches(
    routine: &mut NirRoutine,
    storage: &NirRoutineStorageAnalysis,
) -> bool {
    let cfg = NirCfg::from_routine(routine);
    let analysis = NirPredicateAnalysis::analyze(routine, storage);
    let candidates = predicate_threading_candidates(routine, &analysis, storage);
    let blocks = routine
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect::<BTreeMap<_, _>>();
    let mut rewrites = Vec::<(BlockId, BlockId, NirEdge)>::new();

    for candidate in candidates {
        let Some(predicate) = analysis.branch_predicate(candidate) else {
            continue;
        };
        let Some(NirBlock {
            terminator:
                NirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                },
            ..
        }) = blocks.get(&candidate).copied()
        else {
            continue;
        };
        for predecessor in cfg.predecessors(candidate) {
            let Some(decision) = analysis.edge_proves(*predecessor, candidate, predicate) else {
                continue;
            };
            rewrites.push((
                *predecessor,
                candidate,
                if decision {
                    then_edge.clone()
                } else {
                    else_edge.clone()
                },
            ));
        }
    }
    if rewrites.is_empty() {
        return false;
    }

    let mut changed = false;
    for block in &mut routine.blocks {
        for (predecessor, candidate, replacement) in &rewrites {
            if block.id != *predecessor {
                continue;
            }
            for edge in terminator_edges_mut(&mut block.terminator) {
                if edge.target == *candidate && edge.args.is_empty() {
                    *edge = replacement.clone();
                    changed = true;
                }
            }
        }
    }
    if changed {
        remove_unreachable_blocks(routine);
    }
    changed
}

fn fold_uniform_block_parameters(routine: &mut NirRoutine) {
    let cfg = NirCfg::from_routine(routine);
    let dominance = NirDominance::from_cfg(&cfg);
    let mut replacements = BTreeMap::<TempId, NirValue>::new();
    let mut removed = BTreeMap::<BlockId, Vec<usize>>::new();

    for block in &routine.blocks {
        for (index, param) in block.params.iter().enumerate() {
            let incoming = routine
                .blocks
                .iter()
                .flat_map(|predecessor| terminator_edges(&predecessor.terminator))
                .filter(|edge| edge.target == block.id)
                .filter_map(|edge| edge.args.get(index))
                .collect::<Vec<_>>();
            let Some(value) = incoming.first().cloned().cloned() else {
                continue;
            };
            if !incoming.iter().all(|candidate| **candidate == value)
                || !uniform_value_dominates(&value, block.id, routine, &dominance)
            {
                continue;
            }
            replacements.insert(param.dest, value);
            removed.entry(block.id).or_default().push(index);
        }
    }

    if replacements.is_empty() {
        return;
    }
    for block in &mut routine.blocks {
        if let Some(indices) = removed.get(&block.id) {
            for index in indices.iter().rev() {
                block.params.remove(*index);
            }
        }
        for edge in terminator_edges_mut(&mut block.terminator) {
            if let Some(indices) = removed.get(&edge.target) {
                for index in indices.iter().rev() {
                    edge.args.remove(*index);
                }
            }
        }
        for op in &mut block.ops {
            rewrite_op_values(op, &replacements);
        }
        rewrite_terminator_values(&mut block.terminator, &replacements);
    }
}

fn uniform_value_dominates(
    value: &NirValue,
    target: BlockId,
    routine: &NirRoutine,
    dominance: &NirDominance,
) -> bool {
    match value {
        NirValue::IntegerConst { .. }
        | NirValue::Null { .. }
        | NirValue::AddressConst { .. }
        | NirValue::StaticAddr { .. }
        | NirValue::RoutineAddr { .. } => true,
        NirValue::Temp { id, .. } => routine
            .temps
            .iter()
            .find(|temp| temp.id == *id)
            .is_some_and(|temp| {
                temp.def.op_index.is_some() && dominance.dominates(temp.def.block, target)
            }),
        NirValue::Param(_) | NirValue::GlobalAddr(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PureExpression {
    AddrOf {
        ty: NirType,
        place: NirPlace,
    },
    Unary {
        ty: NirType,
        op: NirUnaryOp,
        src: NirValue,
    },
    Cast {
        from: NirType,
        to: NirType,
        src: NirValue,
    },
    Binary {
        ty: NirType,
        op: NirBinaryOp,
        left: NirValue,
        right: NirValue,
    },
    Compare {
        ty: NirType,
        operand_ty: NirType,
        op: NirCompareOp,
        left: NirValue,
        right: NirValue,
    },
}

fn eliminate_dominated_pure_redundancy(routine: &mut NirRoutine) {
    let cfg = NirCfg::from_routine(routine);
    let dominance = NirDominance::from_cfg(&cfg);
    let use_def = NirUseDef::from_routine(routine);
    let Some(entry) = dominance.root() else {
        return;
    };
    gvn_block(
        routine,
        entry,
        Vec::new(),
        BTreeMap::new(),
        &dominance,
        &use_def,
    );
}

fn gvn_block(
    routine: &mut NirRoutine,
    block_id: BlockId,
    mut available: Vec<(PureExpression, NirValue)>,
    mut replacements: BTreeMap<TempId, NirValue>,
    dominance: &NirDominance,
    use_def: &NirUseDef,
) {
    let Some(index) = routine.blocks.iter().position(|block| block.id == block_id) else {
        return;
    };
    let ops = std::mem::take(&mut routine.blocks[index].ops);
    let mut retained = Vec::with_capacity(ops.len());
    for mut op in ops {
        rewrite_op_values(&mut op, &replacements);
        let Some((dest, ty)) = op_def(&op) else {
            retained.push(op);
            continue;
        };
        let Some(expression) = pure_expression(&op) else {
            replacements.remove(&dest);
            retained.push(op);
            continue;
        };
        if let Some((_, value)) = available
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == &expression)
            && reuse_does_not_extend_live_range(value, dest, block_id, use_def)
        {
            replacements.insert(dest, value.clone());
            continue;
        }
        let value = NirValue::Temp {
            id: dest,
            ty: ty.clone(),
        };
        available.push((expression, value));
        replacements.remove(&dest);
        retained.push(op);
    }
    rewrite_terminator_values(&mut routine.blocks[index].terminator, &replacements);
    routine.blocks[index].ops = retained;

    for child in dominance.children(block_id) {
        gvn_block(
            routine,
            *child,
            available.clone(),
            replacements.clone(),
            dominance,
            use_def,
        );
    }
}

fn reuse_does_not_extend_live_range(
    canonical: &NirValue,
    duplicate: TempId,
    block: BlockId,
    use_def: &NirUseDef,
) -> bool {
    // MIR6502 can rematerialize many pure values more cheaply than preserving a
    // long-lived temp. Reuse only when the canonical temp already lives at
    // least as long as the redundant result, so GVN cannot create spill
    // pressure merely to remove an operation.
    let NirValue::Temp { id: canonical, .. } = canonical else {
        return false;
    };
    let last_use = |temp| {
        let uses = use_def.uses(temp);
        if uses.is_empty() || uses.iter().any(|site| site.block() != block) {
            return None;
        }
        uses.iter()
            .map(|site| site.op_index().unwrap_or(usize::MAX))
            .max()
    };
    let Some(canonical_last_use) = last_use(*canonical) else {
        return false;
    };
    let Some(duplicate_last_use) = last_use(duplicate) else {
        return false;
    };
    canonical_last_use >= duplicate_last_use
}

fn pure_expression(op: &NirOp) -> Option<PureExpression> {
    match op {
        NirOp::AddrOf { ty, place, .. } => Some(PureExpression::AddrOf {
            ty: ty.clone(),
            place: place.clone(),
        }),
        NirOp::Unary { ty, op, src, .. } => Some(PureExpression::Unary {
            ty: ty.clone(),
            op: *op,
            src: src.clone(),
        }),
        NirOp::Cast { src, from, to, .. } => Some(PureExpression::Cast {
            from: from.clone(),
            to: to.clone(),
            src: src.clone(),
        }),
        NirOp::Binary {
            ty,
            op,
            left,
            right,
            ..
        } => Some(PureExpression::Binary {
            ty: ty.clone(),
            op: *op,
            left: left.clone(),
            right: right.clone(),
        }),
        NirOp::Compare {
            ty,
            operand_ty,
            op,
            left,
            right,
            ..
        } => Some(PureExpression::Compare {
            ty: ty.clone(),
            operand_ty: operand_ty.clone(),
            op: *op,
            left: left.clone(),
            right: right.clone(),
        }),
        _ => None,
    }
}

fn remove_unreachable_blocks(routine: &mut NirRoutine) {
    let cfg = NirCfg::from_routine(routine);
    let reachable = cfg.reachable().clone();
    routine.blocks.retain(|block| reachable.contains(&block.id));
}

fn optimize_values_in_routine(routine: &mut NirRoutine) {
    let cfg = NirCfg::from_routine(routine);
    let result = solve_dataflow(&cfg, &NirValuePropagationProblem::new(routine, &cfg));

    for block in &mut routine.blocks {
        let mut facts = result
            .in_state(block.id)
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_default();
        let mut optimized = Vec::with_capacity(block.ops.len());
        for op in block.ops.drain(..) {
            if let Some(op) = simplify_op_with_facts(op, &mut facts) {
                optimized.push(op);
            }
        }
        block.ops = optimized;
        rewrite_terminator_values(&mut block.terminator, &facts.replacements);
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct NirValueFacts {
    replacements: BTreeMap<TempId, NirValue>,
    offsets: BTreeMap<TempId, OffsetAlias>,
}

impl NirValueFacts {
    fn intersect_with(&mut self, other: &Self) {
        self.replacements
            .retain(|temp, value| other.replacements.get(temp) == Some(value));
        self.offsets
            .retain(|temp, offset| other.offsets.get(temp) == Some(offset));
    }
}

struct NirValuePropagationProblem<'a> {
    entry: Option<BlockId>,
    blocks: BTreeMap<BlockId, &'a NirBlock>,
}

impl<'a> NirValuePropagationProblem<'a> {
    fn new(routine: &'a NirRoutine, cfg: &NirCfg) -> Self {
        Self {
            entry: cfg.entry(),
            blocks: routine
                .blocks
                .iter()
                .map(|block| (block.id, block))
                .collect(),
        }
    }
}

impl NirDataflowProblem for NirValuePropagationProblem<'_> {
    type State = Option<NirValueFacts>;

    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Forward
    }

    fn bottom(&self) -> Self::State {
        None
    }

    fn boundary(&self, block: BlockId) -> Option<Self::State> {
        (Some(block) == self.entry).then(|| Some(NirValueFacts::default()))
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
        for op in &self.blocks.get(&block)?.ops {
            simplify_op_with_facts(op.clone(), &mut facts);
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
        let Some(block) = self.blocks.get(&from) else {
            return false;
        };
        match &block.terminator {
            NirTerminator::Goto(edge) => edge.target == to,
            NirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                let mut condition = condition.clone();
                rewrite_value(&mut condition, &facts.replacements);
                match condition {
                    NirValue::IntegerConst { bits: 0, .. } => else_edge.target == to,
                    NirValue::IntegerConst { .. } => then_edge.target == to,
                    NirValue::Null { .. }
                    | NirValue::AddressConst { .. }
                    | NirValue::StaticAddr { .. }
                    | NirValue::Temp { .. }
                    | NirValue::Param(_)
                    | NirValue::GlobalAddr(_)
                    | NirValue::RoutineAddr { .. } => {
                        then_edge.target == to || else_edge.target == to
                    }
                }
            }
            NirTerminator::Open
            | NirTerminator::Fallthrough
            | NirTerminator::Return(_)
            | NirTerminator::Exit => false,
        }
    }
}

fn simplify_op_with_facts(mut op: NirOp, facts: &mut NirValueFacts) -> Option<NirOp> {
    rewrite_op_values(&mut op, &facts.replacements);
    if let Some((id, value)) = folded_constant(&op) {
        facts.replacements.insert(id, value);
        facts.offsets.remove(&id);
        return None;
    }
    if let Some((id, value)) = identity_alias(&op) {
        facts.replacements.insert(id, value);
        facts.offsets.remove(&id);
        return None;
    }
    if let Some(simplification) = offset_simplification(&op, &facts.offsets) {
        match simplification {
            OffsetSimplification::Alias { dest, value } => {
                facts.replacements.insert(dest, value);
                facts.offsets.remove(&dest);
                return None;
            }
            OffsetSimplification::Keep {
                dest,
                offset,
                op: new_op,
            } => {
                facts.replacements.remove(&dest);
                facts.offsets.insert(dest, offset);
                if let Some(new_op) = new_op {
                    op = new_op;
                }
            }
        }
    } else if let Some(id) = op_def(&op).map(|(id, _)| id) {
        facts.replacements.remove(&id);
        facts.offsets.remove(&id);
    }
    Some(op)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OffsetAlias {
    base: NirValue,
    offset: u64,
    width: ByteSize,
}

enum OffsetSimplification {
    Alias {
        dest: TempId,
        value: NirValue,
    },
    Keep {
        dest: TempId,
        offset: OffsetAlias,
        op: Option<NirOp>,
    },
}

fn identity_alias(op: &NirOp) -> Option<(TempId, NirValue)> {
    let NirOp::Binary {
        dest,
        ty,
        op,
        left,
        right,
    } = op
    else {
        return None;
    };
    if !is_optimizable_integer_type(ty) {
        return None;
    }
    let alias = match op {
        NirBinaryOp::Add | NirBinaryOp::Or | NirBinaryOp::Xor if is_zero(right) => left,
        NirBinaryOp::Add | NirBinaryOp::Or | NirBinaryOp::Xor if is_zero(left) => right,
        NirBinaryOp::Sub if is_zero(right) => left,
        NirBinaryOp::And if is_all_ones(right, ty) => left,
        NirBinaryOp::And if is_all_ones(left, ty) => right,
        NirBinaryOp::Mul
        | NirBinaryOp::Div
        | NirBinaryOp::Mod
        | NirBinaryOp::Lsh
        | NirBinaryOp::Rsh
        | NirBinaryOp::Sub
        | NirBinaryOp::And
        | NirBinaryOp::Add
        | NirBinaryOp::Or
        | NirBinaryOp::Xor => return None,
    };
    alias_value_for_type(alias, ty).map(|value| (*dest, value))
}

fn offset_simplification(
    op: &NirOp,
    offsets: &BTreeMap<TempId, OffsetAlias>,
) -> Option<OffsetSimplification> {
    let NirOp::Binary {
        dest,
        ty,
        op,
        left,
        right,
    } = op
    else {
        return None;
    };
    if !is_optimizable_integer_type(ty) {
        return None;
    }
    let width = ty.width?;
    let mask = width_mask(width)?;
    let (base, offset, uses_prior_offset) = match op {
        NirBinaryOp::Add => {
            if let Some(right_const) = const_integer_bits(right) {
                let left = offset_base(left, ty, offsets)?;
                (
                    left.base,
                    left.offset.wrapping_add(right_const) & mask,
                    left.uses_prior_offset,
                )
            } else if let Some(left_const) = const_integer_bits(left) {
                let right = offset_base(right, ty, offsets)?;
                (
                    right.base,
                    right.offset.wrapping_add(left_const) & mask,
                    right.uses_prior_offset,
                )
            } else {
                return None;
            }
        }
        NirBinaryOp::Sub => {
            let right_const = const_integer_bits(right)?;
            let left = offset_base(left, ty, offsets)?;
            (
                left.base,
                left.offset.wrapping_sub(right_const) & mask,
                left.uses_prior_offset,
            )
        }
        NirBinaryOp::Mul
        | NirBinaryOp::Div
        | NirBinaryOp::Mod
        | NirBinaryOp::Lsh
        | NirBinaryOp::Rsh
        | NirBinaryOp::And
        | NirBinaryOp::Or
        | NirBinaryOp::Xor => return None,
    };

    let base = alias_value_for_type(&base, ty)?;
    if offset == 0 {
        return Some(OffsetSimplification::Alias {
            dest: *dest,
            value: base,
        });
    }

    let offset = OffsetAlias {
        base: base.clone(),
        offset,
        width,
    };
    let op = if uses_prior_offset {
        Some(NirOp::Binary {
            dest: *dest,
            ty: ty.clone(),
            op: NirBinaryOp::Add,
            left: base,
            right: value_for_type(offset.offset, ty)?,
        })
    } else {
        None
    };
    Some(OffsetSimplification::Keep {
        dest: *dest,
        offset,
        op,
    })
}

struct OffsetBase {
    base: NirValue,
    offset: u64,
    uses_prior_offset: bool,
}

fn offset_base(
    value: &NirValue,
    ty: &NirType,
    offsets: &BTreeMap<TempId, OffsetAlias>,
) -> Option<OffsetBase> {
    if let NirValue::Temp { id, .. } = value
        && let Some(offset) = offsets.get(id)
        && offset.width == ty.width?
    {
        return Some(OffsetBase {
            base: offset.base.clone(),
            offset: offset.offset,
            uses_prior_offset: true,
        });
    }
    Some(OffsetBase {
        base: alias_value_for_type(value, ty)?,
        offset: 0,
        uses_prior_offset: false,
    })
}

fn alias_value_for_type(value: &NirValue, ty: &NirType) -> Option<NirValue> {
    if is_optimizable_integer_type(ty) && value_width(value) == ty.width {
        Some(value.clone())
    } else {
        None
    }
}

fn is_optimizable_integer_type(ty: &NirType) -> bool {
    ty.kind.integer().is_some() && ty.width.is_some_and(|width| width.get() <= 8) && !ty.pointer
}

fn is_zero(value: &NirValue) -> bool {
    matches!(const_integer_bits(value), Some(0))
}

fn is_all_ones(value: &NirValue, ty: &NirType) -> bool {
    let Some(mask) = ty.width.and_then(width_mask) else {
        return false;
    };
    matches!(const_integer_bits(value), Some(value) if value == mask)
}

fn width_mask(width: ByteSize) -> Option<u64> {
    match width.get() {
        0 => None,
        1..=7 => Some((1u64 << (width.get() * 8)) - 1),
        8 => Some(u64::MAX),
        _ => None,
    }
}

fn simplify_constant_branches(routine: &mut NirRoutine) {
    for block in &mut routine.blocks {
        if let NirTerminator::Branch {
            condition: NirValue::IntegerConst { bits: value, .. },
            then_edge,
            else_edge,
        } = &block.terminator
        {
            block.terminator = if *value == 0 {
                NirTerminator::Goto(else_edge.clone())
            } else {
                NirTerminator::Goto(then_edge.clone())
            };
        }
    }
    remove_unreachable_blocks(routine);
}

fn eliminate_dead_pure_temps(routine: &mut NirRoutine) {
    loop {
        let cfg = NirCfg::from_routine(routine);
        let use_def = NirUseDef::from_routine(routine);
        let liveness = NirTempLiveness::analyze(routine, &cfg, &use_def);
        let mut changed = false;

        for block in &mut routine.blocks {
            let mut live = liveness.live_out(block.id).clone();
            collect_terminator_uses(&block.terminator, &mut live);
            let mut kept = Vec::with_capacity(block.ops.len());

            for op in block.ops.drain(..).rev() {
                if let Some((dest, _)) = op_def(&op)
                    && is_pure_temp_op(&op)
                    && !live.contains(&dest)
                {
                    changed = true;
                    continue;
                }
                if let Some((dest, _)) = op_def(&op) {
                    live.remove(&dest);
                }
                collect_op_uses(&op, &mut live);
                kept.push(op);
            }
            kept.reverse();
            block.ops = kept;
        }

        if !changed {
            break;
        }
    }
}

fn folded_constant(op: &NirOp) -> Option<(TempId, NirValue)> {
    match op {
        NirOp::Unary { dest, ty, op, src } => {
            let value = const_integer_bits(src)?;
            let result = match op {
                NirUnaryOp::Plus => value,
                NirUnaryOp::Neg => value.wrapping_neg(),
            };
            Some((*dest, value_for_type(result, ty)?))
        }
        NirOp::Cast { dest, src, to, .. } => {
            Some((*dest, value_for_type(const_integer_bits(src)?, to)?))
        }
        NirOp::Binary {
            dest,
            ty,
            op,
            left,
            right,
        } => Some((
            *dest,
            value_for_type(eval_binary(*op, left, right, ty)?, ty)?,
        )),
        NirOp::PointerOffset { .. } => None,
        NirOp::Compare {
            dest,
            ty,
            operand_ty,
            op,
            left,
            right,
        } => {
            if !matches!(ty.kind, NirTypeKind::Bool) {
                return None;
            }
            let result = eval_compare(*op, left, right, operand_ty)?;
            Some((*dest, NirValue::ConstU8(u8::from(result))))
        }
        NirOp::Real(_)
        | NirOp::Load { .. }
        | NirOp::VolatileLoad { .. }
        | NirOp::AddrOf { .. }
        | NirOp::Store { .. }
        | NirOp::VolatileStore { .. }
        | NirOp::CopyBytes { .. }
        | NirOp::Call { .. }
        | NirOp::ForeignCode { .. }
        | NirOp::Unsupported { .. } => None,
    }
}

fn eval_compare(
    op: NirCompareOp,
    left: &NirValue,
    right: &NirValue,
    operand_ty: &NirType,
) -> Option<bool> {
    if operand_ty.kind.is_address() {
        let left = const_address(left)?;
        let right = const_address(right)?;
        return (left.address_space == right.address_space)
            .then(|| apply_compare(op, left.value, right.value));
    }
    let left = const_integer_bits(left)?;
    let right = const_integer_bits(right)?;
    let integer = operand_ty.kind.integer()?;
    Some(if integer.signed {
        apply_compare(
            op,
            signed_integer_value(left, integer.bits),
            signed_integer_value(right, integer.bits),
        )
    } else {
        apply_compare(op, left, right)
    })
}

fn const_address(value: &NirValue) -> Option<crate::target::AddressValue> {
    match value {
        NirValue::Null { ty } => Some(crate::target::AddressValue::new(
            address_space(&ty.kind)?,
            0,
        )),
        NirValue::AddressConst { address, .. } => Some(*address),
        NirValue::IntegerConst { .. }
        | NirValue::StaticAddr { .. }
        | NirValue::Temp { .. }
        | NirValue::Param(_)
        | NirValue::GlobalAddr(_)
        | NirValue::RoutineAddr { .. } => None,
    }
}

fn address_space(kind: &NirTypeKind) -> Option<crate::target::AddressSpaceId> {
    match kind {
        NirTypeKind::Pointer { address_space, .. }
        | NirTypeKind::Callable { address_space, .. } => Some(*address_space),
        _ => None,
    }
}

fn apply_compare<T: Ord>(op: NirCompareOp, left: T, right: T) -> bool {
    match op {
        NirCompareOp::Eq => left == right,
        NirCompareOp::Ne => left != right,
        NirCompareOp::Lt => left < right,
        NirCompareOp::Le => left <= right,
        NirCompareOp::Gt => left > right,
        NirCompareOp::Ge => left >= right,
    }
}

fn eval_binary(op: NirBinaryOp, left: &NirValue, right: &NirValue, ty: &NirType) -> Option<u64> {
    let left = const_integer_bits(left)?;
    let right = const_integer_bits(right)?;
    let mask = ty.width.and_then(width_mask)?;
    let width_bits = ty.width?.get() * 8;
    match op {
        NirBinaryOp::Add => Some(left.wrapping_add(right) & mask),
        NirBinaryOp::Sub => Some(left.wrapping_sub(right) & mask),
        NirBinaryOp::Mul => Some(left.wrapping_mul(right) & mask),
        NirBinaryOp::Div if right != 0 => Some(left / right),
        NirBinaryOp::Mod if right != 0 => Some(left % right),
        NirBinaryOp::Lsh if right < u64::from(width_bits) => {
            Some(left.wrapping_shl(right as u32) & mask)
        }
        NirBinaryOp::Rsh if right < u64::from(width_bits) => Some(left.wrapping_shr(right as u32)),
        NirBinaryOp::And => Some(left & right),
        NirBinaryOp::Or => Some(left | right),
        NirBinaryOp::Xor => Some(left ^ right),
        NirBinaryOp::Div | NirBinaryOp::Mod | NirBinaryOp::Lsh | NirBinaryOp::Rsh => None,
    }
}

fn value_for_type(value: u64, ty: &NirType) -> Option<NirValue> {
    Some(NirValue::integer_const(value, ty.kind.integer()?))
}

fn const_integer_bits(value: &NirValue) -> Option<u64> {
    match value {
        NirValue::IntegerConst { bits, .. } => Some(*bits),
        NirValue::StaticAddr { .. }
        | NirValue::Null { .. }
        | NirValue::AddressConst { .. }
        | NirValue::Temp { .. }
        | NirValue::Param(_)
        | NirValue::GlobalAddr(_)
        | NirValue::RoutineAddr { .. } => None,
    }
}

fn signed_integer_value(bits: u64, width: u8) -> i64 {
    if width == 0 {
        return 0;
    }
    let shift = 64u32.saturating_sub(u32::from(width));
    ((bits << shift) as i64) >> shift
}

fn rewrite_op_values(op: &mut NirOp, constants: &BTreeMap<TempId, NirValue>) {
    match op {
        NirOp::Store { place, src, .. } | NirOp::VolatileStore { place, src, .. } => {
            rewrite_place_values(place, constants);
            rewrite_value(src, constants);
        }
        NirOp::Load { place, .. }
        | NirOp::VolatileLoad { place, .. }
        | NirOp::AddrOf { place, .. } => {
            rewrite_place_values(place, constants);
        }
        NirOp::CopyBytes {
            destination,
            source,
            ..
        } => {
            rewrite_place_values(destination, constants);
            rewrite_place_values(source, constants);
        }
        NirOp::Unary { src, .. } | NirOp::Cast { src, .. } => rewrite_value(src, constants),
        NirOp::Binary { left, right, .. } | NirOp::Compare { left, right, .. } => {
            rewrite_value(left, constants);
            rewrite_value(right, constants);
        }
        NirOp::PointerOffset { base, offset, .. } => {
            rewrite_value(base, constants);
            rewrite_value(offset, constants);
        }
        NirOp::Real(real) => rewrite_real_op_values(real, constants),
        NirOp::Call { callee, args, .. } => {
            if let NirCallee::Indirect { target, .. } = callee {
                rewrite_value(target, constants);
            }
            for arg in args {
                rewrite_value(arg, constants);
            }
        }
        NirOp::ForeignCode { .. } | NirOp::Unsupported { .. } => {}
    }
}

fn rewrite_real_op_values(op: &mut NirRealOp, constants: &BTreeMap<TempId, NirValue>) {
    match op {
        NirRealOp::Copy {
            destination,
            source,
        } => {
            rewrite_place_values(destination, constants);
            if let NirRealSource::Place(source) = source {
                rewrite_place_values(source, constants);
            }
        }
        NirRealOp::Unary {
            destination,
            operand,
            ..
        } => {
            rewrite_place_values(destination, constants);
            rewrite_real_source_values(operand, constants);
        }
        NirRealOp::Binary {
            destination,
            left,
            right,
            ..
        } => {
            rewrite_place_values(destination, constants);
            rewrite_real_source_values(left, constants);
            rewrite_real_source_values(right, constants);
        }
        NirRealOp::Compare { left, right, .. } => {
            rewrite_real_source_values(left, constants);
            rewrite_real_source_values(right, constants);
        }
        NirRealOp::IntegerToReal {
            destination,
            source,
            ..
        } => {
            rewrite_place_values(destination, constants);
            rewrite_value(source, constants);
        }
        NirRealOp::RealToInteger { source, .. } => {
            rewrite_place_values(source, constants);
        }
    }
}

fn rewrite_terminator_values(
    terminator: &mut NirTerminator,
    constants: &BTreeMap<TempId, NirValue>,
) {
    match terminator {
        NirTerminator::Goto(edge) => {
            for arg in &mut edge.args {
                rewrite_value(arg, constants);
            }
        }
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            rewrite_value(condition, constants);
            for arg in then_edge.args.iter_mut().chain(&mut else_edge.args) {
                rewrite_value(arg, constants);
            }
        }
        NirTerminator::Return(Some(condition)) => {
            rewrite_value(condition, constants);
        }
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Return(None)
        | NirTerminator::Exit => {}
    }
}

fn rewrite_place_values(place: &mut NirPlace, constants: &BTreeMap<TempId, NirValue>) {
    match &mut place.kind {
        NirPlaceKind::Deref { addr } => rewrite_value(addr, constants),
        NirPlaceKind::Index {
            base_addr, index, ..
        } => {
            rewrite_value(base_addr, constants);
            rewrite_value(index, constants);
        }
        NirPlaceKind::Field { base, .. } => rewrite_place_values(base, constants),
        NirPlaceKind::Param { .. }
        | NirPlaceKind::Local { .. }
        | NirPlaceKind::Global { .. }
        | NirPlaceKind::Absolute(_) => {}
    }
}

fn rewrite_real_source_values(source: &mut NirRealSource, constants: &BTreeMap<TempId, NirValue>) {
    if let NirRealSource::Place(place) = source {
        rewrite_place_values(place, constants);
    }
}

fn rewrite_value(value: &mut NirValue, constants: &BTreeMap<TempId, NirValue>) {
    let mut visited = BTreeSet::new();
    while let NirValue::Temp { id, .. } = value {
        if !visited.insert(*id) {
            break;
        }
        let Some(replacement) = constants.get(id) else {
            break;
        };
        if value_width(replacement) != value_width(value) {
            break;
        }
        *value = replacement.clone();
    }
}

fn terminator_edges(terminator: &NirTerminator) -> impl Iterator<Item = &NirEdge> {
    let edges = match terminator {
        NirTerminator::Goto(edge) => [Some(edge), None],
        NirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => [Some(then_edge), Some(else_edge)],
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Return(_)
        | NirTerminator::Exit => [None, None],
    };
    edges.into_iter().flatten()
}

fn terminator_edges_mut(terminator: &mut NirTerminator) -> impl Iterator<Item = &mut NirEdge> {
    let edges = match terminator {
        NirTerminator::Goto(edge) => [Some(edge), None],
        NirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => [Some(then_edge), Some(else_edge)],
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Return(_)
        | NirTerminator::Exit => [None, None],
    };
    edges.into_iter().flatten()
}

fn collect_op_uses(op: &NirOp, out: &mut BTreeSet<TempId>) {
    match op {
        NirOp::Store { place, src, .. } | NirOp::VolatileStore { place, src, .. } => {
            collect_place_uses(place, out);
            collect_value_use(src, out);
        }
        NirOp::Load { place, .. }
        | NirOp::VolatileLoad { place, .. }
        | NirOp::AddrOf { place, .. } => collect_place_uses(place, out),
        NirOp::CopyBytes {
            destination,
            source,
            ..
        } => {
            collect_place_uses(destination, out);
            collect_place_uses(source, out);
        }
        NirOp::Unary { src, .. } | NirOp::Cast { src, .. } => collect_value_use(src, out),
        NirOp::Binary { left, right, .. } | NirOp::Compare { left, right, .. } => {
            collect_value_use(left, out);
            collect_value_use(right, out);
        }
        NirOp::PointerOffset { base, offset, .. } => {
            collect_value_use(base, out);
            collect_value_use(offset, out);
        }
        NirOp::Real(real) => collect_real_op_uses(real, out),
        NirOp::Call { callee, args, .. } => {
            if let NirCallee::Indirect { target, .. } = callee {
                collect_value_use(target, out);
            }
            for arg in args {
                collect_value_use(arg, out);
            }
        }
        NirOp::ForeignCode { .. } | NirOp::Unsupported { .. } => {}
    }
}

fn collect_real_op_uses(op: &NirRealOp, out: &mut BTreeSet<TempId>) {
    match op {
        NirRealOp::Copy {
            destination,
            source,
        } => {
            collect_place_uses(destination, out);
            if let NirRealSource::Place(source) = source {
                collect_place_uses(source, out);
            }
        }
        NirRealOp::Unary {
            destination,
            operand,
            ..
        } => {
            collect_place_uses(destination, out);
            collect_real_source_uses(operand, out);
        }
        NirRealOp::Binary {
            destination,
            left,
            right,
            ..
        } => {
            collect_place_uses(destination, out);
            collect_real_source_uses(left, out);
            collect_real_source_uses(right, out);
        }
        NirRealOp::Compare { left, right, .. } => {
            collect_real_source_uses(left, out);
            collect_real_source_uses(right, out);
        }
        NirRealOp::IntegerToReal {
            destination,
            source,
            ..
        } => {
            collect_place_uses(destination, out);
            collect_value_use(source, out);
        }
        NirRealOp::RealToInteger { source, .. } => collect_place_uses(source, out),
    }
}

fn collect_terminator_uses(terminator: &NirTerminator, out: &mut BTreeSet<TempId>) {
    match terminator {
        NirTerminator::Goto(edge) => {
            for arg in &edge.args {
                collect_value_use(arg, out);
            }
        }
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            collect_value_use(condition, out);
            for arg in then_edge.args.iter().chain(&else_edge.args) {
                collect_value_use(arg, out);
            }
        }
        NirTerminator::Return(Some(condition)) => {
            collect_value_use(condition, out);
        }
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Return(None)
        | NirTerminator::Exit => {}
    }
}

fn collect_place_uses(place: &NirPlace, out: &mut BTreeSet<TempId>) {
    match &place.kind {
        NirPlaceKind::Deref { addr } => collect_value_use(addr, out),
        NirPlaceKind::Index {
            base_addr, index, ..
        } => {
            collect_value_use(base_addr, out);
            collect_value_use(index, out);
        }
        NirPlaceKind::Field { base, .. } => collect_place_uses(base, out),
        NirPlaceKind::Param { .. }
        | NirPlaceKind::Local { .. }
        | NirPlaceKind::Global { .. }
        | NirPlaceKind::Absolute(_) => {}
    }
}

fn collect_real_source_uses(source: &NirRealSource, out: &mut BTreeSet<TempId>) {
    if let NirRealSource::Place(place) = source {
        collect_place_uses(place, out);
    }
}

fn collect_value_use(value: &NirValue, out: &mut BTreeSet<TempId>) {
    if let NirValue::Temp { id, .. } = value {
        out.insert(*id);
    }
}

fn is_pure_temp_op(op: &NirOp) -> bool {
    matches!(
        op,
        NirOp::Unary { .. }
            | NirOp::Cast { .. }
            | NirOp::PointerOffset { .. }
            | NirOp::Binary { .. }
            | NirOp::Compare { .. }
    )
}

fn op_def(op: &NirOp) -> Option<(TempId, &NirType)> {
    match op {
        NirOp::Load { dest, ty, .. }
        | NirOp::VolatileLoad { dest, ty, .. }
        | NirOp::AddrOf { dest, ty, .. }
        | NirOp::Unary { dest, ty, .. }
        | NirOp::PointerOffset { dest, ty, .. }
        | NirOp::Binary { dest, ty, .. }
        | NirOp::Compare { dest, ty, .. } => Some((*dest, ty)),
        NirOp::Real(NirRealOp::Compare {
            result,
            result_type,
            ..
        })
        | NirOp::Real(NirRealOp::RealToInteger {
            result,
            result_type,
            ..
        }) => Some((*result, result_type)),
        NirOp::Cast { dest, to, .. } => Some((*dest, to)),
        NirOp::Call {
            result: Some(result),
            ..
        } => Some((result.dest, &result.ty)),
        NirOp::Store { .. }
        | NirOp::VolatileStore { .. }
        | NirOp::CopyBytes { .. }
        | NirOp::Real(_)
        | NirOp::Call { result: None, .. }
        | NirOp::ForeignCode { .. }
        | NirOp::Unsupported { .. } => None,
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
            if let Some((id, ty)) = op_def(op) {
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
mod value_fact_tests {
    use super::*;
    use crate::nir::LocalId;

    fn condition_type() -> NirType {
        NirType {
            kind: NirTypeKind::Bool,
            summary: "condition".to_string(),
            width: Some(crate::target::ByteSize::ONE),
            pointer: false,
        }
    }

    #[test]
    fn join_keeps_only_facts_available_with_the_same_value_on_every_path() {
        let mut left = NirValueFacts {
            replacements: BTreeMap::from([
                (TempId(0), NirValue::ConstU8(1)),
                (TempId(1), NirValue::ConstU8(2)),
            ]),
            offsets: BTreeMap::from([(
                TempId(2),
                OffsetAlias {
                    base: NirValue::ConstU8(3),
                    offset: 4,
                    width: ByteSize::ONE,
                },
            )]),
        };
        let right = NirValueFacts {
            replacements: BTreeMap::from([
                (TempId(0), NirValue::ConstU8(1)),
                (TempId(1), NirValue::ConstU8(9)),
            ]),
            offsets: BTreeMap::new(),
        };

        left.intersect_with(&right);

        assert_eq!(
            left.replacements,
            BTreeMap::from([(TempId(0), NirValue::ConstU8(1))])
        );
        assert!(left.offsets.is_empty());
    }

    #[test]
    fn folded_branch_condition_marks_only_the_selected_edge_executable() {
        let condition = condition_type();
        let routine = NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![NirTemp {
                id: TempId(0),
                ty: condition.clone(),
                def: NirTempDef {
                    block: BlockId(0),
                    op_index: Some(0),
                },
            }],
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Compare {
                        dest: TempId(0),
                        ty: condition.clone(),
                        operand_ty: NirType {
                            kind: NirTypeKind::U8,
                            summary: "Byte".to_string(),
                            width: Some(crate::target::ByteSize::ONE),
                            pointer: false,
                        },
                        op: NirCompareOp::Eq,
                        left: NirValue::ConstU8(1),
                        right: NirValue::ConstU8(1),
                    }],
                    terminator: NirTerminator::Branch {
                        condition: NirValue::Temp {
                            id: TempId(0),
                            ty: condition,
                        },
                        then_edge: NirEdge {
                            target: BlockId(1),
                            args: Vec::new(),
                        },
                        else_edge: NirEdge {
                            target: BlockId(2),
                            args: Vec::new(),
                        },
                    },
                },
                NirBlock {
                    id: BlockId(1),
                    label: "taken".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
                NirBlock {
                    id: BlockId(2),
                    label: "dead".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                },
            ],
        };
        let cfg = NirCfg::from_routine(&routine);
        let result = solve_dataflow(&cfg, &NirValuePropagationProblem::new(&routine, &cfg));

        assert!(matches!(result.in_state(BlockId(1)), Some(Some(_))));
        assert_eq!(result.in_state(BlockId(2)), Some(&None));
    }

    #[test]
    fn uniform_incoming_block_arguments_fold_to_the_common_value() {
        let byte = NirType {
            kind: NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(crate::target::ByteSize::ONE),
            pointer: false,
        };
        let mut routine = NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: vec![NirTemp {
                id: TempId(0),
                ty: byte.clone(),
                def: NirTempDef {
                    block: BlockId(3),
                    op_index: None,
                },
            }],
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Branch {
                        condition: NirValue::ConstU8(1),
                        then_edge: NirEdge {
                            target: BlockId(1),
                            args: Vec::new(),
                        },
                        else_edge: NirEdge {
                            target: BlockId(2),
                            args: Vec::new(),
                        },
                    },
                },
                NirBlock {
                    id: BlockId(1),
                    label: "left".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Goto(NirEdge {
                        target: BlockId(3),
                        args: vec![NirValue::ConstU8(7)],
                    }),
                },
                NirBlock {
                    id: BlockId(2),
                    label: "right".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Goto(NirEdge {
                        target: BlockId(3),
                        args: vec![NirValue::ConstU8(7)],
                    }),
                },
                NirBlock {
                    id: BlockId(3),
                    label: "join".to_string(),
                    params: vec![NirBlockParam {
                        dest: TempId(0),
                        ty: byte.clone(),
                    }],
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(Some(NirValue::Temp {
                        id: TempId(0),
                        ty: byte,
                    })),
                },
            ],
        };

        fold_uniform_block_parameters(&mut routine);

        assert!(routine.blocks[3].params.is_empty());
        assert!(matches!(
            routine.blocks[3].terminator,
            NirTerminator::Return(Some(NirValue::IntegerConst { bits: 7, .. }))
        ));
        assert!(matches!(
            &routine.blocks[1].terminator,
            NirTerminator::Goto(NirEdge { args, .. }) if args.is_empty()
        ));
    }

    #[test]
    fn gvn_reuses_a_dominating_address_value() {
        let pointer = NirType {
            kind: NirTypeKind::Pointer {
                pointee: Some(Box::new(NirTypeKind::U8)),
                address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
            },
            summary: "Byte*".to_string(),
            width: Some(crate::target::ByteSize::new(2)),
            pointer: true,
        };
        let place = NirPlace {
            kind: NirPlaceKind::Local {
                id: LocalId(0),
                name: "value".to_string(),
            },
            ty: None,
        };
        let mut routine = NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::AddrOf {
                        dest: TempId(0),
                        ty: pointer.clone(),
                        place: place.clone(),
                    },
                    NirOp::AddrOf {
                        dest: TempId(1),
                        ty: pointer.clone(),
                        place,
                    },
                    NirOp::Store {
                        place: NirPlace {
                            kind: NirPlaceKind::Local {
                                id: LocalId(1),
                                name: "out".to_string(),
                            },
                            ty: Some(pointer.clone()),
                        },
                        src: NirValue::Temp {
                            id: TempId(1),
                            ty: pointer.clone(),
                        },
                        ty: pointer.clone(),
                    },
                ],
                terminator: NirTerminator::Return(Some(NirValue::Temp {
                    id: TempId(0),
                    ty: pointer.clone(),
                })),
            }],
        };

        eliminate_dominated_pure_redundancy(&mut routine);

        assert_eq!(routine.blocks[0].ops.len(), 2);
        assert!(matches!(
            &routine.blocks[0].ops[1],
            NirOp::Store {
                src: NirValue::Temp { id: TempId(0), .. },
                ..
            }
        ));
        assert!(matches!(
            &routine.blocks[0].terminator,
            NirTerminator::Return(Some(NirValue::Temp { id: TempId(0), .. }))
        ));
    }
}
