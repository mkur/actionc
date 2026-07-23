use super::indexes::{
    DelayedByteIndexPlan, indexed_addr_has_delayed_index, indexed_addr_parts,
    materialize_indexed_address_for_consumer, materialize_indexed_byte_read_to_a,
    materialize_indexed_write_from_value,
};
use super::*;
use crate::mir6502::analysis::effects::MirFlagSet;
use crate::mir6502::ir::{MirRegisterSet, MirRoutine};
use crate::mir6502::rewrite::context::{MirExitStateChange, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::MirPostHomeRewritePlan;
use crate::mir6502::rewrite::posthome::structural_plan;

#[cfg(test)]
pub(super) fn try_materialize_store_expr_producers(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    try_materialize_store_expr_producers_with_deadness(
        ops, index, terminator, config, layout, true, out,
    )
}

pub(super) fn select_store_expr_producers(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    try_materialize_store_expr_producers_with_deadness(
        ops, index, terminator, config, layout, false, out,
    )
}

fn try_materialize_store_expr_producers_with_deadness(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(plan) =
        collect_store_expr_plan(ops, index, terminator, layout, require_local_deadness)
    else {
        return 0;
    };
    let consumed = plan.consumed;
    materialize_store_expr_plan(plan, config, layout, out);
    consumed
}

#[derive(Debug, Clone)]
enum StoreExpr {
    Value {
        value: MirValue,
        width: MirWidth,
    },
    Binary {
        op: MirBinaryOp,
        left: Box<StoreExpr>,
        right: Box<StoreExpr>,
        width: MirWidth,
    },
}

#[derive(Debug, Clone)]
struct StoreExprPlan {
    consumed: usize,
    dst: MirMem,
    expr: StoreExpr,
    width: MirWidth,
}

fn collect_store_expr_plan(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    layout: &MaterializeLayout,
    require_local_deadness: bool,
) -> Option<StoreExprPlan> {
    let mut exprs = BTreeMap::<MirTempId, StoreExpr>::new();
    let mut cursor = index;
    while let Some((temp, expr)) = store_expr_producer(ops.get(cursor)?, &exprs, layout) {
        exprs.insert(temp, expr);
        cursor += 1;
    }
    if exprs.is_empty() {
        return None;
    }

    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width,
    } = ops.get(cursor)?
    else {
        return None;
    };
    let root_temp = split_def_as_temp(store_src)?;
    let expr = exprs.get(&root_temp)?.clone();
    if !matches!(expr, StoreExpr::Binary { .. }) {
        return None;
    }
    if !store_expr_width_can_store(&expr, *width) {
        return None;
    }
    if *width == MirWidth::Byte
        && store_expr_width(&expr) == MirWidth::Word
        && binary_flags_may_be_live_after(ops, cursor.saturating_add(1), terminator)
    {
        return None;
    }
    for temp in exprs.keys().copied() {
        if (require_local_deadness && temp_is_used_after(ops, cursor.saturating_add(1), temp))
            || !store_expr_temp_has_single_consumer(ops, index, cursor, temp)
        {
            return None;
        }
    }

    Some(StoreExprPlan {
        consumed: cursor + 1 - index,
        dst: store_dst.clone(),
        expr,
        width: *width,
    })
}

fn store_expr_producer(
    op: &MirOp,
    exprs: &BTreeMap<MirTempId, StoreExpr>,
    layout: &MaterializeLayout,
) -> Option<(MirTempId, StoreExpr)> {
    match op {
        MirOp::LoadImm { dst, value, width } => {
            let value = match width {
                MirWidth::Byte => MirValue::ConstU8(*value as u8),
                MirWidth::Word => MirValue::ConstU16(*value),
            };
            Some((
                split_def_as_temp(dst)?,
                StoreExpr::Value {
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
            StoreExpr::Value {
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
            StoreExpr::Value {
                value: pointer_value_from_mem(mem),
                width: MirWidth::Word,
            },
        )),
        MirOp::Move { dst, src, width } => Some((
            split_def_as_temp(dst)?,
            StoreExpr::Value {
                value: store_expr_plain_value(src, exprs, layout)?,
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
        } if store_expr_binary_op_is_supported(*op, *width)
            && store_expr_binary_carry_is_safe(*op, *width, *carry_in) =>
        {
            let mut left = store_expr_operand(left, exprs, layout)?;
            let mut right = store_expr_operand(right, exprs, layout)?;
            if !store_expr_binary_operands_supported(*op, *width, &left, &right) {
                if store_expr_binary_op_is_commutative(*op)
                    && store_expr_binary_operands_supported(*op, *width, &right, &left)
                {
                    std::mem::swap(&mut left, &mut right);
                } else {
                    return None;
                }
            }
            Some((
                split_def_as_temp(dst)?,
                StoreExpr::Binary {
                    op: *op,
                    left: Box::new(left),
                    right: Box::new(right),
                    width: *width,
                },
            ))
        }
        _ => None,
    }
}

fn store_expr_binary_op_is_supported(op: MirBinaryOp, width: MirWidth) -> bool {
    match width {
        MirWidth::Byte => matches!(
            op,
            MirBinaryOp::Add
                | MirBinaryOp::Sub
                | MirBinaryOp::And
                | MirBinaryOp::Or
                | MirBinaryOp::Xor
        ),
        MirWidth::Word => matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub),
    }
}

fn store_expr_binary_op_is_commutative(op: MirBinaryOp) -> bool {
    matches!(
        op,
        MirBinaryOp::Add | MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor
    )
}

fn store_expr_operand(
    value: &MirValue,
    exprs: &BTreeMap<MirTempId, StoreExpr>,
    layout: &MaterializeLayout,
) -> Option<StoreExpr> {
    if let Some(temp) = store_expr_value_as_temp(value)
        && let Some(expr) = exprs.get(&temp)
    {
        return Some(expr.clone());
    }
    Some(StoreExpr::Value {
        value: store_expr_plain_value(value, exprs, layout)?,
        width: store_expr_value_width(value),
    })
}

fn store_expr_plain_value(
    value: &MirValue,
    exprs: &BTreeMap<MirTempId, StoreExpr>,
    layout: &MaterializeLayout,
) -> Option<MirValue> {
    if let Some(temp) = store_expr_value_as_temp(value) {
        return store_expr_as_plain_value(exprs.get(&temp)?, layout);
    }
    if value_uses_temp(value) {
        None
    } else {
        Some(value.clone())
    }
}

fn store_expr_as_plain_value(expr: &StoreExpr, layout: &MaterializeLayout) -> Option<MirValue> {
    match expr {
        StoreExpr::Value { value, .. } => Some(value.clone()),
        StoreExpr::Binary { .. } => {
            let (lo, hi) = store_expr_word_byte_values(expr, layout)?;
            Some(MirValue::Word {
                lo: Box::new(lo),
                hi: Box::new(hi),
            })
        }
    }
}

fn store_expr_value_width(value: &MirValue) -> MirWidth {
    match value {
        MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::PointerCell(_) => MirWidth::Word,
        MirValue::Word { .. } => MirWidth::Word,
        _ => MirWidth::Byte,
    }
}

fn store_expr_value_as_temp(value: &MirValue) -> Option<MirTempId> {
    match value {
        MirValue::Def(MirDef::VTemp(temp)) => Some(*temp),
        _ => None,
    }
}

fn store_expr_binary_carry_is_safe(
    op: MirBinaryOp,
    width: MirWidth,
    carry_in: Option<MirCarryIn>,
) -> bool {
    match width {
        MirWidth::Word => carry_in.is_none(),
        MirWidth::Byte => {
            let expected = match op {
                MirBinaryOp::Add => Some(MirCarryIn::Clear),
                MirBinaryOp::Sub => Some(MirCarryIn::Set),
                _ => None,
            };
            carry_in.is_none() || carry_in == expected
        }
    }
}

fn store_expr_binary_operands_supported(
    op: MirBinaryOp,
    width: MirWidth,
    left: &StoreExpr,
    right: &StoreExpr,
) -> bool {
    match width {
        MirWidth::Byte => {
            matches!(right, StoreExpr::Value { .. })
                && (matches!(left, StoreExpr::Value { .. })
                    || matches!(
                        left,
                        StoreExpr::Binary {
                            width: MirWidth::Byte,
                            ..
                        }
                    ))
        }
        MirWidth::Word => {
            matches!(left, StoreExpr::Value { .. })
                && matches!(right, StoreExpr::Value { .. })
                && matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub)
        }
    }
}

fn store_expr_width_can_store(expr: &StoreExpr, store_width: MirWidth) -> bool {
    let expr_width = store_expr_width(expr);
    expr_width == store_width || store_width == MirWidth::Byte
}

fn store_expr_width(expr: &StoreExpr) -> MirWidth {
    match expr {
        StoreExpr::Value { width, .. } | StoreExpr::Binary { width, .. } => *width,
    }
}

fn store_expr_temp_has_single_consumer(
    ops: &[MirOp],
    start: usize,
    store_index: usize,
    temp: MirTempId,
) -> bool {
    let mut uses = 0usize;
    for op in &ops[start..=store_index] {
        if op_uses_temp_more_than_once(op, temp) {
            return false;
        }
        if op_uses_temp(op, temp) {
            uses += 1;
        }
    }
    uses == 1
}

fn materialize_store_expr_plan(
    plan: StoreExprPlan,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    match plan.width {
        MirWidth::Byte => materialize_store_expr_byte_to_mem(&plan.expr, plan.dst, layout, out),
        MirWidth::Word => {
            materialize_store_expr_word_to_mem(plan.expr, plan.dst, config, layout, out)
        }
    }
}

fn materialize_store_expr_byte_to_mem(
    expr: &StoreExpr,
    dst: MirMem,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    materialize_store_expr_byte_to_a(expr, layout, out);
    out.push(MirOp::Store {
        dst: MirAddr::Direct(dst),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
}

fn materialize_store_expr_byte_to_a(
    expr: &StoreExpr,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    match expr {
        StoreExpr::Value { .. } => {
            let Some(value) = store_expr_low_value(expr, layout) else {
                return;
            };
            out.push(MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: value,
                width: MirWidth::Byte,
            });
        }
        StoreExpr::Binary {
            op,
            left,
            right,
            width: _,
        } => {
            let Some(right) = store_expr_low_value(right, layout) else {
                return;
            };
            materialize_store_expr_byte_to_a(left, layout, out);
            out.push(MirOp::Binary {
                op: *op,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right,
                width: MirWidth::Byte,
                carry_in: explicit_byte_carry(*op, None),
                carry_out: MirCarryOut::Ignore,
            });
        }
    }
}

fn materialize_store_expr_word_to_mem(
    expr: StoreExpr,
    dst: MirMem,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    match expr {
        StoreExpr::Value {
            value,
            width: MirWidth::Word,
        } => materialize_value_to_mem_for_width(value, MirWidth::Word, dst, layout, out),
        StoreExpr::Value {
            value,
            width: MirWidth::Byte,
        } => {
            materialize_value_to_mem(value, dst.clone(), out);
            materialize_value_to_mem(MirValue::ConstU8(0), offset_mem(&dst, 1), out);
        }
        StoreExpr::Binary {
            op,
            left,
            right,
            width: MirWidth::Word,
        } => {
            let left_width = store_expr_width(&left);
            let right_width = store_expr_width(&right);
            let Some(left) = store_expr_as_plain_value(&left, layout) else {
                return;
            };
            let Some(right) = store_expr_as_plain_value(&right, layout) else {
                return;
            };
            match (left_width, right_width, op) {
                (MirWidth::Byte, MirWidth::Byte, MirBinaryOp::Add | MirBinaryOp::Sub) => {
                    materialize_byte_byte_binary_word_store_consumer(op, dst, left, right, out);
                    return;
                }
                (MirWidth::Word, MirWidth::Byte, MirBinaryOp::Add | MirBinaryOp::Sub) => {
                    materialize_word_byte_binary_store_consumer(
                        op, dst, left, right, config, layout, out,
                    );
                    return;
                }
                (MirWidth::Byte, MirWidth::Word, MirBinaryOp::Add) => {
                    materialize_word_byte_binary_store_consumer(
                        op, dst, right, left, config, layout, out,
                    );
                    return;
                }
                _ => {}
            }
            materialize_word_binary_store_consumer(op, dst, left, right, config, layout, out);
        }
        StoreExpr::Binary { .. } => {
            materialize_store_expr_byte_to_mem(&expr, dst.clone(), layout, out);
            materialize_value_to_mem(MirValue::ConstU8(0), offset_mem(&dst, 1), out);
        }
    }
}

fn store_expr_word_byte_values(
    expr: &StoreExpr,
    layout: &MaterializeLayout,
) -> Option<(MirValue, MirValue)> {
    match expr {
        StoreExpr::Value { value, width } => match width {
            MirWidth::Byte => Some((value.clone(), MirValue::ConstU8(0))),
            MirWidth::Word => Some(split_value(value.clone(), layout)),
        },
        StoreExpr::Binary { .. } => None,
    }
}

fn store_expr_low_value(expr: &StoreExpr, layout: &MaterializeLayout) -> Option<MirValue> {
    store_expr_word_byte_values(expr, layout).map(|(lo, _)| lo)
}

pub(super) fn try_fuse_cast_store_consumer(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    if let Some(consumed) = try_fuse_loaded_extend_store_consumer(ops, index, out) {
        return consumed;
    }
    if let Some(consumed) = try_fuse_loaded_truncate_store_consumer(ops, index, out) {
        return consumed;
    }

    let Some(cast) = ops.get(index) else {
        return 0;
    };
    let Some(store) = ops.get(index + 1) else {
        return 0;
    };
    match (cast, store) {
        (
            MirOp::Extend {
                dst,
                src,
                from_width: MirWidth::Byte,
                to_width: MirWidth::Word,
                signed: _,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst),
                src: MirValue::Def(store_src),
                width: MirWidth::Word,
            },
        ) if store_src == dst && !value_uses_temp(src) => {
            materialize_extend_store_consumer(store_dst.clone(), src.clone(), out);
            2
        }
        (
            MirOp::Truncate {
                dst,
                src,
                from_width: MirWidth::Word,
                to_width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst),
                src: MirValue::Def(store_src),
                width: MirWidth::Byte,
            },
        ) if store_src == dst && !value_uses_temp(src) => {
            let (lo, _hi) = split_value_as_word(src.clone(), layout);
            materialize_value_to_mem(lo, store_dst.clone(), out);
            2
        }
        _ => 0,
    }
}

fn try_fuse_loaded_extend_store_consumer(
    ops: &[MirOp],
    index: usize,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let load = ops.get(index)?;
    let cast = ops.get(index + 1)?;
    let store = ops.get(index + 2)?;
    let MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width: MirWidth::Byte,
    } = load
    else {
        return None;
    };
    let load_temp = split_def_as_temp(load_dst)?;
    let MirOp::Extend {
        dst: cast_dst,
        src,
        from_width: MirWidth::Byte,
        to_width: MirWidth::Word,
        signed: _,
    } = cast
    else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    } = store
    else {
        return None;
    };
    if store_src != cast_dst {
        return None;
    }
    let producer = MirValue::PointerCell(load_src.clone());
    let src = replace_temp_value(src.clone(), load_temp, &producer);
    if value_uses_temp(&src) {
        return None;
    }
    materialize_extend_store_consumer(store_dst.clone(), src, out);
    Some(3)
}

