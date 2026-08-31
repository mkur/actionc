use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::ir::{
    MirAddr, MirBinaryOp, MirBlock, MirBlockId, MirCarryIn, MirCompareOp, MirCond, MirDef,
    MirFlagTest, MirMem, MirOp, MirReg, MirRoutine, MirTerminator, MirUpdateOp, MirValue, MirWidth,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum MirCountDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirCountedLoopShape {
    HeadTested,
    BottomGuarded { guard: MirBlockId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirCountedLoop {
    pub(in crate::mir6502) preheader: MirBlockId,
    pub(in crate::mir6502) header: MirBlockId,
    pub(in crate::mir6502) body: MirBlockId,
    pub(in crate::mir6502) latch: MirBlockId,
    pub(in crate::mir6502) exit: MirBlockId,
    pub(in crate::mir6502) induction: MirMem,
    pub(in crate::mir6502) width: MirWidth,
    pub(in crate::mir6502) initial_value: MirValue,
    pub(in crate::mir6502) bound: u16,
    pub(in crate::mir6502) direction: MirCountDirection,
    pub(in crate::mir6502) step: u16,
    pub(in crate::mir6502) signed: bool,
    pub(in crate::mir6502) initial_guard_required: bool,
    pub(in crate::mir6502) final_value_observable: bool,
    pub(in crate::mir6502) shape: MirCountedLoopShape,
}

pub(in crate::mir6502) fn analyze_counted_loops(routine: &MirRoutine) -> Vec<MirCountedLoop> {
    let Ok(cfg) = MirCfg::from_routine(routine) else {
        return Vec::new();
    };
    let mut loops = Vec::new();
    for header in &routine.blocks {
        if let Some(candidate) = analyze_bottom_guarded_loop(routine, &cfg, header) {
            loops.push(candidate);
        }
        if let Some(candidate) = analyze_head_tested_loop(routine, &cfg, header) {
            loops.push(candidate);
        }
    }
    loops
}

fn analyze_head_tested_loop(
    routine: &MirRoutine,
    cfg: &MirCfg,
    header: &MirBlock,
) -> Option<MirCountedLoop> {
    let (induction, compare) = header_compare(header)?;
    let (direction, body, exit, bound, signed) = compare_loop_edges(header, compare)?;
    let predecessors = cfg.predecessors(header.id);
    if predecessors.len() != 2 {
        return None;
    }

    let mut preheader = None;
    let mut latch = None;
    let mut initial_value = None;
    for predecessor in predecessors {
        let block = block_by_id(routine, *predecessor)?;
        if let Some((update_direction, step)) = latch_update(block, header.id, &induction) {
            if update_direction == direction && step == 1 {
                latch = Some(block.id);
            }
        } else if let Some(initial) = preheader_initial(block, header.id, &induction) {
            preheader = Some(block.id);
            initial_value = Some(initial);
        }
    }
    let (preheader, latch, initial_value) = (preheader?, latch?, initial_value?);
    let initial_guard_required =
        !initial_value_satisfies_guard(&initial_value, direction, compare.op, bound, signed);
    Some(MirCountedLoop {
        preheader,
        header: header.id,
        body,
        latch,
        exit,
        induction: induction.clone(),
        width: MirWidth::Byte,
        initial_value,
        bound,
        direction,
        step: 1,
        signed,
        initial_guard_required,
        final_value_observable: mem_may_be_read_from(routine, cfg, exit, &induction),
        shape: MirCountedLoopShape::HeadTested,
    })
}

fn analyze_bottom_guarded_loop(
    routine: &MirRoutine,
    cfg: &MirCfg,
    header: &MirBlock,
) -> Option<MirCountedLoop> {
    let (header_induction, compare, compare_index) =
        if let Some((induction, compare)) = header_compare(header) {
            (Some(induction), compare, 1)
        } else {
            (None, single_compare(header)?, 0)
        };
    if compare.op != MirCompareOp::Ge
        || compare.signed
        || compare.width != MirWidth::Byte
        || compare.right != &MirValue::ConstU8(0)
        || compare.left != &MirValue::Def(MirDef::Reg(MirReg::A))
    {
        return None;
    }
    let MirTerminator::Branch {
        cond,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return None;
    };
    if branch_flag_test(header, cond, compare_index) != Some(MirFlagTest::CSet)
        || !then_edge.args.is_empty()
        || !else_edge.args.is_empty()
    {
        return None;
    }
    let body = then_edge.target;
    let exit = else_edge.target;
    let predecessors = cfg.predecessors(header.id);
    if predecessors.len() != 2 {
        return None;
    }

    for latch_id in predecessors {
        let latch = block_by_id(routine, *latch_id)?;
        let Some(induction) = decrement_latch_mem(latch, header.id) else {
            continue;
        };
        if header_induction
            .as_ref()
            .is_some_and(|header_mem| header_mem != &induction)
        {
            continue;
        }
        let Some((guard, guard_exit)) = bottom_zero_guard(routine, cfg, latch.id, &induction)
        else {
            continue;
        };
        if guard_exit != exit {
            continue;
        }
        let Some((preheader_id, initial_value)) = predecessors
            .iter()
            .filter(|candidate| **candidate != latch.id)
            .find_map(|candidate| {
                let preheader = block_by_id(routine, *candidate)?;
                preheader_initial(preheader, header.id, &induction)
                    .map(|initial| (preheader.id, initial))
            })
        else {
            continue;
        };
        let initial_guard_required = !matches!(initial_value, MirValue::ConstU8(_));
        return Some(MirCountedLoop {
            preheader: preheader_id,
            header: header.id,
            body,
            latch: latch.id,
            exit,
            induction: induction.clone(),
            width: MirWidth::Byte,
            initial_value,
            bound: 0,
            direction: MirCountDirection::Descending,
            step: 1,
            signed: false,
            initial_guard_required,
            final_value_observable: mem_may_be_read_from(routine, cfg, exit, &induction),
            shape: MirCountedLoopShape::BottomGuarded { guard },
        });
    }
    None
}

#[derive(Clone, Copy)]
struct HeaderCompare<'a> {
    op: MirCompareOp,
    left: &'a MirValue,
    right: &'a MirValue,
    width: MirWidth,
    signed: bool,
}

fn header_compare(header: &MirBlock) -> Option<(MirMem, HeaderCompare<'_>)> {
    let [
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(induction),
            width: MirWidth::Byte,
        },
        MirOp::Compare {
            op,
            left,
            right,
            width,
            signed,
            ..
        },
    ] = header.ops.as_slice()
    else {
        return None;
    };
    Some((
        induction.clone(),
        HeaderCompare {
            op: *op,
            left,
            right,
            width: *width,
            signed: *signed,
        },
    ))
}

fn single_compare(header: &MirBlock) -> Option<HeaderCompare<'_>> {
    let [
        MirOp::Compare {
            op,
            left,
            right,
            width,
            signed,
            ..
        },
    ] = header.ops.as_slice()
    else {
        return None;
    };
    Some(HeaderCompare {
        op: *op,
        left,
        right,
        width: *width,
        signed: *signed,
    })
}

