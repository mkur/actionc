//! Bounded Y induction through the shared private-cell and machine proofs.
use super::*;
use crate::mir6502::analysis::{comparison_loops::ComparisonLoop, sites::MirSite};
use crate::mir6502::rewrite::context::PreHomeRewriteContext;

pub(super) fn prove(
    r: &super::super::ir::MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    f: &ComparisonLoop,
) -> bool {
    let Some(header) = r.blocks.iter().find(|b| b.id == f.header) else {
        return false;
    };
    let Some(pre) = r.blocks.iter().find(|b| b.id == f.preheader) else {
        return false;
    };
    let Some(latch) = r.blocks.iter().find(|b| b.id == f.latch) else {
        return false;
    };
    let [
        MirOp::Load {
            dst: MirDef::VTemp(index),
            ..
        },
        MirOp::Compare {
            dst: MirCondDest::Temp(condition),
            ..
        },
    ] = header.ops.as_slice()
    else {
        return false;
    };
    if !c
        .exclusive_byte_temp_flow(
            *index,
            MirSite::Op {
                block: f.header,
                op_index: 0,
            },
            MirSite::Op {
                block: f.header,
                op_index: 1,
            },
        )
        .is_proven()
        || !c
            .exclusive_byte_temp_flow(
                *condition,
                MirSite::Op {
                    block: f.header,
                    op_index: 1,
                },
                MirSite::Terminator { block: f.header },
            )
            .is_proven()
        || !c
            .register_dead_after(MirReg::A, c.point(MirSite::Terminator { block: f.header }))
            .is_proven()
    {
        return false;
    }
    let mut allowed = BTreeSet::from([MirSite::Op {
        block: f.header,
        op_index: 0,
    }]);
    let body = r.blocks.iter().find(|b| b.id == f.body).unwrap();
    for &i in f.left.construction.union(&f.right.construction) {
        if matches!(&body.ops[i],MirOp::Load{src:MirAddr::Direct(mem),width:MirWidth::Byte,..} if mem==&f.induction)
        {
            allowed.insert(MirSite::Op {
                block: f.body,
                op_index: i,
            });
        }
    }
    if let [
        MirOp::Load {
            dst: MirDef::VTemp(a),
            ..
        },
        MirOp::Binary {
            dst: MirDef::VTemp(b),
            ..
        },
        MirOp::Store { .. },
    ] = latch.ops.as_slice()
    {
        for (temp, def, usage) in [(*a, 0, 1), (*b, 1, 2)] {
            if !c
                .exclusive_byte_temp_flow(
                    temp,
                    MirSite::Op {
                        block: f.latch,
                        op_index: def,
                    },
                    MirSite::Op {
                        block: f.latch,
                        op_index: usage,
                    },
                )
                .is_proven()
            {
                return false;
            }
        }
        allowed.insert(MirSite::Op {
            block: f.latch,
            op_index: 0,
        });
    } else {
        allowed.insert(MirSite::Op {
            block: f.latch,
            op_index: 0,
        });
    }
    c.private_cell_reads_confined(
        r,
        &f.induction,
        MirSite::Op {
            block: f.preheader,
            op_index: pre.ops.len() - 1,
        },
        &allowed,
    )
}

pub(super) fn apply(
    r: &mut super::super::ir::MirRoutine,
    f: &ComparisonLoop,
    initial_store: usize,
) {
    let pre = r.blocks.iter_mut().find(|b| b.id == f.preheader).unwrap();
    pre.ops.remove(initial_store);
    pre.ops.push(MirOp::LoadImm {
        dst: MirDef::Reg(MirReg::Y),
        value: u16::from(f.initial),
        width: MirWidth::Byte,
    });
    let body = r.blocks.iter_mut().find(|b| b.id == f.body).unwrap();
    body.ops.retain(|op|!matches!(op,MirOp::Move{dst:MirDef::Reg(MirReg::Y),src:MirValue::PointerCell(mem),width:MirWidth::Byte} if mem==&f.induction));
    let header = r.blocks.iter_mut().find(|b| b.id == f.header).unwrap();
    let MirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &header.terminator
    else {
        unreachable!()
    };
    let back = then_edge.clone();
    let complete = else_edge.clone();
    // The admitted initial value is strictly inside the interval. Enter the
    // body directly; the latch checks every subsequent iteration and completion.
    header.ops.clear();
    header.terminator = MirTerminator::Jump(back.clone());
    let latch = r.blocks.iter_mut().find(|b| b.id == f.latch).unwrap();
    latch.ops = vec![
        MirOp::UpdateReg {
            op: MirUpdateOp::Inc,
            reg: MirReg::Y,
        },
        MirOp::Compare {
            dst: MirCondDest::Flags,
            op: MirCompareOp::Lt,
            left: MirValue::Def(MirDef::Reg(MirReg::Y)),
            right: MirValue::ConstU8(f.end),
            width: MirWidth::Byte,
            signed: false,
        },
    ];
    latch.terminator = MirTerminator::Branch {
        cond: MirCond::FlagTest(MirFlagTest::CClear),
        then_edge: back,
        else_edge: complete,
    };
}
