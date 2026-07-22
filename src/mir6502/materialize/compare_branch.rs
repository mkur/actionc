use super::defs::split_def_as_temp;
use super::layout::MaterializeLayout;
use super::pointers::pointer_value_from_mem;
#[cfg(test)]
use super::stats::MirPeepholeStats;
use super::temp_rewrite::replace_temp_value;
#[cfg(test)]
use super::temp_uses::terminator_uses_temp;
use super::temp_uses::{op_uses_temp, op_uses_temp_more_than_once, value_uses_temp};
use super::temp_widths::collect_temp_widths;
#[cfg(test)]
use super::temps::{def_is_used_after, temp_is_used_after};
use super::values::split_value_as_word;
#[cfg(test)]
use crate::mir6502::ir::RoutineId;
use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirBinaryOp, MirBlock, MirBlockId, MirCarryIn, MirCarryOut,
    MirCompareOp, MirCond, MirCondDest, MirDef, MirEdge, MirFixedZpSlot, MirFlagTest, MirMem,
    MirOp, MirPointerPair, MirReg, MirTempId, MirTerminator, MirValue, MirWidth,
};
use crate::mir6502::passes::Mir6502Config;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct ByteAddWordCompareCandidate {
    pub consumed: usize,
    pub binary: MirOp,
    pub compare_dst: MirCondDest,
    pub compare_op: MirCompareOp,
    pub compare_right: MirValue,
}

impl ByteAddWordCompareCandidate {
    pub(in crate::mir6502) fn proof_replacement(&self) -> Vec<MirOp> {
        vec![
            self.binary.clone(),
            MirOp::Compare {
                dst: self.compare_dst.clone(),
                op: self.compare_op,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: self.compare_right.clone(),
                width: MirWidth::Byte,
                signed: false,
            },
        ]
    }
}

pub(in crate::mir6502) fn byte_add_word_compare_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<ByteAddWordCompareCandidate> {
    let MirOp::Load {
        dst: add_value_dst,
        src: MirAddr::Direct(add_mem),
        width: MirWidth::Byte,
    } = ops.get(index)?
    else {
        return None;
    };
    let add_value_temp = split_def_as_byte_compare_temp(add_value_dst)?;
    if !byte_binary_compare_reorderable_mem(add_mem) {
        return None;
    }

    let MirOp::Binary {
        op: MirBinaryOp::Add,
        dst: sum_dst,
        left,
        right,
        width: MirWidth::Word,
        carry_in: None,
        carry_out: MirCarryOut::Ignore,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let sum_temp = split_def_as_temp(sum_dst)?;
    let add_value = MirValue::Def(add_value_dst.clone());
    let addend = if left == &add_value {
        byte_constant(right)?
    } else if right == &add_value {
        byte_constant(left)?
    } else {
        return None;
    };

    let MirOp::Load {
        dst: compare_value_dst,
        src: MirAddr::Direct(compare_mem),
        width: MirWidth::Byte,
    } = ops.get(index + 2)?
    else {
        return None;
    };
    let compare_value_temp = split_def_as_byte_compare_temp(compare_value_dst)?;
    if !byte_binary_compare_reorderable_mem(compare_mem)
        || add_value_temp == compare_value_temp
        || add_value_temp == sum_temp
        || compare_value_temp == sum_temp
    {
        return None;
    }

    let MirOp::Compare {
        dst: compare_dst,
        op,
        left: compare_left,
        right: compare_right,
        width: MirWidth::Word,
        signed: false,
    } = ops.get(index + 3)?
    else {
        return None;
    };
    let (compare_op, other) = if compare_left == &MirValue::Def(sum_dst.clone()) {
        (*op, compare_right)
    } else if compare_right == &MirValue::Def(sum_dst.clone()) {
        (reverse_compare_operands(*op), compare_left)
    } else {
        return None;
    };
    if zero_extended_byte_temp(other)? != compare_value_temp {
        return None;
    }

    Some(ByteAddWordCompareCandidate {
        consumed: 4,
        binary: MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::PointerCell(add_mem.clone()),
            right: MirValue::ConstU8(addend),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::Clear),
            carry_out: MirCarryOut::Produce,
        },
        compare_dst: compare_dst.clone(),
        compare_op,
        compare_right: MirValue::PointerCell(compare_mem.clone()),
    })
}

fn byte_constant(value: &MirValue) -> Option<u8> {
    match value {
        MirValue::ConstU8(value) => Some(*value),
        MirValue::ConstU16(value) => u8::try_from(*value).ok(),
        _ => None,
    }
}

fn zero_extended_byte_temp(value: &MirValue) -> Option<MirTempId> {
    let MirValue::Word { lo, hi } = value else {
        return None;
    };
    if !matches!(&**hi, MirValue::ConstU8(0) | MirValue::ConstU16(0)) {
        return None;
    }
    match &**lo {
        MirValue::Def(MirDef::VTemp(temp) | MirDef::VTempByte { id: temp, byte: 0 }) => Some(*temp),
        _ => None,
    }
}

pub(super) fn expand_compare_branch_consumers(
    blocks: &mut Vec<MirBlock>,
    layout: &MaterializeLayout,
    _config: &Mir6502Config,
) {
    let mut next_id = blocks
        .iter()
        .map(|block| block.id.0)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let original_len = blocks.len();
    for index in 0..original_len {
        if try_expand_short_circuit_branch(index, blocks, layout, &mut next_id) {
            continue;
        }
        if try_expand_byte_compare_branch(index, blocks, &mut next_id) {
            continue;
        }
        try_expand_word_compare_branch(index, blocks, layout, &mut next_id);
    }
}

