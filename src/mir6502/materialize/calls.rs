use super::call_result::{
    PreparedStoreAddress, call_preserves_prepared_store_addr, call_result_store_addr_supported,
    call_result_value, materialize_call_result_to_prepared_store_addr,
    materialize_call_result_to_store_addr, prepare_call_result_store_addr,
};
use super::indexes::{
    DelayedByteIndexPlan, indexed_addr_has_delayed_index, indexed_addr_parts,
    materialize_indexed_address_for_consumer, materialize_indexed_read_to_def,
};
use super::*;
use crate::mir6502::analysis::effects::MirFlagSet;
use crate::mir6502::analysis::param_availability::MirParamHomeByte;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::MirRoutine;
use crate::mir6502::rewrite::context::{
    MirExitStateChange, MirProof, PostHomeRewriteContext, PreHomeRewriteContext,
};
use crate::mir6502::rewrite::plan::{
    MirChangeSet, MirEffectDelta, MirPostHomeRewritePlan, MirRemovedDefinition, MirRewritePlan,
};
use crate::mir6502::rewrite::posthome::structural_plan;
use std::collections::BTreeMap;

#[cfg(test)]
pub(super) fn fold_call_arg_producers(ops: Vec<MirOp>) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < ops.len() {
        if let Some(candidate) = call_arg_producer_rewrite_candidate(&ops, index)
            && candidate
                .temps
                .iter()
                .all(|temp| !temp_is_used_after(&ops, index + candidate.consumed, *temp))
        {
            out.push(candidate.replacement);
            index += candidate.consumed;
            continue;
        }
        out.push(ops[index].clone());
        index += 1;
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct CallArgProducerRewriteCandidate {
    pub consumed: usize,
    pub temps: Vec<MirTempId>,
    pub replacement: MirOp,
}

#[cfg(test)]
pub(super) fn try_materialize_call_arg_expr_producers(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    out: &mut Vec<MirOp>,
) -> CallArgExprMaterializeResult {
    let Some(candidate) = call_arg_expr_rewrite_candidate(ops, index, config, layout) else {
        return CallArgExprMaterializeResult::default();
    };
    if candidate
        .temps
        .iter()
        .any(|temp| temp_is_used_after(ops, index + candidate.consumed, *temp))
    {
        return CallArgExprMaterializeResult::default();
    }
    helpers.extend(candidate.required_helpers);
    out.extend(candidate.replacement);
    CallArgExprMaterializeResult {
        consumed: candidate.consumed,
        indexed_word_loads: candidate.indexed_word_loads,
        indexed_word_arithmetic: candidate.indexed_word_arithmetic,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct CallArgExprRewriteCandidate {
    pub consumed: usize,
    pub temps: Vec<MirTempId>,
    pub replacement: Vec<MirOp>,
    pub required_helpers: Vec<MirRuntimeHelper>,
    pub indexed_word_loads: usize,
    pub indexed_word_arithmetic: usize,
}

pub(in crate::mir6502) fn call_arg_expr_rewrite_candidate(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
) -> Option<CallArgExprRewriteCandidate> {
    let plan = collect_call_arg_expr_plan(ops, index, config, layout)?;
    let indexed_word_loads = plan
        .args
        .iter()
        .filter(|arg| {
            matches!(
                arg,
                PlannedCallArg::Expr {
                    expr: CallArgExpr::IndexedWordLoad { .. },
                    ..
                }
            )
        })
        .count();
    let indexed_word_arithmetic = plan
        .args
        .iter()
        .filter(|arg| {
            matches!(
                arg,
                PlannedCallArg::Expr { expr, .. }
                    if indexed_word_const_binary_parts(expr).is_some()
            )
        })
        .count();
    let mut replacement = Vec::new();
    let mut required_helpers = Vec::new();
    materialize_call_arg_expr_plan(&plan, layout, &mut required_helpers, &mut replacement);
    Some(CallArgExprRewriteCandidate {
        consumed: plan.consumed,
        temps: plan.temps,
        replacement,
        required_helpers,
        indexed_word_loads,
        indexed_word_arithmetic,
    })
}

#[cfg(test)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct CallArgExprMaterializeResult {
    pub(super) consumed: usize,
    pub(super) indexed_word_loads: usize,
    pub(super) indexed_word_arithmetic: usize,
}

#[derive(Debug, Clone)]
enum CallArgExpr {
    Value {
        value: MirValue,
        width: MirWidth,
    },
    IndexedWordLoad {
        addr: MirAddr,
    },
    Binary {
        op: MirBinaryOp,
        left: Box<CallArgExpr>,
        right: Box<CallArgExpr>,
        width: MirWidth,
    },
}

#[derive(Debug, Clone)]
struct CallArgExprPlan {
    consumed: usize,
    temps: Vec<MirTempId>,
    target: MirCallTarget,
    abi: MirCallAbi,
    args: Vec<PlannedCallArg>,
    result: Option<super::super::ir::MirCallResult>,
    effects: MirEffects,
}

#[derive(Debug, Clone)]
enum PlannedCallArg {
    Expr {
        expr: CallArgExpr,
        width: MirWidth,
        home: MirArgHome,
    },
    Existing(MirCallArg),
}

#[cfg(test)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReturnSlotCallArgForwardStats {
    pub(super) candidates: usize,
    pub(super) forwarded: usize,
    pub(super) blocked_home_overlap: usize,
}

#[cfg(test)]
pub(super) fn forward_return_slot_call_result_args(
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
) -> (Vec<MirOp>, ReturnSlotCallArgForwardStats) {
    let mut out = Vec::with_capacity(ops.len());
    let mut stats = ReturnSlotCallArgForwardStats::default();
    let mut index = 0usize;
    while index < ops.len() {
        let Some((first, second, blocked_home_overlap)) =
            return_slot_call_result_arg_forward_at(&ops, index, terminator)
        else {
            out.push(ops[index].clone());
            index += 1;
            continue;
        };
        stats.candidates += 1;
        if blocked_home_overlap {
            stats.blocked_home_overlap += 1;
            out.push(ops[index].clone());
            index += 1;
            continue;
        }
        stats.forwarded += 1;
        out.push(first);
        out.push(second);
        index += 2;
    }
    (out, stats)
}

#[cfg(test)]
fn return_slot_call_result_arg_forward_at(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> Option<(MirOp, MirOp, bool)> {
    let candidate = return_slot_call_arg_forward_candidate(ops, index)?;
    if temp_is_used_after(ops, index + 2, candidate.temp)
        || terminator_uses_temp(terminator, candidate.temp)
    {
        return None;
    }
    Some((
        candidate.replacement[0].clone(),
        candidate.replacement[1].clone(),
        candidate.blocked_home_overlap,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct ReturnSlotCallArgForwardCandidate {
    pub temp: MirTempId,
    pub result_width: MirWidth,
    pub return_slot: MirFixedZpSlot,
    pub blocked_home_overlap: bool,
    pub replacement: [MirOp; 2],
}

pub(in crate::mir6502) fn return_slot_call_arg_forward_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<ReturnSlotCallArgForwardCandidate> {
    let MirOp::Call {
        target: first_target,
        abi: first_abi,
        args: first_args,
        result: Some(result),
        effects: first_effects,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirResultHome::ReturnSlot { offset } = result.home else {
        return None;
    };
    let temp = split_def_as_temp(&result.dst)?;
    let MirOp::Call {
        target,
        abi,
        args,
        result: second_result,
        effects,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    if !op_uses_temp(&ops[index + 1], temp) {
        return None;
    }

    let blocked_home_overlap = args.iter().any(|arg| {
        call_arg_home_overlaps_return_slot(&arg.home, arg.width, offset, result.width)
            && !(arg.value == MirValue::Def(result.dst.clone()) && arg.width == result.width)
    });
    if blocked_home_overlap {
        return Some(ReturnSlotCallArgForwardCandidate {
            temp,
            result_width: result.width,
            return_slot: match return_slot_mem(offset) {
                MirMem::FixedZeroPage(slot) => slot,
                _ => unreachable!("return slots use fixed zero page"),
            },
            blocked_home_overlap: true,
            replacement: [ops[index].clone(), ops[index + 1].clone()],
        });
    }

    let replacement = match result.width {
        MirWidth::Byte => MirValue::PointerCell(return_slot_mem(offset)),
        MirWidth::Word => pointer_value_from_mem(&return_slot_mem(offset)),
    };
    let rewritten_target = replace_call_target_temp(target.clone(), temp, &replacement);
    let rewritten_args = args
        .iter()
        .cloned()
        .map(|mut arg| {
            arg.value = replace_temp_value(arg.value, temp, &replacement);
            arg
        })
        .collect();

    Some(ReturnSlotCallArgForwardCandidate {
        temp,
        result_width: result.width,
        return_slot: match return_slot_mem(offset) {
            MirMem::FixedZeroPage(slot) => slot,
            _ => unreachable!("return slots use fixed zero page"),
        },
        blocked_home_overlap: false,
        replacement: [
            MirOp::Call {
                target: first_target.clone(),
                abi: first_abi.clone(),
                args: first_args.clone(),
                result: None,
                effects: first_effects.clone(),
            },
            MirOp::Call {
                target: rewritten_target,
                abi: abi.clone(),
                args: rewritten_args,
                result: second_result.clone(),
                effects: effects.clone(),
            },
        ],
    })
}

fn call_arg_home_overlaps_return_slot(
    home: &MirArgHome,
    width: MirWidth,
    result_offset: u16,
    result_width: MirWidth,
) -> bool {
    let result_start = return_slot_address(result_offset);
    let result_end = result_start.saturating_add(match result_width {
        MirWidth::Byte => 0,
        MirWidth::Word => 1,
    });
    let mut addresses = Vec::new();
    collect_call_arg_home_addresses(home, width, &mut addresses);
    addresses
        .into_iter()
        .any(|address| (result_start..=result_end).contains(&address))
}

fn return_slot_address(offset: u16) -> u16 {
    let MirMem::FixedZeroPage(slot) = return_slot_mem(offset) else {
        unreachable!("return slots are fixed zero-page storage")
    };
    u16::from(slot.0)
}

fn collect_call_arg_home_addresses(home: &MirArgHome, width: MirWidth, out: &mut Vec<u16>) {
    match home {
        MirArgHome::FixedZeroPage(slot) => {
            out.push(u16::from(slot.0));
            if width == MirWidth::Word {
                out.push(u16::from(slot.0.saturating_add(1)));
            }
        }
        MirArgHome::Absolute(address) => {
            out.push(*address);
            if width == MirWidth::Word {
                out.push(address.saturating_add(1));
            }
        }
        MirArgHome::StackFrame { base, offset } => {
            out.push(base.saturating_add(*offset));
            if width == MirWidth::Word {
                out.push(base.saturating_add(*offset).saturating_add(1));
            }
        }
        MirArgHome::BytePair { lo, hi } => {
            collect_call_arg_home_addresses(lo, MirWidth::Byte, out);
            collect_call_arg_home_addresses(hi, MirWidth::Byte, out);
        }
        MirArgHome::Reg(_) | MirArgHome::RegisterPair { .. } | MirArgHome::ZeroPage(_) => {}
    }
}

fn collect_call_arg_expr_plan(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
) -> Option<CallArgExprPlan> {
    let mut exprs = BTreeMap::<MirTempId, CallArgExpr>::new();
    let mut cursor = index;
    while let Some((temp, expr)) = call_arg_expr_producer(ops.get(cursor)?, &exprs, config, layout)
    {
        exprs.insert(temp, expr);
        cursor += 1;
    }
    if exprs.is_empty() {
        return None;
    }

    let MirOp::Call {
        target,
        abi,
        args,
        result,
        effects,
    } = ops.get(cursor)?
    else {
        return None;
    };
    if call_target_uses_collected_temp(target, &exprs) || !call_arg_expr_homes_supported(args) {
        return None;
    }
    let has_indexed_word_load = exprs
        .values()
        .any(|expr| matches!(expr, CallArgExpr::IndexedWordLoad { .. }));
    if has_indexed_word_load
        && args
            .iter()
            .any(|arg| matches!(arg.home, MirArgHome::Reg(MirReg::Y)))
    {
        return None;
    }
    if has_indexed_word_load
        && matches!(target, MirCallTarget::Indirect { .. })
        && exprs
            .values()
            .any(indexed_word_load_reads_indirect_target_scratch)
    {
        return None;
    }

    let mut planned_args = Vec::new();
    let mut saw_expr = false;
    for arg in args {
        if let Some(temp) = call_arg_expr_temp(&arg.value, arg.width)
            && let Some(expr) = exprs.get(&temp)
        {
            if matches!(expr, CallArgExpr::IndexedWordLoad { .. })
                && !matches!(
                    (&arg.value, arg.width),
                    (MirValue::Def(MirDef::VTemp(id)), MirWidth::Word) if *id == temp
                )
            {
                return None;
            }
            saw_expr = true;
            planned_args.push(PlannedCallArg::Expr {
                expr: expr.clone(),
                width: arg.width,
                home: arg.home.clone(),
            });
            continue;
        }
        if value_uses_collected_temp(&arg.value, &exprs) {
            return None;
        }
        if matches!(arg.home, MirArgHome::RegisterPair { .. }) {
            return None;
        }
        planned_args.push(PlannedCallArg::Existing(arg.clone()));
    }
    if !saw_expr {
        return None;
    }

    Some(CallArgExprPlan {
        consumed: cursor + 1 - index,
        temps: exprs.keys().copied().collect(),
        target: target.clone(),
        abi: abi.clone(),
        args: planned_args,
        result: result.clone(),
        effects: effects.clone(),
    })
}

fn call_arg_expr_producer(
    op: &MirOp,
    exprs: &BTreeMap<MirTempId, CallArgExpr>,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
) -> Option<(MirTempId, CallArgExpr)> {
    match op {
        MirOp::LoadImm { dst, value, width } => {
            let temp = split_def_as_temp(dst)?;
            let value = match width {
                MirWidth::Byte => MirValue::ConstU8(*value as u8),
                MirWidth::Word => MirValue::ConstU16(*value),
            };
            Some((
                temp,
                CallArgExpr::Value {
                    value,
                    width: *width,
                },
            ))
        }
        MirOp::Load {
            dst,
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => Some((
            split_def_as_temp(dst)?,
            CallArgExpr::Value {
                value: MirValue::PointerCell(mem.clone()),
                width: MirWidth::Byte,
            },
        )),
        MirOp::Load {
            dst,
            src: MirAddr::Direct(mem),
            width: MirWidth::Word,
        } => Some((
            split_def_as_temp(dst)?,
            CallArgExpr::Value {
                value: pointer_value_from_mem(mem),
                width: MirWidth::Word,
            },
        )),
        MirOp::Load {
            dst,
            src,
            width: MirWidth::Word,
        } if indexed_addr_parts(src).is_some() => Some((
            split_def_as_temp(dst)?,
            CallArgExpr::IndexedWordLoad {
                addr: resolve_call_arg_indexed_addr(src, exprs, layout)?,
            },
        )),
        MirOp::Move { dst, src, width } => Some((
            split_def_as_temp(dst)?,
            CallArgExpr::Value {
                value: call_arg_expr_value(src, exprs, layout)?,
                width: *width,
            },
        )),
        MirOp::Binary {
            op,
            dst,
            left,
            right,
            width,
            carry_in,
            carry_out: MirCarryOut::Ignore,
        } if call_arg_expr_binary_is_supported(*op, *width, *carry_in, config) => {
            Some((split_def_as_temp(dst)?, {
                let left = call_arg_expr_operand(left, exprs, layout)?;
                let right = call_arg_expr_operand(right, exprs, layout)?;
                if !call_arg_expr_operands_supported(*op, *width, &left, &right) {
                    return None;
                }
                CallArgExpr::Binary {
                    op: *op,
                    left: Box::new(left),
                    right: Box::new(right),
                    width: *width,
                }
            }))
        }
        _ => None,
    }
}

fn resolve_call_arg_indexed_addr(
    addr: &MirAddr,
    exprs: &BTreeMap<MirTempId, CallArgExpr>,
    layout: &MaterializeLayout,
) -> Option<MirAddr> {
    let mut resolved = addr.clone();
    for (temp, expr) in exprs {
        let replacement = expr_as_plain_value(expr, layout)?;
        resolved = replace_temp_addr(resolved, *temp, &replacement);
    }
    let parts = indexed_addr_parts(&resolved)?;
    if value_uses_temp(&parts.base) || value_uses_temp(&parts.index) {
        return None;
    }
    Some(resolved)
}

fn indexed_word_load_reads_indirect_target_scratch(expr: &CallArgExpr) -> bool {
    let CallArgExpr::IndexedWordLoad { addr } = expr else {
        return false;
    };
    let Some(parts) = indexed_addr_parts(addr) else {
        return false;
    };
    [INDIRECT_CALL_TARGET_LO, INDIRECT_CALL_TARGET_HI]
        .into_iter()
        .map(|slot| MirMem::FixedZeroPage(MirFixedZpSlot(slot)))
        .any(|mem| value_reads_mem(&parts.base, &mem) || value_reads_mem(&parts.index, &mem))
}

fn call_arg_expr_operands_supported(
    op: MirBinaryOp,
    width: MirWidth,
    left: &CallArgExpr,
    right: &CallArgExpr,
) -> bool {
    if matches!((op, width), (MirBinaryOp::Mul, MirWidth::Byte)) {
        return call_arg_expr_can_materialize_byte(left)
            && call_arg_expr_can_materialize_byte(right);
    }
    if indexed_word_const_binary_operands(op, width, left, right).is_some() {
        return true;
    }
    matches!(left, CallArgExpr::Value { .. }) && matches!(right, CallArgExpr::Value { .. })
}

fn indexed_word_const_binary_parts(expr: &CallArgExpr) -> Option<(&MirAddr, u16, MirBinaryOp)> {
    let CallArgExpr::Binary {
        op,
        left,
        right,
        width: MirWidth::Word,
    } = expr
    else {
        return None;
    };
    indexed_word_const_binary_operands(*op, MirWidth::Word, left, right)
}

fn indexed_word_const_binary_operands<'a>(
    op: MirBinaryOp,
    width: MirWidth,
    left: &'a CallArgExpr,
    right: &'a CallArgExpr,
) -> Option<(&'a MirAddr, u16, MirBinaryOp)> {
    if width != MirWidth::Word {
        return None;
    }
    match (op, left, right) {
        (MirBinaryOp::Add | MirBinaryOp::Sub, CallArgExpr::IndexedWordLoad { addr }, value) => {
            Some((addr, call_arg_expr_constant(value)?, op))
        }
        (MirBinaryOp::Add, value, CallArgExpr::IndexedWordLoad { addr }) => {
            Some((addr, call_arg_expr_constant(value)?, op))
        }
        _ => None,
    }
}

fn call_arg_expr_constant(expr: &CallArgExpr) -> Option<u16> {
    let CallArgExpr::Value { value, .. } = expr else {
        return None;
    };
    match value {
        MirValue::ConstU8(value) => Some(u16::from(*value)),
        MirValue::ConstU16(value) => Some(*value),
        _ => None,
    }
}

fn call_arg_expr_can_materialize_byte(expr: &CallArgExpr) -> bool {
    match expr {
        CallArgExpr::Value { .. } => true,
        CallArgExpr::IndexedWordLoad { .. } => false,
        CallArgExpr::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub,
            left,
            right,
            ..
        } => matches!(
            (&**left, &**right),
            (CallArgExpr::Value { .. }, CallArgExpr::Value { .. })
        ),
        CallArgExpr::Binary { .. } => false,
    }
}

fn call_arg_expr_binary_is_supported(
    op: MirBinaryOp,
    width: MirWidth,
    carry_in: Option<MirCarryIn>,
    config: &Mir6502Config,
) -> bool {
    match (op, width, carry_in) {
        (MirBinaryOp::Add, _, None | Some(MirCarryIn::Clear))
        | (MirBinaryOp::Sub, _, None | Some(MirCarryIn::Set)) => true,
        (MirBinaryOp::Mul, MirWidth::Byte, None) => config.select_runtime_helpers,
        _ => false,
    }
}

fn call_arg_expr_operand(
    value: &MirValue,
    exprs: &BTreeMap<MirTempId, CallArgExpr>,
    layout: &MaterializeLayout,
) -> Option<CallArgExpr> {
    if let Some(temp) = value_as_temp(value)
        && let Some(expr) = exprs.get(&temp)
    {
        return Some(expr.clone());
    }
    let width = match value {
        MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::PointerCell(_) => MirWidth::Word,
        _ => MirWidth::Byte,
    };
    Some(CallArgExpr::Value {
        value: call_arg_expr_value(value, exprs, layout)?,
        width,
    })
}

fn call_arg_expr_value(
    value: &MirValue,
    exprs: &BTreeMap<MirTempId, CallArgExpr>,
    layout: &MaterializeLayout,
) -> Option<MirValue> {
    if let Some(temp) = value_as_temp(value) {
        let expr = exprs.get(&temp)?;
        return expr_as_plain_value(expr, layout);
    }
    if value_uses_temp(value) {
        None
    } else {
        Some(value.clone())
    }
}

fn expr_as_plain_value(expr: &CallArgExpr, layout: &MaterializeLayout) -> Option<MirValue> {
    match expr {
        CallArgExpr::Value { value, .. } => Some(value.clone()),
        CallArgExpr::IndexedWordLoad { .. } => None,
        CallArgExpr::Binary { .. } => {
            let (lo, hi) = expr_word_byte_values(expr, layout)?;
            Some(MirValue::Word {
                lo: Box::new(lo),
                hi: Box::new(hi),
            })
        }
    }
}

fn call_arg_expr_homes_supported(args: &[MirCallArg]) -> bool {
    let has_register_pair = args.iter().any(|arg| {
        matches!(
            arg.home,
            MirArgHome::RegisterPair {
                lo: MirReg::A,
                hi: MirReg::X
            }
        )
    });
    args.iter().all(|arg| match arg.home {
        MirArgHome::Reg(MirReg::A | MirReg::X) if has_register_pair => false,
        MirArgHome::Reg(_) => true,
        MirArgHome::RegisterPair {
            lo: MirReg::A,
            hi: MirReg::X,
        } => true,
        _ => false,
    })
}

fn materialize_call_arg_expr_plan(
    plan: &CallArgExprPlan,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    out: &mut Vec<MirOp>,
) {
    let target = materialize_call_target(plan.target.clone(), layout, out);
    for arg in &plan.args {
        if let Some(reg) = planned_call_arg_reg_home(arg)
            && reg != MirReg::A
        {
            materialize_planned_call_arg(arg, layout, helpers, out);
        }
    }
    for arg in &plan.args {
        if planned_call_arg_reg_home(arg) == Some(MirReg::A) {
            materialize_planned_call_arg(arg, layout, helpers, out);
        }
    }
    for arg in &plan.args {
        if matches!(
            arg,
            PlannedCallArg::Expr {
                home: MirArgHome::RegisterPair { .. },
                ..
            } | PlannedCallArg::Existing(MirCallArg {
                home: MirArgHome::RegisterPair { .. },
                ..
            })
        ) {
            materialize_planned_call_arg(arg, layout, helpers, out);
        }
    }
    let args = plan
        .args
        .iter()
        .flat_map(materialized_planned_call_args)
        .collect::<Vec<_>>();
    out.push(MirOp::Call {
        target,
        abi: MirCallAbi {
            params: plan
                .args
                .iter()
                .flat_map(materialized_planned_call_homes)
                .collect(),
            result: None,
            clobbers: plan.abi.clobbers,
            preserves: plan.abi.preserves,
        },
        args,
        result: None,
        effects: plan.effects.clone(),
    });
    if let Some(result) = &plan.result {
        materialize_call_result(result.dst.clone(), result.width, result.home.clone(), out);
    }
}

fn planned_call_arg_reg_home(arg: &PlannedCallArg) -> Option<MirReg> {
    match arg {
        PlannedCallArg::Expr {
            home: MirArgHome::Reg(reg),
            ..
        }
        | PlannedCallArg::Existing(MirCallArg {
            home: MirArgHome::Reg(reg),
            ..
        }) => Some(*reg),
        _ => None,
    }
}

fn materialize_planned_call_arg(
    arg: &PlannedCallArg,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    out: &mut Vec<MirOp>,
) {
    match arg {
        PlannedCallArg::Expr {
            expr,
            width: MirWidth::Byte,
            home: MirArgHome::Reg(reg),
        } => materialize_expr_byte_to_reg(expr, *reg, layout, out),
        PlannedCallArg::Expr {
            expr,
            width: MirWidth::Word,
            home:
                MirArgHome::RegisterPair {
                    lo: MirReg::A,
                    hi: MirReg::X,
                },
        } => materialize_expr_word_to_ax(expr, layout, helpers, out),
        PlannedCallArg::Existing(arg) => materialize_call_arg(arg, out),
        PlannedCallArg::Expr { .. } => {}
    }
}

fn materialize_expr_byte_to_reg(
    expr: &CallArgExpr,
    reg: MirReg,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    match expr {
        CallArgExpr::Binary {
            op,
            left,
            right,
            width,
        } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
            let Some((left, right)) = expr_binary_low_operands(left, right, layout) else {
                return;
            };
            let right = materialize_binary_rhs_to_fixed_scratch_avoiding(right, 0, &left, &[], out);
            materialize_call_arg_to_reg(left, MirReg::A, out);
            out.push(MirOp::Binary {
                op: *op,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right,
                width: MirWidth::Byte,
                carry_in: Some(match op {
                    MirBinaryOp::Add => MirCarryIn::Clear,
                    MirBinaryOp::Sub => MirCarryIn::Set,
                    _ => unreachable!(),
                }),
                carry_out: if matches!(width, MirWidth::Word) {
                    MirCarryOut::Produce
                } else {
                    MirCarryOut::Ignore
                },
            });
            if reg != MirReg::A {
                materialize_call_arg_to_reg(MirValue::Def(MirDef::Reg(MirReg::A)), reg, out);
            }
        }
        _ => {
            let Some(value) = expr_low_value(expr, layout) else {
                return;
            };
            materialize_call_arg_to_reg(value, reg, out);
        }
    }
}

fn materialize_expr_word_to_ax(
    expr: &CallArgExpr,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    out: &mut Vec<MirOp>,
) {
    match expr {
        CallArgExpr::IndexedWordLoad { addr } => {
            let Some(parts) = indexed_addr_parts(addr) else {
                return;
            };
            materialize_indexed_address_for_consumer(
                parts.clone(),
                DEFAULT_POINTER_PAIR,
                layout,
                None,
                out,
            );
            out.push(MirOp::LoadIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                dst: MirDef::Reg(MirReg::A),
                offset: parts.offset,
            });
            out.push(MirOp::Store {
                dst: MirAddr::Direct(return_slot_mem(0)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            });
            out.push(MirOp::LoadIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                dst: MirDef::Reg(MirReg::A),
                offset: parts.offset.saturating_add(1),
            });
            out.push(MirOp::Move {
                dst: MirDef::Reg(MirReg::X),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            });
            materialize_call_arg_to_reg(MirValue::PointerCell(return_slot_mem(0)), MirReg::A, out);
        }
        expr @ CallArgExpr::Binary {
            width: MirWidth::Word,
            ..
        } if indexed_word_const_binary_parts(expr).is_some() => {
            let Some((addr, constant, op)) = indexed_word_const_binary_parts(expr) else {
                return;
            };
            materialize_indexed_word_const_binary_to_ax(addr, constant, op, layout, out);
        }
        CallArgExpr::Binary {
            op: MirBinaryOp::Mul,
            left,
            right,
            width: MirWidth::Byte,
        } => {
            helpers.push(MirRuntimeHelper::Mul);
            materialize_byte_mul_expr_to_ax(left, right, layout, out);
        }
        CallArgExpr::Binary {
            op,
            left,
            right,
            width: MirWidth::Word,
        } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
            let Some((left_lo, right_lo)) = expr_binary_low_operands(left, right, layout) else {
                return;
            };
            let Some((left_hi, right_hi)) = expr_binary_high_operands(left, right, layout) else {
                return;
            };
            let right_lo =
                materialize_binary_rhs_to_fixed_scratch_avoiding(right_lo, 1, &left_lo, &[0], out);
            materialize_call_arg_to_reg(left_lo, MirReg::A, out);
            out.push(MirOp::Binary {
                op: *op,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: right_lo,
                width: MirWidth::Byte,
                carry_in: Some(match op {
                    MirBinaryOp::Add => MirCarryIn::Clear,
                    MirBinaryOp::Sub => MirCarryIn::Set,
                    _ => unreachable!(),
                }),
                carry_out: MirCarryOut::Produce,
            });
            out.push(MirOp::Store {
                dst: MirAddr::Direct(return_slot_mem(0)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            });
            let right_hi =
                materialize_binary_rhs_to_fixed_scratch_avoiding(right_hi, 1, &left_hi, &[0], out);
            materialize_call_arg_to_reg(left_hi, MirReg::A, out);
            out.push(MirOp::Binary {
                op: *op,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: right_hi,
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            });
            out.push(MirOp::Move {
                dst: MirDef::Reg(MirReg::X),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            });
            materialize_call_arg_to_reg(MirValue::PointerCell(return_slot_mem(0)), MirReg::A, out);
        }
        _ => {
            let Some((lo, hi)) = expr_word_byte_values(expr, layout) else {
                return;
            };
            materialize_call_arg_to_reg(hi, MirReg::X, out);
            materialize_call_arg_to_reg(lo, MirReg::A, out);
        }
    }
}

fn materialize_indexed_word_const_binary_to_ax(
    addr: &MirAddr,
    constant: u16,
    op: MirBinaryOp,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    let Some(parts) = indexed_addr_parts(addr) else {
        return;
    };
    materialize_indexed_address_for_consumer(
        parts.clone(),
        DEFAULT_POINTER_PAIR,
        layout,
        None,
        out,
    );
    out.push(MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset: parts.offset,
    });
    out.push(MirOp::Binary {
        op,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: MirValue::ConstU8(constant as u8),
        width: MirWidth::Byte,
        carry_in: Some(match op {
            MirBinaryOp::Add => MirCarryIn::Clear,
            MirBinaryOp::Sub => MirCarryIn::Set,
            _ => unreachable!("indexed word constant arithmetic only supports add/sub"),
        }),
        carry_out: MirCarryOut::Produce,
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(return_slot_mem(0)),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    out.push(MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset: parts.offset.saturating_add(1),
    });
    out.push(MirOp::Binary {
        op,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: MirValue::ConstU8((constant >> 8) as u8),
        width: MirWidth::Byte,
        carry_in: Some(MirCarryIn::FromPrevious),
        carry_out: MirCarryOut::Ignore,
    });
    out.push(MirOp::Move {
        dst: MirDef::Reg(MirReg::X),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    materialize_call_arg_to_reg(MirValue::PointerCell(return_slot_mem(0)), MirReg::A, out);
}

fn materialize_byte_mul_expr_to_ax(
    left: &CallArgExpr,
    right: &CallArgExpr,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    let left_scratch = return_slot_mem(4);
    materialize_expr_byte_to_reg(left, MirReg::A, layout, out);
    out.push(MirOp::Store {
        dst: MirAddr::Direct(left_scratch.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    materialize_expr_byte_to_reg(right, MirReg::A, layout, out);
    out.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0x84))),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0x85))),
        src: MirValue::ConstU8(0),
        width: MirWidth::Byte,
    });
    materialize_call_arg_to_reg(MirValue::PointerCell(left_scratch), MirReg::A, out);
    out.push(MirOp::Move {
        dst: MirDef::Reg(MirReg::X),
        src: MirValue::ConstU8(0),
        width: MirWidth::Byte,
    });
    out.push(MirOp::RuntimeHelper {
        helper: MirRuntimeHelper::Mul,
        args: Vec::new(),
        result: None,
        effects: helper_effects(),
    });
}

fn materialize_binary_rhs_to_fixed_scratch_avoiding(
    value: MirValue,
    preferred_scratch_offset: u16,
    left: &MirValue,
    reserved_offsets: &[u16],
    out: &mut Vec<MirOp>,
) -> MirValue {
    let MirValue::PointerCell(mem) = value else {
        return value;
    };
    let scratch_offset =
        binary_rhs_scratch_offset(preferred_scratch_offset, left, reserved_offsets);
    let scratch = return_slot_mem(scratch_offset);
    materialize_call_arg_to_mem(MirValue::PointerCell(mem), scratch.clone(), out);
    MirValue::PointerCell(scratch)
}

fn binary_rhs_scratch_offset(preferred: u16, left: &MirValue, reserved_offsets: &[u16]) -> u16 {
    [preferred, 0, 1, 2, 3]
        .into_iter()
        .find(|offset| {
            !reserved_offsets.contains(offset) && !value_reads_mem(left, &return_slot_mem(*offset))
        })
        .unwrap_or(preferred)
}

fn value_reads_mem(value: &MirValue, mem: &MirMem) -> bool {
    match value {
        MirValue::PointerCell(source) => source == mem,
        MirValue::Word { lo, hi } => value_reads_mem(lo, mem) || value_reads_mem(hi, mem),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => false,
    }
}

fn expr_binary_low_operands(
    left: &CallArgExpr,
    right: &CallArgExpr,
    layout: &MaterializeLayout,
) -> Option<(MirValue, MirValue)> {
    Some((
        expr_low_value(left, layout)?,
        expr_low_value(right, layout)?,
    ))
}

fn expr_binary_high_operands(
    left: &CallArgExpr,
    right: &CallArgExpr,
    layout: &MaterializeLayout,
) -> Option<(MirValue, MirValue)> {
    Some((
        expr_high_value(left, layout)?,
        expr_high_value(right, layout)?,
    ))
}

fn expr_word_byte_values(
    expr: &CallArgExpr,
    layout: &MaterializeLayout,
) -> Option<(MirValue, MirValue)> {
    match expr {
        CallArgExpr::Value { value, width } => match width {
            MirWidth::Byte => Some((value.clone(), MirValue::ConstU8(0))),
            MirWidth::Word => Some(split_value(value.clone(), layout)),
        },
        CallArgExpr::IndexedWordLoad { .. } => None,
        CallArgExpr::Binary { .. } => None,
    }
}

fn expr_low_value(expr: &CallArgExpr, layout: &MaterializeLayout) -> Option<MirValue> {
    expr_word_byte_values(expr, layout).map(|(lo, _)| lo)
}

fn expr_high_value(expr: &CallArgExpr, layout: &MaterializeLayout) -> Option<MirValue> {
    expr_word_byte_values(expr, layout).map(|(_, hi)| hi)
}

fn materialized_planned_call_args(arg: &PlannedCallArg) -> Vec<MirCallArg> {
    match arg {
        PlannedCallArg::Expr {
            width: MirWidth::Byte,
            home: MirArgHome::Reg(reg),
            ..
        } => vec![MirCallArg {
            value: MirValue::Def(MirDef::Reg(*reg)),
            width: MirWidth::Byte,
            home: MirArgHome::Reg(*reg),
        }],
        PlannedCallArg::Expr {
            width: MirWidth::Word,
            home:
                MirArgHome::RegisterPair {
                    lo: MirReg::A,
                    hi: MirReg::X,
                },
            ..
        } => vec![
            MirCallArg {
                value: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
                home: MirArgHome::Reg(MirReg::A),
            },
            MirCallArg {
                value: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
                home: MirArgHome::Reg(MirReg::X),
            },
        ],
        PlannedCallArg::Existing(arg) => vec![materialized_call_arg_summary(arg)],
        PlannedCallArg::Expr { .. } => Vec::new(),
    }
}

fn materialized_planned_call_homes(arg: &PlannedCallArg) -> Vec<MirArgHome> {
    materialized_planned_call_args(arg)
        .into_iter()
        .map(|arg| arg.home)
        .collect()
}

fn value_as_temp(value: &MirValue) -> Option<MirTempId> {
    match value {
        MirValue::Def(MirDef::VTemp(temp)) => Some(*temp),
        _ => None,
    }
}

fn call_arg_expr_temp(value: &MirValue, width: MirWidth) -> Option<MirTempId> {
    match (value, width) {
        (MirValue::Def(MirDef::VTemp(temp)), _) => Some(*temp),
        (MirValue::Def(MirDef::VTempByte { id, byte: 0 }), MirWidth::Byte) => Some(*id),
        (MirValue::Word { lo, hi }, MirWidth::Word) if matches!(&**hi, MirValue::ConstU8(0)) => {
            value_as_temp(lo)
        }
        _ => None,
    }
}

fn value_uses_collected_temp(value: &MirValue, exprs: &BTreeMap<MirTempId, CallArgExpr>) -> bool {
    exprs
        .keys()
        .any(|temp| count_temp_uses_in_value(value, *temp) > 0)
}

fn call_target_uses_collected_temp(
    target: &MirCallTarget,
    exprs: &BTreeMap<MirTempId, CallArgExpr>,
) -> bool {
    exprs.keys().any(|temp| {
        let mut count = 0;
        count_call_target_temp_uses(target, *temp, &mut count);
        count > 0
    })
}

fn count_temp_uses_in_value(value: &MirValue, temp: MirTempId) -> usize {
    let mut count = 0;
    count_value_temp_uses(value, temp, &mut count);
    count
}

pub(in crate::mir6502) fn call_arg_producer_rewrite_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<CallArgProducerRewriteCandidate> {
    let mut producers = Vec::new();
    let mut cursor = index;
    while let Some((temp, value)) = call_arg_producer_value(ops.get(cursor)?) {
        if producers
            .iter()
            .any(|producer: &(MirTempId, MirValue, usize)| op_uses_temp(&ops[cursor], producer.0))
        {
            return None;
        }
        producers.push((temp, value, cursor));
        cursor += 1;
    }
    if producers.is_empty() {
        return None;
    }

    let MirOp::Call {
        target,
        abi,
        args,
        result,
        effects,
    } = ops.get(cursor)?
    else {
        return None;
    };

    for (temp, _, producer_index) in &producers {
        if ops[producer_index.saturating_add(1)..cursor]
            .iter()
            .any(|op| op_uses_temp(op, *temp))
        {
            return None;
        }
        let mut call_uses = 0usize;
        count_call_target_temp_uses(target, *temp, &mut call_uses);
        for arg in args {
            count_value_temp_uses(&arg.value, *temp, &mut call_uses);
        }
        if call_uses != 1 {
            return None;
        }
    }

    let mut rewritten_target = target.clone();
    let mut rewritten_args = args.clone();
    for (temp, replacement, _) in &producers {
        rewritten_target = replace_call_target_temp(rewritten_target, *temp, replacement);
        for arg in &mut rewritten_args {
            arg.value = replace_temp_value(arg.value.clone(), *temp, replacement);
        }
    }

    Some(CallArgProducerRewriteCandidate {
        consumed: cursor + 1 - index,
        temps: producers.iter().map(|(temp, _, _)| *temp).collect(),
        replacement: MirOp::Call {
            target: rewritten_target,
            abi: abi.clone(),
            args: rewritten_args,
            result: result.clone(),
            effects: effects.clone(),
        },
    })
}

fn replace_call_target_temp(
    target: MirCallTarget,
    temp: MirTempId,
    replacement: &MirValue,
) -> MirCallTarget {
    match target {
        MirCallTarget::Indirect { target, width } => MirCallTarget::Indirect {
            target: replace_temp_value(target, temp, replacement),
            width,
        },
        other => other,
    }
}

fn call_arg_producer_value(op: &MirOp) -> Option<(MirTempId, MirValue)> {
    match op {
        MirOp::LoadImm { dst, value, width } => {
            let temp = split_def_as_temp(dst)?;
            let value = match width {
                MirWidth::Byte => MirValue::ConstU8(*value as u8),
                MirWidth::Word => MirValue::ConstU16(*value),
            };
            Some((temp, value))
        }
        MirOp::Load {
            dst,
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => Some((split_def_as_temp(dst)?, MirValue::PointerCell(mem.clone()))),
        MirOp::Load {
            dst,
            src: MirAddr::Direct(mem),
            width: MirWidth::Word,
        } => Some((split_def_as_temp(dst)?, pointer_value_from_mem(mem))),
        MirOp::LeaAddr {
            dst,
            target,
            width: MirWidth::Word,
        } => Some((split_def_as_temp(dst)?, storage_address_value(target))),
        MirOp::Move { dst, src, .. } if !value_uses_temp(src) => {
            Some((split_def_as_temp(dst)?, src.clone()))
        }
        _ => None,
    }
}

pub(in crate::mir6502) fn discover_param_home_consumers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = param_word_store_consumer_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
            if let Some(plan) = param_call_target_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn param_home_consumer_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .map(|op| match op {
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Param { .. }),
                width: MirWidth::Word,
                ..
            } => 1,
            MirOp::Call {
                target: MirCallTarget::Indirect { target, .. },
                ..
            } => value_param_home_count(&target),
            _ => 0,
        })
        .sum()
}

