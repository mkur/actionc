use super::*;
use crate::mir6502::analysis::effects::{MirFlagSet, MirHomeByte, classify_op};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirRegisterSet, MirRoutine};
use crate::mir6502::rewrite::context::{MirExitStateChange, MirProof, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::MirPostHomeRewritePlan;
use crate::mir6502::rewrite::posthome::structural_plan;

const DELAYED_BYTE_INDEX_ENABLED: bool = true;

#[derive(Debug, Clone)]
pub(super) struct DelayedByteIndexPlan {
    #[cfg(test)]
    producer_ops: BTreeSet<usize>,
    exprs: BTreeMap<MirTempId, DelayedByteIndexExpr>,
    producer_ops_by_temp: BTreeMap<MirTempId, BTreeSet<usize>>,
}

impl DelayedByteIndexPlan {
    pub(super) fn empty() -> Self {
        Self {
            #[cfg(test)]
            producer_ops: BTreeSet::new(),
            exprs: BTreeMap::new(),
            producer_ops_by_temp: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn producer_ops(&self) -> &BTreeSet<usize> {
        &self.producer_ops
    }

    pub(super) fn expr_for_value(&self, value: &MirValue) -> Option<&DelayedByteIndexExpr> {
        let MirValue::Def(MirDef::VTemp(temp)) = value else {
            return None;
        };
        self.exprs.get(temp)
    }

    pub(super) fn producer_ops_for_value(&self, value: &MirValue) -> Option<&BTreeSet<usize>> {
        let MirValue::Def(MirDef::VTemp(temp)) = value else {
            return None;
        };
        self.producer_ops_by_temp.get(temp)
    }
}

#[derive(Debug, Clone)]
pub(super) enum DelayedByteIndexExpr {
    Value(MirValue),
    Binary {
        op: MirBinaryOp,
        left: Box<DelayedByteIndexExpr>,
        right: MirValue,
        carry_in: Option<MirCarryIn>,
    },
}

#[derive(Debug, Clone)]
struct DelayedByteIndexCandidate {
    producer_index: usize,
    expr: DelayedByteIndexExpr,
    temps: BTreeSet<MirTempId>,
    mems: Vec<MirMem>,
}

#[derive(Debug, Clone)]
pub(super) struct IndexedAddrParts {
    pub(super) base: MirValue,
    pub(super) index: MirValue,
    pub(super) elem_size: u16,
    pub(super) offset: u16,
}

pub(super) fn collect_delayed_byte_index_plan(ops: &[MirOp]) -> DelayedByteIndexPlan {
    if !DELAYED_BYTE_INDEX_ENABLED {
        return DelayedByteIndexPlan::empty();
    }

    let mut candidates = BTreeMap::<MirTempId, DelayedByteIndexCandidate>::new();
    for (index, op) in ops.iter().enumerate() {
        if let Some((temp, candidate)) = delayed_byte_index_candidate(op, index, &candidates) {
            candidates.insert(temp, candidate);
        }
    }

    #[cfg(test)]
    let mut producer_ops = BTreeSet::new();
    let mut exprs = BTreeMap::new();
    let mut producer_ops_by_temp = BTreeMap::new();
    for (use_index, op) in ops.iter().enumerate() {
        let Some(root) = indexed_addr_temp_index(op) else {
            continue;
        };
        let Some(candidate) = candidates.get(&root) else {
            continue;
        };
        if !delayed_byte_index_candidate_is_safe(ops, use_index, candidate, &candidates) {
            continue;
        }
        #[cfg(test)]
        {
            for temp in &candidate.temps {
                if let Some(dep) = candidates.get(temp) {
                    producer_ops.insert(dep.producer_index);
                }
            }
        }
        producer_ops_by_temp.insert(
            root,
            candidate
                .temps
                .iter()
                .filter_map(|temp| candidates.get(temp).map(|dep| dep.producer_index))
                .collect(),
        );
        exprs.insert(root, candidate.expr.clone());
    }

    DelayedByteIndexPlan {
        #[cfg(test)]
        producer_ops,
        exprs,
        producer_ops_by_temp,
    }
}

pub(super) fn materialize_delayed_byte_indexed_read(
    dst: MirDef,
    base: MirValue,
    index: &DelayedByteIndexExpr,
    offset: u16,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if let Some(base) = address_value_mem(&base).map(|mem| offset_mem(&mem, offset)) {
        materialize_delayed_byte_index_to_y(index, out);
        out.push(MirOp::Load {
            dst,
            src: MirAddr::AbsoluteIndexedY { base },
            width: MirWidth::Byte,
        });
        return;
    }
    if offset == 0 {
        materialize_base_address(base, DEFAULT_POINTER_PAIR, layout, out);
        materialize_delayed_byte_index_to_y(index, out);
        out.push(MirOp::Load {
            dst,
            src: MirAddr::FixedIndirectIndexedY {
                zp: MirFixedZpSlot(POINTER_SCRATCH_LO),
            },
            width: MirWidth::Byte,
        });
    } else {
        materialize_delayed_indexed_address_to_consumer(
            base,
            index,
            1,
            DEFAULT_POINTER_PAIR,
            layout,
            out,
        );
        out.push(MirOp::LoadIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            dst,
            offset,
        });
    }
}

pub(super) fn materialize_delayed_byte_indexed_write(
    base: MirValue,
    index: &DelayedByteIndexExpr,
    offset: u16,
    src: MirValue,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if let Some(base) = address_value_mem(&base).map(|mem| offset_mem(&mem, offset)) {
        materialize_delayed_byte_index_to_y(index, out);
        let src = materialize_byte_value_to_a(src, out);
        out.push(MirOp::Store {
            dst: MirAddr::AbsoluteIndexedY { base },
            src,
            width: MirWidth::Byte,
        });
        return;
    }
    if offset == 0 {
        materialize_base_address(base, DEFAULT_POINTER_PAIR, layout, out);
        materialize_delayed_byte_index_to_y(index, out);
        let src = materialize_byte_value_to_a(src, out);
        out.push(MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY {
                zp: MirFixedZpSlot(POINTER_SCRATCH_LO),
            },
            src,
            width: MirWidth::Byte,
        });
    } else {
        materialize_delayed_indexed_address_to_consumer(
            base,
            index,
            1,
            DEFAULT_POINTER_PAIR,
            layout,
            out,
        );
        let src = materialize_byte_value_to_a(src, out);
        out.push(MirOp::StoreIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            src,
            offset,
        });
    }
}

pub(super) fn materialize_delayed_indexed_address_to_consumer(
    base: MirValue,
    index: &DelayedByteIndexExpr,
    elem_size: u16,
    consumer: MirAddressConsumer,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    materialize_base_address(base, consumer, layout, out);
    materialize_delayed_byte_index_to_a(index, out);
    out.push(MirOp::AdvanceAddress {
        consumer,
        index: MirValue::Def(MirDef::Reg(MirReg::A)),
        scale: elem_size.min(u8::MAX as u16) as u8,
    });
}