fn try_fuse_loaded_truncate_store_consumer(
    ops: &[MirOp],
    index: usize,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let load = ops.get(index)?;
    let cast = ops.get(index + 1)?;
    let store = ops.get(index + 2)?;
    let MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width: MirWidth::Word,
    } = load
    else {
        return None;
    };
    let load_temp = split_def_as_temp(load_dst)?;
    let MirOp::Truncate {
        dst: cast_dst,
        src,
        from_width: MirWidth::Word,
        to_width: MirWidth::Byte,
    } = cast
    else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Byte,
    } = store
    else {
        return None;
    };
    if store_src != cast_dst {
        return None;
    }
    let producer = pointer_value_from_mem(load_src);
    let src = replace_temp_value(src.clone(), load_temp, &producer);
    let (lo, _hi) = match src {
        MirValue::Word { lo, hi } => (*lo, *hi),
        other => (other, MirValue::ConstU8(0)),
    };
    if value_uses_temp(&lo) {
        return None;
    }
    materialize_value_to_mem(lo, store_dst.clone(), out);
    Some(3)
}

fn materialize_extend_store_consumer(dst: MirMem, src: MirValue, out: &mut Vec<MirOp>) {
    materialize_value_to_mem(src, dst.clone(), out);
    materialize_value_to_mem(MirValue::ConstU8(0), offset_mem(&dst, 1), out);
}

pub(super) fn materialize_value_to_mem(value: MirValue, dst: MirMem, out: &mut Vec<MirOp>) {
    out.push(MirOp::Move {
        dst: MirDef::Reg(MirReg::A),
        src: value,
        width: MirWidth::Byte,
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(dst),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
}

#[cfg(test)]
pub(super) fn try_fuse_direct_copy_store_consumer(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    try_fuse_direct_copy_store_consumer_with_deadness(ops, index, layout, true, out)
}

pub(super) fn select_direct_copy_store_consumer(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    try_fuse_direct_copy_store_consumer_with_deadness(ops, index, layout, false, out)
}

fn try_fuse_direct_copy_store_consumer_with_deadness(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> usize {
    if let Some(consumed) =
        try_fuse_direct_load_copy_store_consumer(ops, index, layout, require_local_deadness, out)
    {
        return consumed;
    }
    if let Some(consumed) =
        try_fuse_direct_move_store_consumer(ops, index, layout, require_local_deadness, out)
    {
        return consumed;
    }
    try_fuse_direct_load_store_consumer(ops, index, layout, require_local_deadness, out)
}

fn try_fuse_direct_move_store_consumer(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let MirOp::Move { dst, src, width } = ops.get(index)? else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: store_width,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    if store_src != dst || store_width != width || value_uses_temp(src) {
        return None;
    }
    if require_local_deadness && def_is_used_after(ops, index + 2, dst) {
        return None;
    }
    materialize_value_to_mem_for_width(src.clone(), *width, store_dst.clone(), layout, out);
    Some(2)
}

fn try_fuse_direct_load_copy_store_consumer(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width: load_width,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirOp::Move {
        dst: copy_dst,
        src: MirValue::Def(copy_src),
        width: copy_width,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: store_width,
    } = ops.get(index + 2)?
    else {
        return None;
    };
    if copy_src != load_dst
        || store_src != copy_dst
        || copy_width != load_width
        || store_width != load_width
        || !direct_copy_is_safe(load_src, store_dst, *load_width)
        || (require_local_deadness && def_is_used_after(ops, index + 3, load_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 3, copy_dst))
    {
        return None;
    }
    materialize_direct_copy(
        load_src.clone(),
        store_dst.clone(),
        *load_width,
        layout,
        out,
    );
    Some(3)
}

fn try_fuse_direct_load_store_consumer(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: store_width,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    if store_src != load_dst || store_width != width {
        return 0;
    }
    if !direct_copy_is_safe(load_src, store_dst, *width) {
        return 0;
    }
    if require_local_deadness && def_is_used_after(ops, index + 2, load_dst) {
        return 0;
    }
    materialize_direct_copy(load_src.clone(), store_dst.clone(), *width, layout, out);
    2
}

fn direct_copy_is_safe(src: &MirMem, dst: &MirMem, width: MirWidth) -> bool {
    match width {
        MirWidth::Byte => true,
        MirWidth::Word => {
            let src_hi = offset_mem(src, 1);
            let dst_hi = offset_mem(dst, 1);
            src == dst || (dst != &src_hi && &dst_hi != src)
        }
    }
}

fn materialize_direct_copy(
    src: MirMem,
    dst: MirMem,
    width: MirWidth,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    materialize_value_to_mem_for_width(MirValue::PointerCell(src), width, dst, layout, out);
}

fn materialize_value_to_mem_for_width(
    value: MirValue,
    width: MirWidth,
    dst: MirMem,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    match width {
        MirWidth::Byte => materialize_value_to_mem(value, dst, out),
        MirWidth::Word => {
            let (lo, hi) = split_value_as_word(value, layout);
            materialize_value_to_mem(lo, dst.clone(), out);
            materialize_value_to_mem(hi, offset_mem(&dst, 1), out);
        }
    }
}

pub(super) fn select_word_carry_chain_store_consumer(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(MirOp::Load {
        dst: base_dst,
        src: MirAddr::Direct(base_mem),
        width: MirWidth::Word,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(base_temp) = split_def_as_temp(base_dst) else {
        return 0;
    };
    let Some(MirOp::Load {
        dst: addend_dst,
        src: MirAddr::Deref {
            ptr: addend_ptr,
            offset,
        },
        width: MirWidth::Byte,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    if addend_ptr != &MirValue::Def(MirDef::VTemp(base_temp)) {
        return 0;
    }
    let Some(addend_temp) = split_def_as_temp(addend_dst) else {
        return 0;
    };

    let Some(MirOp::Binary {
        op: first_op @ (MirBinaryOp::Add | MirBinaryOp::Sub),
        dst: first_dst,
        left: first_left,
        right: first_right,
        width: MirWidth::Word,
        carry_in: None,
        carry_out: MirCarryOut::Ignore,
    }) = ops.get(index + 2)
    else {
        return 0;
    };
    let Some(mut result_temp) = split_def_as_temp(first_dst) else {
        return 0;
    };
    let base_value = MirValue::Def(MirDef::VTemp(base_temp));
    let addend_value = MirValue::Def(MirDef::VTemp(addend_temp));
    let first_operands_match = match first_op {
        MirBinaryOp::Add => {
            (first_left == &base_value && first_right == &addend_value)
                || (first_left == &addend_value && first_right == &base_value)
        }
        MirBinaryOp::Sub => first_left == &base_value && first_right == &addend_value,
        _ => false,
    };
    if !first_operands_match {
        return 0;
    }

    let mut cursor = index + 3;
    let mut tail = Vec::<(MirBinaryOp, MirValue)>::new();
    while let Some(MirOp::Binary {
        op: op @ (MirBinaryOp::Add | MirBinaryOp::Sub),
        dst,
        left,
        right,
        width: MirWidth::Word,
        carry_in: None,
        carry_out: MirCarryOut::Ignore,
    }) = ops.get(cursor)
    {
        let previous = MirValue::Def(MirDef::VTemp(result_temp));
        let operand = if left == &previous && word_carry_chain_tail_value(right) {
            right.clone()
        } else if *op == MirBinaryOp::Add && right == &previous && word_carry_chain_tail_value(left)
        {
            left.clone()
        } else {
            break;
        };
        let Some(next_temp) = split_def_as_temp(dst) else {
            break;
        };
        tail.push((*op, operand));
        result_temp = next_temp;
        cursor += 1;
    }

    let Some(MirOp::Store {
        dst: MirAddr::Direct(target),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    }) = ops.get(cursor)
    else {
        return 0;
    };
    if split_def_as_temp(store_src) != Some(result_temp)
        || !word_carry_chain_target_supported(target)
    {
        return 0;
    }
    let followup = word_carry_chain_deref_store(ops, cursor + 1, result_temp);

    let source_value = pointer_value_from_mem(base_mem);
    out.push(MirOp::MaterializeAddress {
        consumer: DEFAULT_POINTER_PAIR,
        value: source_value,
    });
    out.push(MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset: *offset,
    });
    let addend = MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_INDEX_SCRATCH_LO));
    out.push(MirOp::Store {
        dst: MirAddr::Direct(addend.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });

    let accumulator = MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO));
    materialize_word_carry_chain_byte_update(
        *first_op,
        accumulator.clone(),
        MirValue::PointerCell(addend),
        config,
        layout,
        out,
    );
    for (op, operand) in tail {
        materialize_word_binary_store_consumer(
            op,
            accumulator.clone(),
            pointer_value_from_mem(&accumulator),
            operand,
            config,
            layout,
            out,
        );
    }
    materialize_value_to_mem_for_width(
        pointer_value_from_mem(&accumulator),
        MirWidth::Word,
        target.clone(),
        layout,
        out,
    );
    if let Some((offset, followup_target)) = &followup {
        out.push(MirOp::LoadIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            dst: MirDef::Reg(MirReg::A),
            offset: *offset,
        });
        out.push(MirOp::Store {
            dst: MirAddr::Direct(followup_target.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        });
    }
    cursor + 1 + usize::from(followup.is_some()) * 2 - index
}

fn word_carry_chain_deref_store(
    ops: &[MirOp],
    index: usize,
    pointer_temp: MirTempId,
) -> Option<(u16, MirMem)> {
    let MirOp::Load {
        dst,
        src:
            MirAddr::Deref {
                ptr: MirValue::Def(MirDef::VTemp(ptr)),
                offset,
            },
        width: MirWidth::Byte,
    } = ops.get(index)?
    else {
        return None;
    };
    if *ptr != pointer_temp {
        return None;
    }
    let value_temp = split_def_as_temp(dst)?;
    let MirOp::Store {
        dst: MirAddr::Direct(target),
        src: MirValue::Def(src),
        width: MirWidth::Byte,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    (split_def_as_temp(src) == Some(value_temp)).then(|| (*offset, target.clone()))
}

fn word_carry_chain_tail_value(value: &MirValue) -> bool {
    matches!(value, MirValue::ConstU8(_) | MirValue::ConstU16(_))
}

fn word_carry_chain_target_supported(target: &MirMem) -> bool {
    !matches!(
        target,
        MirMem::Absolute(_)
            | MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))
            | MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))
    )
}