fn compare_loop_edges(
    header: &MirBlock,
    compare: HeaderCompare<'_>,
) -> Option<(MirCountDirection, MirBlockId, MirBlockId, u16, bool)> {
    if compare.left != &MirValue::Def(MirDef::Reg(MirReg::A)) || compare.width != MirWidth::Byte {
        return None;
    }
    let bound = match compare.right {
        MirValue::ConstU8(value) => u16::from(*value),
        MirValue::ConstU16(value) if *value <= u16::from(u8::MAX) => *value,
        _ => return None,
    };
    let MirTerminator::Branch {
        cond,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return None;
    };
    if !then_edge.args.is_empty() || !else_edge.args.is_empty() {
        return None;
    }
    match (compare.op, branch_flag_test(header, cond, 1)) {
        (MirCompareOp::Lt, Some(MirFlagTest::CClear)) => Some((
            MirCountDirection::Ascending,
            then_edge.target,
            else_edge.target,
            bound,
            compare.signed,
        )),
        (MirCompareOp::Ge, Some(MirFlagTest::CSet)) => Some((
            MirCountDirection::Descending,
            then_edge.target,
            else_edge.target,
            bound,
            compare.signed,
        )),
        _ => None,
    }
}

fn latch_update(
    block: &MirBlock,
    header: MirBlockId,
    induction: &MirMem,
) -> Option<(MirCountDirection, u16)> {
    let MirTerminator::Jump(edge) = &block.terminator else {
        return None;
    };
    if edge.target != header || !edge.args.is_empty() {
        return None;
    }
    match block.ops.last()? {
        MirOp::UpdateMem {
            op,
            mem,
            width: MirWidth::Byte,
        } if mem == induction => Some((
            match op {
                MirUpdateOp::Inc => MirCountDirection::Ascending,
                MirUpdateOp::Dec => MirCountDirection::Descending,
            },
            1,
        )),
        _ => None,
    }
}