fn delayed_byte_index_candidate(
    op: &MirOp,
    producer_index: usize,
    candidates: &BTreeMap<MirTempId, DelayedByteIndexCandidate>,
) -> Option<(MirTempId, DelayedByteIndexCandidate)> {
    match op {
        MirOp::Load {
            dst,
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } if delayed_index_mem_is_stable_source(mem) => {
            let temp = split_def_as_temp(dst)?;
            let mut temps = BTreeSet::new();
            temps.insert(temp);
            let mems = vec![mem.clone()];
            Some((
                temp,
                DelayedByteIndexCandidate {
                    producer_index,
                    expr: DelayedByteIndexExpr::Value(MirValue::PointerCell(mem.clone())),
                    temps,
                    mems,
                },
            ))
        }
        MirOp::LoadImm {
            dst,
            value,
            width: MirWidth::Byte,
        } if *value <= 0x00FF => {
            let temp = split_def_as_temp(dst)?;
            let mut temps = BTreeSet::new();
            temps.insert(temp);
            Some((
                temp,
                DelayedByteIndexCandidate {
                    producer_index,
                    expr: DelayedByteIndexExpr::Value(MirValue::ConstU8(*value as u8)),
                    temps,
                    mems: Vec::new(),
                },
            ))
        }
        MirOp::Move {
            dst,
            src,
            width: MirWidth::Byte,
        } => {
            let temp = split_def_as_temp(dst)?;
            let expr = delayed_stable_value(src)?;
            let mut temps = BTreeSet::new();
            temps.insert(temp);
            Some((
                temp,
                DelayedByteIndexCandidate {
                    producer_index,
                    expr: DelayedByteIndexExpr::Value(expr),
                    temps,
                    mems: delayed_value_mems(src),
                },
            ))
        }
        MirOp::Binary {
            op,
            dst,
            left,
            right,
            width: MirWidth::Byte,
            carry_in,
            carry_out: MirCarryOut::Ignore,
        } if delayed_byte_binary_op_is_supported(*op)
            && !matches!(carry_in, Some(MirCarryIn::FromPrevious)) =>
        {
            let temp = split_def_as_temp(dst)?;
            let left = delayed_expr_from_value(left, candidates)?;
            let right = delayed_simple_value(right, candidates)?;
            let mut temps = BTreeSet::new();
            temps.insert(temp);
            temps.extend(left.temps.iter().copied());
            temps.extend(right.temps.iter().copied());
            let mut mems = left.mems;
            mems.extend(right.mems);
            Some((
                temp,
                DelayedByteIndexCandidate {
                    producer_index,
                    expr: DelayedByteIndexExpr::Binary {
                        op: *op,
                        left: Box::new(left.expr),
                        right: right.value,
                        carry_in: *carry_in,
                    },
                    temps,
                    mems,
                },
            ))
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct DelayedResolvedExpr {
    expr: DelayedByteIndexExpr,
    temps: BTreeSet<MirTempId>,
    mems: Vec<MirMem>,
}

#[derive(Debug, Clone)]
struct DelayedResolvedValue {
    value: MirValue,
    temps: BTreeSet<MirTempId>,
    mems: Vec<MirMem>,
}

fn delayed_expr_from_value(
    value: &MirValue,
    candidates: &BTreeMap<MirTempId, DelayedByteIndexCandidate>,
) -> Option<DelayedResolvedExpr> {
    if let MirValue::Def(MirDef::VTemp(temp)) = value {
        let candidate = candidates.get(temp)?;
        return Some(DelayedResolvedExpr {
            expr: candidate.expr.clone(),
            temps: candidate.temps.clone(),
            mems: candidate.mems.clone(),
        });
    }
    let value = delayed_stable_value(value)?;
    let mems = delayed_value_mems(&value);
    Some(DelayedResolvedExpr {
        expr: DelayedByteIndexExpr::Value(value),
        temps: BTreeSet::new(),
        mems,
    })
}

fn delayed_simple_value(
    value: &MirValue,
    candidates: &BTreeMap<MirTempId, DelayedByteIndexCandidate>,
) -> Option<DelayedResolvedValue> {
    if let MirValue::Def(MirDef::VTemp(temp)) = value {
        let candidate = candidates.get(temp)?;
        if let DelayedByteIndexExpr::Value(value) = &candidate.expr {
            return Some(DelayedResolvedValue {
                value: value.clone(),
                temps: candidate.temps.clone(),
                mems: candidate.mems.clone(),
            });
        }
        return None;
    }
    let value = delayed_stable_value(value)?;
    let mems = delayed_value_mems(&value);
    Some(DelayedResolvedValue {
        value,
        temps: BTreeSet::new(),
        mems,
    })
}

fn delayed_stable_value(value: &MirValue) -> Option<MirValue> {
    match value {
        MirValue::ConstU8(_)
        | MirValue::PointerCell(_)
        | MirValue::StorageAddrByte { .. }
        | MirValue::RoutineAddrByte { .. } => Some(value.clone()),
        MirValue::ConstU16(value) if *value <= 0x00FF => Some(MirValue::ConstU8(*value as u8)),
        _ => None,
    }
}

fn delayed_value_mems(value: &MirValue) -> Vec<MirMem> {
    let mut mems = Vec::new();
    match value {
        MirValue::PointerCell(mem) | MirValue::StorageAddrByte { mem, .. } => {
            mems.push(mem.clone());
        }
        MirValue::Word { lo, hi } => {
            mems.extend(delayed_value_mems(lo));
            mems.extend(delayed_value_mems(hi));
        }
        _ => {}
    }
    mems
}

fn indexed_addr_temp_index(op: &MirOp) -> Option<MirTempId> {
    let index = match op {
        MirOp::Load {
            src:
                MirAddr::ComputedIndex {
                    index,
                    elem_size: 1,
                    ..
                }
                | MirAddr::PointerIndex {
                    index,
                    elem_size: 1,
                    ..
                },
            width: MirWidth::Byte,
            ..
        }
        | MirOp::Store {
            dst:
                MirAddr::ComputedIndex {
                    index,
                    elem_size: 1,
                    ..
                }
                | MirAddr::PointerIndex {
                    index,
                    elem_size: 1,
                    ..
                },
            width: MirWidth::Byte,
            ..
        } => index,
        _ => return None,
    };
    let MirValue::Def(MirDef::VTemp(temp)) = index else {
        return None;
    };
    Some(*temp)
}

fn delayed_byte_index_candidate_is_safe(
    ops: &[MirOp],
    use_index: usize,
    candidate: &DelayedByteIndexCandidate,
    candidates: &BTreeMap<MirTempId, DelayedByteIndexCandidate>,
) -> bool {
    let Some(first_producer) = candidate
        .temps
        .iter()
        .filter_map(|temp| {
            candidates
                .get(temp)
                .map(|candidate| candidate.producer_index)
        })
        .min()
    else {
        return false;
    };
    if first_producer >= use_index {
        return false;
    }

    let producer_indices = candidate
        .temps
        .iter()
        .filter_map(|temp| {
            candidates
                .get(temp)
                .map(|candidate| candidate.producer_index)
        })
        .collect::<BTreeSet<_>>();
    let mut allowed_users = producer_indices.clone();
    allowed_users.insert(use_index);

    for temp in &candidate.temps {
        let Some(producer) = candidates.get(temp) else {
            return false;
        };
        // A delayed expression is structurally single-owner: every local use
        // of its producer chain must belong to that expression and indexed
        // consumer. Terminator and successor uses are deliberately left to
        // the shared routine-wide definition/deadness proof.
        for (op_index, op) in ops.iter().enumerate().skip(producer.producer_index + 1) {
            if !op_uses_temp(op, *temp) {
                continue;
            }
            if !allowed_users.contains(&op_index) || op_uses_temp_more_than_once(op, *temp) {
                return false;
            }
        }
    }

    if ops[first_producer + 1..use_index]
        .iter()
        .any(op_uses_previous_carry)
    {
        return false;
    }
    candidate.mems.iter().all(|mem| {
        ops[first_producer + 1..use_index]
            .iter()
            .all(|op| !op_may_write_mem(op, mem))
    })
}

fn delayed_byte_binary_op_is_supported(op: MirBinaryOp) -> bool {
    matches!(
        op,
        MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor
    )
}

fn delayed_index_mem_is_stable_source(mem: &MirMem) -> bool {
    !matches!(
        mem,
        MirMem::Spill { .. } | MirMem::ZeroPage(_) | MirMem::FixedZeroPage(_)
    )
}

fn materialize_delayed_byte_index_to_y(index: &DelayedByteIndexExpr, out: &mut Vec<MirOp>) {
    match index {
        DelayedByteIndexExpr::Value(value) => materialize_index_to_y(value.clone(), out),
        DelayedByteIndexExpr::Binary { .. } => {
            materialize_delayed_byte_index_to_a(index, out);
            out.push(MirOp::Move {
                dst: MirDef::Reg(MirReg::Y),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            });
        }
    }
}

fn materialize_delayed_byte_index_to_a(index: &DelayedByteIndexExpr, out: &mut Vec<MirOp>) {
    match index {
        DelayedByteIndexExpr::Value(value) => materialize_byte_value_to_a_reg(value.clone(), out),
        DelayedByteIndexExpr::Binary {
            op,
            left,
            right,
            carry_in,
        } => {
            materialize_delayed_byte_index_to_a(left, out);
            out.push(MirOp::Binary {
                op: *op,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: right.clone(),
                width: MirWidth::Byte,
                carry_in: normalized_byte_carry_in(*op, *carry_in),
                carry_out: MirCarryOut::Ignore,
            });
        }
    }
}

fn materialize_byte_value_to_a_reg(value: MirValue, out: &mut Vec<MirOp>) {
    match value {
        MirValue::ConstU8(value) => out.push(MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: u16::from(value),
            width: MirWidth::Byte,
        }),
        MirValue::PointerCell(mem) => out.push(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }),
        other => out.push(MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: other,
            width: MirWidth::Byte,
        }),
    }
}