pub(super) fn expand_proven_byte_add_word_compare_branches(
    blocks: &mut Vec<MirBlock>,
    proven_sites: &BTreeSet<(MirBlockId, usize)>,
) -> usize {
    let mut next_id = blocks
        .iter()
        .map(|block| block.id.0)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let original_len = blocks.len();
    let mut expanded = 0usize;
    for block_index in 0..original_len {
        let block_id = blocks[block_index].id;
        let Some(start) = proven_sites
            .iter()
            .find_map(|(block, start)| (*block == block_id).then_some(*start))
        else {
            continue;
        };
        let Some(candidate) = byte_add_word_compare_candidate(&blocks[block_index].ops, start)
        else {
            continue;
        };
        let Some((cond_temp, then_block, else_block)) = branch_bool_temp(&blocks[block_index])
        else {
            continue;
        };
        if candidate.compare_dst != MirCondDest::Temp(cond_temp)
            || start + candidate.consumed != blocks[block_index].ops.len()
        {
            continue;
        }

        let low_compare = fresh_block_id(&mut next_id);
        let mut low_ops = Vec::new();
        let low_terminator = materialize_byte_compare_branch(
            &mut low_ops,
            blocks,
            &mut next_id,
            candidate.compare_op,
            MirValue::Def(MirDef::Reg(MirReg::A)),
            candidate.compare_right,
            then_block,
            else_block,
        );
        blocks.push(MirBlock {
            id: low_compare,
            label: format!("cmp_byte_add_lo_{}", low_compare.0),
            params: Vec::new(),
            ops: low_ops,
            terminator: low_terminator,
        });

        blocks[block_index].ops.truncate(start);
        blocks[block_index].ops.push(candidate.binary);
        let carry_set_target = match candidate.compare_op {
            MirCompareOp::Ne | MirCompareOp::Gt | MirCompareOp::Ge => then_block,
            MirCompareOp::Eq | MirCompareOp::Lt | MirCompareOp::Le => else_block,
        };
        blocks[block_index].terminator = branch_terminator(
            MirCond::FlagTest(MirFlagTest::CSet),
            carry_set_target,
            low_compare,
        );
        expanded += 1;
    }
    expanded
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct DualIndirectCompareCandidate {
    pub consumed: usize,
    pub replacement: Vec<MirOp>,
}

pub(in crate::mir6502) fn dual_indirect_compare_candidate(
    block: &MirBlock,
    index: usize,
) -> Option<DualIndirectCompareCandidate> {
    let MirOp::Load {
        dst: first_pointer_dst,
        src: MirAddr::Direct(first_pointer_mem),
        width: MirWidth::Word,
    } = block.ops.get(index)?
    else {
        return None;
    };
    let first_pointer_temp = split_def_as_temp(first_pointer_dst)?;
    let MirOp::Load {
        dst: first_byte_dst,
        src:
            MirAddr::Deref {
                ptr: MirValue::Def(MirDef::VTemp(first_deref_temp)),
                offset: first_offset,
            },
        width: MirWidth::Byte,
    } = block.ops.get(index + 1)?
    else {
        return None;
    };
    let first_byte_temp = split_def_as_temp(first_byte_dst)?;
    let MirOp::Load {
        dst: second_pointer_dst,
        src: MirAddr::Direct(second_pointer_mem),
        width: MirWidth::Word,
    } = block.ops.get(index + 2)?
    else {
        return None;
    };
    let second_pointer_temp = split_def_as_temp(second_pointer_dst)?;
    let MirOp::Load {
        dst: second_byte_dst,
        src:
            MirAddr::Deref {
                ptr: MirValue::Def(MirDef::VTemp(second_deref_temp)),
                offset: second_offset,
            },
        width: MirWidth::Byte,
    } = block.ops.get(index + 3)?
    else {
        return None;
    };
    let second_byte_temp = split_def_as_temp(second_byte_dst)?;
    let MirOp::Compare {
        dst: MirCondDest::Temp(compare_temp),
        op,
        left: MirValue::Def(MirDef::VTemp(compare_left)),
        right: MirValue::Def(MirDef::VTemp(compare_right)),
        width: MirWidth::Byte,
        signed: false,
    } = block.ops.get(index + 4)?
    else {
        return None;
    };
    let MirTerminator::Branch {
        cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(branch_temp))),
        ..
    } = &block.terminator
    else {
        return None;
    };
    if index + 5 != block.ops.len()
        || first_pointer_temp != *first_deref_temp
        || second_pointer_temp != *second_deref_temp
        || first_byte_temp != *compare_left
        || second_byte_temp != *compare_right
        || compare_temp != branch_temp
        || first_offset != second_offset
        || *first_offset > u16::from(u8::MAX)
        || first_pointer_mem == second_pointer_mem
        || !dual_compare_pointer_source_is_safe(first_pointer_mem)
        || !dual_compare_pointer_source_is_safe(second_pointer_mem)
    {
        return None;
    }

    let left = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(super::POINTER_INDEX_SCRATCH_LO),
    });
    let right = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(super::POINTER_SCRATCH_LO),
    });
    let (op, compare_left, compare_right) = match op {
        MirCompareOp::Eq | MirCompareOp::Ne | MirCompareOp::Lt | MirCompareOp::Ge => {
            (*op, left, right)
        }
        MirCompareOp::Gt => (MirCompareOp::Lt, right, left),
        MirCompareOp::Le => (MirCompareOp::Ge, right, left),
    };
    Some(DualIndirectCompareCandidate {
        consumed: 5,
        replacement: vec![
            MirOp::MaterializeAddress {
                consumer: left,
                value: pointer_value_from_mem(first_pointer_mem),
            },
            MirOp::MaterializeAddress {
                consumer: right,
                value: pointer_value_from_mem(second_pointer_mem),
            },
            MirOp::CompareIndirectBytes {
                dst: MirCondDest::Temp(*compare_temp),
                op,
                left: compare_left,
                right: compare_right,
                offset: *first_offset,
                signed: false,
            },
        ],
    })
}