fn decrement_latch_mem(block: &MirBlock, header: MirBlockId) -> Option<MirMem> {
    let MirTerminator::Jump(edge) = &block.terminator else {
        return None;
    };
    if edge.target != header || !edge.args.is_empty() {
        return None;
    }
    match block.ops.as_slice() {
        [
            MirOp::UpdateMem {
                op: MirUpdateOp::Dec,
                mem,
                width: MirWidth::Byte,
            },
        ] => Some(mem.clone()),
        [
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0xff),
                width: MirWidth::Byte,
                carry_in: None | Some(MirCarryIn::Clear),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ] => Some(mem.clone()),
        _ => None,
    }
}

fn bottom_zero_guard(
    routine: &MirRoutine,
    cfg: &MirCfg,
    latch: MirBlockId,
    induction: &MirMem,
) -> Option<(MirBlockId, MirBlockId)> {
    let predecessors = cfg.predecessors(latch);
    if predecessors.len() != 1 {
        return None;
    }
    let guard = block_by_id(routine, *predecessors.first()?)?;
    let compare_index = match guard.ops.as_slice() {
        [
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(guard_mem),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                op: MirCompareOp::Lt,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                signed: false,
                ..
            },
        ] if guard_mem == induction => 1,
        [
            MirOp::Compare {
                op: MirCompareOp::Lt,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                signed: false,
                ..
            },
        ] => 0,
        _ => return None,
    };
    let MirTerminator::Branch {
        cond,
        then_edge,
        else_edge,
    } = &guard.terminator
    else {
        return None;
    };
    if branch_flag_test(guard, cond, compare_index) != Some(MirFlagTest::CClear)
        || !then_edge.args.is_empty()
        || !else_edge.args.is_empty()
        || else_edge.target != latch
    {
        return None;
    }
    Some((guard.id, then_edge.target))
}

fn branch_flag_test(block: &MirBlock, cond: &MirCond, producer_op: usize) -> Option<MirFlagTest> {
    match cond {
        MirCond::FlagTest(test) => Some(test.clone()),
        MirCond::FusedCompare {
            producer,
            flag_test,
        } if producer.block == block.id && producer.op_index == producer_op => {
            Some(flag_test.clone())
        }
        _ => None,
    }
}

fn preheader_initial(block: &MirBlock, header: MirBlockId, induction: &MirMem) -> Option<MirValue> {
    let MirTerminator::Jump(edge) = &block.terminator else {
        return None;
    };
    if edge.target != header || !edge.args.is_empty() {
        return None;
    }
    let MirOp::Store {
        dst: MirAddr::Direct(mem),
        src,
        width: MirWidth::Byte,
    } = block.ops.last()?
    else {
        return None;
    };
    if mem != induction {
        return None;
    }
    if src == &MirValue::Def(MirDef::Reg(MirReg::A)) {
        match block.ops.get(block.ops.len().saturating_sub(2)) {
            Some(MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(value),
                width: MirWidth::Byte,
            }) => return Some(MirValue::ConstU8(*value)),
            Some(MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value,
                width: MirWidth::Byte,
            }) if *value <= u16::from(u8::MAX) => {
                return Some(MirValue::ConstU8(*value as u8));
            }
            _ => {}
        }
    }
    Some(src.clone())
}

fn initial_value_satisfies_guard(
    initial: &MirValue,
    direction: MirCountDirection,
    compare: MirCompareOp,
    bound: u16,
    signed: bool,
) -> bool {
    if signed {
        return false;
    }
    let value = match initial {
        MirValue::ConstU8(value) => u16::from(*value),
        MirValue::ConstU16(value) if *value <= u16::from(u8::MAX) => *value,
        _ => return false,
    };
    matches!(
        (direction, compare),
        (MirCountDirection::Ascending, MirCompareOp::Lt) if value < bound
    ) || matches!(
        (direction, compare),
        (MirCountDirection::Descending, MirCompareOp::Ge) if value >= bound
    )
}

fn mem_may_be_read_from(
    routine: &MirRoutine,
    cfg: &MirCfg,
    start: MirBlockId,
    mem: &MirMem,
) -> bool {
    let mut pending = vec![start];
    let mut reachable = BTreeSet::new();
    while let Some(block) = pending.pop() {
        if !reachable.insert(block) {
            continue;
        }
        pending.extend(cfg.successors(block).iter().copied());
    }
    reachable.iter().any(|block_id| {
        block_by_id(routine, *block_id).is_some_and(|block| {
            block.ops.iter().any(|op| {
                matches!(
                    op,
                    MirOp::Load {
                        src: MirAddr::Direct(source),
                        ..
                    } if source == mem
                )
            })
        })
    })
}

fn block_by_id(routine: &MirRoutine, id: MirBlockId) -> Option<&MirBlock> {
    routine.blocks.iter().find(|block| block.id == id)
}