fn normalized_byte_carry_in(op: MirBinaryOp, carry_in: Option<MirCarryIn>) -> Option<MirCarryIn> {
    match (op, carry_in) {
        (MirBinaryOp::Add, None) => Some(MirCarryIn::Clear),
        (MirBinaryOp::Sub, None) => Some(MirCarryIn::Set),
        _ => carry_in,
    }
}

pub(super) fn try_fuse_indexed_byte_copy(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(MirOp::Load {
        dst: load_dst,
        src,
        width: MirWidth::Byte,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(src_parts) = indexed_addr_parts(src) else {
        return 0;
    };
    let Some(MirOp::Store {
        dst,
        src: MirValue::Def(store_src),
        width: MirWidth::Byte,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    if store_src != load_dst {
        return 0;
    }
    let Some(dst_parts) = indexed_addr_parts(dst) else {
        return 0;
    };

    materialize_indexed_address_for_consumer(
        dst_parts.clone(),
        DEST_POINTER_PAIR,
        layout,
        Some(delayed_byte_indexes),
        out,
    );
    materialize_indexed_address_for_consumer(
        src_parts.clone(),
        DEFAULT_POINTER_PAIR,
        layout,
        Some(delayed_byte_indexes),
        out,
    );
    out.push(MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset: src_parts.offset,
    });
    out.push(MirOp::StoreIndirect {
        consumer: DEST_POINTER_PAIR,
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        offset: dst_parts.offset,
    });
    2
}

pub(super) fn materialize_indexed_address_for_consumer(
    parts: IndexedAddrParts,
    consumer: MirAddressConsumer,
    layout: &MaterializeLayout,
    delayed_byte_indexes: Option<&DelayedByteIndexPlan>,
    out: &mut Vec<MirOp>,
) -> bool {
    if let Some(delayed) = delayed_byte_indexes.and_then(|plan| plan.expr_for_value(&parts.index)) {
        materialize_delayed_indexed_address_to_consumer(
            parts.base,
            delayed,
            parts.elem_size,
            consumer,
            layout,
            out,
        );
        true
    } else {
        materialize_indexed_address_to_consumer(
            parts.base,
            parts.index,
            parts.elem_size,
            consumer,
            layout,
            out,
        );
        false
    }
}

pub(super) fn indexed_addr_has_delayed_index(
    parts: &IndexedAddrParts,
    delayed_byte_indexes: &DelayedByteIndexPlan,
) -> bool {
    delayed_byte_indexes.expr_for_value(&parts.index).is_some()
}

pub(super) fn try_fuse_indexed_word_copy(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    // Delayed byte-index producers are currently only skipped for byte-width,
    // elem-size-1 accesses. If that plan grows to cover word copies or wider
    // element sizes, this fused path must rematerialize delayed indexes too.
    let Some(MirOp::Load {
        dst: load_dst,
        src,
        width: MirWidth::Word,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some((src_base, src_index, src_elem_size, src_offset)) =
        indexed_addr_parts_resolved_for_copy(ops, index, src)
    else {
        return 0;
    };
    let Some(MirOp::Store {
        dst,
        src: MirValue::Def(store_src),
        width: MirWidth::Word,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    if store_src != load_dst {
        return 0;
    }
    let Some((dst_base, dst_index, dst_elem_size, dst_offset)) =
        indexed_addr_parts_resolved_for_copy(ops, index + 1, dst)
    else {
        return 0;
    };

    materialize_indexed_address_to_consumer(
        dst_base,
        dst_index,
        dst_elem_size,
        INDEX_POINTER_PAIR,
        layout,
        out,
    );
    materialize_indexed_address_to_consumer(
        src_base,
        src_index,
        src_elem_size,
        DEFAULT_POINTER_PAIR,
        layout,
        out,
    );
    out.push(MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset: src_offset,
    });
    out.push(MirOp::StoreIndirect {
        consumer: INDEX_POINTER_PAIR,
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        offset: dst_offset,
    });
    out.push(MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset: src_offset.saturating_add(1),
    });
    out.push(MirOp::StoreIndirect {
        consumer: INDEX_POINTER_PAIR,
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        offset: dst_offset.saturating_add(1),
    });
    2
}

pub(super) fn indexed_addr_parts(addr: &MirAddr) -> Option<IndexedAddrParts> {
    match addr {
        MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset,
        } => Some(IndexedAddrParts {
            base: base.clone(),
            index: index.clone(),
            elem_size: *elem_size,
            offset: *offset,
        }),
        MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset,
        } => Some(IndexedAddrParts {
            base: pointer_value_from_mem(ptr),
            index: index.clone(),
            elem_size: *elem_size,
            offset: *offset,
        }),
        _ => None,
    }
}

fn indexed_addr_parts_resolved_for_copy(
    ops: &[MirOp],
    use_index: usize,
    addr: &MirAddr,
) -> Option<(MirValue, MirValue, u16, u16)> {
    let parts = indexed_addr_parts(addr)?;
    Some((
        resolve_indexed_base_producer(ops, use_index, parts.base),
        resolve_indexed_byte_index_producer(ops, use_index, parts.index),
        parts.elem_size,
        parts.offset,
    ))
}

pub(super) fn indexed_word_copy_rematerialized_producer_ops(
    ops: &[MirOp],
    index: usize,
) -> BTreeSet<usize> {
    [index, index.saturating_add(1)]
        .into_iter()
        .filter_map(|use_index| match ops.get(use_index) {
            Some(MirOp::Load { src, .. }) | Some(MirOp::Store { dst: src, .. }) => {
                indexed_addr_parts(src).map(|parts| (use_index, parts))
            }
            _ => None,
        })
        .flat_map(|(use_index, parts)| {
            let base = resolve_indexed_base_producer(ops, use_index, parts.base.clone());
            let index = resolve_indexed_byte_index_producer(ops, use_index, parts.index.clone());
            [(parts.base, base), (parts.index, index)]
                .into_iter()
                .filter_map(move |(original, resolved)| {
                    if original == resolved {
                        return None;
                    }
                    let MirValue::Def(MirDef::VTemp(temp)) = original else {
                        return None;
                    };
                    find_temp_producer(ops, use_index, temp)
                        .map(|(producer_index, _)| producer_index)
                })
        })
        .collect()
}

fn resolve_indexed_base_producer(ops: &[MirOp], use_index: usize, value: MirValue) -> MirValue {
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
        } if indexed_producer_mem_is_stable_source(mem)
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

fn resolve_indexed_byte_index_producer(
    ops: &[MirOp],
    use_index: usize,
    value: MirValue,
) -> MirValue {
    let MirValue::Def(MirDef::VTemp(temp)) = value else {
        return value;
    };
    let Some((producer_index, producer)) = find_temp_producer(ops, use_index, temp) else {
        return MirValue::Def(MirDef::VTemp(temp));
    };
    match producer {
        MirOp::Load {
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
            ..
        } if indexed_producer_mem_is_stable_source(mem)
            && mem_is_stable_until(ops, producer_index + 1, use_index, mem) =>
        {
            MirValue::PointerCell(mem.clone())
        }
        MirOp::LoadImm {
            value,
            width: MirWidth::Byte,
            ..
        } if *value <= 0x00FF => MirValue::ConstU8(*value as u8),
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

fn indexed_producer_mem_is_stable_source(mem: &MirMem) -> bool {
    !matches!(mem, MirMem::Spill { .. } | MirMem::ZeroPage(_))
}

pub(in crate::mir6502) fn discover_indexed_base_pointer_staging(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for materialize_index in 0..block.ops.len() {
            let Some((consumer, base_lo, base_hi)) =
                materialize_indexed_base_pointer_cells(&block.ops[materialize_index])
            else {
                continue;
            };
            if !materialized_pointer_has_word_store_consumers(
                &block.ops,
                materialize_index,
                consumer,
            ) {
                continue;
            }
            let Some(staging) = indexed_base_pointer_staging_shape_at(
                &block.ops,
                materialize_index,
                &base_lo,
                &base_hi,
            ) else {
                continue;
            };
            let start = staging.lo_load_index;
            let remove = BTreeSet::from([
                staging.lo_load_index,
                staging.lo_store_index,
                staging.hi_load_index,
                staging.hi_store_index,
            ]);
            let mut replacement = Vec::new();
            for index in start..=materialize_index {
                if remove.contains(&index) {
                    continue;
                }
                let mut op = block.ops[index].clone();
                if index == materialize_index
                    && let MirOp::MaterializeIndexedAddress { base, .. } = &mut op
                {
                    *base = pointer_value_from_mem(&staging.source_lo);
                }
                replacement.push(op);
            }
            if let Some(plan) = structural_plan(
                routine,
                context,
                block.id,
                start..materialize_index + 1,
                replacement,
                MirExitStateChange::default(),
                "indexed-base-pointer-staging",
                0,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_scaled_y_word_reads(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirPostHomeRewritePlan> {
    const STAT: &str = "scaled-y-word-read";
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for materialize_index in 0..block.ops.len() {
            let MirOp::MaterializeIndexedAddress {
                consumer: MirAddressConsumer::IndirectIndexedY(pair),
                base,
                index,
                scale: 2,
            } = &block.ops[materialize_index]
            else {
                continue;
            };
            if !scaled_y_posthome_index_is_eligible(index)
                || !scaled_y_posthome_base_is_eligible(base, layout)
            {
                continue;
            }
            let MirPointerPair::Fixed { lo } = pair else {
                continue;
            };
            let consumer = MirAddressConsumer::IndirectIndexedY(*pair);
            let Some(last_access) = scaled_y_read_window_end(
                &block.ops,
                materialize_index,
                consumer,
                [
                    MirHomeByte::FixedZeroPage(*lo),
                    MirHomeByte::FixedZeroPage(MirFixedZpSlot(lo.0.saturating_add(1))),
                ],
            ) else {
                continue;
            };

            let materialize_point = context.point(MirSite::Op {
                block: block.id,
                op_index: materialize_index,
            });
            if let MirProof::Blocked(blocker) =
                context.register_dead_after(MirReg::A, materialize_point)
            {
                context.record_blocker(STAT, block.id, materialize_index, &blocker);
                continue;
            }
            if let MirProof::Blocked(blocker) =
                context.flags_dead_after(MirFlagSet::all(), materialize_point)
            {
                context.record_blocker(STAT, block.id, materialize_index, &blocker);
                continue;
            }

            let scaled_consumer = MirAddressConsumer::ScaledIndirectIndexedY(*pair);
            let replacement = block.ops[materialize_index..=last_access]
                .iter()
                .cloned()
                .map(|mut op| {
                    match &mut op {
                        MirOp::MaterializeIndexedAddress {
                            consumer: op_consumer,
                            ..
                        }
                        | MirOp::LoadIndirect {
                            consumer: op_consumer,
                            ..
                        } if *op_consumer == consumer => *op_consumer = scaled_consumer,
                        _ => {}
                    }
                    op
                })
                .collect();
            let exit_state_change = MirExitStateChange {
                registers: MirRegisterSet {
                    y: true,
                    ..MirRegisterSet::default()
                },
                homes: BTreeSet::from([
                    MirHomeByte::FixedZeroPage(*lo),
                    MirHomeByte::FixedZeroPage(MirFixedZpSlot(lo.0.saturating_add(1))),
                ]),
                ..MirExitStateChange::default()
            };
            if let Some(plan) = structural_plan(
                routine,
                context,
                block.id,
                materialize_index..last_access + 1,
                replacement,
                exit_state_change,
                STAT,
                0,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

fn scaled_y_read_window_end(
    ops: &[MirOp],
    materialize_index: usize,
    consumer: MirAddressConsumer,
    pair_homes: [MirHomeByte; 2],
) -> Option<usize> {
    let mut low_access = None;
    for (index, op) in ops.iter().enumerate().skip(materialize_index + 1) {
        match op {
            MirOp::LoadIndirect {
                consumer: op_consumer,
                offset,
                ..
            } if *op_consumer == consumer => match (*offset, low_access) {
                (0, None) => low_access = Some(index),
                (1, None) => return Some(index),
                (1, Some(_)) => return Some(index),
                _ => return None,
            },
            MirOp::MaterializeAddress {
                consumer: op_consumer,
                ..
            }
            | MirOp::MaterializeIndexedAddress {
                consumer: op_consumer,
                ..
            }
            | MirOp::AdvanceAddress {
                consumer: op_consumer,
                ..
            } if op_consumer.pointer_pair() == consumer.pointer_pair() => break,
            MirOp::StoreIndirect {
                consumer: op_consumer,
                ..
            } if op_consumer.pointer_pair() == consumer.pointer_pair() => return None,
            MirOp::IndirectByteCompound { target, source, .. }
                if target.pointer_pair() == consumer.pointer_pair()
                    || source.pointer_pair() == consumer.pointer_pair() =>
            {
                return None;
            }
            _ => {
                let effects = classify_op(op);
                if effects.reads_reg(MirReg::Y)
                    || effects.may_clobber_reg_compat(MirReg::Y)
                    || pair_homes.iter().any(|home| {
                        effects.homes.reads.contains(home)
                            || effects.homes.writes.contains(home)
                            || effects.addresses.pair_reads.contains(home)
                            || effects.addresses.pair_writes.contains(home)
                    })
                {
                    return None;
                }
            }
        }
    }
    low_access
}

fn scaled_y_posthome_index_is_eligible(index: &MirValue) -> bool {
    matches!(
        index,
        MirValue::ConstU8(_)
            | MirValue::ConstU16(0..=0x00FF)
            | MirValue::PointerCell(_)
            | MirValue::StorageAddrByte { .. }
            | MirValue::RoutineAddrByte { .. }
            | MirValue::Def(MirDef::Reg(_))
    )
}

fn scaled_y_posthome_base_is_eligible(base: &MirValue, layout: &MaterializeLayout) -> bool {
    let (lo, hi) = split_value_as_word(base.clone(), layout);
    [lo, hi].iter().all(|byte| {
        matches!(
            byte,
            MirValue::ConstU8(_)
                | MirValue::ConstU16(0..=0x00FF)
                | MirValue::PointerCell(_)
                | MirValue::StorageAddrByte { .. }
                | MirValue::RoutineAddrByte { .. }
        )
    })
}

#[derive(Debug, Clone)]
struct IndexedBasePointerStaging {
    lo_load_index: usize,
    lo_store_index: usize,
    hi_load_index: usize,
    hi_store_index: usize,
    source_lo: MirMem,
}

fn materialize_indexed_base_pointer_cells(
    op: &MirOp,
) -> Option<(MirAddressConsumer, MirMem, MirMem)> {
    let MirOp::MaterializeIndexedAddress {
        consumer,
        base: MirValue::Word { lo, hi },
        scale: 2,
        ..
    } = op
    else {
        return None;
    };
    let (MirValue::PointerCell(base_lo), MirValue::PointerCell(base_hi)) =
        (lo.as_ref(), hi.as_ref())
    else {
        return None;
    };
    Some((*consumer, base_lo.clone(), base_hi.clone()))
}

fn materialized_pointer_has_word_store_consumers(
    ops: &[MirOp],
    materialize_index: usize,
    consumer: MirAddressConsumer,
) -> bool {
    let mut stores = 0usize;
    for op in ops.iter().skip(materialize_index + 1) {
        match op {
            MirOp::StoreIndirect {
                consumer: store_consumer,
                ..
            } if *store_consumer == consumer => {
                stores = stores.saturating_add(1);
                if stores >= 2 {
                    return true;
                }
            }
            MirOp::MaterializeAddress {
                consumer: op_consumer,
                ..
            }
            | MirOp::MaterializeIndexedAddress {
                consumer: op_consumer,
                ..
            }
            | MirOp::AdvanceAddress {
                consumer: op_consumer,
                ..
            } if *op_consumer == consumer => return false,
            MirOp::Call { .. }
            | MirOp::RuntimeHelper { .. }
            | MirOp::Barrier { .. }
            | MirOp::MachineBlock { .. } => return false,
            _ => {}
        }
    }
    false
}

fn indexed_base_pointer_staging_shape_at(
    ops: &[MirOp],
    materialize_index: usize,
    base_lo: &MirMem,
    base_hi: &MirMem,
) -> Option<IndexedBasePointerStaging> {
    let hi_store_index = find_previous_store_a_to_mem(ops, materialize_index, base_hi)?;
    let (hi_load_index, source_hi) = load_a_direct_before_store(ops, hi_store_index)?;
    let lo_store_index = find_previous_store_a_to_mem(ops, hi_load_index, base_lo)?;
    let (lo_load_index, source_lo) = load_a_direct_before_store(ops, lo_store_index)?;
    if source_hi != offset_mem(&source_lo, 1) {
        return None;
    }
    if !indexed_producer_mem_is_stable_source(&source_lo) {
        return None;
    }
    let staging_indices = [lo_load_index, lo_store_index, hi_load_index, hi_store_index];
    if !staged_pointer_fold_is_safe(
        ops,
        materialize_index,
        base_lo,
        base_hi,
        &source_lo,
        &source_hi,
        &staging_indices,
    ) {
        return None;
    }
    Some(IndexedBasePointerStaging {
        lo_load_index,
        lo_store_index,
        hi_load_index,
        hi_store_index,
        source_lo,
    })
}

fn find_previous_store_a_to_mem(ops: &[MirOp], before: usize, mem: &MirMem) -> Option<usize> {
    ops[..before].iter().rposition(|op| {
        matches!(
            op,
            MirOp::Store {
                dst: MirAddr::Direct(dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            } if dst == mem
        )
    })
}

fn load_a_direct_before_store(ops: &[MirOp], store_index: usize) -> Option<(usize, MirMem)> {
    let load_index = store_index.checked_sub(1)?;
    let MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(src),
        width: MirWidth::Byte,
    } = &ops[load_index]
    else {
        return None;
    };
    Some((load_index, src.clone()))
}

fn staged_pointer_fold_is_safe(
    ops: &[MirOp],
    materialize_index: usize,
    base_lo: &MirMem,
    base_hi: &MirMem,
    source_lo: &MirMem,
    source_hi: &MirMem,
    staging_indices: &[usize; 4],
) -> bool {
    let lo_load_index = staging_indices[0];
    let lo_store_index = staging_indices[1];
    let hi_load_index = staging_indices[2];
    let hi_store_index = staging_indices[3];
    if !mem_is_stable_except(
        ops,
        lo_load_index + 1,
        materialize_index,
        source_lo,
        staging_indices,
    ) || !mem_is_stable_except(
        ops,
        hi_load_index + 1,
        materialize_index,
        source_hi,
        staging_indices,
    ) {
        return false;
    }
    for (index, op) in ops.iter().enumerate().take(materialize_index) {
        if staging_indices.contains(&index) {
            continue;
        }
        if index > lo_store_index && (op_reads_mem(op, base_lo) || op_may_write_mem(op, base_lo)) {
            return false;
        }
        if index > hi_store_index && (op_reads_mem(op, base_hi) || op_may_write_mem(op, base_hi)) {
            return false;
        }
    }
    true
}

fn mem_is_stable_except(
    ops: &[MirOp],
    start: usize,
    end: usize,
    mem: &MirMem,
    ignored_indices: &[usize; 4],
) -> bool {
    ops[start..end].iter().enumerate().all(|(offset, op)| {
        let index = start + offset;
        ignored_indices.contains(&index) || !op_may_write_mem(op, mem)
    })
}

pub(super) fn try_fuse_dynamic_inline_byte_index(
    ops: &[MirOp],
    index: usize,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(MirOp::LeaAddr {
        dst: base_dst,
        target,
        width: MirWidth::Word,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(base_temp) = split_def_as_temp(base_dst) else {
        return 0;
    };
    let Some((index_temp, index_value, consumed_index_ops)) =
        dynamic_byte_index_value(ops, index + 1)
    else {
        return 0;
    };
    let access_index = index + 1 + consumed_index_ops;
    let Some(access) = ops.get(access_index) else {
        return 0;
    };

    match access {
        MirOp::Load {
            dst: load_dst,
            src:
                MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(access_base)),
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size: 1,
                    offset,
                },
            width: MirWidth::Byte,
        } if *access_base == base_temp && *access_index_temp == index_temp => {
            if let Some(MirOp::Store {
                dst: store_dst,
                src: MirValue::Def(store_src),
                width: MirWidth::Byte,
            }) = ops.get(access_index + 1)
                && store_src == load_dst
            {
                if indexed_addr_parts(store_dst).is_some() {
                    return 0;
                }
                materialize_index_to_y(index_value, out);
                out.push(MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::AbsoluteIndexedY {
                        base: offset_mem(target, *offset),
                    },
                    width: MirWidth::Byte,
                });
                out.push(MirOp::Store {
                    dst: store_dst.clone(),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                });
                return consumed_index_ops + 3;
            }
            0
        }
        MirOp::Store {
            dst:
                MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(access_base)),
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size: 1,
                    offset,
                },
            src,
            width: MirWidth::Byte,
        } if *access_base == base_temp && *access_index_temp == index_temp => {
            materialize_index_to_y(index_value, out);
            let src = materialize_byte_value_to_a(src.clone(), out);
            out.push(MirOp::Store {
                dst: MirAddr::AbsoluteIndexedY {
                    base: offset_mem(target, *offset),
                },
                src,
                width: MirWidth::Byte,
            });
            consumed_index_ops + 2
        }
        _ => 0,
    }
}

fn dynamic_byte_index_value(ops: &[MirOp], index: usize) -> Option<(MirTempId, MirValue, usize)> {
    match ops.get(index)? {
        MirOp::Load {
            dst,
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => Some((
            split_def_as_temp(dst)?,
            MirValue::PointerCell(mem.clone()),
            1,
        )),
        MirOp::LoadImm {
            dst,
            value,
            width: MirWidth::Byte,
        } => Some((split_def_as_temp(dst)?, MirValue::ConstU8(*value as u8), 1)),
        MirOp::Move {
            dst,
            src,
            width: MirWidth::Byte,
        } if !value_uses_temp(src) => Some((split_def_as_temp(dst)?, src.clone(), 1)),
        _ => None,
    }
}

pub(super) fn try_prepare_dynamic_byte_index(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some((index_temp, index_value, consumed_index_ops)) = dynamic_byte_index_value(ops, index)
    else {
        return 0;
    };
    let access_index = index + consumed_index_ops;
    let Some(access) = ops.get(access_index) else {
        return 0;
    };
    match access {
        MirOp::Load {
            dst: load_dst,
            src:
                MirAddr::PointerIndex {
                    ptr,
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size: 1,
                    offset,
                },
            width: MirWidth::Byte,
        } if *access_index_temp == index_temp => {
            let Some(MirOp::Store {
                dst: store_dst,
                src: MirValue::Def(store_src),
                width: MirWidth::Byte,
            }) = ops.get(access_index + 1)
            else {
                return 0;
            };
            if store_src != load_dst {
                return 0;
            }
            if indexed_addr_parts(store_dst).is_some() {
                return 0;
            }
            materialize_dynamic_byte_index_read(
                pointer_value_from_mem(ptr),
                index_value,
                *offset,
                store_dst,
                layout,
                out,
            );
            consumed_index_ops + 2
        }
        MirOp::Store {
            dst:
                MirAddr::PointerIndex {
                    ptr,
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size: 1,
                    offset,
                },
            src,
            width: MirWidth::Byte,
        } if *access_index_temp == index_temp => {
            materialize_dynamic_byte_index_write(
                pointer_value_from_mem(ptr),
                index_value,
                *offset,
                src.clone(),
                layout,
                out,
            );
            consumed_index_ops + 1
        }
        _ => 0,
    }
}

pub(super) fn try_prepare_dynamic_word_index(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    if let Some(consumed) =
        try_prepare_pointer_word_index_with_lea(ops, index, routine_id, layout, out)
    {
        return consumed;
    }

    let Some((index_temp, index_value, consumed_index_ops)) = dynamic_byte_index_value(ops, index)
    else {
        return 0;
    };
    let access_index = index + consumed_index_ops;
    let Some(access) = ops.get(access_index) else {
        return 0;
    };
    match access {
        MirOp::Load {
            dst: load_dst,
            src:
                MirAddr::PointerIndex {
                    ptr,
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size,
                    offset,
                },
            width: MirWidth::Word,
        } if *access_index_temp == index_temp && *elem_size > 1 => {
            let Some(MirOp::Store {
                dst: MirAddr::Direct(store_dst),
                src: MirValue::Def(store_src),
                width: MirWidth::Word,
            }) = ops.get(access_index + 1)
            else {
                return 0;
            };
            if store_src != load_dst {
                return 0;
            }
            materialize_dynamic_word_index_read(
                pointer_value_from_mem(ptr),
                index_value,
                *offset,
                store_dst,
                out,
            );
            consumed_index_ops + 2
        }
        MirOp::Store {
            dst:
                MirAddr::PointerIndex {
                    ptr,
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size,
                    offset,
                },
            src,
            width: MirWidth::Word,
        } if *access_index_temp == index_temp && *elem_size > 1 => {
            materialize_dynamic_word_index_write(
                pointer_value_from_mem(ptr),
                index_value,
                *offset,
                src.clone(),
                layout,
                out,
            );
            consumed_index_ops + 1
        }
        MirOp::Store {
            dst:
                MirAddr::ComputedIndex {
                    base,
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size,
                    offset,
                },
            src,
            width: MirWidth::Word,
        } if *access_index_temp == index_temp && *elem_size > 1 => {
            let Some(base) = base_pointer_from_address_value(base) else {
                return 0;
            };
            materialize_dynamic_word_index_write(
                base,
                index_value,
                *offset,
                src.clone(),
                layout,
                out,
            );
            consumed_index_ops + 1
        }
        _ => 0,
    }
}

fn try_prepare_pointer_word_index_with_lea(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let MirOp::LeaAddr {
        dst: base_dst,
        target,
        width: MirWidth::Word,
    } = ops.get(index)?
    else {
        return None;
    };
    let base_temp = split_def_as_temp(base_dst)?;
    let (index_temp, index_value, consumed_index_ops) = dynamic_byte_index_value(ops, index + 1)?;
    let access_index = index + 1 + consumed_index_ops;
    match ops.get(access_index)? {
        MirOp::Load {
            dst: load_dst,
            src:
                MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(access_base)),
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size,
                    offset,
                },
            width: MirWidth::Word,
        } if *access_base == base_temp && *access_index_temp == index_temp && *elem_size > 1 => {
            let MirOp::Store {
                dst: MirAddr::Direct(store_dst),
                src: MirValue::Def(store_src),
                width: MirWidth::Word,
            } = ops.get(access_index + 1)?
            else {
                return None;
            };
            if store_src != load_dst {
                return None;
            }
            materialize_dynamic_word_index_read(
                indexed_base_from_lea_target(target, routine_id, layout),
                index_value,
                *offset,
                store_dst,
                out,
            );
            Some(consumed_index_ops + 3)
        }
        MirOp::Load {
            dst: load_dst,
            src:
                MirAddr::PointerIndex {
                    ptr,
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size,
                    offset,
                },
            width: MirWidth::Word,
        } if ptr == target && *access_index_temp == index_temp && *elem_size > 1 => {
            let MirOp::Store {
                dst: MirAddr::Direct(store_dst),
                src: MirValue::Def(store_src),
                width: MirWidth::Word,
            } = ops.get(access_index + 1)?
            else {
                return None;
            };
            if store_src != load_dst {
                return None;
            }
            materialize_dynamic_word_index_read(
                pointer_value_from_mem(ptr),
                index_value,
                *offset,
                store_dst,
                out,
            );
            Some(consumed_index_ops + 3)
        }
        MirOp::Store {
            dst:
                MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(access_base)),
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size,
                    offset,
                },
            src,
            width: MirWidth::Word,
        } if *access_base == base_temp && *access_index_temp == index_temp && *elem_size > 1 => {
            materialize_dynamic_word_index_write(
                indexed_base_from_lea_target(target, routine_id, layout),
                index_value,
                *offset,
                src.clone(),
                layout,
                out,
            );
            Some(consumed_index_ops + 2)
        }
        MirOp::Store {
            dst:
                MirAddr::PointerIndex {
                    ptr,
                    index: MirValue::Def(MirDef::VTemp(access_index_temp)),
                    elem_size,
                    offset,
                },
            src,
            width: MirWidth::Word,
        } if ptr == target && *access_index_temp == index_temp && *elem_size > 1 => {
            materialize_dynamic_word_index_write(
                pointer_value_from_mem(ptr),
                index_value,
                *offset,
                src.clone(),
                layout,
                out,
            );
            Some(consumed_index_ops + 2)
        }
        _ => None,
    }
}

pub(super) fn storage_address_value(mem: &MirMem) -> MirValue {
    MirValue::Word {
        lo: Box::new(MirValue::StorageAddrByte {
            mem: mem.clone(),
            byte: 0,
        }),
        hi: Box::new(MirValue::StorageAddrByte {
            mem: mem.clone(),
            byte: 1,
        }),
    }
}

fn base_pointer_from_address_value(value: &MirValue) -> Option<MirValue> {
    match value {
        MirValue::GlobalAddr(id) => Some(pointer_value_from_mem(&MirMem::Global {
            id: *id,
            offset: 0,
        })),
        MirValue::StaticAddr(id) => Some(pointer_value_from_mem(&MirMem::Static {
            id: *id,
            offset: 0,
        })),
        MirValue::ConstU16(address) => Some(MirValue::ConstU16(*address)),
        _ => None,
    }
}

fn materialize_dynamic_word_index_read(
    base: MirValue,
    index: MirValue,
    offset: u16,
    dst: &MirMem,
    out: &mut Vec<MirOp>,
) {
    materialize_dynamic_word_index_address(base, index, DEFAULT_POINTER_PAIR, out);
    out.push(MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset,
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(dst.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    out.push(MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset: offset.saturating_add(1),
    });
    out.push(MirOp::Store {
        dst: MirAddr::Direct(offset_mem(dst, 1)),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
}

fn materialize_dynamic_word_index_write(
    base: MirValue,
    index: MirValue,
    offset: u16,
    src: MirValue,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    materialize_dynamic_word_index_address(base, index, DEFAULT_POINTER_PAIR, out);
    let (lo, hi) = split_value_as_word(src, layout);
    out.push(MirOp::StoreIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        src: lo,
        offset,
    });
    out.push(MirOp::StoreIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        src: hi,
        offset: offset.saturating_add(1),
    });
}

pub(super) fn materialize_dynamic_byte_index_read(
    base: MirValue,
    index: MirValue,
    offset: u16,
    dst: &MirAddr,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if materialize_direct_byte_indexed_read(
        MirDef::Reg(MirReg::A),
        &base,
        index.clone(),
        offset,
        out,
    ) {
        out.push(MirOp::Store {
            dst: dst.clone(),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        });
        return;
    }
    if offset == 0 {
        materialize_byte_indexed_read(MirDef::Reg(MirReg::A), base, index, layout, out);
        out.push(MirOp::Store {
            dst: dst.clone(),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        });
        return;
    }
    materialize_dynamic_index_address(base, index, 1, DEFAULT_POINTER_PAIR, out);
    out.push(MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset,
    });
    out.push(MirOp::Store {
        dst: dst.clone(),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
}

pub(super) fn materialize_dynamic_byte_index_write(
    base: MirValue,
    index: MirValue,
    offset: u16,
    src: MirValue,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if materialize_direct_byte_indexed_write(&base, index.clone(), offset, src.clone(), out) {
        return;
    }
    if offset == 0 {
        materialize_byte_indexed_write(base, index, src, layout, out);
        return;
    }
    materialize_dynamic_index_address(base, index, 1, DEFAULT_POINTER_PAIR, out);
    let src = materialize_byte_value_to_a(src, out);
    out.push(MirOp::StoreIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        src,
        offset,
    });
}

pub(super) fn materialize_indexed_read_to_def(
    dst: MirDef,
    parts: IndexedAddrParts,
    width: MirWidth,
    layout: &MaterializeLayout,
    delayed_byte_indexes: Option<&DelayedByteIndexPlan>,
    out: &mut Vec<MirOp>,
) -> bool {
    if width == MirWidth::Byte
        && parts.elem_size == 1
        && let Some(delayed) =
            delayed_byte_indexes.and_then(|plan| plan.expr_for_value(&parts.index))
    {
        materialize_delayed_byte_indexed_read(dst, parts.base, delayed, parts.offset, layout, out);
        return true;
    }
    materialize_computed_index_read(
        dst,
        parts.base,
        parts.index,
        parts.elem_size,
        parts.offset,
        width,
        layout,
        out,
    );
    false
}

pub(super) fn materialize_indexed_write_from_value(
    parts: IndexedAddrParts,
    src: MirValue,
    width: MirWidth,
    layout: &MaterializeLayout,
    delayed_byte_indexes: Option<&DelayedByteIndexPlan>,
    out: &mut Vec<MirOp>,
) -> bool {
    if width == MirWidth::Byte
        && parts.elem_size == 1
        && let Some(delayed) =
            delayed_byte_indexes.and_then(|plan| plan.expr_for_value(&parts.index))
    {
        materialize_delayed_byte_indexed_write(parts.base, delayed, parts.offset, src, layout, out);
        return true;
    }
    materialize_computed_index_write(
        parts.base,
        parts.index,
        parts.elem_size,
        parts.offset,
        src,
        width,
        layout,
        out,
    );
    false
}

pub(super) fn materialize_indexed_byte_read_to_a(
    src: &MirAddr,
    layout: &MaterializeLayout,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    out: &mut Vec<MirOp>,
) -> Option<bool> {
    let parts = indexed_addr_parts(src)?;
    (parts.elem_size == 1).then(|| {
        materialize_indexed_read_to_def(
            MirDef::Reg(MirReg::A),
            parts,
            MirWidth::Byte,
            layout,
            Some(delayed_byte_indexes),
            out,
        )
    })
}

pub(super) fn materialize_computed_index_read(
    dst: MirDef,
    base: MirValue,
    index: MirValue,
    elem_size: u16,
    offset: u16,
    width: MirWidth,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    let split_dst = (width == MirWidth::Word).then(|| split_def(dst.clone()));
    if (width == MirWidth::Byte || split_dst.as_ref().is_some_and(Option::is_some))
        && let Some(offset) = folded_const_index_offset(&index, elem_size, offset, width)
    {
        materialize_base_address(base, DEFAULT_POINTER_PAIR, layout, out);
        match width {
            MirWidth::Byte => out.push(MirOp::LoadIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                dst,
                offset,
            }),
            MirWidth::Word => {
                let (lo_dst, hi_dst) = split_dst.flatten().expect("checked above");
                out.push(MirOp::LoadIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    dst: lo_dst,
                    offset,
                });
                out.push(MirOp::LoadIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    dst: hi_dst,
                    offset: offset + 1,
                });
            }
        }
        return;
    }
    if width == MirWidth::Byte && elem_size == 1 && offset == 0 && index_value_is_byte_sized(&index)
    {
        materialize_byte_indexed_read(dst, base, index, layout, out);
        return;
    }
    if scaled_y_word_read_is_eligible(&base, &index, elem_size, offset, width, layout) {
        materialize_indexed_address_to_consumer(
            base,
            index,
            elem_size,
            DEFAULT_SCALED_Y_POINTER_PAIR,
            layout,
            out,
        );
        match width {
            MirWidth::Byte => out.push(MirOp::LoadIndirect {
                consumer: DEFAULT_SCALED_Y_POINTER_PAIR,
                dst,
                offset,
            }),
            MirWidth::Word => {
                let (lo_dst, hi_dst) = split_dst.flatten().expect("eligibility requires split dst");
                out.push(MirOp::LoadIndirect {
                    consumer: DEFAULT_SCALED_Y_POINTER_PAIR,
                    dst: lo_dst,
                    offset: 0,
                });
                out.push(MirOp::LoadIndirect {
                    consumer: DEFAULT_SCALED_Y_POINTER_PAIR,
                    dst: hi_dst,
                    offset: 1,
                });
            }
        }
        return;
    }
    materialize_indexed_address(base.clone(), index.clone(), elem_size, layout, out);
    match width {
        MirWidth::Byte => out.push(MirOp::LoadIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            dst,
            offset,
        }),
        MirWidth::Word => {
            if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                out.push(MirOp::LoadIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    dst: lo_dst,
                    offset,
                });
                out.push(MirOp::LoadIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    dst: hi_dst,
                    offset: offset.saturating_add(1),
                });
            } else {
                out.push(MirOp::Load {
                    dst,
                    src: MirAddr::ComputedIndex {
                        base,
                        index,
                        elem_size,
                        offset,
                    },
                    width,
                });
            }
        }
    }
}

