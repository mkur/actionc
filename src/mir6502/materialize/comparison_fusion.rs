//! Exclusive captured-character comparisons, preserving the original read order.
use super::*;
use crate::mir6502::analysis::{comparison_loops::ComparisonLoop, sites::MirSite};
use crate::mir6502::ir::{MirBlock, MirEdge};
use crate::mir6502::rewrite::context::PreHomeRewriteContext;

pub(super) fn prove(
    r: &super::super::ir::MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    f: &ComparisonLoop,
) -> Option<Option<MirCompareOp>> {
    let body = r.blocks.iter().find(|b| b.id == f.body)?;
    let mismatch = r.blocks.iter().find(|b| b.id == f.mismatch)?;
    let MirOp::Compare {
        dst: MirCondDest::Temp(condition),
        ..
    } = body.ops.last()?
    else {
        return None;
    };
    if !c
        .exclusive_byte_temp_flow(
            *condition,
            MirSite::Op {
                block: f.body,
                op_index: body.ops.len() - 1,
            },
            MirSite::Terminator { block: f.body },
        )
        .is_proven()
    {
        return None;
    }

    let mut allowed = BTreeSet::from([
        MirSite::Op {
            block: f.body,
            op_index: body.ops.len() - 3,
        },
        MirSite::Op {
            block: f.body,
            op_index: body.ops.len() - 2,
        },
    ]);
    let ordering = if let [
        MirOp::Load {
            dst: MirDef::VTemp(a),
            src: MirAddr::Direct(left),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::VTemp(b),
            src: MirAddr::Direct(right),
            width: MirWidth::Byte,
        },
        MirOp::Compare {
            dst: MirCondDest::Temp(result),
            op,
            left: MirValue::Def(MirDef::VTemp(x)),
            right: MirValue::Def(MirDef::VTemp(y)),
            width: MirWidth::Byte,
            signed: false,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
            src: MirValue::Def(MirDef::VTemp(z)),
            width: MirWidth::Byte,
        },
    ] = mismatch.ops.as_slice()
    {
        if left != &f.left.capture
            || right != &f.right.capture
            || a != x
            || b != y
            || result != z
            || !matches!(
                op,
                MirCompareOp::Gt | MirCompareOp::Ge | MirCompareOp::Lt | MirCompareOp::Le
            )
            || !matches!(mismatch.terminator, MirTerminator::Return)
            || c.cfg().predecessors(mismatch.id) != &BTreeSet::from([f.body])
        {
            return None;
        }
        for (temp, def, use_index) in [(*a, 0, 2), (*b, 1, 2), (*result, 2, 3)] {
            if !c
                .exclusive_byte_temp_flow(
                    temp,
                    MirSite::Op {
                        block: mismatch.id,
                        op_index: def,
                    },
                    MirSite::Op {
                        block: mismatch.id,
                        op_index: use_index,
                    },
                )
                .is_proven()
            {
                return None;
            }
        }
        allowed.extend([
            MirSite::Op {
                block: mismatch.id,
                op_index: 0,
            },
            MirSite::Op {
                block: mismatch.id,
                op_index: 1,
            },
        ]);
        Some(*op)
    } else {
        None
    };
    for operand in [&f.left, &f.right] {
        if !c.private_cell_reads_confined(
            r,
            &operand.capture,
            MirSite::Op {
                block: f.body,
                op_index: operand.load + 1,
            },
            &allowed,
        ) {
            return None;
        }
    }
    Some(ordering)
}

pub(super) fn replace_ordering(
    r: &mut super::super::ir::MirRoutine,
    f: &ComparisonLoop,
    op: MirCompareOp,
) {
    let next = r.blocks.iter().map(|b| b.id.0).max().unwrap() + 1;
    let one = MirBlockId(next);
    let zero = MirBlockId(next + 1);
    let mismatch = r.blocks.iter_mut().find(|b| b.id == f.mismatch).unwrap();
    mismatch.ops.clear();
    // The mismatch edge proves Z clear. Gt/Ge share C set, Lt/Le C clear.
    mismatch.terminator = MirTerminator::Branch {
        cond: MirCond::FlagTest(if matches!(op, MirCompareOp::Gt | MirCompareOp::Ge) {
            MirFlagTest::CSet
        } else {
            MirFlagTest::CClear
        }),
        then_edge: MirEdge {
            target: one,
            args: vec![],
        },
        else_edge: MirEdge {
            target: zero,
            args: vec![],
        },
    };
    for (id, value) in [(one, 1), (zero, 0)] {
        r.blocks.push(MirBlock {
            id,
            label: format!("comparison_result_{}", id.0),
            params: vec![],
            ops: vec![MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
                src: MirValue::ConstU8(value),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Return,
        });
    }
}