fn materialize_word_carry_chain_byte_update(
    op: MirBinaryOp,
    target: MirMem,
    value: MirValue,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if config.enable_word_inc_update {
        out.push(match op {
            MirBinaryOp::Add => MirOp::AddByteToWordMem { mem: target, value },
            MirBinaryOp::Sub => MirOp::SubByteFromWordMem { mem: target, value },
            _ => unreachable!("word carry-chain byte updates only support add/sub"),
        });
        return;
    }
    materialize_word_byte_binary_store_consumer(
        op,
        target.clone(),
        pointer_value_from_mem(&target),
        value,
        config,
        layout,
        out,
    );
}

#[cfg(test)]
pub(super) fn try_fuse_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    select_word_store_consumer_with_deadness(ops, index, config, layout, true, out)
}

pub(super) fn select_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    select_word_store_consumer_with_deadness(ops, index, config, layout, false, out)
}

fn select_word_store_consumer_with_deadness(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> usize {
    if let Some(consumed) = try_fuse_loaded_extend_unary_neg_word_store_consumer(
        ops,
        index,
        require_local_deadness,
        out,
    ) {
        return consumed;
    }

    if let Some(consumed) = try_fuse_unary_neg_word_store_consumer(ops, index, out) {
        return consumed;
    }

    if let Some(consumed) = try_fuse_two_loaded_word_store_consumer(ops, index, config, layout, out)
    {
        return consumed;
    }

    if let Some(consumed) = try_fuse_loaded_byte_word_store_consumer(
        ops,
        index,
        config,
        layout,
        require_local_deadness,
        out,
    ) {
        return consumed;
    }

    if let Some(consumed) = try_fuse_loaded_word_store_consumer(ops, index, config, layout, out) {
        return consumed;
    }

    let Some(binary) = ops.get(index) else {
        return 0;
    };
    let Some(store) = ops.get(index + 1) else {
        return 0;
    };
    let MirOp::Binary {
        op,
        dst,
        left,
        right,
        width: MirWidth::Word,
        ..
    } = binary
    else {
        return 0;
    };
    if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) {
        return 0;
    }
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    } = store
    else {
        return 0;
    };
    if store_src != dst
        || value_uses_temp(left)
        || !word_binary_store_consumer_supported(*op, store_dst, left, right)
    {
        return 0;
    }

    materialize_word_binary_store_consumer(
        *op,
        store_dst.clone(),
        left.clone(),
        right.clone(),
        config,
        layout,
        out,
    );
    2
}

#[cfg(test)]
pub(super) fn try_fuse_byte_mul_add_sub_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    helpers: &mut Vec<MirRuntimeHelper>,
    out: &mut Vec<MirOp>,
) -> usize {
    select_byte_mul_add_sub_word_store_consumer_with_deadness(
        ops,
        index,
        config,
        layout,
        temp_widths,
        true,
        helpers,
        out,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn select_byte_mul_add_sub_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    helpers: &mut Vec<MirRuntimeHelper>,
    out: &mut Vec<MirOp>,
) -> usize {
    select_byte_mul_add_sub_word_store_consumer_with_deadness(
        ops,
        index,
        config,
        layout,
        temp_widths,
        false,
        helpers,
        out,
    )
}

#[allow(clippy::too_many_arguments)]
fn select_byte_mul_add_sub_word_store_consumer_with_deadness(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    require_local_deadness: bool,
    helpers: &mut Vec<MirRuntimeHelper>,
    out: &mut Vec<MirOp>,
) -> usize {
    if !config.select_runtime_helpers {
        return 0;
    }
    let Some(MirOp::Binary {
        op: MirBinaryOp::Mul,
        dst: mul_dst,
        left: mul_left,
        right: mul_right,
        width: MirWidth::Byte,
        ..
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(MirOp::Binary {
        op,
        dst: arith_dst,
        left,
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out: MirCarryOut::Ignore,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub)
        || !store_expr_binary_carry_is_safe(*op, MirWidth::Byte, *carry_in)
    {
        return 0;
    }
    let Some(addend) = byte_mul_arithmetic_addend(*op, mul_dst, left, right) else {
        return 0;
    };
    if value_uses_temp(addend) {
        return 0;
    }
    let Some(MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    }) = ops.get(index + 2)
    else {
        return 0;
    };
    if store_src != arith_dst
        || (require_local_deadness && def_is_used_after(ops, index + 2, mul_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 3, arith_dst))
    {
        return 0;
    }

    let helper = MirRuntimeHelper::Mul;
    helpers.push(helper.clone());
    materialize_runtime_helper_binary(
        helper,
        None,
        mul_left.clone(),
        mul_right.clone(),
        MirWidth::Byte,
        MirWidth::Word,
        layout,
        temp_widths,
        out,
    );

    let scratch_lo = MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO));
    let scratch_hi = offset_mem(&scratch_lo, 1);
    out.push(MirOp::Store {
        dst: MirAddr::Direct(scratch_lo.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(scratch_hi.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::X)),
        width: MirWidth::Byte,
    });
    materialize_byte_binary_store_consumer(
        *op,
        store_dst.clone(),
        MirValue::PointerCell(scratch_lo),
        addend.clone(),
        explicit_byte_carry(*op, *carry_in),
        MirCarryOut::Produce,
        out,
    );
    materialize_byte_binary_store_consumer(
        *op,
        offset_mem(store_dst, 1),
        MirValue::PointerCell(scratch_hi),
        MirValue::ConstU8(0),
        Some(MirCarryIn::FromPrevious),
        MirCarryOut::Ignore,
        out,
    );
    3
}

fn byte_mul_arithmetic_addend<'a>(
    op: MirBinaryOp,
    mul_dst: &MirDef,
    left: &'a MirValue,
    right: &'a MirValue,
) -> Option<&'a MirValue> {
    let mul_value = MirValue::Def(mul_dst.clone());
    match op {
        MirBinaryOp::Add if left == &mul_value => Some(right),
        MirBinaryOp::Add if right == &mul_value => Some(left),
        MirBinaryOp::Sub if left == &mul_value => Some(right),
        _ => None,
    }
}

pub(super) fn try_fuse_byte_mul_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    helpers: &mut Vec<MirRuntimeHelper>,
    out: &mut Vec<MirOp>,
) -> usize {
    if !config.select_runtime_helpers {
        return 0;
    }
    let Some(MirOp::Binary {
        op: MirBinaryOp::Mul,
        dst,
        left,
        right,
        width: MirWidth::Byte,
        ..
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    if store_src != dst {
        return 0;
    }

    let helper = MirRuntimeHelper::Mul;
    helpers.push(helper.clone());
    materialize_runtime_helper_binary(
        helper,
        None,
        left.clone(),
        right.clone(),
        MirWidth::Byte,
        MirWidth::Word,
        layout,
        temp_widths,
        out,
    );
    out.push(MirOp::Store {
        dst: MirAddr::Direct(store_dst.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(offset_mem(store_dst, 1)),
        src: MirValue::Def(MirDef::Reg(MirReg::X)),
        width: MirWidth::Byte,
    });
    2
}

fn try_fuse_loaded_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let load = ops.get(index)?;
    let binary = ops.get(index + 1)?;
    let store = ops.get(index + 2)?;
    let MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width: MirWidth::Word,
    } = load
    else {
        return None;
    };
    let load_temp = split_def_as_temp(load_dst)?;
    let MirOp::Binary {
        op,
        dst: binary_dst,
        left,
        right,
        width: MirWidth::Word,
        ..
    } = binary
    else {
        return None;
    };
    if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) {
        return None;
    }
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    } = store
    else {
        return None;
    };
    if store_src != binary_dst {
        return None;
    }

    let producer = pointer_value_from_mem(load_src);
    let left = replace_temp_value(left.clone(), load_temp, &producer);
    let right = replace_temp_value(right.clone(), load_temp, &producer);
    if value_uses_temp(&left)
        || value_uses_temp(&right)
        || !word_binary_store_consumer_supported(*op, store_dst, &left, &right)
    {
        return None;
    }

    materialize_word_binary_store_consumer(
        *op,
        store_dst.clone(),
        left,
        right,
        config,
        layout,
        out,
    );
    Some(3)
}

fn try_fuse_loaded_byte_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let byte_load = ops.get(index)?;
    let word_load = ops.get(index + 1)?;
    let binary = ops.get(index + 2)?;
    let store = ops.get(index + 3)?;
    let MirOp::Load {
        dst: byte_dst,
        src: MirAddr::Direct(byte_src),
        width: MirWidth::Byte,
    } = byte_load
    else {
        return None;
    };
    let MirOp::Load {
        dst: word_dst,
        src: MirAddr::Direct(word_src),
        width: MirWidth::Word,
    } = word_load
    else {
        return None;
    };
    let byte_temp = split_def_as_temp(byte_dst)?;
    let word_temp = split_def_as_temp(word_dst)?;
    let MirOp::Binary {
        op,
        dst: binary_dst,
        left,
        right,
        width: MirWidth::Word,
        ..
    } = binary
    else {
        return None;
    };
    if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) {
        return None;
    }
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    } = store
    else {
        return None;
    };
    if store_src != binary_dst {
        return None;
    }
    if (require_local_deadness && def_is_used_after(ops, index + 4, byte_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 4, word_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 4, binary_dst))
    {
        return None;
    }

    let byte_producer = MirValue::PointerCell(byte_src.clone());
    let word_producer = pointer_value_from_mem(word_src);
    let left = replace_temp_value(
        replace_temp_value(left.clone(), byte_temp, &byte_producer),
        word_temp,
        &word_producer,
    );
    let right = replace_temp_value(
        replace_temp_value(right.clone(), byte_temp, &byte_producer),
        word_temp,
        &word_producer,
    );
    if value_uses_temp(&left) || value_uses_temp(&right) {
        return None;
    }

    let (word_value, byte_value) = word_byte_binary_operands(*op, left, right)?;
    materialize_word_byte_binary_store_consumer(
        *op,
        store_dst.clone(),
        word_value,
        byte_value,
        config,
        layout,
        out,
    );
    Some(4)
}