fn scaled_y_word_read_is_eligible(
    base: &MirValue,
    index: &MirValue,
    elem_size: u16,
    offset: u16,
    width: MirWidth,
    layout: &MaterializeLayout,
) -> bool {
    if elem_size != 2 || !index_value_is_byte_sized(index) {
        return false;
    }
    if !matches!(
        (width, offset),
        (MirWidth::Byte, 0 | 1) | (MirWidth::Word, 0)
    ) {
        return false;
    }
    let (lo, hi) = split_value_as_word(base.clone(), layout);
    [lo, hi]
        .iter()
        .all(|byte| !matches!(byte, MirValue::Def(MirDef::Reg(MirReg::A | MirReg::Y))))
}

fn indexed_base_from_lea_target(
    target: &MirMem,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> MirValue {
    if layout.is_descriptor_storage(routine_id, target) {
        pointer_value_from_mem(target)
    } else {
        storage_address_value(target)
    }
}

pub(super) fn materialize_computed_index_write(
    base: MirValue,
    index: MirValue,
    elem_size: u16,
    offset: u16,
    src: MirValue,
    width: MirWidth,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if let Some(offset) = folded_const_index_offset(&index, elem_size, offset, width) {
        materialize_base_address(base, DEFAULT_POINTER_PAIR, layout, out);
        match width {
            MirWidth::Byte => {
                let src = materialize_byte_value_to_a(src, out);
                out.push(MirOp::StoreIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    src,
                    offset,
                });
            }
            MirWidth::Word => {
                let (lo, hi) = split_value_as_word(src, layout);
                out.push(MirOp::StoreIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    src: lo,
                    offset,
                });
                out.push(MirOp::StoreIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    src: hi,
                    offset: offset + 1,
                });
            }
        }
        return;
    }
    if width == MirWidth::Byte && elem_size == 1 && offset == 0 && index_value_is_byte_sized(&index)
    {
        materialize_byte_indexed_write(base, index, src, layout, out);
        return;
    }
    materialize_indexed_address(base, index, elem_size, layout, out);
    match width {
        MirWidth::Byte => {
            let src = materialize_byte_value_to_a(src, out);
            out.push(MirOp::StoreIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                src,
                offset,
            });
        }
        MirWidth::Word => {
            let (lo, hi) = split_value_as_word(src, layout);
            out.push(MirOp::StoreIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                src: lo,
                offset,
            });
            out.push(MirOp::StoreIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                src: hi,
                offset: offset.saturating_add(1),
            });
        }
    }
}

