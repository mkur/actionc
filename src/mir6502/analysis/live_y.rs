//! Must-initialization for complete bases and explicit live Y across CFG edges.
use super::{
    cfg::MirCfg,
    effects::{MirHomeByte, classify_op},
};
use crate::mir6502::ir::*;
use std::collections::{BTreeMap, BTreeSet};

fn consumers(op: &MirOp) -> Vec<MirAddressConsumer> {
    match op {
        MirOp::LoadIndirect { consumer, .. } => vec![*consumer],
        MirOp::CompareIndirectBytes { left, right, .. } => vec![*left, *right],
        _ => vec![],
    }
}
fn lanes(pair: MirPointerPair) -> [MirHomeByte; 2] {
    match pair {
        MirPointerPair::Fixed { lo } => [
            MirHomeByte::FixedZeroPage(lo),
            MirHomeByte::FixedZeroPage(MirFixedZpSlot(lo.0.wrapping_add(1))),
        ],
        MirPointerPair::Virtual(slot) => [
            MirHomeByte::VirtualZeroPage { slot, offset: 0 },
            MirHomeByte::VirtualZeroPage { slot, offset: 1 },
        ],
    }
}
fn transfer(op: &MirOp, pairs: &[MirPointerPair], ready: &mut BTreeSet<usize>) {
    let effects = classify_op(op);
    for (i, pair) in pairs.iter().enumerate() {
        if effects.homes.unknown_writes
            || lanes(*pair).iter().any(|lane| {
                effects.homes.writes.contains(lane) || effects.addresses.pair_writes.contains(lane)
            })
        {
            ready.remove(&i);
        }
    }
    if let MirOp::MaterializeAddress { consumer, .. }
    | MirOp::MaterializeIndexedAddress { consumer, .. } = op
    {
        if !consumer.preserves_index_in_y() {
            if let Some(i) = pairs.iter().position(|p| p == &consumer.pointer_pair()) {
                ready.insert(i);
            }
        }
    }
    let y = pairs.len();
    if effects.may_clobber_reg_compat(MirReg::Y) {
        let known = matches!(
            op,
            MirOp::Move {
                dst: MirDef::Reg(MirReg::Y),
                ..
            } | MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::Y),
                ..
            } | MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                ..
            }
        );
        let advance = matches!(op, MirOp::UpdateReg { reg: MirReg::Y, .. });
        if known && (!effects.machine.register_reads.y || ready.contains(&y)) {
            ready.insert(y);
        } else if !advance {
            ready.remove(&y);
        }
    }
}

pub(in crate::mir6502) fn invalid_uses(r: &MirRoutine) -> Vec<MirBlockId> {
    let mut pairs = vec![];
    for c in r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .flat_map(consumers)
        .filter(|c| c.uses_live_y())
    {
        if !pairs.contains(&c.pointer_pair()) {
            pairs.push(c.pointer_pair());
        }
    }
    if pairs.is_empty() {
        return vec![];
    }
    let Ok(cfg) = MirCfg::from_routine(r) else {
        return vec![];
    };
    let universe = (0..=pairs.len()).collect::<BTreeSet<_>>();
    let mut outputs = cfg
        .reachable()
        .iter()
        .map(|&id| (id, universe.clone()))
        .collect::<BTreeMap<_, _>>();
    let input = |id, outputs: &BTreeMap<MirBlockId, BTreeSet<usize>>| {
        if Some(id) == cfg.entry() {
            return BTreeSet::new();
        }
        cfg.predecessors(id)
            .iter()
            .filter_map(|p| outputs.get(p))
            .fold(universe.clone(), |s, p| {
                s.intersection(p).copied().collect()
            })
    };
    loop {
        let mut changed = false;
        for b in r.blocks.iter().filter(|b| cfg.reachable().contains(&b.id)) {
            let mut ready = input(b.id, &outputs);
            for op in &b.ops {
                transfer(op, &pairs, &mut ready)
            }
            if outputs.get(&b.id) != Some(&ready) {
                outputs.insert(b.id, ready);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut invalid = vec![];
    for b in r.blocks.iter().filter(|b| cfg.reachable().contains(&b.id)) {
        let mut ready = input(b.id, &outputs);
        for op in &b.ops {
            for c in consumers(op).into_iter().filter(|c| c.uses_live_y()) {
                let i = pairs.iter().position(|p| p == &c.pointer_pair()).unwrap();
                if !ready.contains(&i) || !ready.contains(&pairs.len()) {
                    invalid.push(b.id)
                }
            }
            transfer(op, &pairs, &mut ready);
        }
    }
    invalid.sort();
    invalid.dedup();
    invalid
}