fn value_param_home_count(value: &MirValue) -> usize {
    match value {
        MirValue::PointerCell(MirMem::Param { .. }) => 1,
        MirValue::Word { lo, hi } => value_param_home_count(lo) + value_param_home_count(hi),
        _ => 0,
    }
}

fn param_word_store_consumer_plan(
    block: MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let MirOp::Load {
        dst,
        src: MirAddr::Direct(src @ MirMem::Param { .. }),
        width: MirWidth::Word,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    if store_src != dst {
        return None;
    }
    let point = context.point(MirSite::Op {
        block,
        op_index: index,
    });
    let MirProof::Proven(lo) =
        context.parameter_register_at(MirParamHomeByte::from_mem(src)?, point)
    else {
        return None;
    };
    let MirProof::Proven(hi) =
        context.parameter_register_at(MirParamHomeByte::from_mem(&offset_mem(src, 1))?, point)
    else {
        return None;
    };
    let replacement = vec![
        MirOp::Store {
            dst: MirAddr::Direct(store_dst.clone()),
            src: MirValue::Def(MirDef::Reg(lo)),
            width: MirWidth::Byte,
        },
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::Def(MirDef::Reg(hi)),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(offset_mem(store_dst, 1)),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ];
    let definitions = crate::mir6502::rewrite::pilots::prove_removed_window_definitions(
        block,
        ops,
        index,
        index + 2,
        &replacement,
        context,
    )?;
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 2,
        replacement,
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::MaterializedStoreConsumer,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "param-word-store-consumer",
        observations: Vec::new(),
        family_priority: 0,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn param_call_target_plan(
    block: MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let MirOp::Call {
        target: MirCallTarget::Indirect { target, width },
        abi,
        args,
        result,
        effects,
    } = ops.get(index)?
    else {
        return None;
    };
    let point = context.point(MirSite::Op {
        block,
        op_index: index,
    });
    let rewritten = rewrite_param_value_from_context(target, point, context);
    if rewritten == *target {
        return None;
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 1,
        replacement: vec![MirOp::Call {
            target: MirCallTarget::Indirect {
                target: rewritten,
                width: *width,
            },
            abi: abi.clone(),
            args: args.clone(),
            result: result.clone(),
            effects: effects.clone(),
        }],
        removed_defs: Vec::new(),
        exit_effect_delta: MirEffectDelta::MaterializedCallArguments,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "param-call-target-forward",
        observations: Vec::new(),
        family_priority: 1,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn rewrite_param_value_from_context(
    value: &MirValue,
    point: crate::mir6502::analysis::sites::MirProgramPoint,
    context: &PreHomeRewriteContext<'_, '_>,
) -> MirValue {
    match value {
        MirValue::PointerCell(mem @ MirMem::Param { .. }) => {
            match context.parameter_register_at(
                MirParamHomeByte::from_mem(mem).expect("parameter home"),
                point,
            ) {
                MirProof::Proven(reg) => MirValue::Def(MirDef::Reg(reg)),
                MirProof::Blocked(_) => value.clone(),
            }
        }
        MirValue::Word { lo, hi } => MirValue::Word {
            lo: Box::new(rewrite_param_value_from_context(lo, point, context)),
            hi: Box::new(rewrite_param_value_from_context(hi, point, context)),
        },
        _ => value.clone(),
    }
}

pub(in crate::mir6502) fn discover_param_home_reloads(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, op) in block.ops.iter().enumerate() {
            let MirOp::Load {
                dst: MirDef::Reg(dst),
                src: MirAddr::Direct(src @ MirMem::Param { .. }),
                width: MirWidth::Byte,
            } = op
            else {
                continue;
            };
            let MirProof::Proven(source) = context.parameter_register_at(
                MirParamHomeByte::from_mem(&src).expect("parameter home"),
                context.point(MirSite::Op {
                    block: block.id,
                    op_index: index,
                }),
            ) else {
                continue;
            };
            let (replacement, exit_change) = if source == *dst {
                (
                    Vec::new(),
                    MirExitStateChange {
                        flags: MirFlagSet {
                            z: true,
                            n: true,
                            ..MirFlagSet::default()
                        },
                        ..MirExitStateChange::default()
                    },
                )
            } else {
                (
                    vec![MirOp::Move {
                        dst: MirDef::Reg(*dst),
                        src: MirValue::Def(MirDef::Reg(source)),
                        width: MirWidth::Byte,
                    }],
                    MirExitStateChange::default(),
                )
            };
            if let Some(plan) = structural_plan(
                routine,
                context,
                block.id,
                index..index + 1,
                replacement,
                exit_change,
                "param-home-reload-forward",
                0,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(super) fn materialize_call(
    target: MirCallTarget,
    abi: MirCallAbi,
    args: Vec<MirCallArg>,
    result: Option<super::super::ir::MirCallResult>,
    effects: MirEffects,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    let target = materialize_call_target(target, layout, out);
    let mut byte_args = Vec::new();
    let mut byte_homes = Vec::new();
    for arg in args {
        match arg {
            MirCallArg {
                value,
                width: MirWidth::Word,
                home: MirArgHome::RegisterPair { lo, hi },
            } => {
                let (lo_value, hi_value) = split_value(value, layout);
                byte_args.push(MirCallArg {
                    value: lo_value,
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(lo),
                });
                byte_args.push(MirCallArg {
                    value: hi_value,
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(hi),
                });
                byte_homes.push(MirArgHome::Reg(lo));
                byte_homes.push(MirArgHome::Reg(hi));
            }
            MirCallArg {
                value,
                width: MirWidth::Word,
                home:
                    MirArgHome::BytePair {
                        lo: lo_home,
                        hi: hi_home,
                    },
            } => {
                let (lo_value, hi_value) = split_value(value, layout);
                let lo_home = *lo_home;
                let hi_home = *hi_home;
                byte_args.push(MirCallArg {
                    value: lo_value,
                    width: MirWidth::Byte,
                    home: lo_home.clone(),
                });
                byte_args.push(MirCallArg {
                    value: hi_value,
                    width: MirWidth::Byte,
                    home: hi_home.clone(),
                });
                byte_homes.push(lo_home);
                byte_homes.push(hi_home);
            }
            MirCallArg {
                value,
                width: MirWidth::Word,
                home: MirArgHome::FixedZeroPage(slot),
            } => {
                let (lo, hi) = split_value(value, layout);
                byte_args.push(MirCallArg {
                    value: lo,
                    width: MirWidth::Byte,
                    home: MirArgHome::FixedZeroPage(slot),
                });
                byte_args.push(MirCallArg {
                    value: hi,
                    width: MirWidth::Byte,
                    home: MirArgHome::FixedZeroPage(MirFixedZpSlot(slot.0.saturating_add(1))),
                });
                byte_homes.push(MirArgHome::FixedZeroPage(slot));
                byte_homes.push(MirArgHome::FixedZeroPage(MirFixedZpSlot(
                    slot.0.saturating_add(1),
                )));
            }
            MirCallArg {
                value,
                width: MirWidth::Word,
                home: MirArgHome::StackFrame { base, offset },
            } => {
                let (lo, hi) = split_value(value, layout);
                byte_args.push(MirCallArg {
                    value: lo,
                    width: MirWidth::Byte,
                    home: MirArgHome::StackFrame { base, offset },
                });
                byte_args.push(MirCallArg {
                    value: hi,
                    width: MirWidth::Byte,
                    home: MirArgHome::StackFrame {
                        base,
                        offset: offset.saturating_add(1),
                    },
                });
                byte_homes.push(MirArgHome::StackFrame { base, offset });
                byte_homes.push(MirArgHome::StackFrame {
                    base,
                    offset: offset.saturating_add(1),
                });
            }
            other => {
                byte_homes.push(other.home.clone());
                byte_args.push(other);
            }
        }
    }

    for arg in &byte_args {
        if !matches!(arg.home, MirArgHome::Reg(_)) {
            materialize_call_arg(arg, out);
        }
    }
    for arg in &byte_args {
        if matches!(arg.home, MirArgHome::Reg(_)) {
            materialize_call_arg(arg, out);
        }
    }
    let materialized_args = byte_args
        .iter()
        .map(materialized_call_arg_summary)
        .collect::<Vec<_>>();
    out.push(MirOp::Call {
        target,
        abi: MirCallAbi {
            params: byte_homes,
            result: None,
            clobbers: abi.clobbers,
            preserves: abi.preserves,
        },
        args: materialized_args,
        result: None,
        effects,
    });
    if let Some(result) = result {
        materialize_call_result(result.dst, result.width, result.home, out);
    }
}

fn materialize_call_arg(arg: &MirCallArg, out: &mut Vec<MirOp>) {
    match arg.home {
        MirArgHome::Reg(reg) => materialize_call_arg_to_reg(arg.value.clone(), reg, out),
        MirArgHome::StackFrame { base, offset } => {
            materialize_call_arg_to_mem(
                arg.value.clone(),
                MirMem::Absolute(base.saturating_add(offset)),
                out,
            );
        }
        MirArgHome::FixedZeroPage(slot) => {
            materialize_call_arg_to_mem(arg.value.clone(), MirMem::FixedZeroPage(slot), out);
        }
        MirArgHome::Absolute(address) => {
            materialize_call_arg_to_mem(arg.value.clone(), MirMem::Absolute(address), out);
        }
        MirArgHome::RegisterPair { .. } | MirArgHome::BytePair { .. } | MirArgHome::ZeroPage(_) => {
        }
    }
}

fn materialize_call_arg_to_reg(value: MirValue, reg: MirReg, out: &mut Vec<MirOp>) {
    match value {
        MirValue::PointerCell(mem) => out.push(MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }),
        value => out.push(MirOp::Move {
            dst: MirDef::Reg(reg),
            src: value,
            width: MirWidth::Byte,
        }),
    }
}

fn materialize_call_arg_to_mem(value: MirValue, dst: MirMem, out: &mut Vec<MirOp>) {
    let src = match value {
        MirValue::PointerCell(mem) => {
            out.push(MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            });
            MirValue::Def(MirDef::Reg(MirReg::A))
        }
        value => value,
    };
    out.push(MirOp::Store {
        dst: MirAddr::Direct(dst),
        src,
        width: MirWidth::Byte,
    });
}

fn materialized_call_arg_summary(arg: &MirCallArg) -> MirCallArg {
    match arg.home {
        MirArgHome::Reg(reg) => MirCallArg {
            value: MirValue::Def(MirDef::Reg(reg)),
            width: MirWidth::Byte,
            home: MirArgHome::Reg(reg),
        },
        MirArgHome::StackFrame { base, offset } => MirCallArg {
            value: call_stack_arg_summary_value(&arg.value),
            width: MirWidth::Byte,
            home: MirArgHome::StackFrame { base, offset },
        },
        MirArgHome::FixedZeroPage(slot) => MirCallArg {
            value: call_stack_arg_summary_value(&arg.value),
            width: MirWidth::Byte,
            home: MirArgHome::FixedZeroPage(slot),
        },
        MirArgHome::Absolute(address) => MirCallArg {
            value: call_stack_arg_summary_value(&arg.value),
            width: MirWidth::Byte,
            home: MirArgHome::Absolute(address),
        },
        MirArgHome::RegisterPair { .. } | MirArgHome::BytePair { .. } | MirArgHome::ZeroPage(_) => {
            arg.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct CallResultStoreRewriteCandidate {
    pub result_temp: MirTempId,
    pub result_width: MirWidth,
    pub return_slot: MirFixedZpSlot,
    pub replacement: [MirOp; 2],
}

pub(in crate::mir6502) fn call_result_store_rewrite_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<CallResultStoreRewriteCandidate> {
    let MirOp::Call {
        target,
        abi,
        args,
        result: Some(result),
        effects,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirOp::Store {
        dst,
        src: MirValue::Def(store_src),
        width,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let MirResultHome::ReturnSlot { offset } = result.home else {
        return None;
    };
    if store_src != &result.dst
        || width != &result.width
        || !call_result_store_addr_supported(result.width, dst)
    {
        return None;
    }
    let result_temp = split_def_as_temp(&result.dst)?;
    let return_slot = match return_slot_mem(offset) {
        MirMem::FixedZeroPage(slot) => slot,
        _ => unreachable!("return slots use fixed zero page"),
    };
    Some(CallResultStoreRewriteCandidate {
        result_temp,
        result_width: result.width,
        return_slot,
        replacement: [
            MirOp::Call {
                target: target.clone(),
                abi: abi.clone(),
                args: args.clone(),
                result: None,
                effects: effects.clone(),
            },
            MirOp::Store {
                dst: dst.clone(),
                src: call_result_value(result.width, result.home.clone())?,
                width: result.width,
            },
        ],
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct LoadedArgCallResultStoreRewriteCandidate {
    pub arg_temp: MirTempId,
    pub result_temp: MirTempId,
    pub result_width: MirWidth,
    pub return_slot: MirFixedZpSlot,
    pub replacement: [MirOp; 3],
}

pub(in crate::mir6502) fn loaded_arg_call_result_store_rewrite_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<LoadedArgCallResultStoreRewriteCandidate> {
    let MirOp::Load {
        dst: load_dst,
        src,
        width: MirWidth::Byte,
    } = ops.get(index)?
    else {
        return None;
    };
    let arg_temp = split_def_as_temp(load_dst)?;
    let MirOp::Call {
        target,
        abi,
        args,
        result: Some(result),
        effects,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let MirOp::Store {
        dst,
        src: MirValue::Def(store_src),
        width,
    } = ops.get(index + 2)?
    else {
        return None;
    };
    let MirResultHome::ReturnSlot { offset } = result.home else {
        return None;
    };
    if args.len() != 1
        || store_src != &result.dst
        || width != &result.width
        || result.width != MirWidth::Byte
        || !call_result_store_addr_supported(result.width, dst)
        || load_addr_reads_fixed_pair(src, super::DEST_POINTER_SCRATCH_LO)
        || count_loaded_arg_uses(target, args, arg_temp) != 1
        || !matches!(args[0].value, MirValue::Def(ref def) if def == load_dst)
    {
        return None;
    }
    let result_temp = split_def_as_temp(&result.dst)?;
    let return_slot = match return_slot_mem(offset) {
        MirMem::FixedZeroPage(slot) => slot,
        _ => unreachable!("return slots use fixed zero page"),
    };
    let mut rewritten_args = args.clone();
    rewritten_args[0].value = MirValue::Def(MirDef::Reg(MirReg::A));
    Some(LoadedArgCallResultStoreRewriteCandidate {
        arg_temp,
        result_temp,
        result_width: result.width,
        return_slot,
        replacement: [
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: src.clone(),
                width: MirWidth::Byte,
            },
            MirOp::Call {
                target: target.clone(),
                abi: abi.clone(),
                args: rewritten_args,
                result: None,
                effects: effects.clone(),
            },
            MirOp::Store {
                dst: dst.clone(),
                src: call_result_value(result.width, result.home.clone())?,
                width: result.width,
            },
        ],
    })
}

#[cfg(test)]
pub(super) fn try_fuse_call_result_store_consumer(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    peephole_stats: &mut MirPeepholeStats,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(MirOp::Call {
        target,
        abi,
        args,
        result: Some(result),
        effects,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(MirOp::Store {
        dst: store_dst,
        src: MirValue::Def(store_src),
        width,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    if store_src != &result.dst || width != &result.width {
        return 0;
    }
    if !matches!(result.home, MirResultHome::ReturnSlot { .. }) {
        return 0;
    }
    if def_is_used_after(ops, index + 2, &result.dst) {
        return 0;
    }
    if !call_result_store_addr_supported(result.width, store_dst) {
        return 0;
    }

    let prepared_dst = resolve_store_addr_producers(ops, index, store_dst.clone());
    let mut prepared_out = Vec::new();
    if let Some(prepared) = prepare_call_result_store_addr_with_delayed_index(
        result.width,
        &prepared_dst,
        routine_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        &mut prepared_out,
    ) {
        peephole_stats.record(routine_id, "call-result-ea-preserve-candidate");
        if call_preserves_prepared_store_addr(target, abi, args, effects, prepared) {
            out.extend(prepared_out);
            materialize_call(
                target.clone(),
                abi.clone(),
                args.clone(),
                None,
                effects.clone(),
                layout,
                out,
            );
            materialize_call_result_to_prepared_store_addr(
                result.width,
                result.home.clone(),
                prepared,
                layout,
                out,
            );
            peephole_stats.record(routine_id, "call-result-ea-preserve");
            return 2;
        }
        peephole_stats.record(routine_id, "call-result-ea-preserve-blocked-clobber");
    }

    materialize_call(
        target.clone(),
        abi.clone(),
        args.clone(),
        None,
        effects.clone(),
        layout,
        out,
    );
    materialize_call_result_to_store_addr(
        result.width,
        result.home.clone(),
        store_dst.clone(),
        routine_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        out,
    );
    2
}

#[cfg(test)]
pub(super) fn try_fuse_loaded_arg_call_result_store_consumer(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    peephole_stats: &mut MirPeepholeStats,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(MirOp::Load {
        dst: load_dst,
        src: load_src,
        width: MirWidth::Byte,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(load_temp) = split_def_as_temp(load_dst) else {
        return 0;
    };
    let Some(MirOp::Call {
        target,
        abi,
        args,
        result: Some(result),
        effects,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    let Some(MirOp::Store {
        dst: store_dst,
        src: MirValue::Def(store_src),
        width,
    }) = ops.get(index + 2)
    else {
        return 0;
    };
    if args.len() != 1
        || store_src != &result.dst
        || width != &result.width
        || result.width != MirWidth::Byte
        || !matches!(result.home, MirResultHome::ReturnSlot { .. })
        || def_is_used_after(ops, index + 3, &result.dst)
        || temp_is_used_after(ops, index + 2, load_temp)
        || !call_result_store_addr_supported(result.width, store_dst)
        || load_addr_reads_fixed_pair(load_src, super::DEST_POINTER_SCRATCH_LO)
    {
        return 0;
    }
    if count_loaded_arg_uses(target, args, load_temp) != 1 {
        return 0;
    }
    if !matches!(args[0].value, MirValue::Def(ref def) if def == load_dst) {
        return 0;
    }

    let prepared_dst = resolve_store_addr_producers(ops, index, store_dst.clone());
    let mut prepared_out = Vec::new();
    let Some(prepared) = prepare_call_result_store_addr_with_delayed_index(
        result.width,
        &prepared_dst,
        routine_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        &mut prepared_out,
    ) else {
        return 0;
    };
    peephole_stats.record(routine_id, "call-result-ea-preserve-loaded-arg-candidate");
    if !call_preserves_prepared_store_addr(target, abi, args, effects, prepared) {
        peephole_stats.record(
            routine_id,
            "call-result-ea-preserve-loaded-arg-blocked-clobber",
        );
        return 0;
    }

    out.extend(prepared_out);
    materialize_byte_load_addr_to_a(
        load_src.clone(),
        routine_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        out,
    );
    let mut rewritten_args = args.clone();
    rewritten_args[0].value = MirValue::Def(MirDef::Reg(MirReg::A));
    materialize_call(
        target.clone(),
        abi.clone(),
        rewritten_args,
        None,
        effects.clone(),
        layout,
        out,
    );
    materialize_call_result_to_prepared_store_addr(
        result.width,
        result.home.clone(),
        prepared,
        layout,
        out,
    );
    peephole_stats.record(routine_id, "call-result-ea-preserve-loaded-arg");
    3
}

pub(super) fn try_materialize_forwarded_call_result_store(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    peephole_stats: &mut MirPeepholeStats,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(MirOp::Call {
        target,
        abi,
        args,
        result: None,
        effects,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(MirOp::Store { dst, src, width }) = ops.get(index + 1) else {
        return 0;
    };
    let Some(home) = abi.result.clone() else {
        return 0;
    };
    if call_result_value(*width, home.clone()).as_ref() != Some(src)
        || !call_result_store_addr_supported(*width, dst)
    {
        return 0;
    }

    let prepared_dst = resolve_store_addr_producers(ops, index, dst.clone());
    let mut prepared_out = Vec::new();
    if let Some(prepared) = prepare_call_result_store_addr_with_delayed_index(
        *width,
        &prepared_dst,
        routine_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        &mut prepared_out,
    ) {
        peephole_stats.record(routine_id, "call-result-ea-preserve-candidate");
        if call_preserves_prepared_store_addr(target, abi, args, effects, prepared) {
            out.extend(prepared_out);
            materialize_call(
                target.clone(),
                abi.clone(),
                args.clone(),
                None,
                effects.clone(),
                layout,
                out,
            );
            materialize_call_result_to_prepared_store_addr(*width, home, prepared, layout, out);
            peephole_stats.record(routine_id, "call-result-ea-preserve");
            return 2;
        }
        peephole_stats.record(routine_id, "call-result-ea-preserve-blocked-clobber");
    }

    materialize_call(
        target.clone(),
        abi.clone(),
        args.clone(),
        None,
        effects.clone(),
        layout,
        out,
    );
    materialize_call_result_to_store_addr(
        *width,
        home,
        dst.clone(),
        routine_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        out,
    );
    2
}

pub(super) fn try_materialize_loaded_arg_forwarded_call_result_store(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    peephole_stats: &mut MirPeepholeStats,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src,
        width: MirWidth::Byte,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(MirOp::Call {
        target,
        abi,
        args,
        result: None,
        effects,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    let Some(MirOp::Store {
        dst,
        src: store_src,
        width: MirWidth::Byte,
    }) = ops.get(index + 2)
    else {
        return 0;
    };
    let Some(home) = abi.result.clone() else {
        return 0;
    };
    if args.len() != 1
        || args[0].value != MirValue::Def(MirDef::Reg(MirReg::A))
        || call_result_value(MirWidth::Byte, home.clone()).as_ref() != Some(store_src)
    {
        return 0;
    }

    let prepared_dst = resolve_store_addr_producers(ops, index, dst.clone());
    let mut prepared_out = Vec::new();
    let Some(prepared) = prepare_call_result_store_addr_with_delayed_index(
        MirWidth::Byte,
        &prepared_dst,
        routine_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        &mut prepared_out,
    ) else {
        return 0;
    };
    peephole_stats.record(routine_id, "call-result-ea-preserve-loaded-arg-candidate");
    if !call_preserves_prepared_store_addr(target, abi, args, effects, prepared) {
        peephole_stats.record(
            routine_id,
            "call-result-ea-preserve-loaded-arg-blocked-clobber",
        );
        return 0;
    }

    out.extend(prepared_out);
    materialize_byte_load_addr_to_a(
        src.clone(),
        routine_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        out,
    );
    materialize_call(
        target.clone(),
        abi.clone(),
        args.clone(),
        None,
        effects.clone(),
        layout,
        out,
    );
    materialize_call_result_to_prepared_store_addr(MirWidth::Byte, home, prepared, layout, out);
    peephole_stats.record(routine_id, "call-result-ea-preserve-loaded-arg");
    3
}

fn count_loaded_arg_uses(target: &MirCallTarget, args: &[MirCallArg], temp: MirTempId) -> usize {
    let mut uses = 0usize;
    count_call_target_temp_uses(target, temp, &mut uses);
    for arg in args {
        count_value_temp_uses(&arg.value, temp, &mut uses);
    }
    uses
}

fn prepare_call_result_store_addr_with_delayed_index(
    width: MirWidth,
    dst: &MirAddr,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    out: &mut Vec<MirOp>,
) -> Option<PreparedStoreAddress> {
    if let Some(prepared) =
        prepare_delayed_index_store_addr(width, dst, delayed_byte_indexes, layout, out)
    {
        return Some(prepared);
    }
    prepare_call_result_store_addr(width, dst, routine_id, layout, temp_widths, out)
}

fn prepare_delayed_index_store_addr(
    width: MirWidth,
    dst: &MirAddr,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> Option<PreparedStoreAddress> {
    let consumer = super::DEST_POINTER_PAIR;
    match dst {
        MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. } => {
            let parts = indexed_addr_parts(dst)?;
            checked_indirect_offset(parts.offset, width)?;
            indexed_addr_has_delayed_index(&parts, delayed_byte_indexes).then_some(())?;
            materialize_indexed_address_for_consumer(
                parts.clone(),
                consumer,
                layout,
                Some(delayed_byte_indexes),
                out,
            );
            Some(PreparedStoreAddress {
                consumer,
                offset: parts.offset,
            })
        }
        _ => None,
    }
}

fn checked_indirect_offset(offset: u16, width: MirWidth) -> Option<u16> {
    let last_byte = offset.checked_add(width_bytes(width).saturating_sub(1))?;
    (last_byte <= u8::MAX as u16).then_some(offset)
}

fn materialize_byte_load_addr_to_a(
    src: MirAddr,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    out: &mut Vec<MirOp>,
) {
    match src {
        MirAddr::Deref { ptr, offset } => super::materialize_pointer_deref_read_byte(
            MirDef::Reg(MirReg::A),
            ptr,
            offset,
            routine_id,
            layout,
            temp_widths,
            out,
        ),
        MirAddr::PointerCell { ptr, offset } => super::materialize_pointer_deref_read_byte(
            MirDef::Reg(MirReg::A),
            pointer_value_from_mem(&ptr),
            offset,
            routine_id,
            layout,
            temp_widths,
            out,
        ),
        MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. } => {
            let parts = indexed_addr_parts(&src).expect("indexed byte load matched above");
            materialize_indexed_read_to_def(
                MirDef::Reg(MirReg::A),
                parts,
                MirWidth::Byte,
                layout,
                Some(delayed_byte_indexes),
                out,
            );
        }
        other => out.push(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: other,
            width: MirWidth::Byte,
        }),
    }
}

fn load_addr_reads_fixed_pair(addr: &MirAddr, lo: u8) -> bool {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            value_reads_fixed_pair(base, lo) || value_reads_fixed_pair(index, lo)
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            mem_is_fixed_pair(ptr, lo) || value_reads_fixed_pair(index, lo)
        }
        MirAddr::PointerCell { ptr, .. } => mem_is_fixed_pair(ptr, lo),
        MirAddr::Deref { ptr, .. } => value_reads_fixed_pair(ptr, lo),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => false,
    }
}

fn value_reads_fixed_pair(value: &MirValue, lo: u8) -> bool {
    match value {
        MirValue::PointerCell(mem) => mem_is_fixed_pair(mem, lo),
        MirValue::Word { lo: low, hi } => {
            value_reads_fixed_pair(low, lo) || value_reads_fixed_pair(hi, lo)
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => false,
    }
}

fn mem_is_fixed_pair(mem: &MirMem, lo: u8) -> bool {
    matches!(mem, MirMem::FixedZeroPage(slot) if slot.0 == lo || slot.0 == lo.saturating_add(1))
}

fn resolve_store_addr_producers(ops: &[MirOp], use_index: usize, addr: MirAddr) -> MirAddr {
    match addr {
        MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset,
        } => MirAddr::ComputedIndex {
            base: resolve_prepared_base_producer(ops, use_index, base),
            index,
            elem_size,
            offset,
        },
        MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset,
        } => MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset,
        },
        MirAddr::Deref { ptr, offset } => MirAddr::Deref {
            ptr: resolve_prepared_base_producer(ops, use_index, ptr),
            offset,
        },
        other => other,
    }
}

fn resolve_prepared_base_producer(ops: &[MirOp], use_index: usize, value: MirValue) -> MirValue {
    let MirValue::Def(MirDef::VTemp(temp)) = value else {
        return value;
    };
    let Some((producer_index, producer)) = find_temp_producer(ops, use_index, temp) else {
        return MirValue::Def(MirDef::VTemp(temp));
    };
    match producer {
        MirOp::Load {
            src: MirAddr::Direct(mem),
            width: MirWidth::Word,
            ..
        } if prepared_producer_mem_is_stable_source(mem)
            && mem_is_stable_until(ops, producer_index + 1, use_index, mem)
            && mem_is_stable_until(ops, producer_index + 1, use_index, &offset_mem(mem, 1)) =>
        {
            pointer_value_from_mem(mem)
        }
        MirOp::LeaAddr {
            target,
            width: MirWidth::Word,
            ..
        } => storage_address_value(target),
        _ => MirValue::Def(MirDef::VTemp(temp)),
    }
}

fn find_temp_producer(ops: &[MirOp], use_index: usize, temp: MirTempId) -> Option<(usize, &MirOp)> {
    ops[..use_index]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, op)| op_def(op).and_then(split_def_as_temp) == Some(temp))
}

fn mem_is_stable_until(ops: &[MirOp], start: usize, end: usize, mem: &MirMem) -> bool {
    ops[start..end].iter().all(|op| !op_may_write_mem(op, mem))
}

fn prepared_producer_mem_is_stable_source(mem: &MirMem) -> bool {
    !matches!(
        mem,
        MirMem::Spill { .. } | MirMem::ZeroPage(_) | MirMem::FixedZeroPage(_)
    )
}

fn materialize_call_target(
    target: MirCallTarget,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> MirCallTarget {
    let MirCallTarget::Indirect { target, width } = target else {
        return target;
    };
    let (lo, hi) = split_value_as_word(target, layout);
    out.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
            INDIRECT_CALL_TARGET_LO,
        ))),
        src: lo,
        width: MirWidth::Byte,
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
            INDIRECT_CALL_TARGET_HI,
        ))),
        src: hi,
        width: MirWidth::Byte,
    });
    MirCallTarget::Indirect {
        target: MirValue::Word {
            lo: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
                MirFixedZpSlot(INDIRECT_CALL_TARGET_LO),
            ))),
            hi: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
                MirFixedZpSlot(INDIRECT_CALL_TARGET_HI),
            ))),
        },
        width,
    }
}

fn call_stack_arg_summary_value(value: &MirValue) -> MirValue {
    match value {
        MirValue::ConstU8(_) | MirValue::Def(MirDef::Reg(_)) => value.clone(),
        _ => MirValue::ConstU8(0),
    }
}

fn materialize_call_result(
    dst: MirDef,
    width: MirWidth,
    home: MirResultHome,
    out: &mut Vec<MirOp>,
) {
    let MirResultHome::ReturnSlot { offset } = home else {
        return;
    };
    match width {
        MirWidth::Byte => out.push(MirOp::Load {
            dst,
            src: MirAddr::Direct(return_slot_mem(offset)),
            width: MirWidth::Byte,
        }),
        MirWidth::Word => {
            if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                out.push(MirOp::Load {
                    dst: lo_dst,
                    src: MirAddr::Direct(return_slot_mem(offset)),
                    width: MirWidth::Byte,
                });
                out.push(MirOp::Load {
                    dst: hi_dst,
                    src: MirAddr::Direct(return_slot_mem(offset.saturating_add(1))),
                    width: MirWidth::Byte,
                });
            } else {
                out.push(MirOp::Load {
                    dst,
                    src: MirAddr::Direct(return_slot_mem(offset)),
                    width,
                });
            }
        }
    }
}