fn folded_const_index_offset(
    index: &MirValue,
    elem_size: u16,
    offset: u16,
    width: MirWidth,
) -> Option<u16> {
    let MirValue::ConstU8(index) = index else {
        return None;
    };
    let offset = u16::from(*index)
        .checked_mul(elem_size)?
        .checked_add(offset)?;
    let last_byte = offset.checked_add(width_bytes(width).saturating_sub(1))?;
    if last_byte <= u8::MAX as u16 {
        Some(offset)
    } else {
        None
    }
}

fn materialize_byte_indexed_read(
    dst: MirDef,
    base: MirValue,
    index: MirValue,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if materialize_direct_byte_indexed_read(dst.clone(), &base, index.clone(), 0, out) {
        return;
    }
    if !index_value_is_byte_sized(&index) {
        materialize_indexed_address(base, index, 1, layout, out);
        out.push(MirOp::LoadIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            dst,
            offset: 0,
        });
        return;
    }
    materialize_base_address(base, DEFAULT_POINTER_PAIR, layout, out);
    materialize_index_to_y(index, out);
    out.push(MirOp::Load {
        dst,
        src: MirAddr::FixedIndirectIndexedY {
            zp: MirFixedZpSlot(POINTER_SCRATCH_LO),
        },
        width: MirWidth::Byte,
    });
}