fn word_byte_binary_operands(
    op: MirBinaryOp,
    left: MirValue,
    right: MirValue,
) -> Option<(MirValue, MirValue)> {
    let left_is_word = value_is_pointer_word(&left);
    let right_is_word = value_is_pointer_word(&right);
    match (op, left_is_word, right_is_word) {
        (_, true, false) => Some((left, right)),
        (MirBinaryOp::Add, false, true) => Some((right, left)),
        _ => None,
    }
}

fn value_is_pointer_word(value: &MirValue) -> bool {
    matches!(
        value,
        MirValue::Word { lo, hi }
            if matches!(lo.as_ref(), MirValue::PointerCell(_))
                && matches!(hi.as_ref(), MirValue::PointerCell(_))
    )
}

fn value_is_pointer_word_for_mem(value: &MirValue, mem: &MirMem) -> bool {
    match value {
        MirValue::Word { lo, hi } => {
            matches!(lo.as_ref(), MirValue::PointerCell(lo_mem) if lo_mem == mem)
                && matches!(hi.as_ref(), MirValue::PointerCell(hi_mem) if hi_mem == &offset_mem(mem, 1))
        }
        _ => false,
    }
}

fn is_one_value(value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(1) | MirValue::ConstU16(1) => true,
        MirValue::Word { lo, hi } => {
            matches!(lo.as_ref(), MirValue::ConstU8(1))
                && matches!(hi.as_ref(), MirValue::ConstU8(0))
        }
        _ => false,
    }
}

fn is_eight_value(value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(8) | MirValue::ConstU16(8) => true,
        MirValue::Word { lo, hi } => {
            matches!(lo.as_ref(), MirValue::ConstU8(8))
                && matches!(hi.as_ref(), MirValue::ConstU8(0))
        }
        _ => false,
    }
}

fn zero_extended_byte_value(value: &MirValue) -> Option<MirValue> {
    match value {
        MirValue::ConstU8(value) => Some(MirValue::ConstU8(*value)),
        MirValue::ConstU16(value) => u8::try_from(*value).ok().map(MirValue::ConstU8),
        MirValue::Word { lo, hi } if matches!(hi.as_ref(), MirValue::ConstU8(0)) => {
            match lo.as_ref() {
                MirValue::ConstU8(value) => Some(MirValue::ConstU8(*value)),
                _ => None,
            }
        }
        _ => None,
    }
}

enum WordMemUpdateValue {
    Inc,
    Dec,
    AddByte(MirValue),
    SubByte(MirValue),
}

fn word_binary_store_consumer_supported(
    op: MirBinaryOp,
    mem: &MirMem,
    left: &MirValue,
    right: &MirValue,
) -> bool {
    word_value_splits_to_constants(right) || word_mem_update_value(op, mem, left, right).is_some()
}

fn word_mem_update_value(
    op: MirBinaryOp,
    mem: &MirMem,
    left: &MirValue,
    right: &MirValue,
) -> Option<WordMemUpdateValue> {
    match op {
        MirBinaryOp::Add if value_is_pointer_word_for_mem(left, mem) => {
            word_add_mem_update_value(right)
        }
        MirBinaryOp::Add if value_is_pointer_word_for_mem(right, mem) => {
            word_add_mem_update_value(left)
        }
        MirBinaryOp::Sub if value_is_pointer_word_for_mem(left, mem) => {
            word_sub_mem_update_value(right)
        }
        _ => None,
    }
}

fn word_byte_mem_update_value(
    op: MirBinaryOp,
    mem: &MirMem,
    left: &MirValue,
    right: &MirValue,
) -> Option<WordMemUpdateValue> {
    match op {
        MirBinaryOp::Add if value_is_pointer_word_for_mem(left, mem) => {
            word_add_mem_update_value(right)
        }
        MirBinaryOp::Add if value_is_pointer_word_for_mem(right, mem) => {
            word_add_mem_update_value(left)
        }
        MirBinaryOp::Sub if value_is_pointer_word_for_mem(left, mem) => {
            word_sub_mem_update_value(right)
        }
        _ => None,
    }
}

fn word_add_mem_update_value(value: &MirValue) -> Option<WordMemUpdateValue> {
    if is_one_value(value) {
        Some(WordMemUpdateValue::Inc)
    } else {
        zero_extended_byte_value(value).map(WordMemUpdateValue::AddByte)
    }
}

fn word_sub_mem_update_value(value: &MirValue) -> Option<WordMemUpdateValue> {
    if is_one_value(value) {
        Some(WordMemUpdateValue::Dec)
    } else {
        zero_extended_byte_value(value).map(WordMemUpdateValue::SubByte)
    }
}

fn try_fuse_two_loaded_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let first_load = ops.get(index)?;
    let second_load = ops.get(index + 1)?;
    let binary = ops.get(index + 2)?;
    let store = ops.get(index + 3)?;
    let MirOp::Load {
        dst: first_dst,
        src: MirAddr::Direct(first_src),
        width: MirWidth::Word,
    } = first_load
    else {
        return None;
    };
    let MirOp::Load {
        dst: second_dst,
        src: MirAddr::Direct(second_src),
        width: MirWidth::Word,
    } = second_load
    else {
        return None;
    };
    let first_temp = split_def_as_temp(first_dst)?;
    let second_temp = split_def_as_temp(second_dst)?;
    let MirOp::Binary {
        op,
        dst: binary_dst,
        left,
        right,
        width: MirWidth::Word,
        ..
    } = binary
    else {
        return None;
    };
    if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) {
        return None;
    }
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    } = store
    else {
        return None;
    };
    if store_src != binary_dst {
        return None;
    }

    let first_producer = pointer_value_from_mem(first_src);
    let second_producer = pointer_value_from_mem(second_src);
    let left = replace_temp_value(
        replace_temp_value(left.clone(), first_temp, &first_producer),
        second_temp,
        &second_producer,
    );
    let right = replace_temp_value(
        replace_temp_value(right.clone(), first_temp, &first_producer),
        second_temp,
        &second_producer,
    );
    if value_uses_temp(&left) || value_uses_temp(&right) {
        return None;
    }

    materialize_word_binary_store_consumer(
        *op,
        store_dst.clone(),
        left,
        right,
        config,
        layout,
        out,
    );
    Some(4)
}

fn try_fuse_loaded_extend_unary_neg_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let load = ops.get(index)?;
    let extend = ops.get(index + 1)?;
    let unary = ops.get(index + 2)?;
    let store = ops.get(index + 3)?;
    let MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width: MirWidth::Byte,
    } = load
    else {
        return None;
    };
    let load_temp = split_def_as_temp(load_dst)?;
    let MirOp::Extend {
        dst: extend_dst,
        src: extend_src,
        from_width: MirWidth::Byte,
        to_width: MirWidth::Word,
        signed: false,
    } = extend
    else {
        return None;
    };
    if extend_src != &MirValue::Def(MirDef::VTemp(load_temp)) {
        return None;
    }
    let MirOp::Unary {
        op: MirUnaryOp::Neg,
        dst: unary_dst,
        src: MirValue::Def(unary_src),
        width: MirWidth::Word,
    } = unary
    else {
        return None;
    };
    if unary_src != extend_dst {
        return None;
    }
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    } = store
    else {
        return None;
    };
    if store_src != unary_dst
        || (require_local_deadness && def_is_used_after(ops, index + 4, load_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 4, extend_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 4, unary_dst))
    {
        return None;
    }

    materialize_zero_extended_byte_neg_to_word_mem(
        store_dst.clone(),
        MirValue::PointerCell(load_src.clone()),
        out,
    );
    Some(4)
}

fn try_fuse_unary_neg_word_store_consumer(
    ops: &[MirOp],
    index: usize,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let unary = ops.get(index)?;
    let store = ops.get(index + 1)?;
    let MirOp::Unary {
        op: MirUnaryOp::Neg,
        dst,
        src,
        width: MirWidth::Byte,
    } = unary
    else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    } = store
    else {
        return None;
    };
    if store_src != dst {
        return None;
    }
    let MirValue::ConstU8(value) = src else {
        return None;
    };
    let lo = 0u8.wrapping_sub(*value);
    let hi = if lo == 0 { 0x00 } else { 0xFF };
    materialize_value_to_mem(MirValue::ConstU8(lo), store_dst.clone(), out);
    materialize_value_to_mem(MirValue::ConstU8(hi), offset_mem(store_dst, 1), out);
    Some(2)
}

fn materialize_zero_extended_byte_neg_to_word_mem(
    dst: MirMem,
    src: MirValue,
    out: &mut Vec<MirOp>,
) {
    materialize_byte_binary_store_consumer(
        MirBinaryOp::Sub,
        dst.clone(),
        MirValue::ConstU8(0),
        src,
        Some(MirCarryIn::Set),
        MirCarryOut::Produce,
        out,
    );
    materialize_byte_binary_store_consumer(
        MirBinaryOp::Sub,
        offset_mem(&dst, 1),
        MirValue::ConstU8(0),
        MirValue::ConstU8(0),
        Some(MirCarryIn::FromPrevious),
        MirCarryOut::Ignore,
        out,
    );
}

fn materialize_word_binary_store_consumer(
    op: MirBinaryOp,
    dst: MirMem,
    left: MirValue,
    right: MirValue,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if config.enable_word_inc_update {
        match word_mem_update_value(op, &dst, &left, &right) {
            Some(WordMemUpdateValue::Inc) => {
                out.push(MirOp::UpdateMem {
                    op: MirUpdateOp::Inc,
                    mem: dst,
                    width: MirWidth::Word,
                });
                return;
            }
            Some(WordMemUpdateValue::Dec) => {
                out.push(MirOp::UpdateMem {
                    op: MirUpdateOp::Dec,
                    mem: dst,
                    width: MirWidth::Word,
                });
                return;
            }
            Some(WordMemUpdateValue::AddByte(value)) => {
                out.push(MirOp::AddByteToWordMem { mem: dst, value });
                return;
            }
            Some(WordMemUpdateValue::SubByte(value)) => {
                out.push(MirOp::SubByteFromWordMem { mem: dst, value });
                return;
            }
            None => {}
        }
    }
    let (left_lo, left_hi) = split_value_as_word(left, layout);
    let (right_lo, right_hi) = split_value(right, layout);
    materialize_byte_binary_store_consumer(
        op,
        dst.clone(),
        left_lo,
        right_lo,
        Some(match op {
            MirBinaryOp::Add => MirCarryIn::Clear,
            MirBinaryOp::Sub => MirCarryIn::Set,
            _ => unreachable!(),
        }),
        MirCarryOut::Produce,
        out,
    );
    materialize_byte_binary_store_consumer(
        op,
        offset_mem(&dst, 1),
        left_hi,
        right_hi,
        Some(MirCarryIn::FromPrevious),
        MirCarryOut::Ignore,
        out,
    );
}

