//! Bounded comparison-loop facts. This does not change the counted-loop contract.
use std::collections::BTreeSet;

use super::effects::classify_op;
use super::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::ir::*;
use crate::mir6502::rewrite::context::{MirProof, PreHomeRewriteContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum ComparisonLoopBlocker {
    Shape,
    Entry,
    Index,
    BaseFlow,
    Source,
    Effects,
    Escape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct ComparisonOperand {
    pub source: MirMem,
    pub displacement: u16,
    pub construction: BTreeSet<usize>,
    pub load: usize,
    pub capture: MirMem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct ComparisonLoop {
    pub generation: MirRoutineGeneration,
    pub preheader: MirBlockId,
    pub header: MirBlockId,
    pub body: MirBlockId,
    pub latch: MirBlockId,
    pub exits: BTreeSet<MirBlockId>,
    pub mismatch: MirBlockId,
    pub complete: MirBlockId,
    pub induction: MirMem,
    pub initial: u8,
    pub end: u8,
    pub left: ComparisonOperand,
    pub right: ComparisonOperand,
}

fn block(r: &MirRoutine, id: MirBlockId) -> Option<&MirBlock> {
    r.blocks.iter().find(|b| b.id == id)
}
fn site(b: MirBlockId, i: usize) -> MirSite {
    MirSite::Op {
        block: b,
        op_index: i,
    }
}
fn ordinary(r: &MirRoutine, mem: &MirMem, width: u16) -> bool {
    let (slots, base, offset) = match mem {
        MirMem::Local { id, offset } => (&r.frame.locals, MirStorageBase::Local(*id), *offset),
        MirMem::Param { id, offset } => (&r.frame.params, MirStorageBase::Param(*id), *offset),
        _ => return false,
    };
    slots.iter().any(|s| {
        s.base == base
            && offset
                .checked_add(width)
                .is_some_and(|end| end <= s.storage_size)
    })
}
pub(in crate::mir6502) fn same_object(a: &MirMem, b: &MirMem) -> bool {
    match (a, b) {
        (MirMem::Local { id: a, .. }, MirMem::Local { id: b, .. }) => a == b,
        (MirMem::Param { id: a, .. }, MirMem::Param { id: b, .. }) => a == b,
        _ => a == b,
    }
}

fn word_base(
    r: &MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    body: &MirBlock,
    value: &MirValue,
    usage: usize,
    chain: &mut BTreeSet<usize>,
    depth: usize,
) -> Result<(MirMem, u16), ComparisonLoopBlocker> {
    if depth > 8 {
        return Err(ComparisonLoopBlocker::BaseFlow);
    }
    let MirValue::Def(MirDef::VTemp(temp)) = value else {
        return Err(ComparisonLoopBlocker::BaseFlow);
    };
    let MirProof::Proven(defs) = c.exclusive_word_temp_definition(*temp, site(body.id, usage))
    else {
        return Err(ComparisonLoopBlocker::BaseFlow);
    };
    let MirSite::Op { block, op_index } = defs[0].site else {
        return Err(ComparisonLoopBlocker::BaseFlow);
    };
    if block != body.id || op_index >= usage || !chain.insert(op_index) {
        return Err(ComparisonLoopBlocker::BaseFlow);
    }
    match &body.ops[op_index] {
        MirOp::Load {
            src: MirAddr::Direct(mem),
            width: MirWidth::Word,
            ..
        } if ordinary(r, mem, 2) => Ok((mem.clone(), 0)),
        MirOp::Move {
            src,
            width: MirWidth::Word,
            ..
        } => word_base(r, c, body, src, op_index, chain, depth + 1),
        MirOp::Binary {
            op: MirBinaryOp::Add,
            left,
            right: MirValue::ConstU16(offset),
            width: MirWidth::Word,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
            ..
        } => {
            let (mem, base) = word_base(r, c, body, left, op_index, chain, depth + 1)?;
            Ok((mem, base.wrapping_add(*offset)))
        }
        _ => Err(ComparisonLoopBlocker::Source),
    }
}

fn operand(
    r: &MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    b: &MirBlock,
    i: usize,
    induction: &MirMem,
) -> Result<ComparisonOperand, ComparisonLoopBlocker> {
    let Some(MirOp::Load {
        dst: MirDef::VTemp(temp),
        src:
            MirAddr::ComputedIndex {
                base,
                index,
                elem_size: 1,
                offset,
            },
        width: MirWidth::Byte,
    }) = b.ops.get(i)
    else {
        return Err(ComparisonLoopBlocker::Shape);
    };
    let MirValue::Def(MirDef::VTemp(idx)) = index else {
        return Err(ComparisonLoopBlocker::Index);
    };
    let index_site = (0..i)
        .find(|&n| {
            matches!(&b.ops[n],MirOp::Load{dst:MirDef::VTemp(t),
        src:MirAddr::Direct(mem),width:MirWidth::Byte} if t==idx && mem==induction)
        })
        .ok_or(ComparisonLoopBlocker::Index)?;
    if !c
        .exclusive_byte_temp_flow(*idx, site(b.id, index_site), site(b.id, i))
        .is_proven()
    {
        return Err(ComparisonLoopBlocker::Index);
    }
    let mut construction = BTreeSet::from([index_site]);
    let (source, displacement) = word_base(r, c, b, base, i, &mut construction, 0)?;
    let Some(MirOp::Store {
        dst: MirAddr::Direct(capture),
        src: MirValue::Def(MirDef::VTemp(t)),
        width: MirWidth::Byte,
    }) = b.ops.get(i + 1)
    else {
        return Err(ComparisonLoopBlocker::Shape);
    };
    if t != temp
        || !ordinary(r, capture, 1)
        || !c
            .exclusive_byte_temp_flow(*temp, site(b.id, i), site(b.id, i + 1))
            .is_proven()
    {
        return Err(ComparisonLoopBlocker::BaseFlow);
    }
    Ok(ComparisonOperand {
        source,
        displacement: displacement.wrapping_add(*offset),
        construction,
        load: i,
        capture: capture.clone(),
    })
}

/// Admit one exact natural-loop profile with a normal completion and mismatch
/// exit. All edges are checked through the shared CFG/dominance snapshot.
pub(in crate::mir6502) fn prove(
    r: &MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    header: MirBlockId,
) -> Result<ComparisonLoop, ComparisonLoopBlocker> {
    use ComparisonLoopBlocker as E;
    let h = block(r, header).ok_or(E::Shape)?;
    let [
        MirOp::Load {
            dst: MirDef::VTemp(index),
            src: MirAddr::Direct(induction),
            width: MirWidth::Byte,
        },
        MirOp::Compare {
            dst: MirCondDest::Temp(cond),
            op,
            left: MirValue::Def(MirDef::VTemp(idx)),
            right: MirValue::ConstU8(bound),
            width: MirWidth::Byte,
            signed: false,
        },
    ] = h.ops.as_slice()
    else {
        return Err(E::Shape);
    };
    if index != idx || !ordinary(r, induction, 1) {
        return Err(E::Index);
    }
    let end = match op {
        MirCompareOp::Le => bound.checked_add(1),
        MirCompareOp::Lt => Some(*bound),
        _ => None,
    }
    .ok_or(E::Index)?;
    let MirTerminator::Branch {
        cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(t))),
        then_edge,
        else_edge,
    } = &h.terminator
    else {
        return Err(E::Shape);
    };
    if t != cond || !then_edge.args.is_empty() || !else_edge.args.is_empty() {
        return Err(E::Shape);
    }
    let b = block(r, then_edge.target).ok_or(E::Shape)?;
    let MirTerminator::Branch {
        then_edge: unequal,
        else_edge: equal,
        ..
    } = &b.terminator
    else {
        return Err(E::Shape);
    };
    let l = block(r, equal.target).ok_or(E::Shape)?;
    if !matches!(&l.terminator,MirTerminator::Jump(e) if e.target==header && e.args.is_empty()) {
        return Err(E::Shape);
    }
    let preds = c.cfg().predecessors(header);
    if preds.len() != 2 || !preds.contains(&l.id) {
        return Err(E::Entry);
    }
    let preheader = *preds.iter().find(|&&p| p != l.id).ok_or(E::Entry)?;
    let p = block(r, preheader).ok_or(E::Entry)?;
    if !matches!(&p.terminator,MirTerminator::Jump(e) if e.target==header && e.args.is_empty())
        || c.cfg().predecessors(b.id) != &BTreeSet::from([header])
        || c.cfg().predecessors(l.id) != &BTreeSet::from([b.id])
        || !c.block_dominates(header, b.id)
        || !c.block_dominates(header, l.id)
    {
        return Err(E::Entry);
    }
    let initial = match p.ops.last() {
        Some(MirOp::Store {
            dst: MirAddr::Direct(mem),
            src: MirValue::ConstU8(n),
            width: MirWidth::Byte,
        }) if mem == induction => *n,
        _ => return Err(E::Index),
    };
    if initial >= end {
        return Err(E::Index);
    }
    let valid_latch = matches!(l.ops.as_slice(),[
        MirOp::Load{dst:MirDef::VTemp(a),src:MirAddr::Direct(m),width:MirWidth::Byte},
        MirOp::Binary{op:MirBinaryOp::Add,dst:MirDef::VTemp(b),left:MirValue::Def(MirDef::VTemp(x)),
            right:MirValue::ConstU8(1),width:MirWidth::Byte,carry_in:None,carry_out:MirCarryOut::Ignore},
        MirOp::Store{dst:MirAddr::Direct(n),src:MirValue::Def(MirDef::VTemp(y)),width:MirWidth::Byte}
    ] if m==induction && n==induction && a==x && b==y)
        || matches!(l.ops.as_slice(),[MirOp::UpdateMem{op:MirUpdateOp::Inc,mem,width:MirWidth::Byte}] if mem==induction);
    if !valid_latch {
        return Err(E::Index);
    }
    let loads: Vec<_> = b
        .ops
        .iter()
        .enumerate()
        .filter_map(|(i, op)| {
            matches!(
                op,
                MirOp::Load {
                    src: MirAddr::ComputedIndex { .. },
                    width: MirWidth::Byte,
                    ..
                }
            )
            .then_some(i)
        })
        .collect();
    let [left_site, right_site] = loads.as_slice() else {
        return Err(E::Shape);
    };
    let left = operand(r, c, b, *left_site, induction)?;
    let right = operand(r, c, b, *right_site, induction)?;
    if same_object(&left.source, &left.capture)
        || same_object(&right.source, &right.capture)
        || same_object(&left.source, &right.capture)
        || same_object(&right.source, &left.capture)
        || same_object(induction, &left.capture)
        || same_object(induction, &right.capture)
        || left.capture == right.capture
    {
        return Err(E::Source);
    }
    let mut allowed = left
        .construction
        .union(&right.construction)
        .copied()
        .collect::<BTreeSet<_>>();
    allowed.extend([left.load, left.load + 1, right.load, right.load + 1]);
    for (i, op) in b.ops.iter().enumerate() {
        if allowed.contains(&i) {
            continue;
        }
        match op {
            MirOp::Load {
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
                ..
            } if mem == &left.capture || mem == &right.capture => {}
            MirOp::Compare {
                op: MirCompareOp::Ne,
                width: MirWidth::Byte,
                signed: false,
                ..
            } if i + 1 == b.ops.len() => {}
            _ => return Err(E::Effects),
        }
    }
    let [
        MirOp::Load {
            dst: MirDef::VTemp(a),
            src: MirAddr::Direct(a_home),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::VTemp(z),
            src: MirAddr::Direct(z_home),
            width: MirWidth::Byte,
        },
        MirOp::Compare {
            dst: MirCondDest::Temp(condition),
            op: MirCompareOp::Ne,
            left: MirValue::Def(MirDef::VTemp(x)),
            right: MirValue::Def(MirDef::VTemp(y)),
            width: MirWidth::Byte,
            signed: false,
        },
    ] = &b.ops[b.ops.len().saturating_sub(3)..]
    else {
        return Err(E::Shape);
    };
    if a_home != &left.capture
        || z_home != &right.capture
        || a != x
        || z != y
        || !c
            .exclusive_byte_temp_flow(*a, site(b.id, b.ops.len() - 3), site(b.id, b.ops.len() - 1))
            .is_proven()
        || !c
            .exclusive_byte_temp_flow(*z, site(b.id, b.ops.len() - 2), site(b.id, b.ops.len() - 1))
            .is_proven()
    {
        return Err(E::BaseFlow);
    }
    if !matches!(&b.terminator,MirTerminator::Branch{cond:MirCond::BoolValue(MirValue::Def(MirDef::VTemp(t))),..} if t==condition)
        || !unequal.args.is_empty()
        || !equal.args.is_empty()
        || [header, b.id, l.id].contains(&unequal.target)
        || [header, b.id, l.id].contains(&else_edge.target)
    {
        return Err(E::Shape);
    }
    for op in r.blocks.iter().flat_map(|b| &b.ops) {
        if classify_op(op).memory.address_references.iter().any(|mem| {
            [&left.source, &right.source, induction]
                .iter()
                .any(|cell| same_object(mem, cell))
        }) {
            return Err(E::Escape);
        }
    }
    Ok(ComparisonLoop {
        generation: c.generation(),
        preheader,
        header,
        body: b.id,
        latch: l.id,
        exits: BTreeSet::from([unequal.target, else_edge.target]),
        mismatch: unequal.target,
        complete: else_edge.target,
        induction: induction.clone(),
        initial,
        end,
        left,
        right,
    })
}

#[cfg(test)]
pub(in crate::mir6502) mod tests {
    use super::*;
    use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
    pub fn program() -> MirProgram {
        let source = include_str!("../../../surveys/tn/reproducers/comparison-loops.act")
            .replace("\r\n", "\n");
        let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        crate::mir6502::lower_program(&crate::nir::optimize_program(&nir).unwrap()).unwrap()
    }
    pub fn facts(r: &MirRoutine) -> Vec<ComparisonLoop> {
        let s = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        let c = PreHomeRewriteContext::new(&s);
        r.blocks
            .iter()
            .filter_map(|b| c.comparison_loop(r, b.id).ok())
            .collect()
    }
    #[test]
    fn comparison_loops_prove_both_early_return_regions_and_complete_bases() {
        let p = program();
        assert_eq!(facts(&p.routines[0]).len(), 2);
        for (r, count) in p.routines.iter().zip([2, 1, 1, 1, 0]) {
            assert_eq!(facts(r).len(), count, "{}", r.name)
        }
        let f = facts(&p.routines[0]);
        assert_eq!((f[0].left.displacement, f[1].left.displacement), (6, 17));
        assert_eq!(f[0].exits.len(), 2);
    }
    #[test]
    fn comparison_loops_reject_foreign_entries_and_effectful_bodies() {
        let p = program();
        let r = &p.routines[1];
        let f = facts(r).remove(0);
        let mut changed = r.clone();
        changed.blocks[0].terminator = MirTerminator::Jump(MirEdge {
            target: f.body,
            args: vec![],
        });
        assert!(facts(&changed).is_empty());
        let mut changed = r.clone();
        changed
            .blocks
            .iter_mut()
            .find(|b| b.id == f.body)
            .unwrap()
            .ops
            .push(MirOp::Barrier {
                effects: MirEffects::default(),
            });
        assert!(facts(&changed).is_empty());
    }
    #[test]
    fn comparison_loops_reject_incomplete_base_and_index_wrap() {
        let p = program();
        let r = &p.routines[1];
        let f = facts(r).remove(0);
        let mut changed = r.clone();
        let b = changed.blocks.iter_mut().find(|b| b.id == f.body).unwrap();
        let n = *f
            .left
            .construction
            .iter()
            .find(|&&i| {
                matches!(
                    b.ops[i],
                    MirOp::Load {
                        width: MirWidth::Word,
                        ..
                    }
                )
            })
            .unwrap();
        if let MirOp::Load { width, .. } = &mut b.ops[n] {
            *width = MirWidth::Byte
        }
        assert!(facts(&changed).is_empty());
        let mut changed = r.clone();
        if let MirOp::Compare { right, .. } = &mut changed
            .blocks
            .iter_mut()
            .find(|b| b.id == f.header)
            .unwrap()
            .ops[1]
        {
            *right = MirValue::ConstU8(255)
        }
        assert!(facts(&changed).is_empty());
    }
    #[test]
    fn comparison_loops_reject_zero_trip_extra_uses_and_wrong_character_pair() {
        let p = program();
        let r = &p.routines[1];
        let f = facts(r).remove(0);
        let mut changed = r.clone();
        if let MirOp::Store { src, .. } = changed
            .blocks
            .iter_mut()
            .find(|b| b.id == f.preheader)
            .unwrap()
            .ops
            .last_mut()
            .unwrap()
        {
            *src = MirValue::ConstU8(f.end);
        }
        assert!(facts(&changed).is_empty());
        let mut changed = r.clone();
        let body = changed.blocks.iter_mut().find(|b| b.id == f.body).unwrap();
        if let MirOp::Compare { right, .. } = body.ops.last_mut().unwrap() {
            *right = MirValue::ConstU8(0)
        }
        assert!(facts(&changed).is_empty());
        let mut changed = r.clone();
        let body = changed.blocks.iter_mut().find(|b| b.id == f.body).unwrap();
        let index = *f
            .left
            .construction
            .iter()
            .find(|&&i| {
                matches!(
                    body.ops[i],
                    MirOp::Load {
                        width: MirWidth::Byte,
                        ..
                    }
                )
            })
            .unwrap();
        let extra = body.ops[index].clone();
        body.ops.insert(index + 1, extra);
        assert!(facts(&changed).is_empty());
    }
    #[test]
    fn comparison_loops_reject_source_writes_and_absolute_backing() {
        let p = program();
        let r = &p.routines[1];
        let f = facts(r).remove(0);
        let mut changed = r.clone();
        changed
            .blocks
            .iter_mut()
            .find(|b| b.id == f.body)
            .unwrap()
            .ops
            .insert(
                0,
                MirOp::Store {
                    dst: MirAddr::Direct(f.left.source.clone()),
                    src: MirValue::ConstU8(0),
                    width: MirWidth::Byte,
                },
            );
        assert!(facts(&changed).is_empty());
        let mut changed = r.clone();
        changed.frame.params[0].base = MirStorageBase::Absolute(0xD000);
        assert!(facts(&changed).is_empty());
    }
}