fn materialize_byte_indexed_write(
    base: MirValue,
    index: MirValue,
    src: MirValue,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if materialize_direct_byte_indexed_write(&base, index.clone(), 0, src.clone(), out) {
        return;
    }
    if !index_value_is_byte_sized(&index) {
        materialize_indexed_address(base, index, 1, layout, out);
        let src = materialize_byte_value_to_a(src, out);
        out.push(MirOp::StoreIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            src,
            offset: 0,
        });
        return;
    }
    materialize_base_address(base, DEFAULT_POINTER_PAIR, layout, out);
    materialize_index_to_y(index, out);
    let src = materialize_byte_value_to_a(src, out);
    out.push(MirOp::Store {
        dst: MirAddr::FixedIndirectIndexedY {
            zp: MirFixedZpSlot(POINTER_SCRATCH_LO),
        },
        src,
        width: MirWidth::Byte,
    });
}

fn materialize_direct_byte_indexed_read(
    dst: MirDef,
    base: &MirValue,
    index: MirValue,
    offset: u16,
    out: &mut Vec<MirOp>,
) -> bool {
    if !index_value_is_byte_sized(&index) {
        return false;
    }
    let Some(base) = address_value_mem(base).map(|mem| offset_mem(&mem, offset)) else {
        return false;
    };
    materialize_index_to_y(index, out);
    out.push(MirOp::Load {
        dst,
        src: MirAddr::AbsoluteIndexedY { base },
        width: MirWidth::Byte,
    });
    true
}