fn materialize_byte_byte_binary_word_store_consumer(
    op: MirBinaryOp,
    dst: MirMem,
    left: MirValue,
    right: MirValue,
    out: &mut Vec<MirOp>,
) {
    materialize_byte_binary_store_consumer(
        op,
        dst.clone(),
        left,
        right,
        Some(match op {
            MirBinaryOp::Add => MirCarryIn::Clear,
            MirBinaryOp::Sub => MirCarryIn::Set,
            _ => unreachable!(),
        }),
        MirCarryOut::Produce,
        out,
    );
    materialize_byte_binary_store_consumer(
        op,
        offset_mem(&dst, 1),
        MirValue::ConstU8(0),
        MirValue::ConstU8(0),
        Some(MirCarryIn::FromPrevious),
        MirCarryOut::Ignore,
        out,
    );
}

fn materialize_word_byte_binary_store_consumer(
    op: MirBinaryOp,
    dst: MirMem,
    left: MirValue,
    right: MirValue,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if config.enable_word_inc_update {
        match word_byte_mem_update_value(op, &dst, &left, &right) {
            Some(WordMemUpdateValue::Inc) => {
                out.push(MirOp::UpdateMem {
                    op: MirUpdateOp::Inc,
                    mem: dst,
                    width: MirWidth::Word,
                });
                return;
            }
            Some(WordMemUpdateValue::Dec) => {
                out.push(MirOp::UpdateMem {
                    op: MirUpdateOp::Dec,
                    mem: dst,
                    width: MirWidth::Word,
                });
                return;
            }
            Some(WordMemUpdateValue::AddByte(value)) => {
                out.push(MirOp::AddByteToWordMem { mem: dst, value });
                return;
            }
            Some(WordMemUpdateValue::SubByte(value)) => {
                out.push(MirOp::SubByteFromWordMem { mem: dst, value });
                return;
            }
            None => {}
        }
    }

    let (left_lo, left_hi) = split_value_as_word(left, layout);
    materialize_byte_binary_store_consumer(
        op,
        dst.clone(),
        left_lo,
        right,
        Some(match op {
            MirBinaryOp::Add => MirCarryIn::Clear,
            MirBinaryOp::Sub => MirCarryIn::Set,
            _ => unreachable!(),
        }),
        MirCarryOut::Produce,
        out,
    );
    materialize_byte_binary_store_consumer(
        op,
        offset_mem(&dst, 1),
        left_hi,
        MirValue::ConstU8(0),
        Some(MirCarryIn::FromPrevious),
        MirCarryOut::Ignore,
        out,
    );
}