fn dual_compare_pointer_source_is_safe(mem: &MirMem) -> bool {
    matches!(
        mem,
        MirMem::Param { .. }
            | MirMem::Local { .. }
            | MirMem::Global { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
    )
}

#[cfg(test)]
pub(super) fn fold_compare_operand_producers_before_branches(
    blocks: &mut [MirBlock],
    routine_id: RoutineId,
    peephole_stats: &mut MirPeepholeStats,
) {
    for block in blocks {
        let mut out = Vec::with_capacity(block.ops.len());
        let mut index = 0;
        let mut changed = false;
        while index < block.ops.len() {
            let consumed =
                try_fuse_compare_operand_producers(&block.ops, index, &block.terminator, &mut out);
            if consumed != 0 {
                peephole_stats.record(routine_id, "compare-operand-consumer-prebranch");
                index += consumed;
                changed = true;
            } else {
                out.push(block.ops[index].clone());
                index += 1;
            }
        }
        if changed {
            block.ops = out;
        }
        let narrowed = narrow_byte_bitwise_zero_compares(&mut block.ops, &block.terminator);
        peephole_stats.record_many(
            routine_id,
            "byte-derived-word-bitwise-zero-compare-narrowed",
            narrowed,
        );
    }
}

#[cfg(test)]
fn narrow_byte_bitwise_zero_compares(ops: &mut [MirOp], terminator: &MirTerminator) -> usize {
    let mut narrowed = 0usize;
    for index in 0..ops.len().saturating_sub(1) {
        let Some(candidate) = byte_bitwise_zero_compare_narrowing_candidate(ops, index) else {
            continue;
        };
        if temp_is_used_after(ops, index + 2, candidate.temp)
            || terminator_uses_temp(terminator, candidate.temp)
        {
            continue;
        }
        ops[index] = candidate.replacement[0].clone();
        ops[index + 1] = candidate.replacement[1].clone();
        narrowed += 1;
    }
    narrowed
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct CompareNarrowingCandidate {
    pub temp: MirTempId,
    pub replacement: [MirOp; 2],
}

pub(in crate::mir6502) fn byte_bitwise_zero_compare_narrowing_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<CompareNarrowingCandidate> {
    let temp_widths = collect_temp_widths(ops);
    let MirOp::Binary {
        op,
        dst,
        left,
        right,
        width: MirWidth::Word,
        carry_in,
        carry_out: MirCarryOut::Ignore,
    } = ops.get(index)?
    else {
        return None;
    };
    if !matches!(op, MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor) || carry_in.is_some() {
        return None;
    }
    let temp = split_def_as_temp(dst)?;
    let MirOp::Compare {
        op: MirCompareOp::Eq | MirCompareOp::Ne,
        left: compare_left,
        right: MirValue::ConstU8(0) | MirValue::ConstU16(0),
        width: MirWidth::Word,
        signed: false,
        ..
    } = ops.get(index + 1)?
    else {
        return None;
    };
    if compare_left != &MirValue::Def(dst.clone())
        || op_uses_temp_more_than_once(&ops[index + 1], temp)
    {
        return None;
    }
    let left = byte_derived_word_operand(left, &temp_widths)?;
    let right = byte_derived_word_operand(right, &temp_widths)?;

    let mut binary = ops[index].clone();
    let MirOp::Binary {
        left: binary_left,
        right: binary_right,
        width: binary_width,
        ..
    } = &mut binary
    else {
        unreachable!()
    };
    *binary_left = left;
    *binary_right = right;
    *binary_width = MirWidth::Byte;
    let mut compare = ops[index + 1].clone();
    let MirOp::Compare {
        right: compare_right,
        width: compare_width,
        ..
    } = &mut compare
    else {
        unreachable!()
    };
    *compare_right = MirValue::ConstU8(0);
    *compare_width = MirWidth::Byte;
    Some(CompareNarrowingCandidate {
        temp,
        replacement: [binary, compare],
    })
}

fn byte_derived_word_operand(
    value: &MirValue,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
) -> Option<MirValue> {
    match value {
        MirValue::ConstU8(value) => Some(MirValue::ConstU8(*value)),
        MirValue::ConstU16(value) if *value <= u8::MAX as u16 => {
            Some(MirValue::ConstU8(*value as u8))
        }
        MirValue::Def(MirDef::VTemp(temp)) if temp_widths.get(temp) == Some(&MirWidth::Byte) => {
            Some(value.clone())
        }
        MirValue::Def(MirDef::VTempByte { byte: 0, .. } | MirDef::Reg(_))
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => Some(value.clone()),
        MirValue::Word { lo, hi } if matches!(&**hi, MirValue::ConstU8(0)) => {
            byte_derived_word_operand(lo, temp_widths)
        }
        _ => None,
    }
}

#[cfg(test)]
pub(super) fn try_fuse_byte_compare_consumer(
    ops: &[MirOp],
    index: usize,
    out: &mut Vec<MirOp>,
) -> usize {
    if let Some(consumed) = try_fuse_two_loaded_byte_compare_consumer(ops, index, out) {
        return consumed;
    }

    let Some(load) = ops.get(index) else {
        return 0;
    };
    let Some(compare) = ops.get(index + 1) else {
        return 0;
    };
    let MirOp::Load {
        dst: load_dst,
        src: MirAddr::Direct(load_src),
        width: MirWidth::Byte,
    } = load
    else {
        return 0;
    };
    let Some(load_temp) = split_def_as_temp(load_dst) else {
        return 0;
    };
    let MirOp::Compare {
        dst,
        op,
        left,
        right,
        width: MirWidth::Byte,
        signed,
    } = compare
    else {
        return 0;
    };
    if !matches!(right, MirValue::ConstU8(_) | MirValue::ConstU16(_)) {
        return 0;
    }

    let producer = MirValue::PointerCell(load_src.clone());
    let left = replace_temp_value(left.clone(), load_temp, &producer);
    let right = replace_temp_value(right.clone(), load_temp, &producer);
    if value_uses_temp(&left) || value_uses_temp(&right) {
        return 0;
    }

    out.push(MirOp::Compare {
        dst: dst.clone(),
        op: *op,
        left,
        right,
        width: MirWidth::Byte,
        signed: *signed,
    });
    2
}

#[cfg(test)]
pub(super) fn try_fuse_compare_operand_producers(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(plan) = collect_compare_operand_plan(ops, index, terminator) else {
        return 0;
    };
    out.push(MirOp::Compare {
        dst: plan.dst,
        op: plan.op,
        left: plan.left,
        right: plan.right,
        width: plan.width,
        signed: plan.signed,
    });
    plan.consumed
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct CompareOperandRewriteCandidate {
    pub consumed: usize,
    pub replacement: MirOp,
}

pub(in crate::mir6502) fn compare_operand_rewrite_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<CompareOperandRewriteCandidate> {
    let plan = collect_compare_operand_shape(ops, index)?;
    Some(CompareOperandRewriteCandidate {
        consumed: plan.consumed,
        replacement: MirOp::Compare {
            dst: plan.dst,
            op: plan.op,
            left: plan.left,
            right: plan.right,
            width: plan.width,
            signed: plan.signed,
        },
    })
}

struct CompareOperandPlan {
    consumed: usize,
    dst: MirCondDest,
    op: MirCompareOp,
    left: MirValue,
    right: MirValue,
    width: MirWidth,
    signed: bool,
}

#[cfg(test)]
fn collect_compare_operand_plan(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> Option<CompareOperandPlan> {
    let plan = collect_compare_operand_shape(ops, index)?;
    let compare_index = index + plan.consumed - 1;
    for temp in compare_operand_producer_temps(&ops[index..compare_index]) {
        if temp_is_used_after(ops, compare_index.saturating_add(1), temp)
            || terminator_uses_temp(terminator, temp)
        {
            return None;
        }
    }
    Some(plan)
}

fn collect_compare_operand_shape(ops: &[MirOp], index: usize) -> Option<CompareOperandPlan> {
    let mut replacements = BTreeMap::<MirTempId, MirValue>::new();
    let mut cursor = index;
    while let Some((temp, value)) = compare_operand_producer(ops.get(cursor)?, &replacements) {
        replacements.insert(temp, value);
        cursor += 1;
    }
    if replacements.is_empty() {
        return None;
    }

    let MirOp::Compare {
        dst,
        op,
        left,
        right,
        width,
        signed,
    } = ops.get(cursor)?
    else {
        return None;
    };

    let left = replace_compare_operand_temps(left.clone(), &replacements);
    let right = replace_compare_operand_temps(right.clone(), &replacements);
    if value_uses_temp(&left) || value_uses_temp(&right) {
        return None;
    }
    let mut saw_use = false;
    for temp in replacements.keys().copied() {
        if !compare_operand_temp_has_single_consumer(ops, index, cursor, temp) {
            return None;
        }
        saw_use |= op_uses_temp(ops.get(cursor)?, temp);
    }
    if !saw_use {
        return None;
    }

    Some(CompareOperandPlan {
        consumed: cursor + 1 - index,
        dst: dst.clone(),
        op: *op,
        left,
        right,
        width: *width,
        signed: *signed,
    })
}

#[cfg(test)]
fn compare_operand_producer_temps(ops: &[MirOp]) -> BTreeSet<MirTempId> {
    let mut replacements = BTreeMap::<MirTempId, MirValue>::new();
    let mut temps = BTreeSet::new();
    for op in ops {
        let Some((temp, value)) = compare_operand_producer(op, &replacements) else {
            break;
        };
        replacements.insert(temp, value);
        temps.insert(temp);
    }
    temps
}

fn compare_operand_producer(
    op: &MirOp,
    replacements: &BTreeMap<MirTempId, MirValue>,
) -> Option<(MirTempId, MirValue)> {
    match op {
        MirOp::LoadImm { dst, value, width } => Some((
            split_def_as_temp(dst)?,
            match width {
                MirWidth::Byte => MirValue::ConstU8(*value as u8),
                MirWidth::Word => MirValue::ConstU16(*value),
            },
        )),
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
        MirOp::Move { dst, src, .. } => {
            let value = replace_compare_operand_temps(src.clone(), replacements);
            if value_uses_temp(&value) {
                return None;
            }
            Some((split_def_as_temp(dst)?, value))
        }
        _ => None,
    }
}

fn replace_compare_operand_temps(
    mut value: MirValue,
    replacements: &BTreeMap<MirTempId, MirValue>,
) -> MirValue {
    for (temp, replacement) in replacements {
        value = replace_temp_value(value, *temp, replacement);
    }
    value
}

fn compare_operand_temp_has_single_consumer(
    ops: &[MirOp],
    start: usize,
    compare_index: usize,
    temp: MirTempId,
) -> bool {
    let mut uses = 0usize;
    for op in &ops[start..=compare_index] {
        if op_uses_temp_more_than_once(op, temp) {
            return false;
        }
        if op_uses_temp(op, temp) {
            uses += 1;
        }
    }
    uses == 1
}

#[cfg(test)]
pub(super) fn byte_binary_compare_consumer_observation(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> Option<&'static str> {
    let MirOp::Binary {
        op,
        dst,
        left,
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirOp::Compare {
        left: compare_left,
        right: compare_right,
        width: MirWidth::Byte,
        ..
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let Some(dst_temp) = split_def_as_byte_compare_temp(dst) else {
        return Some("byte-binary-compare-blocked-non-temp-dst");
    };
    let dst_value = MirValue::Def(dst.clone());
    if compare_left != &dst_value && compare_right != &dst_value {
        return None;
    }
    if let Some(blocker) = byte_binary_compare_op_blocker(*op, *carry_in, *carry_out) {
        return Some(blocker);
    }
    if value_uses_temp(left) || value_uses_temp(right) {
        return Some("byte-binary-compare-blocked-temp-operands");
    }
    if temp_is_used_after(ops, index + 2, dst_temp) || terminator_uses_temp(terminator, dst_temp) {
        return Some("byte-binary-compare-blocked-live-after");
    }
    if compare_right == &dst_value {
        return Some("byte-binary-compare-blocked-rhs-result");
    }
    if !matches!(compare_right, MirValue::ConstU8(_) | MirValue::ConstU16(_)) {
        return Some("byte-binary-compare-blocked-nonconst-rhs");
    }
    Some("byte-binary-compare-forwardable")
}

#[cfg(test)]
pub(super) fn try_fuse_byte_binary_compare_consumer(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(candidate) = byte_binary_compare_rewrite_candidate(ops, index) else {
        return 0;
    };
    if temp_is_used_after(ops, index + 2, candidate.temp)
        || terminator_uses_temp(terminator, candidate.temp)
    {
        return 0;
    }
    out.extend(candidate.replacement);
    2
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct ByteBinaryCompareRewriteCandidate {
    pub temp: MirTempId,
    pub replacement: [MirOp; 2],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct ByteBinaryCompareChainRewriteCandidate {
    pub consumed: usize,
    pub replacement: Vec<MirOp>,
}

/// Fold the common unsigned shape
///
/// ```text
/// compare_value = load cell_a
/// binary_value  = load cell_b
/// result        = binary_value +/- constant
/// condition     = compare_value REL result
/// ```
///
/// before compare/branch expansion.  Selecting the binary result in A and
/// reversing the comparison lets the final CMP consume `cell_a` directly,
/// without giving `result` a transient home.  Restrict the moved reads to
/// ordinary compiler-managed storage: absolute and fixed-ZP cells may be
/// externally observable and therefore keep their original order.
pub(in crate::mir6502) fn byte_binary_compare_chain_rewrite_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<ByteBinaryCompareChainRewriteCandidate> {
    let MirOp::Load {
        dst: compare_value_dst,
        src: MirAddr::Direct(compare_mem),
        width: MirWidth::Byte,
    } = ops.get(index)?
    else {
        return None;
    };
    let compare_value_temp = split_def_as_byte_compare_temp(compare_value_dst)?;
    let MirOp::Load {
        dst: binary_value_dst,
        src: MirAddr::Direct(binary_mem),
        width: MirWidth::Byte,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let binary_value_temp = split_def_as_byte_compare_temp(binary_value_dst)?;
    if !byte_binary_compare_reorderable_mem(compare_mem)
        || !byte_binary_compare_reorderable_mem(binary_mem)
    {
        return None;
    }

    let MirOp::Binary {
        op,
        dst: result_dst,
        left,
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out,
    } = ops.get(index + 2)?
    else {
        return None;
    };
    let result_temp = split_def_as_byte_compare_temp(result_dst)?;
    let binary_value = MirValue::Def(binary_value_dst.clone());
    let (left, right) = if left == &binary_value && !value_uses_temp(right) {
        (MirValue::PointerCell(binary_mem.clone()), right.clone())
    } else if right == &binary_value
        && !value_uses_temp(left)
        && matches!(
            op,
            MirBinaryOp::Add | MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor
        )
    {
        (left.clone(), MirValue::PointerCell(binary_mem.clone()))
    } else {
        return None;
    };
    let carry_in = normalized_standalone_byte_carry(*op, *carry_in, *carry_out)?;

    let MirOp::Compare {
        dst: compare_dst,
        op: compare_op,
        left: compare_left,
        right: compare_right,
        width: MirWidth::Byte,
        signed: false,
    } = ops.get(index + 3)?
    else {
        return None;
    };
    if compare_left != &MirValue::Def(compare_value_dst.clone())
        || compare_right != &MirValue::Def(result_dst.clone())
        || compare_value_temp == binary_value_temp
        || compare_value_temp == result_temp
        || binary_value_temp == result_temp
    {
        return None;
    }

    Some(ByteBinaryCompareChainRewriteCandidate {
        consumed: 4,
        replacement: vec![
            MirOp::Binary {
                op: *op,
                dst: MirDef::Reg(MirReg::A),
                left,
                right,
                width: MirWidth::Byte,
                carry_in,
                carry_out: *carry_out,
            },
            MirOp::Compare {
                dst: compare_dst.clone(),
                op: reverse_compare_operands(*compare_op),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(compare_mem.clone()),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
    })
}

fn byte_binary_compare_reorderable_mem(mem: &MirMem) -> bool {
    matches!(
        mem,
        MirMem::Global { .. }
            | MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
    )
}

fn normalized_standalone_byte_carry(
    op: MirBinaryOp,
    carry_in: Option<MirCarryIn>,
    carry_out: MirCarryOut,
) -> Option<Option<MirCarryIn>> {
    if !matches!(carry_out, MirCarryOut::Ignore) {
        return None;
    }
    match (op, carry_in) {
        (MirBinaryOp::Add, None | Some(MirCarryIn::Clear)) => Some(Some(MirCarryIn::Clear)),
        (MirBinaryOp::Sub, None | Some(MirCarryIn::Set)) => Some(Some(MirCarryIn::Set)),
        (MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor, None) => Some(None),
        _ => None,
    }
}

fn reverse_compare_operands(op: MirCompareOp) -> MirCompareOp {
    match op {
        MirCompareOp::Eq => MirCompareOp::Eq,
        MirCompareOp::Ne => MirCompareOp::Ne,
        MirCompareOp::Lt => MirCompareOp::Gt,
        MirCompareOp::Le => MirCompareOp::Ge,
        MirCompareOp::Gt => MirCompareOp::Lt,
        MirCompareOp::Ge => MirCompareOp::Le,
    }
}

pub(in crate::mir6502) fn byte_binary_compare_rewrite_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<ByteBinaryCompareRewriteCandidate> {
    let binary = ops.get(index)?;
    let compare = ops.get(index + 1)?;
    let MirOp::Binary {
        op,
        dst,
        left,
        right,
        width: MirWidth::Byte,
        carry_in,
        carry_out,
    } = binary
    else {
        return None;
    };
    if !byte_binary_compare_op_is_safe(*op, *carry_in, *carry_out) {
        return None;
    }
    if value_uses_temp(left) || value_uses_temp(right) {
        return None;
    }
    let temp = split_def_as_byte_compare_temp(dst)?;

    let MirOp::Compare {
        dst: compare_dst,
        op: compare_op,
        left: compare_left,
        right: compare_right,
        width: MirWidth::Byte,
        signed,
    } = compare
    else {
        return None;
    };
    if compare_left != &MirValue::Def(dst.clone())
        || !matches!(compare_right, MirValue::ConstU8(_) | MirValue::ConstU16(_))
    {
        return None;
    }

    Some(ByteBinaryCompareRewriteCandidate {
        temp,
        replacement: [
            MirOp::Binary {
                op: *op,
                dst: MirDef::Reg(MirReg::A),
                left: left.clone(),
                right: right.clone(),
                width: MirWidth::Byte,
                carry_in: *carry_in,
                carry_out: *carry_out,
            },
            MirOp::Compare {
                dst: compare_dst.clone(),
                op: *compare_op,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: compare_right.clone(),
                width: MirWidth::Byte,
                signed: *signed,
            },
        ],
    })
}

fn split_def_as_byte_compare_temp(def: &MirDef) -> Option<MirTempId> {
    match def {
        MirDef::VTemp(id) | MirDef::VTempByte { id, .. } => Some(*id),
        _ => None,
    }
}

fn byte_binary_compare_op_is_safe(
    op: MirBinaryOp,
    carry_in: Option<MirCarryIn>,
    carry_out: MirCarryOut,
) -> bool {
    byte_binary_compare_op_blocker(op, carry_in, carry_out).is_none()
}

fn byte_binary_compare_op_blocker(
    op: MirBinaryOp,
    carry_in: Option<MirCarryIn>,
    carry_out: MirCarryOut,
) -> Option<&'static str> {
    if matches!(carry_in, Some(MirCarryIn::FromPrevious)) {
        return Some("byte-binary-compare-blocked-carry-in");
    }
    if !matches!(carry_out, MirCarryOut::Ignore) {
        return Some("byte-binary-compare-blocked-carry-out");
    }
    match op {
        MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor => None,
        MirBinaryOp::Add if matches!(carry_in, Some(MirCarryIn::Clear)) => None,
        MirBinaryOp::Sub if matches!(carry_in, Some(MirCarryIn::Set)) => None,
        MirBinaryOp::Add | MirBinaryOp::Sub => Some("byte-binary-compare-blocked-carry-mode"),
        _ => Some("byte-binary-compare-blocked-op"),
    }
}

#[cfg(test)]
fn try_fuse_two_loaded_byte_compare_consumer(
    ops: &[MirOp],
    index: usize,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let first_load = ops.get(index)?;
    let second_load = ops.get(index + 1)?;
    let compare = ops.get(index + 2)?;
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
    let MirOp::Compare {
        dst,
        op,
        left,
        right,
        width: MirWidth::Byte,
        signed,
    } = compare
    else {
        return None;
    };
    if def_is_used_after(ops, index + 3, first_dst) || def_is_used_after(ops, index + 3, second_dst)
    {
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

    out.push(MirOp::Compare {
        dst: dst.clone(),
        op: *op,
        left,
        right,
        width: MirWidth::Byte,
        signed: *signed,
    });
    Some(3)
}

fn try_expand_byte_compare_branch(
    block_index: usize,
    blocks: &mut Vec<MirBlock>,
    next_id: &mut u32,
) -> bool {
    let Some((cond_temp, then_block, else_block)) = branch_bool_temp(&blocks[block_index]) else {
        return false;
    };
    let Some(compare_index) = blocks[block_index].ops.iter().rposition(|op| {
        matches!(
            op,
            MirOp::Compare {
                dst: MirCondDest::Temp(id),
                width: MirWidth::Byte,
                signed: false,
                ..
            } if *id == cond_temp
        )
    }) else {
        return false;
    };
    if compare_index + 1 != blocks[block_index].ops.len() {
        return false;
    }

    let MirOp::Compare {
        op, left, right, ..
    } = blocks[block_index].ops[compare_index].clone()
    else {
        return false;
    };
    if !matches!(op, MirCompareOp::Le | MirCompareOp::Gt) {
        return false;
    }
    blocks[block_index].ops.remove(compare_index);
    let mut ops = std::mem::take(&mut blocks[block_index].ops);
    let terminator = materialize_byte_compare_branch(
        &mut ops, blocks, next_id, op, left, right, then_block, else_block,
    );
    blocks[block_index].ops = ops;
    blocks[block_index].terminator = terminator;
    true
}

fn try_expand_word_compare_branch(
    block_index: usize,
    blocks: &mut Vec<MirBlock>,
    layout: &MaterializeLayout,
    next_id: &mut u32,
) -> bool {
    let Some((cond_temp, then_block, else_block)) = branch_bool_temp(&blocks[block_index]) else {
        return false;
    };
    let Some(compare_index) = blocks[block_index].ops.iter().rposition(|op| {
        matches!(
            op,
            MirOp::Compare {
                dst: MirCondDest::Temp(id),
                width: MirWidth::Word,
                ..
            } if *id == cond_temp
        )
    }) else {
        return false;
    };
    if compare_index + 1 != blocks[block_index].ops.len() {
        return false;
    }
    let MirOp::Compare {
        op,
        left,
        right,
        signed,
        ..
    } = blocks[block_index].ops.remove(compare_index)
    else {
        return false;
    };
    let (left_lo, left_hi) = split_value_as_word(left, layout);
    let (right_lo, right_hi) = split_value_as_word(right, layout);
    let entry = append_word_compare_branch_blocks(
        blocks, next_id, op, signed, left_lo, left_hi, right_lo, right_hi, then_block, else_block,
    );
    blocks[block_index].terminator = jump_terminator(entry);
    true
}

fn try_expand_short_circuit_branch(
    block_index: usize,
    blocks: &mut Vec<MirBlock>,
    layout: &MaterializeLayout,
    next_id: &mut u32,
) -> bool {
    let Some((cond_temp, then_block, else_block)) = branch_bool_temp(&blocks[block_index]) else {
        return false;
    };
    let ops = &blocks[block_index].ops;
    let Some(final_compare_index) = ops.iter().rposition(|op| {
        matches!(
            op,
            MirOp::Compare {
                dst: MirCondDest::Temp(id),
                op: MirCompareOp::Ne,
                left: MirValue::Def(MirDef::VTemp(_)),
                right: MirValue::ConstU8(0) | MirValue::ConstU16(0),
                width: MirWidth::Byte,
                ..
            } if *id == cond_temp
        )
    }) else {
        return false;
    };
    if final_compare_index + 1 != ops.len() {
        return false;
    }
    let MirOp::Compare {
        left: MirValue::Def(MirDef::VTemp(binary_temp)),
        ..
    } = &ops[final_compare_index]
    else {
        return false;
    };
    let Some(chain) = collect_short_circuit_compare_chain(ops, *binary_temp) else {
        return false;
    };
    if chain.compares.len() < 2 {
        return false;
    }
    let Some(first_compare_index) = chain.compares.iter().map(|(index, _)| *index).min() else {
        return false;
    };
    if chain
        .used_indices
        .iter()
        .any(|index| *index >= final_compare_index)
    {
        return false;
    }
    let original_ops = blocks[block_index].ops.clone();
    for (index, op) in original_ops
        .iter()
        .enumerate()
        .take(final_compare_index)
        .skip(first_compare_index)
    {
        if !chain.used_indices.contains(&index) && !short_circuit_preservable_op(op) {
            return false;
        }
    }

    let mut current_index = block_index;
    let compare_len = chain.compares.len();
    let bool_op = chain.op;
    let compare_indices = chain
        .compares
        .iter()
        .map(|(index, _)| *index)
        .collect::<Vec<_>>();
    for (compare_pos, (_, compare)) in chain.compares.into_iter().enumerate() {
        let next_block = if compare_pos + 1 < compare_len {
            let id = fresh_block_id(next_id);
            blocks.push(MirBlock {
                id,
                label: format!("cmp_sc_{}", id.0),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Return,
            });
            Some((id, blocks.len() - 1))
        } else {
            None
        };
        let (compare_then, compare_else) = match (bool_op, next_block) {
            (MirBinaryOp::And, Some((next_id, _))) => (next_id, else_block),
            (MirBinaryOp::And, None) => (then_block, else_block),
            (MirBinaryOp::Or, Some((next_id, _))) => (then_block, next_id),
            (MirBinaryOp::Or, None) => (then_block, else_block),
            _ => return false,
        };
        let mut current_ops = if compare_pos == 0 {
            original_ops[..first_compare_index].to_vec()
        } else {
            let previous_compare = compare_indices[compare_pos - 1];
            let current_compare = compare_indices[compare_pos];
            original_ops[previous_compare + 1..current_compare]
                .iter()
                .enumerate()
                .filter_map(|(offset, op)| {
                    let index = previous_compare + 1 + offset;
                    (!chain.used_indices.contains(&index)).then_some(op.clone())
                })
                .collect()
        };
        let terminator = materialize_short_circuit_compare_branch(
            compare,
            &mut current_ops,
            blocks,
            layout,
            next_id,
            compare_then,
            compare_else,
        );
        blocks[current_index].ops = current_ops;
        blocks[current_index].terminator = terminator;
        if let Some((_, next_index)) = next_block {
            current_index = next_index;
        }
    }
    true
}

fn short_circuit_preservable_op(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::LoadImm { .. }
            | MirOp::Load { .. }
            | MirOp::Move { .. }
            | MirOp::LeaAddr { .. }
            | MirOp::Extend { .. }
            | MirOp::Truncate { .. }
            | MirOp::Unary { .. }
            | MirOp::Binary { .. }
            | MirOp::Compare { .. }
    )
}

struct ShortCircuitCompareChain {
    op: MirBinaryOp,
    compares: Vec<(usize, ShortCircuitCompare)>,
    used_indices: BTreeSet<usize>,
}

fn collect_short_circuit_compare_chain(
    ops: &[MirOp],
    root_temp: MirTempId,
) -> Option<ShortCircuitCompareChain> {
    let (op, _, _) = bool_binary_temp(ops, root_temp)?;
    let mut compares = Vec::new();
    let mut used_indices = BTreeSet::new();
    collect_short_circuit_compare_chain_inner(
        ops,
        root_temp,
        op,
        &mut compares,
        &mut used_indices,
    )?;
    compares.sort_by_key(|(index, _)| *index);
    Some(ShortCircuitCompareChain {
        op,
        compares,
        used_indices,
    })
}

fn collect_short_circuit_compare_chain_inner(
    ops: &[MirOp],
    temp: MirTempId,
    bool_op: MirBinaryOp,
    compares: &mut Vec<(usize, ShortCircuitCompare)>,
    used_indices: &mut BTreeSet<usize>,
) -> Option<()> {
    if let Some(index) = compare_temp_index(ops, temp) {
        let compare = short_circuit_compare_for_branch(&ops[index])?;
        used_indices.insert(index);
        compares.push((index, compare));
        return Some(());
    }
    let (op, left_temp, right_temp) = bool_binary_temp(ops, temp)?;
    if op != bool_op {
        return None;
    }
    let index = bool_binary_temp_index(ops, temp)?;
    used_indices.insert(index);
    collect_short_circuit_compare_chain_inner(ops, left_temp, bool_op, compares, used_indices)?;
    collect_short_circuit_compare_chain_inner(ops, right_temp, bool_op, compares, used_indices)?;
    Some(())
}

fn bool_binary_temp(ops: &[MirOp], temp: MirTempId) -> Option<(MirBinaryOp, MirTempId, MirTempId)> {
    let MirOp::Binary {
        dst: MirDef::VTemp(_),
        op,
        left: MirValue::Def(MirDef::VTemp(left_temp)),
        right: MirValue::Def(MirDef::VTemp(right_temp)),
        width: MirWidth::Byte,
        ..
    } = ops.get(bool_binary_temp_index(ops, temp)?)?
    else {
        return None;
    };
    if !matches!(op, MirBinaryOp::And | MirBinaryOp::Or) {
        return None;
    }
    Some((*op, *left_temp, *right_temp))
}

fn bool_binary_temp_index(ops: &[MirOp], temp: MirTempId) -> Option<usize> {
    ops.iter().rposition(|op| {
        matches!(
            op,
            MirOp::Binary {
                dst: MirDef::VTemp(id),
                op: MirBinaryOp::And | MirBinaryOp::Or,
                left: MirValue::Def(MirDef::VTemp(_)),
                right: MirValue::Def(MirDef::VTemp(_)),
                width: MirWidth::Byte,
                ..
            } if *id == temp
        )
    })
}

enum ShortCircuitCompare {
    Byte {
        op: MirCompareOp,
        left: MirValue,
        right: MirValue,
    },
    Word {
        op: MirCompareOp,
        signed: bool,
        left: MirValue,
        right: MirValue,
    },
}

fn short_circuit_compare_for_branch(op: &MirOp) -> Option<ShortCircuitCompare> {
    if let MirOp::Compare {
        op,
        left,
        right,
        width: MirWidth::Byte,
        signed: false,
        ..
    } = op
    {
        return Some(ShortCircuitCompare::Byte {
            op: *op,
            left: left.clone(),
            right: right.clone(),
        });
    }
    let MirOp::Compare {
        op,
        left,
        right,
        width: MirWidth::Word,
        signed,
        ..
    } = op
    else {
        return None;
    };
    Some(ShortCircuitCompare::Word {
        op: *op,
        signed: *signed,
        left: left.clone(),
        right: right.clone(),
    })
}

fn materialize_short_circuit_compare_branch(
    compare: ShortCircuitCompare,
    ops: &mut Vec<MirOp>,
    blocks: &mut Vec<MirBlock>,
    layout: &MaterializeLayout,
    next_id: &mut u32,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirTerminator {
    match compare {
        ShortCircuitCompare::Byte { op, left, right } => materialize_byte_compare_branch(
            ops, blocks, next_id, op, left, right, then_block, else_block,
        ),
        ShortCircuitCompare::Word {
            op,
            signed,
            left,
            right,
        } => {
            let (left_lo, left_hi) = split_value_as_word(left, layout);
            let (right_lo, right_hi) = split_value_as_word(right, layout);
            let entry = append_word_compare_branch_blocks(
                blocks, next_id, op, signed, left_lo, left_hi, right_lo, right_hi, then_block,
                else_block,
            );
            jump_terminator(entry)
        }
    }
}

fn materialize_byte_compare_branch(
    ops: &mut Vec<MirOp>,
    blocks: &mut Vec<MirBlock>,
    next_id: &mut u32,
    op: MirCompareOp,
    left: MirValue,
    right: MirValue,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirTerminator {
    ops.push(MirOp::Compare {
        dst: MirCondDest::Flags,
        op,
        left,
        right,
        width: MirWidth::Byte,
        signed: false,
    });
    match op {
        MirCompareOp::Eq => {
            branch_terminator(MirCond::FlagTest(MirFlagTest::ZSet), then_block, else_block)
        }
        MirCompareOp::Ne => branch_terminator(
            MirCond::FlagTest(MirFlagTest::ZClear),
            then_block,
            else_block,
        ),
        MirCompareOp::Lt => branch_terminator(
            MirCond::FlagTest(MirFlagTest::CClear),
            then_block,
            else_block,
        ),
        MirCompareOp::Ge => {
            branch_terminator(MirCond::FlagTest(MirFlagTest::CSet), then_block, else_block)
        }
        MirCompareOp::Le => branch_terminator(
            MirCond::AnyFlagTest([MirFlagTest::CClear, MirFlagTest::ZSet]),
            then_block,
            else_block,
        ),
        MirCompareOp::Gt => {
            let eq = fresh_block_id(next_id);
            blocks.push(flag_branch_block(
                eq,
                "cmp_byte_eq",
                MirFlagTest::ZSet,
                else_block,
                then_block,
            ));
            branch_terminator(MirCond::FlagTest(MirFlagTest::CClear), else_block, eq)
        }
    }
}

fn compare_temp_index(ops: &[MirOp], temp: MirTempId) -> Option<usize> {
    ops.iter().position(|op| {
        matches!(
            op,
            MirOp::Compare {
                dst: MirCondDest::Temp(id),
                ..
            } if *id == temp
        )
    })
}

fn branch_bool_temp(block: &MirBlock) -> Option<(MirTempId, MirBlockId, MirBlockId)> {
    let MirTerminator::Branch {
        cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))),
        ref then_edge,
        ref else_edge,
    } = block.terminator
    else {
        return None;
    };
    Some((id, then_edge.target, else_edge.target))
}

fn append_word_compare_branch_blocks(
    blocks: &mut Vec<MirBlock>,
    next_id: &mut u32,
    op: MirCompareOp,
    signed: bool,
    left_lo: MirValue,
    left_hi: MirValue,
    right_lo: MirValue,
    right_hi: MirValue,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirBlockId {
    if matches!(op, MirCompareOp::Eq | MirCompareOp::Ne) {
        return append_word_eq_ne_branch_blocks(
            blocks, next_id, op, left_lo, left_hi, right_lo, right_hi, then_block, else_block,
        );
    }
    if signed {
        append_signed_word_rel_branch_blocks(
            blocks, next_id, op, left_lo, left_hi, right_lo, right_hi, then_block, else_block,
        )
    } else {
        append_unsigned_word_rel_branch_blocks(
            blocks, next_id, op, left_lo, left_hi, right_lo, right_hi, then_block, else_block,
        )
    }
}

fn append_word_eq_ne_branch_blocks(
    blocks: &mut Vec<MirBlock>,
    next_id: &mut u32,
    op: MirCompareOp,
    left_lo: MirValue,
    left_hi: MirValue,
    right_lo: MirValue,
    right_hi: MirValue,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirBlockId {
    let low = fresh_block_id(next_id);
    let high = fresh_block_id(next_id);
    let (low_diff_target, high_equal_target, high_diff_target) = match op {
        MirCompareOp::Eq => (else_block, then_block, else_block),
        MirCompareOp::Ne => (then_block, else_block, then_block),
        _ => unreachable!("only equality ops reach this helper"),
    };
    blocks.push(compare_branch_block(
        low,
        "cmp_word_lo",
        left_lo,
        right_lo,
        MirCompareOp::Eq,
        MirFlagTest::ZClear,
        low_diff_target,
        high,
    ));
    blocks.push(compare_branch_block(
        high,
        "cmp_word_hi",
        left_hi,
        right_hi,
        MirCompareOp::Eq,
        MirFlagTest::ZSet,
        high_equal_target,
        high_diff_target,
    ));
    low
}

fn append_unsigned_word_rel_branch_blocks(
    blocks: &mut Vec<MirBlock>,
    next_id: &mut u32,
    op: MirCompareOp,
    left_lo: MirValue,
    left_hi: MirValue,
    right_lo: MirValue,
    right_hi: MirValue,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirBlockId {
    match op {
        MirCompareOp::Lt => append_unsigned_lt_blocks(
            blocks, next_id, left_lo, left_hi, right_lo, right_hi, then_block, else_block,
        ),
        MirCompareOp::Ge => append_unsigned_lt_blocks(
            blocks, next_id, left_lo, left_hi, right_lo, right_hi, else_block, then_block,
        ),
        MirCompareOp::Gt => append_unsigned_lt_blocks(
            blocks, next_id, right_lo, right_hi, left_lo, left_hi, then_block, else_block,
        ),
        MirCompareOp::Le => append_unsigned_lt_blocks(
            blocks, next_id, right_lo, right_hi, left_lo, left_hi, else_block, then_block,
        ),
        MirCompareOp::Eq | MirCompareOp::Ne => unreachable!("equality handled separately"),
    }
}

fn append_unsigned_lt_blocks(
    blocks: &mut Vec<MirBlock>,
    next_id: &mut u32,
    left_lo: MirValue,
    left_hi: MirValue,
    right_lo: MirValue,
    right_hi: MirValue,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirBlockId {
    let high = fresh_block_id(next_id);
    let high_not_less = fresh_block_id(next_id);
    let low = fresh_block_id(next_id);
    blocks.push(compare_branch_block(
        high,
        "cmp_word_hi_lt",
        left_hi,
        right_hi,
        MirCompareOp::Lt,
        MirFlagTest::CClear,
        then_block,
        high_not_less,
    ));
    blocks.push(flag_branch_block(
        high_not_less,
        "cmp_word_hi_eq",
        MirFlagTest::ZSet,
        low,
        else_block,
    ));
    blocks.push(compare_branch_block(
        low,
        "cmp_word_lo_lt",
        left_lo,
        right_lo,
        MirCompareOp::Lt,
        MirFlagTest::CClear,
        then_block,
        else_block,
    ));
    high
}

fn append_signed_word_rel_branch_blocks(
    blocks: &mut Vec<MirBlock>,
    next_id: &mut u32,
    op: MirCompareOp,
    left_lo: MirValue,
    left_hi: MirValue,
    right_lo: MirValue,
    right_hi: MirValue,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirBlockId {
    match op {
        MirCompareOp::Lt => append_signed_lt_blocks(
            blocks, next_id, left_lo, left_hi, right_lo, right_hi, then_block, else_block,
        ),
        MirCompareOp::Ge => append_signed_lt_blocks(
            blocks, next_id, left_lo, left_hi, right_lo, right_hi, else_block, then_block,
        ),
        MirCompareOp::Gt => append_signed_lt_blocks(
            blocks, next_id, right_lo, right_hi, left_lo, left_hi, then_block, else_block,
        ),
        MirCompareOp::Le => append_signed_lt_blocks(
            blocks, next_id, right_lo, right_hi, left_lo, left_hi, else_block, then_block,
        ),
        MirCompareOp::Eq | MirCompareOp::Ne => unreachable!("equality handled separately"),
    }
}

fn append_signed_lt_blocks(
    blocks: &mut Vec<MirBlock>,
    next_id: &mut u32,
    left_lo: MirValue,
    left_hi: MirValue,
    right_lo: MirValue,
    right_hi: MirValue,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirBlockId {
    if value_uses_temp(&left_lo)
        || value_uses_temp(&left_hi)
        || value_uses_temp(&right_lo)
        || value_uses_temp(&right_hi)
    {
        return append_signed_lt_sign_dispatch_blocks(
            blocks, next_id, left_lo, left_hi, right_lo, right_hi, then_block, else_block,
        );
    }
    let subtract = fresh_block_id(next_id);
    let overflow_set = fresh_block_id(next_id);
    let overflow_clear = fresh_block_id(next_id);
    blocks.push(MirBlock {
        id: subtract,
        label: format!("cmp_i16_sub_{}", subtract.0),
        params: Vec::new(),
        ops: vec![
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: left_lo,
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: right_lo,
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Set),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: left_hi,
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: right_hi,
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            },
        ],
        terminator: branch_terminator(
            MirCond::FlagTest(MirFlagTest::VSet),
            overflow_set,
            overflow_clear,
        ),
    });
    blocks.push(flag_branch_block(
        overflow_set,
        "cmp_i16_v_set",
        MirFlagTest::NClear,
        then_block,
        else_block,
    ));
    blocks.push(flag_branch_block(
        overflow_clear,
        "cmp_i16_v_clear",
        MirFlagTest::NSet,
        then_block,
        else_block,
    ));
    subtract
}

fn append_signed_lt_sign_dispatch_blocks(
    blocks: &mut Vec<MirBlock>,
    next_id: &mut u32,
    left_lo: MirValue,
    left_hi: MirValue,
    right_lo: MirValue,
    right_hi: MirValue,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirBlockId {
    let left_sign = fresh_block_id(next_id);
    let left_nonnegative = fresh_block_id(next_id);
    let left_negative = fresh_block_id(next_id);
    let same_sign = append_unsigned_lt_blocks(
        blocks,
        next_id,
        left_lo,
        left_hi.clone(),
        right_lo,
        right_hi.clone(),
        then_block,
        else_block,
    );
    blocks.push(compare_branch_block(
        left_sign,
        "cmp_i16_left_sign",
        left_hi,
        MirValue::ConstU8(0x80),
        MirCompareOp::Lt,
        MirFlagTest::CClear,
        left_nonnegative,
        left_negative,
    ));
    blocks.push(compare_branch_block(
        left_nonnegative,
        "cmp_i16_right_sign_pos",
        right_hi.clone(),
        MirValue::ConstU8(0x80),
        MirCompareOp::Lt,
        MirFlagTest::CClear,
        same_sign,
        else_block,
    ));
    blocks.push(compare_branch_block(
        left_negative,
        "cmp_i16_right_sign_neg",
        right_hi,
        MirValue::ConstU8(0x80),
        MirCompareOp::Lt,
        MirFlagTest::CClear,
        then_block,
        same_sign,
    ));
    left_sign
}

fn compare_branch_block(
    id: MirBlockId,
    label_prefix: &str,
    left: MirValue,
    right: MirValue,
    op: MirCompareOp,
    flag_test: MirFlagTest,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirBlock {
    MirBlock {
        id,
        label: format!("{}_{}", label_prefix, id.0),
        params: Vec::new(),
        ops: vec![MirOp::Compare {
            dst: MirCondDest::Flags,
            op,
            left,
            right,
            width: MirWidth::Byte,
            signed: false,
        }],
        terminator: branch_terminator(MirCond::FlagTest(flag_test), then_block, else_block),
    }
}

fn flag_branch_block(
    id: MirBlockId,
    label_prefix: &str,
    flag_test: MirFlagTest,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirBlock {
    MirBlock {
        id,
        label: format!("{}_{}", label_prefix, id.0),
        params: Vec::new(),
        ops: Vec::new(),
        terminator: branch_terminator(MirCond::FlagTest(flag_test), then_block, else_block),
    }
}

fn jump_terminator(target: MirBlockId) -> MirTerminator {
    MirTerminator::Jump(MirEdge::plain(target))
}

fn branch_terminator(
    cond: MirCond,
    then_block: MirBlockId,
    else_block: MirBlockId,
) -> MirTerminator {
    MirTerminator::Branch {
        cond,
        then_edge: MirEdge::plain(then_block),
        else_edge: MirEdge::plain(else_block),
    }
}

fn fresh_block_id(next_id: &mut u32) -> MirBlockId {
    let id = MirBlockId(*next_id);
    *next_id = next_id.saturating_add(1);
    id
}

pub(super) fn compare_branch_plan(
    op: MirCompareOp,
    right: &MirValue,
) -> Option<(MirFlagTest, Option<(MirCompareOp, MirValue)>)> {
    match op {
        MirCompareOp::Eq => Some((MirFlagTest::ZSet, None)),
        MirCompareOp::Ne => Some((MirFlagTest::ZClear, None)),
        MirCompareOp::Lt => Some((MirFlagTest::CClear, None)),
        MirCompareOp::Ge => Some((MirFlagTest::CSet, None)),
        MirCompareOp::Le => {
            let value = const_u8_value(right)?;
            let next = value.checked_add(1)?;
            Some((
                MirFlagTest::CClear,
                Some((MirCompareOp::Lt, MirValue::ConstU8(next))),
            ))
        }
        MirCompareOp::Gt => {
            let value = const_u8_value(right)?;
            let next = value.checked_add(1)?;
            Some((
                MirFlagTest::CSet,
                Some((MirCompareOp::Ge, MirValue::ConstU8(next))),
            ))
        }
    }
}

fn const_u8_value(value: &MirValue) -> Option<u8> {
    match value {
        MirValue::ConstU8(value) => Some(*value),
        MirValue::ConstU16(value) if *value <= 0x00FF => Some(*value as u8),
        _ => None,
    }
}