fn materialize_direct_byte_indexed_write(
    base: &MirValue,
    index: MirValue,
    offset: u16,
    src: MirValue,
    out: &mut Vec<MirOp>,
) -> bool {
    if !index_value_is_byte_sized(&index) {
        return false;
    }
    let Some(base) = address_value_mem(base).map(|mem| offset_mem(&mem, offset)) else {
        return false;
    };
    materialize_index_to_y(index, out);
    let src = materialize_byte_value_to_a(src, out);
    out.push(MirOp::Store {
        dst: MirAddr::AbsoluteIndexedY { base },
        src,
        width: MirWidth::Byte,
    });
    true
}

fn address_value_mem(value: &MirValue) -> Option<MirMem> {
    match value {
        MirValue::ConstU16(address) => Some(MirMem::Absolute(*address)),
        MirValue::StaticAddr(id) => Some(MirMem::Static { id: *id, offset: 0 }),
        MirValue::GlobalAddr(id) => Some(MirMem::Global { id: *id, offset: 0 }),
        MirValue::Word { lo, hi } => {
            let MirValue::StorageAddrByte { mem, byte: 0 } = lo.as_ref() else {
                return None;
            };
            let MirValue::StorageAddrByte {
                mem: hi_mem,
                byte: 1,
            } = hi.as_ref()
            else {
                return None;
            };
            (mem == hi_mem).then(|| mem.clone())
        }
        _ => None,
    }
}