#[cfg(test)]
pub(super) fn try_fuse_byte_store_consumer(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    routine_id: RoutineId,
    block_id: MirBlockId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    peephole_stats: &mut MirPeepholeStats,
    out: &mut Vec<MirOp>,
) -> usize {
    select_byte_store_consumer_with_deadness(
        ops,
        index,
        terminator,
        routine_id,
        block_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        true,
        peephole_stats,
        out,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn select_byte_store_consumer(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    routine_id: RoutineId,
    block_id: MirBlockId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    peephole_stats: &mut MirPeepholeStats,
    out: &mut Vec<MirOp>,
) -> usize {
    select_byte_store_consumer_with_deadness(
        ops,
        index,
        terminator,
        routine_id,
        block_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        false,
        peephole_stats,
        out,
    )
}

#[allow(clippy::too_many_arguments)]
fn select_byte_store_consumer_with_deadness(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    routine_id: RoutineId,
    block_id: MirBlockId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    require_local_deadness: bool,
    peephole_stats: &mut MirPeepholeStats,
    out: &mut Vec<MirOp>,
) -> usize {
    if let Some(consumed) = try_fuse_indexed_rhs_loaded_byte_store_consumer(
        ops,
        index,
        layout,
        delayed_byte_indexes,
        require_local_deadness,
        out,
    ) {
        return consumed;
    }

    if let Some(consumed) = try_fuse_two_loaded_byte_store_consumer(ops, index, out) {
        return consumed;
    }

    if let Some(consumed) =
        try_fuse_loaded_byte_op_chain_store_consumer(ops, index, require_local_deadness, out)
    {
        return consumed;
    }

    if let Some(consumed) = try_fuse_loaded_byte_update_store_consumer(
        ops,
        index,
        terminator,
        require_local_deadness,
        out,
    ) {
        return consumed;
    }

    if let Some(consumed) =
        try_fuse_byte_update_store_consumer(ops, index, terminator, require_local_deadness, out)
    {
        return consumed;
    }

    if let Some(consumed) = try_fuse_loaded_byte_store_consumer(ops, index, out) {
        return consumed;
    }

    if let Some(consumed) = try_fuse_byte_binary_store_consumer(
        ops,
        index,
        terminator,
        routine_id,
        block_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        require_local_deadness,
        peephole_stats,
        out,
    ) {
        return consumed;
    }

    0
}

fn try_fuse_indexed_rhs_loaded_byte_store_consumer(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let indexed_load = ops.get(index)?;
    let direct_load = ops.get(index + 1)?;
    let binary = ops.get(index + 2)?;
    let store = ops.get(index + 3)?;
    let MirOp::Load {
        dst: indexed_dst,
        src: indexed_src,
        width: MirWidth::Byte,
    } = indexed_load
    else {
        return None;
    };
    let MirOp::Load {
        dst: direct_dst,
        src: MirAddr::Direct(direct_src),
        width: MirWidth::Byte,
    } = direct_load
    else {
        return None;
    };
    let MirOp::Binary {
        op,
        dst: binary_dst,
        left,
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out,
    } = binary
    else {
        return None;
    };
    if !matches!(
        op,
        MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor
    ) {
        return None;
    }
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Byte,
    } = store
    else {
        return None;
    };
    if store_src != binary_dst
        || (require_local_deadness && def_is_used_after(ops, index + 4, indexed_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 4, direct_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 4, binary_dst))
    {
        return None;
    }

    let indexed_value = MirValue::Def(indexed_dst.clone());
    let direct_value = MirValue::Def(direct_dst.clone());
    let commutative = matches!(
        op,
        MirBinaryOp::Add | MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor
    );
    if !((left == &direct_value && right == &indexed_value)
        || (commutative && left == &indexed_value && right == &direct_value))
    {
        return None;
    }
    let scratch = MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO));
    if direct_src == &scratch {
        return None;
    }

    materialize_indexed_byte_read_to_a(indexed_src, layout, delayed_byte_indexes, out)?;
    out.push(MirOp::Store {
        dst: MirAddr::Direct(scratch.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    materialize_byte_binary_store_consumer(
        *op,
        store_dst.clone(),
        MirValue::PointerCell(direct_src.clone()),
        MirValue::PointerCell(scratch),
        explicit_byte_carry(*op, *carry_in),
        *carry_out,
        out,
    );
    Some(4)
}

fn try_fuse_byte_binary_store_consumer(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    routine_id: RoutineId,
    block_id: MirBlockId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    require_local_deadness: bool,
    peephole_stats: &mut MirPeepholeStats,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let binary = ops.get(index)?;
    let store = ops.get(index + 1)?;
    let MirOp::Binary {
        op,
        dst: binary_dst,
        left,
        right,
        width,
        carry_in,
        carry_out,
    } = binary
    else {
        return None;
    };
    let MirOp::Store {
        dst: store_dst,
        src: MirValue::Def(store_src),
        width: store_width,
    } = store
    else {
        return None;
    };
    if store_src != binary_dst {
        return None;
    }
    let Some(blocked_reason) = byte_binary_store_forward_blocker(
        ops,
        index,
        terminator,
        *op,
        binary_dst,
        left,
        right,
        *width,
        store_width,
        store_dst,
        require_local_deadness,
    ) else {
        record_binary_store_forward_site(
            peephole_stats,
            routine_id,
            block_id,
            index,
            "applied",
            None,
            ops,
        );
        if can_forward_word_rsh8_to_byte_store(*op, *width, store_width, right) {
            let (_left_lo, left_hi) =
                split_value_with_temp_widths(left.clone(), layout, temp_widths);
            materialize_byte_value_store_consumer_for_addr(
                store_dst.clone(),
                left_hi,
                routine_id,
                layout,
                temp_widths,
                delayed_byte_indexes,
                out,
            );
        } else if can_forward_word_binary_to_word_direct_store(*op, *width, store_width, store_dst)
        {
            let MirAddr::Direct(store_dst) = store_dst else {
                unreachable!();
            };
            materialize_word_binary_direct_store_consumer(
                *op,
                store_dst.clone(),
                left.clone(),
                right.clone(),
                *carry_in,
                *carry_out,
                layout,
                temp_widths,
                out,
            );
        } else {
            let (forward_left, forward_right, forward_carry_in, forward_carry_out) =
                byte_binary_store_forward_operands(
                    *op,
                    left,
                    right,
                    *width,
                    store_width,
                    *carry_in,
                    *carry_out,
                    layout,
                    temp_widths,
                );
            materialize_byte_binary_store_consumer_for_addr(
                *op,
                store_dst.clone(),
                forward_left,
                forward_right,
                forward_carry_in,
                forward_carry_out,
                routine_id,
                layout,
                temp_widths,
                delayed_byte_indexes,
                out,
            );
        }
        return Some(2);
    };
    record_binary_store_forward_site(
        peephole_stats,
        routine_id,
        block_id,
        index,
        "blocked",
        Some(blocked_reason),
        ops,
    );
    None
}

fn byte_binary_store_forward_blocker(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    op: MirBinaryOp,
    dst: &MirDef,
    left: &MirValue,
    right: &MirValue,
    width: MirWidth,
    store_width: &MirWidth,
    store_dst: &MirAddr,
    require_local_deadness: bool,
) -> Option<&'static str> {
    if width != MirWidth::Byte {
        if !can_forward_word_rsh8_to_byte_store(op, width, store_width, right)
            && !can_forward_word_binary_to_byte_store(op, width, store_width)
            && !can_forward_word_binary_to_word_direct_store(op, width, store_width, store_dst)
        {
            return Some("width-not-byte");
        }
        if binary_flags_may_be_live_after(ops, index + 2, terminator) {
            return Some("word-low-byte-flags-live");
        }
    }
    if !can_forward_word_rsh8_to_byte_store(op, width, store_width, right)
        && !can_forward_byte_shift_const_to_store(op, width, store_width, right)
        && !matches!(
            op,
            MirBinaryOp::Add
                | MirBinaryOp::Sub
                | MirBinaryOp::And
                | MirBinaryOp::Or
                | MirBinaryOp::Xor
        )
    {
        return Some("unsupported-op");
    }
    if store_width != &MirWidth::Byte
        && !can_forward_word_binary_to_word_direct_store(op, width, store_width, store_dst)
    {
        return Some("store-width-mismatch");
    }
    if can_forward_word_binary_to_word_direct_store(op, width, store_width, store_dst)
        && (value_uses_temp(left) || value_uses_temp(right))
    {
        return Some("word-operand-temp");
    }
    if !matches!(store_dst, MirAddr::Direct(_)) && value_uses_temp(right) {
        return Some("rhs-temp");
    }
    if !byte_binary_store_addr_is_supported(store_dst) {
        return Some("store-addr-unsupported");
    }
    if matches!(store_dst, MirAddr::Deref { ptr, .. } if value_uses_temp(ptr)) {
        return Some("deref-pointer-temp");
    }
    if !matches!(store_dst, MirAddr::Direct(_))
        && (!value_survives_address_setup(left) || !value_survives_address_setup(right))
    {
        return Some("operand-needs-address-scratch");
    }
    if require_local_deadness && def_is_used_after(ops, index + 2, dst) {
        return Some("result-live-after");
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn byte_binary_store_forward_operands(
    op: MirBinaryOp,
    left: &MirValue,
    right: &MirValue,
    width: MirWidth,
    store_width: &MirWidth,
    carry_in: Option<MirCarryIn>,
    carry_out: MirCarryOut,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
) -> (MirValue, MirValue, Option<MirCarryIn>, MirCarryOut) {
    if can_forward_word_binary_to_byte_store(op, width, store_width) {
        let (left_lo, _) = split_value_with_temp_widths(left.clone(), layout, temp_widths);
        let (right_lo, _) = split_value_with_temp_widths(right.clone(), layout, temp_widths);
        return (
            left_lo,
            right_lo,
            explicit_byte_carry(op, carry_in),
            MirCarryOut::Ignore,
        );
    }
    (
        left.clone(),
        right.clone(),
        explicit_byte_carry(op, carry_in),
        carry_out,
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_word_binary_direct_store_consumer(
    op: MirBinaryOp,
    dst: MirMem,
    left: MirValue,
    right: MirValue,
    carry_in: Option<MirCarryIn>,
    _carry_out: MirCarryOut,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    let (left_lo, left_hi) = split_value_with_temp_widths(left, layout, temp_widths);
    let (right_lo, right_hi) = split_value_with_temp_widths(right, layout, temp_widths);
    let (lo_carry_in, lo_carry_out, hi_carry_in) = match op {
        MirBinaryOp::Add | MirBinaryOp::Sub => (
            explicit_byte_carry(op, carry_in),
            MirCarryOut::Produce,
            Some(MirCarryIn::FromPrevious),
        ),
        MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor => (None, MirCarryOut::Ignore, None),
        _ => unreachable!(),
    };

    materialize_byte_binary_store_consumer(
        op,
        dst.clone(),
        left_lo,
        right_lo,
        lo_carry_in,
        lo_carry_out,
        out,
    );
    materialize_byte_binary_store_consumer(
        op,
        offset_mem(&dst, 1),
        left_hi,
        right_hi,
        hi_carry_in,
        MirCarryOut::Ignore,
        out,
    );
}

fn can_forward_word_binary_to_byte_store(
    op: MirBinaryOp,
    width: MirWidth,
    store_width: &MirWidth,
) -> bool {
    width == MirWidth::Word
        && store_width == &MirWidth::Byte
        && matches!(
            op,
            MirBinaryOp::Add
                | MirBinaryOp::Sub
                | MirBinaryOp::And
                | MirBinaryOp::Or
                | MirBinaryOp::Xor
        )
}

fn can_forward_word_rsh8_to_byte_store(
    op: MirBinaryOp,
    width: MirWidth,
    store_width: &MirWidth,
    right: &MirValue,
) -> bool {
    op == MirBinaryOp::Rsh
        && width == MirWidth::Word
        && store_width == &MirWidth::Byte
        && is_eight_value(right)
}

fn can_forward_byte_shift_const_to_store(
    op: MirBinaryOp,
    width: MirWidth,
    store_width: &MirWidth,
    right: &MirValue,
) -> bool {
    matches!(op, MirBinaryOp::Lsh | MirBinaryOp::Rsh)
        && width == MirWidth::Byte
        && store_width == &MirWidth::Byte
        && matches!(right, MirValue::ConstU8(count) if *count <= 8)
}

fn can_forward_word_binary_to_word_direct_store(
    op: MirBinaryOp,
    width: MirWidth,
    store_width: &MirWidth,
    store_dst: &MirAddr,
) -> bool {
    width == MirWidth::Word
        && store_width == &MirWidth::Word
        && matches!(store_dst, MirAddr::Direct(_))
        && matches!(
            op,
            MirBinaryOp::Add
                | MirBinaryOp::Sub
                | MirBinaryOp::And
                | MirBinaryOp::Or
                | MirBinaryOp::Xor
        )
}

fn binary_flags_may_be_live_after(ops: &[MirOp], start: usize, terminator: &MirTerminator) -> bool {
    terminator_consumes_flags(terminator) && !ops[start..].iter().any(op_writes_flags)
}

fn byte_binary_store_addr_is_supported(store_dst: &MirAddr) -> bool {
    matches!(
        store_dst,
        MirAddr::Direct(_)
            | MirAddr::Deref { .. }
            | MirAddr::PointerCell { .. }
            | MirAddr::ComputedIndex { .. }
            | MirAddr::PointerIndex { .. }
    )
}

fn value_survives_address_setup(value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => true,
        MirValue::Def(MirDef::VTemp(_) | MirDef::VTempByte { .. }) => true,
        MirValue::Def(MirDef::Reg(_)) | MirValue::PointerCell(_) => false,
        MirValue::Word { lo, hi } => {
            value_survives_address_setup(lo) && value_survives_address_setup(hi)
        }
    }
}

fn record_binary_store_forward_site(
    peephole_stats: &mut MirPeepholeStats,
    routine_id: RoutineId,
    block_id: MirBlockId,
    op_index: usize,
    status: &'static str,
    reason: Option<&'static str>,
    ops: &[MirOp],
) {
    let producer_detail = if reason == Some("word-operand-temp") {
        record_word_operand_temp_producer_stats(peephole_stats, routine_id, ops, op_index)
    } else {
        String::new()
    };
    let reason = reason
        .map(|reason| format!(" reason={reason}"))
        .unwrap_or_default();
    peephole_stats.record_site(
        routine_id,
        "binary-store-forward",
        format!(
            "status={status}{reason}{producer_detail} block=b{} op=#{} {}",
            block_id.0,
            op_index,
            binary_store_forward_window(ops, op_index)
        ),
    );
}

fn record_word_operand_temp_producer_stats(
    peephole_stats: &mut MirPeepholeStats,
    routine_id: RoutineId,
    ops: &[MirOp],
    index: usize,
) -> String {
    let Some(MirOp::Binary { left, right, .. }) = ops.get(index) else {
        return String::new();
    };
    let mut pieces = Vec::new();
    for (side, value) in [("left", left), ("right", right)] {
        collect_word_operand_temp_producer_stats(
            peephole_stats,
            routine_id,
            ops,
            index,
            side,
            value,
            &mut pieces,
        );
    }
    if pieces.is_empty() {
        String::new()
    } else {
        format!(" temp-producers=[{}]", pieces.join(", "))
    }
}

fn collect_word_operand_temp_producer_stats(
    peephole_stats: &mut MirPeepholeStats,
    routine_id: RoutineId,
    ops: &[MirOp],
    index: usize,
    side: &'static str,
    value: &MirValue,
    pieces: &mut Vec<String>,
) {
    match value {
        MirValue::Def(MirDef::VTemp(temp)) => {
            record_word_operand_temp_producer_stat(
                peephole_stats,
                routine_id,
                ops,
                index,
                side,
                *temp,
                None,
                pieces,
            );
        }
        MirValue::Def(MirDef::VTempByte { id, byte }) => {
            record_word_operand_temp_producer_stat(
                peephole_stats,
                routine_id,
                ops,
                index,
                side,
                *id,
                Some(*byte),
                pieces,
            );
        }
        MirValue::Word { lo, hi } => {
            collect_word_operand_temp_producer_stats(
                peephole_stats,
                routine_id,
                ops,
                index,
                side,
                lo,
                pieces,
            );
            collect_word_operand_temp_producer_stats(
                peephole_stats,
                routine_id,
                ops,
                index,
                side,
                hi,
                pieces,
            );
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(MirDef::Reg(_))
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. }
        | MirValue::PointerCell(_) => {}
    }
}

fn record_word_operand_temp_producer_stat(
    peephole_stats: &mut MirPeepholeStats,
    routine_id: RoutineId,
    ops: &[MirOp],
    index: usize,
    side: &'static str,
    temp: MirTempId,
    byte: Option<u8>,
    pieces: &mut Vec<String>,
) {
    let Some((producer_index, producer)) = find_temp_producer_before(ops, index, temp) else {
        peephole_stats.record(
            routine_id,
            "binary-store-forward-word-temp-producer-missing",
        );
        pieces.push(format!(
            "{side}:{}=missing",
            format_temp_operand(temp, byte)
        ));
        return;
    };
    let (kind, stat) = word_operand_temp_producer_kind(producer);
    peephole_stats.record(routine_id, stat);
    pieces.push(format!(
        "{side}:{}={kind}@#{producer_index}",
        format_temp_operand(temp, byte)
    ));
}

fn find_temp_producer_before(
    ops: &[MirOp],
    index: usize,
    temp: MirTempId,
) -> Option<(usize, &MirOp)> {
    ops[..index]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, op)| op_def(op).and_then(split_def_as_temp) == Some(temp))
}

fn format_temp_operand(temp: MirTempId, byte: Option<u8>) -> String {
    match byte {
        Some(byte) => format!("v{}.b{byte}", temp.0),
        None => format!("v{}", temp.0),
    }
}

fn word_operand_temp_producer_kind(op: &MirOp) -> (&'static str, &'static str) {
    match op {
        MirOp::Load {
            src: MirAddr::Direct(_),
            width,
            ..
        } => (
            width_kind("load-direct", *width),
            "binary-store-forward-word-temp-producer-load-direct",
        ),
        MirOp::Load {
            src: MirAddr::Deref { .. },
            width,
            ..
        } => (
            width_kind("load-deref", *width),
            "binary-store-forward-word-temp-producer-load-deref",
        ),
        MirOp::Load {
            src: MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. },
            width,
            ..
        } => (
            width_kind("load-indexed", *width),
            "binary-store-forward-word-temp-producer-load-indexed",
        ),
        MirOp::Load {
            src: MirAddr::PointerCell { .. },
            width,
            ..
        } => (
            width_kind("load-pointer-cell", *width),
            "binary-store-forward-word-temp-producer-load-pointer-cell",
        ),
        MirOp::Load { width, .. } => (
            width_kind("load-other", *width),
            "binary-store-forward-word-temp-producer-load-other",
        ),
        MirOp::Binary { width, .. } => (
            width_kind("binary", *width),
            "binary-store-forward-word-temp-producer-binary",
        ),
        MirOp::LoadImm { width, .. } => (
            width_kind("load-imm", *width),
            "binary-store-forward-word-temp-producer-load-imm",
        ),
        MirOp::Move { width, .. } => (
            width_kind("move", *width),
            "binary-store-forward-word-temp-producer-move",
        ),
        MirOp::LeaAddr { .. } => ("lea", "binary-store-forward-word-temp-producer-lea"),
        MirOp::Call { .. } => (
            "call-result",
            "binary-store-forward-word-temp-producer-call-result",
        ),
        MirOp::Extend { .. } => ("extend", "binary-store-forward-word-temp-producer-other"),
        MirOp::Truncate { .. } => ("truncate", "binary-store-forward-word-temp-producer-other"),
        MirOp::Unary { .. } => ("unary", "binary-store-forward-word-temp-producer-other"),
        MirOp::LoadIndirect { .. } => (
            "load-indirect",
            "binary-store-forward-word-temp-producer-load-indirect",
        ),
        MirOp::Store { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::Compare { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => ("other", "binary-store-forward-word-temp-producer-other"),
    }
}

fn width_kind(prefix: &'static str, width: MirWidth) -> &'static str {
    match (prefix, width) {
        ("load-direct", MirWidth::Byte) => "load-direct-byte",
        ("load-direct", MirWidth::Word) => "load-direct-word",
        ("load-deref", MirWidth::Byte) => "load-deref-byte",
        ("load-deref", MirWidth::Word) => "load-deref-word",
        ("load-indexed", MirWidth::Byte) => "load-indexed-byte",
        ("load-indexed", MirWidth::Word) => "load-indexed-word",
        ("load-pointer-cell", MirWidth::Byte) => "load-pointer-cell-byte",
        ("load-pointer-cell", MirWidth::Word) => "load-pointer-cell-word",
        ("load-other", MirWidth::Byte) => "load-other-byte",
        ("load-other", MirWidth::Word) => "load-other-word",
        ("binary", MirWidth::Byte) => "binary-byte",
        ("binary", MirWidth::Word) => "binary-word",
        ("load-imm", MirWidth::Byte) => "load-imm-byte",
        ("load-imm", MirWidth::Word) => "load-imm-word",
        ("move", MirWidth::Byte) => "move-byte",
        ("move", MirWidth::Word) => "move-word",
        _ => prefix,
    }
}

fn binary_store_forward_window(ops: &[MirOp], index: usize) -> String {
    let mut pieces = Vec::new();
    for offset in 0..3 {
        let op_index = index + offset;
        if let Some(op) = ops.get(op_index) {
            pieces.push(format!("#{op_index}={op:?}"));
        }
    }
    format!("window=[{}]", pieces.join("; "))
}

fn try_fuse_byte_update_store_consumer(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let MirOp::Binary {
        op,
        dst,
        left,
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out: MirCarryOut::Ignore,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Byte,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    if store_src != dst
        || !inc_dec_mem_is_safe(store_dst)
        || (require_local_deadness && def_is_used_after(ops, index + 2, dst))
    {
        return None;
    }
    let update = byte_mem_inc_dec_update(*op, left, right, store_dst, *carry_in)?;
    if !tail_allows_inc_dec_update(ops, index + 2, terminator) {
        return None;
    }
    out.push(MirOp::UpdateMem {
        op: update,
        mem: store_dst.clone(),
        width: MirWidth::Byte,
    });
    Some(2)
}

fn try_fuse_loaded_byte_update_store_consumer(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width: MirWidth::Byte,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirOp::Binary {
        op,
        dst,
        left,
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out: MirCarryOut::Ignore,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Byte,
    } = ops.get(index + 2)?
    else {
        return None;
    };
    if store_src != dst
        || store_dst != load_src
        || !inc_dec_mem_is_safe(store_dst)
        || (require_local_deadness && def_is_used_after(ops, index + 3, load_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 3, dst))
    {
        return None;
    }
    let load_temp = split_def_as_temp(load_dst)?;
    let producer = MirValue::PointerCell(load_src.clone());
    let left = replace_temp_value(left.clone(), load_temp, &producer);
    let right = replace_temp_value(right.clone(), load_temp, &producer);
    if value_uses_temp(&left) || value_uses_temp(&right) {
        return None;
    }
    let update = byte_mem_inc_dec_update(*op, &left, &right, store_dst, *carry_in)?;
    if !tail_allows_inc_dec_update(ops, index + 3, terminator) {
        return None;
    }
    out.push(MirOp::UpdateMem {
        op: update,
        mem: store_dst.clone(),
        width: MirWidth::Byte,
    });
    Some(3)
}

fn byte_mem_inc_dec_update(
    op: MirBinaryOp,
    left: &MirValue,
    right: &MirValue,
    mem: &MirMem,
    carry_in: Option<MirCarryIn>,
) -> Option<MirUpdateOp> {
    let mem_value = MirValue::PointerCell(mem.clone());
    match (op, carry_in) {
        (MirBinaryOp::Add, None | Some(MirCarryIn::Clear))
            if (left == &mem_value && value_is_const_u8(right, 1))
                || (right == &mem_value && value_is_const_u8(left, 1)) =>
        {
            Some(MirUpdateOp::Inc)
        }
        (MirBinaryOp::Sub, None | Some(MirCarryIn::Set))
            if left == &mem_value && value_is_const_u8(right, 1) =>
        {
            Some(MirUpdateOp::Dec)
        }
        _ => None,
    }
}

fn value_is_const_u8(value: &MirValue, expected: u8) -> bool {
    matches!(value, MirValue::ConstU8(value) if *value == expected)
}

fn tail_allows_inc_dec_update(
    ops: &[MirOp],
    after_store: usize,
    terminator: &MirTerminator,
) -> bool {
    tail_allows_inc_dec_update_after(ops, after_store, terminator)
}

fn terminator_allows_inc_dec_update(terminator: &MirTerminator) -> bool {
    match terminator {
        MirTerminator::Return
        | MirTerminator::Exit
        | MirTerminator::Jump(_)
        | MirTerminator::Unreachable => true,
        MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::ZSet | MirFlagTest::ZClear),
            ..
        } => true,
        MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::NSet | MirFlagTest::NClear),
            ..
        } => true,
        MirTerminator::Branch { .. } => false,
    }
}

fn tail_allows_inc_dec_update_after(
    ops: &[MirOp],
    start: usize,
    terminator: &MirTerminator,
) -> bool {
    let mut a_live = true;
    let mut carry_live = true;
    let mut overflow_live = true;

    let remaining_ops = ops.get(start..).unwrap_or(&[]);
    for op in remaining_ops {
        if a_live && op_reads_reg(op, MirReg::A) {
            return false;
        }
        if carry_live && op_uses_previous_carry(op) {
            return false;
        }
        if !a_live && !carry_live && !overflow_live {
            return true;
        }
        if op_writes_reg(op, MirReg::A) {
            a_live = false;
        }
        if op_overwrites_carry(op) {
            carry_live = false;
        }
        if op_overwrites_overflow(op) {
            overflow_live = false;
        }
        if op_clobbers_unknown_flag_or_a_effects(op) {
            a_live = false;
            carry_live = false;
            overflow_live = false;
            continue;
        }
        if op_has_opaque_flag_or_a_effects(op) && (a_live || carry_live || overflow_live) {
            return false;
        }
    }

    if !terminator_allows_inc_dec_update(terminator) {
        return false;
    }
    match terminator {
        MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::CSet | MirFlagTest::CClear),
            ..
        } if carry_live => false,
        MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::VSet | MirFlagTest::VClear),
            ..
        } if overflow_live => false,
        _ => true,
    }
}