fn materialize_indexed_address(
    base: MirValue,
    index: MirValue,
    elem_size: u16,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    materialize_indexed_address_to_consumer(
        base,
        index,
        elem_size,
        DEFAULT_POINTER_PAIR,
        layout,
        out,
    );
}

pub(super) fn materialize_indexed_address_to_consumer(
    base: MirValue,
    index: MirValue,
    elem_size: u16,
    consumer: MirAddressConsumer,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if matches!(index, MirValue::ConstU8(0)) {
        materialize_base_address(base, consumer, layout, out);
        return;
    }
    let scale = elem_size.min(u8::MAX as u16) as u8;
    if matches!(scale, 1 | 2) {
        out.push(MirOp::MaterializeIndexedAddress {
            consumer,
            base,
            index,
            scale,
        });
    } else {
        materialize_base_address(base, consumer, layout, out);
        out.push(MirOp::AdvanceAddress {
            consumer,
            index,
            scale,
        });
    }
}

pub(super) fn materialize_base_address(
    base: MirValue,
    consumer: MirAddressConsumer,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    let (lo, hi) = split_value_as_word(base, layout);
    out.push(MirOp::MaterializeAddress {
        consumer,
        value: MirValue::Word {
            lo: Box::new(lo),
            hi: Box::new(hi),
        },
    });
}

fn materialize_dynamic_word_index_address(
    base: MirValue,
    index: MirValue,
    consumer: MirAddressConsumer,
    out: &mut Vec<MirOp>,
) {
    materialize_dynamic_index_address(base, index, 2, consumer, out);
}

fn materialize_dynamic_index_address(
    base: MirValue,
    index: MirValue,
    scale: u8,
    consumer: MirAddressConsumer,
    out: &mut Vec<MirOp>,
) {
    // This mirrors materialize_indexed_address_to_consumer but intentionally
    // preserves the dynamic-index peephole shape for now: byte dynamic indexes
    // keep their AbsoluteIndexedY fast path, and word dynamic indexes feed
    // explicit low/high byte lanes. If this path grows, route it through
    // IndexedAddrParts without losing those code shapes.
    if matches!(index, MirValue::ConstU8(0)) {
        out.push(MirOp::MaterializeAddress {
            consumer,
            value: base,
        });
        return;
    }
    if matches!(scale, 1 | 2) {
        out.push(MirOp::MaterializeIndexedAddress {
            consumer,
            base,
            index,
            scale,
        });
        return;
    }
    out.push(MirOp::MaterializeAddress {
        consumer,
        value: base,
    });
    out.push(MirOp::AdvanceAddress {
        consumer,
        index,
        scale,
    });
}

fn index_value_is_byte_sized(value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(_)
        | MirValue::PointerCell(_)
        | MirValue::StorageAddrByte { .. }
        | MirValue::RoutineAddrByte { .. }
        | MirValue::Def(MirDef::Reg(_))
        | MirValue::Def(MirDef::VTempByte { .. }) => true,
        MirValue::ConstU16(value) => *value <= u8::MAX as u16,
        _ => false,
    }
}

pub(super) fn materialize_index_to_y(value: MirValue, out: &mut Vec<MirOp>) {
    match value {
        MirValue::ConstU8(value) => out.push(MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::Y),
            value: value as u16,
            width: MirWidth::Byte,
        }),
        MirValue::PointerCell(mem) => out.push(MirOp::Load {
            dst: MirDef::Reg(MirReg::Y),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }),
        other => out.push(MirOp::Move {
            dst: MirDef::Reg(MirReg::Y),
            src: other,
            width: MirWidth::Byte,
        }),
    }
}