fn inc_dec_mem_is_safe(mem: &MirMem) -> bool {
    matches!(
        mem,
        MirMem::Global { .. }
            | MirMem::Static { .. }
            | MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Spill { .. }
    )
}

#[cfg(test)]
pub(super) fn try_fold_direct_inc_dec_update(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    if !tail_allows_inc_dec_update_after(ops, index + 3, terminator) {
        return None;
    }
    let (consumed, replacement) = direct_inc_dec_update_shape_at(ops, index)?;
    out.extend(replacement);
    Some(consumed)
}

pub(in crate::mir6502) fn discover_direct_inc_dec_updates(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some((consumed, replacement)) =
                inc_dec_update_shape_at(&block.ops, index, layout)
            {
                let stat = if matches!(replacement.as_slice(), [MirOp::UpdateIndexedMem { .. }]) {
                    "indexed-inc-dec-update"
                } else {
                    "direct-inc-dec-update"
                };
                if let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    inc_dec_exit_change(),
                    stat,
                    0,
                ) {
                    plans.push(plan);
                }
            }
        }
    }
    plans
}

fn inc_dec_update_shape_at(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    direct_inc_dec_update_shape_at(ops, index)
        .or_else(|| indexed_inc_dec_update_shape_at(ops, index, layout))
}

fn direct_inc_dec_update_shape_at(ops: &[MirOp], index: usize) -> Option<(usize, Vec<MirOp>)> {
    let mem = loaded_a_direct_mem(ops.get(index)?)?;
    let MirOp::Binary {
        op,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: MirValue::ConstU8(1),
        width: MirWidth::Byte,
        carry_in,
        carry_out: MirCarryOut::Ignore,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    } = ops.get(index + 2)?
    else {
        return None;
    };
    if store_dst != &mem || !inc_dec_mem_is_safe(store_dst) {
        return None;
    }
    let update = match (op, carry_in) {
        (MirBinaryOp::Add, None | Some(MirCarryIn::Clear)) => MirUpdateOp::Inc,
        (MirBinaryOp::Sub, None | Some(MirCarryIn::Set)) => MirUpdateOp::Dec,
        _ => return None,
    };
    Some((
        3,
        vec![MirOp::UpdateMem {
            op: update,
            mem,
            width: MirWidth::Byte,
        }],
    ))
}

fn indexed_inc_dec_update_shape_at(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    let MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::AbsoluteIndexedX { base },
        width: MirWidth::Byte,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirOp::Binary {
        op,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: MirValue::ConstU8(1),
        width: MirWidth::Byte,
        carry_in,
        carry_out: MirCarryOut::Ignore,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let MirOp::Store {
        dst: MirAddr::AbsoluteIndexedX { base: store_base },
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    } = ops.get(index + 2)?
    else {
        return None;
    };
    if store_base != base || !layout.mem_allows_direct_indexed_update(base) {
        return None;
    }
    let update = match (op, carry_in) {
        (MirBinaryOp::Add, None | Some(MirCarryIn::Clear)) => MirUpdateOp::Inc,
        (MirBinaryOp::Sub, None | Some(MirCarryIn::Set)) => MirUpdateOp::Dec,
        _ => return None,
    };
    Some((
        3,
        vec![MirOp::UpdateIndexedMem {
            op: update,
            base: base.clone(),
        }],
    ))
}

fn inc_dec_exit_change() -> MirExitStateChange {
    MirExitStateChange {
        registers: MirRegisterSet {
            a: true,
            ..MirRegisterSet::default()
        },
        flags: MirFlagSet {
            c: true,
            v: true,
            ..MirFlagSet::default()
        },
        ..MirExitStateChange::default()
    }
}

fn loaded_a_direct_mem(op: &MirOp) -> Option<MirMem> {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }
        | MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::PointerCell(mem),
            width: MirWidth::Byte,
        } => Some(mem.clone()),
        _ => None,
    }
}

fn try_fuse_loaded_byte_op_chain_store_consumer(
    ops: &[MirOp],
    index: usize,
    require_local_deadness: bool,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width: MirWidth::Byte,
    } = ops.get(index)?
    else {
        return None;
    };
    let load_temp = split_def_as_temp(load_dst)?;

    let MirOp::Binary {
        op: first_op,
        dst: first_dst,
        left: first_left,
        right: first_right,
        width: MirWidth::Byte,
        carry_in: first_carry_in,
        carry_out: first_carry_out,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    if !byte_expression_chain_op_is_safe(*first_op, *first_carry_in) {
        return None;
    }
    let first_temp = split_def_as_temp(first_dst)?;

    let MirOp::Binary {
        op: second_op,
        dst: second_dst,
        left: second_left,
        right: second_right,
        width: MirWidth::Byte,
        carry_in: second_carry_in,
        carry_out: second_carry_out,
    } = ops.get(index + 2)?
    else {
        return None;
    };
    if !byte_expression_chain_op_is_safe(*second_op, *second_carry_in) {
        return None;
    }

    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Byte,
    } = ops.get(index + 3)?
    else {
        return None;
    };
    if store_src != second_dst
        || (require_local_deadness && def_is_used_after(ops, index + 3, first_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 4, second_dst))
        || (require_local_deadness && def_is_used_after(ops, index + 2, load_dst))
    {
        return None;
    }

    let producer = MirValue::PointerCell(load_src.clone());
    let first_left = replace_temp_value(first_left.clone(), load_temp, &producer);
    let first_right = replace_temp_value(first_right.clone(), load_temp, &producer);
    if value_uses_temp(&first_left) || value_uses_temp(&first_right) {
        return None;
    }

    let accumulator = MirValue::Def(MirDef::Reg(MirReg::A));
    let second_left = replace_temp_value(second_left.clone(), first_temp, &accumulator);
    let second_right = replace_temp_value(second_right.clone(), first_temp, &accumulator);
    if !matches!(second_left, MirValue::Def(MirDef::Reg(MirReg::A)))
        || value_uses_temp(&second_right)
    {
        return None;
    }

    out.push(MirOp::Move {
        dst: MirDef::Reg(MirReg::A),
        src: first_left,
        width: MirWidth::Byte,
    });
    out.push(MirOp::Binary {
        op: *first_op,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: first_right,
        width: MirWidth::Byte,
        carry_in: explicit_byte_carry(*first_op, *first_carry_in),
        carry_out: *first_carry_out,
    });
    out.push(MirOp::Binary {
        op: *second_op,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: second_right,
        width: MirWidth::Byte,
        carry_in: explicit_byte_carry(*second_op, *second_carry_in),
        carry_out: *second_carry_out,
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(store_dst.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    Some(4)
}

fn byte_expression_chain_op_is_safe(op: MirBinaryOp, carry_in: Option<MirCarryIn>) -> bool {
    matches!(
        op,
        MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor
    ) && !matches!(carry_in, Some(MirCarryIn::FromPrevious))
}

fn try_fuse_two_loaded_byte_store_consumer(
    ops: &[MirOp],
    index: usize,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let first_load = ops.get(index)?;
    let second_load = ops.get(index + 1)?;
    let binary = ops.get(index + 2)?;
    let store = ops.get(index + 3)?;
    let MirOp::Load {
        dst: first_dst,
        src: MirAddr::Direct(first_src),
        width: MirWidth::Byte,
    } = first_load
    else {
        return None;
    };
    let MirOp::Load {
        dst: second_dst,
        src: MirAddr::Direct(second_src),
        width: MirWidth::Byte,
    } = second_load
    else {
        return None;
    };
    let first_temp = split_def_as_temp(first_dst)?;
    let second_temp = split_def_as_temp(second_dst)?;
    let MirOp::Binary {
        op,
        dst: binary_dst,
        left,
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out,
    } = binary
    else {
        return None;
    };
    if !matches!(
        op,
        MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor
    ) {
        return None;
    }
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Byte,
    } = store
    else {
        return None;
    };
    if store_src != binary_dst {
        return None;
    }

    let first_producer = MirValue::PointerCell(first_src.clone());
    let second_producer = MirValue::PointerCell(second_src.clone());
    let left = replace_temp_value(
        replace_temp_value(left.clone(), first_temp, &first_producer),
        second_temp,
        &second_producer,
    );
    let right = replace_temp_value(
        replace_temp_value(right.clone(), first_temp, &first_producer),
        second_temp,
        &second_producer,
    );
    if value_uses_temp(&left) || value_uses_temp(&right) {
        return None;
    }

    materialize_byte_binary_store_consumer(
        *op,
        store_dst.clone(),
        left,
        right,
        explicit_byte_carry(*op, *carry_in),
        *carry_out,
        out,
    );
    Some(4)
}

fn try_fuse_loaded_byte_store_consumer(
    ops: &[MirOp],
    index: usize,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let load = ops.get(index)?;
    let binary = ops.get(index + 1)?;
    let store = ops.get(index + 2)?;
    let MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width: MirWidth::Byte,
    } = load
    else {
        return None;
    };
    let load_temp = split_def_as_temp(load_dst)?;
    let MirOp::Binary {
        op,
        dst: binary_dst,
        left,
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out,
    } = binary
    else {
        return None;
    };
    if !matches!(
        op,
        MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor
    ) {
        return None;
    }
    let MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(store_src),
        width: MirWidth::Byte,
    } = store
    else {
        return None;
    };
    if store_src != binary_dst {
        return None;
    }

    let producer = MirValue::PointerCell(load_src.clone());
    let left = replace_temp_value(left.clone(), load_temp, &producer);
    let right = replace_temp_value(right.clone(), load_temp, &producer);
    if value_uses_temp(&left) || value_uses_temp(&right) {
        return None;
    }

    materialize_byte_binary_store_consumer(
        *op,
        store_dst.clone(),
        left,
        right,
        explicit_byte_carry(*op, *carry_in),
        *carry_out,
        out,
    );
    Some(3)
}

pub(super) fn materialize_byte_binary_store_consumer(
    op: MirBinaryOp,
    dst: MirMem,
    left: MirValue,
    right: MirValue,
    carry_in: Option<MirCarryIn>,
    carry_out: MirCarryOut,
    out: &mut Vec<MirOp>,
) {
    materialize_byte_binary_to_a(op, left, right, carry_in, carry_out, out);
    out.push(MirOp::Store {
        dst: MirAddr::Direct(dst),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
}

fn materialize_byte_binary_store_consumer_for_addr(
    op: MirBinaryOp,
    dst: MirAddr,
    left: MirValue,
    right: MirValue,
    carry_in: Option<MirCarryIn>,
    carry_out: MirCarryOut,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    out: &mut Vec<MirOp>,
) {
    match dst {
        MirAddr::Direct(dst) => {
            materialize_byte_binary_store_consumer(op, dst, left, right, carry_in, carry_out, out)
        }
        MirAddr::Deref { ptr, offset } => {
            let consumer =
                materialize_binary_store_pointer_address(ptr, routine_id, layout, temp_widths, out);
            materialize_byte_binary_to_a(op, left, right, carry_in, carry_out, out);
            out.push(MirOp::StoreIndirect {
                consumer,
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                offset,
            });
        }
        MirAddr::PointerCell { ptr, offset } => {
            let consumer = materialize_binary_store_pointer_address(
                pointer_value_from_mem(&ptr),
                routine_id,
                layout,
                temp_widths,
                out,
            );
            materialize_byte_binary_to_a(op, left, right, carry_in, carry_out, out);
            out.push(MirOp::StoreIndirect {
                consumer,
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                offset,
            });
        }
        MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. } => {
            let parts = indexed_addr_parts(&dst).expect("indexed store target matched above");
            if indexed_addr_has_delayed_index(&parts, delayed_byte_indexes) {
                materialize_indexed_address_for_consumer(
                    parts.clone(),
                    DEFAULT_POINTER_PAIR,
                    layout,
                    Some(delayed_byte_indexes),
                    out,
                );
                materialize_byte_binary_to_a(op, left, right, carry_in, carry_out, out);
                out.push(MirOp::StoreIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    offset: parts.offset,
                });
                return;
            }
            if parts.elem_size == 1 && parts.offset == 0 {
                materialize_binary_store_byte_indexed(
                    parts.base,
                    parts.index,
                    op,
                    left,
                    right,
                    carry_in,
                    carry_out,
                    layout,
                    out,
                );
                return;
            }
            materialize_binary_store_indexed_address(parts.clone(), layout, temp_widths, out);
            materialize_byte_binary_to_a(op, left, right, carry_in, carry_out, out);
            out.push(MirOp::StoreIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                offset: parts.offset,
            });
        }
        _ => unreachable!("unsupported binary store target checked by blocker"),
    }
}

fn materialize_byte_value_store_consumer_for_addr(
    dst: MirAddr,
    value: MirValue,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    out: &mut Vec<MirOp>,
) {
    match dst {
        MirAddr::Direct(dst) => materialize_value_to_mem(value, dst, out),
        MirAddr::Deref { ptr, offset } => {
            let consumer =
                materialize_binary_store_pointer_address(ptr, routine_id, layout, temp_widths, out);
            materialize_value_to_indirect(value, consumer, offset, out);
        }
        MirAddr::PointerCell { ptr, offset } => {
            let consumer = materialize_binary_store_pointer_address(
                pointer_value_from_mem(&ptr),
                routine_id,
                layout,
                temp_widths,
                out,
            );
            materialize_value_to_indirect(value, consumer, offset, out);
        }
        MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. } => {
            let parts = indexed_addr_parts(&dst).expect("indexed store target matched above");
            if indexed_addr_has_delayed_index(&parts, delayed_byte_indexes) {
                if parts.elem_size == 1 {
                    materialize_indexed_write_from_value(
                        parts,
                        value,
                        MirWidth::Byte,
                        layout,
                        Some(delayed_byte_indexes),
                        out,
                    );
                    return;
                }
                materialize_indexed_address_for_consumer(
                    parts.clone(),
                    DEFAULT_POINTER_PAIR,
                    layout,
                    Some(delayed_byte_indexes),
                    out,
                );
                materialize_value_to_indirect(value, DEFAULT_POINTER_PAIR, parts.offset, out);
                return;
            }
            if parts.elem_size == 1 && parts.offset == 0 {
                materialize_byte_value_store_indexed(parts.base, parts.index, value, layout, out);
                return;
            }
            materialize_binary_store_indexed_address(parts.clone(), layout, temp_widths, out);
            materialize_value_to_indirect(value, DEFAULT_POINTER_PAIR, parts.offset, out);
        }
        _ => unreachable!("unsupported binary store target checked by blocker"),
    }
}

fn materialize_byte_value_store_indexed(
    base: MirValue,
    index: MirValue,
    value: MirValue,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    materialize_base_address(base, DEFAULT_POINTER_PAIR, layout, out);
    materialize_index_to_y(index, out);
    out.push(MirOp::Move {
        dst: MirDef::Reg(MirReg::A),
        src: value,
        width: MirWidth::Byte,
    });
    out.push(MirOp::Store {
        dst: MirAddr::FixedIndirectIndexedY {
            zp: MirFixedZpSlot(POINTER_SCRATCH_LO),
        },
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
}

fn materialize_value_to_indirect(
    value: MirValue,
    consumer: MirAddressConsumer,
    offset: u16,
    out: &mut Vec<MirOp>,
) {
    out.push(MirOp::Move {
        dst: MirDef::Reg(MirReg::A),
        src: value,
        width: MirWidth::Byte,
    });
    out.push(MirOp::StoreIndirect {
        consumer,
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        offset,
    });
}

#[allow(clippy::too_many_arguments)]
fn materialize_binary_store_byte_indexed(
    base: MirValue,
    index: MirValue,
    op: MirBinaryOp,
    left: MirValue,
    right: MirValue,
    carry_in: Option<MirCarryIn>,
    carry_out: MirCarryOut,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    materialize_base_address(base, DEFAULT_POINTER_PAIR, layout, out);
    materialize_index_to_y(index, out);
    materialize_byte_binary_to_a(op, left, right, carry_in, carry_out, out);
    out.push(MirOp::Store {
        dst: MirAddr::FixedIndirectIndexedY {
            zp: MirFixedZpSlot(POINTER_SCRATCH_LO),
        },
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
}

fn materialize_binary_store_pointer_address(
    ptr: MirValue,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) -> MirAddressConsumer {
    if value_uses_temp(&ptr) {
        stage_pointer_value_in_default_pair(ptr, layout, temp_widths, out);
        return DEFAULT_POINTER_PAIR;
    }
    materialize_pointer_deref_address(ptr, routine_id, layout, temp_widths, out)
}

fn materialize_binary_store_indexed_address(
    parts: super::indexes::IndexedAddrParts,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    if matches!(parts.index, MirValue::ConstU8(0)) && value_uses_temp(&parts.base) {
        stage_pointer_value_in_default_pair(parts.base, layout, temp_widths, out);
        return;
    }
    materialize_indexed_address_for_consumer(parts, DEFAULT_POINTER_PAIR, layout, None, out);
}

fn stage_pointer_value_in_default_pair(
    value: MirValue,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    let (lo, hi) = split_value_with_temp_widths(value, layout, temp_widths);
    out.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
        src: lo,
        width: MirWidth::Byte,
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
        src: hi,
        width: MirWidth::Byte,
    });
}

fn materialize_byte_binary_to_a(
    op: MirBinaryOp,
    left: MirValue,
    right: MirValue,
    carry_in: Option<MirCarryIn>,
    carry_out: MirCarryOut,
    out: &mut Vec<MirOp>,
) {
    out.push(MirOp::Move {
        dst: MirDef::Reg(MirReg::A),
        src: left,
        width: MirWidth::Byte,
    });
    out.push(MirOp::Binary {
        op,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out,
    });
}

fn explicit_byte_carry(op: MirBinaryOp, carry_in: Option<MirCarryIn>) -> Option<MirCarryIn> {
    carry_in.or(match op {
        MirBinaryOp::Add => Some(MirCarryIn::Clear),
        MirBinaryOp::Sub => Some(MirCarryIn::Set),
        _ => None,
    })
}
