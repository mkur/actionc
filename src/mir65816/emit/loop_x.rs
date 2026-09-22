//! One checked loop-parameter mirror. Memory allocation and all stores remain
//! authoritative; there is no register home or relaxed interference rule.
use super::*;
use crate::nir::{NirIntegerRole, NirTypeKind};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) struct LoopXPlan {
    pub header: BlockId,
    pub body: BlockId,
    pub preheader: BlockId,
    pub param: TempId,
    pub condition: TempId,
    pub increment: Option<TempId>,
    pub home: Location,
    pub threshold: u16,
}

fn immediate(v: &Mir65816Value) -> Option<u16> {
    match v {
        Mir65816Value::U8(v) => Some((*v).into()),
        Mir65816Value::U16(v) => Some(*v),
        _ => None,
    }
}
fn temp(v: &Mir65816Value) -> Option<TempId> {
    match v {
        Mir65816Value::Temp(id, w) if w.get() == 2 => Some(*id),
        _ => None,
    }
}
fn inputs(op: &Mir65816Op) -> Vec<TempId> {
    match op {
        Mir65816Op::Binary { left, right, .. } | Mir65816Op::Compare { left, right, .. } => {
            [temp(left), temp(right)].into_iter().flatten().collect()
        }
        Mir65816Op::Store { value, .. } => temp(value).into_iter().collect(),
        Mir65816Op::Load { .. } => vec![],
        _ => unreachable!("closed scalar admission"),
    }
}
fn output(op: &Mir65816Op) -> Option<TempId> {
    match op {
        Mir65816Op::Binary { dest, .. }
        | Mir65816Op::Compare { dest, .. }
        | Mir65816Op::Load { dest, .. } => Some(*dest),
        _ => None,
    }
}
fn edges(b: &Mir65816Block) -> Vec<&Mir65816Edge> {
    match &b.terminator {
        Mir65816Terminator::Goto(e) => vec![e],
        Mir65816Terminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![else_edge, then_edge],
        _ => vec![],
    }
}
fn unsigned(r: &Mir65816Routine, id: TempId) -> bool {
    r.temps.iter().any(|(t,ty)| *t==id && !ty.pointer && ty.width==Some(ByteSize::new(2)) &&
        matches!(ty.kind,NirTypeKind::Integer(i) if i.bits==16 && !i.signed && i.role==NirIntegerRole::Ordinary))
}

impl LoopXPlan {
    pub fn new(r: &Mir65816Routine, frame: &AllocatedFrame) -> Result<Option<Self>, String> {
        if !scalar::admitted(r)
            || !r
                .temps
                .iter()
                .any(|(id, _)| matches!(frame.temps[id], Location::DirectPage(_)))
        {
            return Ok(None);
        }
        // Pointer-leaf allocation cannot satisfy this dedicated verifier.
        frame.verify_scalar_dp(r)?;
        let byid: BTreeMap<_, _> = r.blocks.iter().map(|b| (b.id, b)).collect();
        let mut successors = BTreeMap::new();
        let mut predecessors: BTreeMap<_, Vec<_>> =
            r.blocks.iter().map(|b| (b.id, vec![])).collect();
        for (i, b) in r.blocks.iter().enumerate() {
            let next = if matches!(b.terminator, Mir65816Terminator::Fallthrough) {
                vec![r.blocks.get(i + 1).ok_or("unresolved fallthrough")?.id]
            } else {
                edges(b).iter().map(|e| e.target).collect()
            };
            for &n in &next {
                predecessors
                    .get_mut(&n)
                    .ok_or("unknown X-plan successor")?
                    .push(b.id);
            }
            successors.insert(b.id, next);
        }
        let mut reachable = BTreeSet::new();
        let mut pending = vec![r.blocks[0].id];
        while let Some(id) = pending.pop() {
            if reachable.insert(id) {
                pending.extend(&successors[&id]);
            }
        }
        let sole = liveness::sole_branch_conditions(r);
        for (&header, h) in &byid {
            if !reachable.contains(&header) || h.ops.len() != 1 {
                continue;
            }
            let Mir65816Op::Compare {
                dest: condition,
                width,
                signed: false,
                operation,
                left,
                right,
            } = &h.ops[0]
            else {
                continue;
            };
            let Some(param) = temp(left) else {
                continue;
            };
            if width.get() != 2
                || !unsigned(r, param)
                || h.params.last().map(|p| p.0) != Some(param)
                || !sole.contains(condition)
            {
                continue;
            }
            let Some(k) = immediate(right) else {
                continue;
            };
            let threshold = match operation {
                NirCompareOp::Lt => k,
                NirCompareOp::Le => {
                    let Some(v) = k.checked_add(1) else {
                        continue;
                    };
                    v
                }
                _ => continue,
            };
            let Mir65816Terminator::Branch {
                condition: Mir65816Value::Temp(c, _),
                then_edge,
                else_edge,
            } = &h.terminator
            else {
                continue;
            };
            if c != condition
                || !then_edge.args.is_empty()
                || !else_edge.args.is_empty()
                || then_edge.target == else_edge.target
            {
                continue;
            }
            let bodies:Vec<_>=[then_edge.target,else_edge.target].into_iter().filter(|id|matches!(&byid[id].terminator,Mir65816Terminator::Goto(e) if e.target==header)).collect();
            if bodies.len() != 1 {
                continue;
            }
            let body = bodies[0];
            let b = byid[&body];
            if body == header || !b.params.is_empty() || predecessors[&body] != [header] {
                continue;
            }
            let incoming = &predecessors[&header];
            if incoming.len() != 2 || incoming.iter().filter(|&&id| id == body).count() != 1 {
                continue;
            }
            let preheader = *incoming.iter().find(|&&id| id != body).unwrap();
            if preheader == header || !reachable.contains(&preheader) {
                continue;
            }
            if !matches!(&byid[&preheader].terminator,Mir65816Terminator::Goto(e) if e.target==header)
            {
                continue;
            }
            // Removing the sole admitted backedge must leave a DAG, including
            // unreachable blocks. Thus no second/nested cycle is hidden here.
            let mut degrees: BTreeMap<_, usize> = r.blocks.iter().map(|b| (b.id, 0)).collect();
            for (&a, ns) in &successors {
                for &n in ns {
                    if (a, n) != (body, header) {
                        *degrees.get_mut(&n).unwrap() += 1;
                    }
                }
            }
            let mut queue: Vec<_> = degrees
                .iter()
                .filter_map(|(&id, &n)| (n == 0).then_some(id))
                .collect();
            let mut visited = 0;
            while let Some(id) = queue.pop() {
                visited += 1;
                for &n in &successors[&id] {
                    if (id, n) == (body, header) {
                        continue;
                    }
                    let count = degrees.get_mut(&n).unwrap();
                    *count -= 1;
                    if *count == 0 {
                        queue.push(n);
                    }
                }
            }
            if visited != r.blocks.len() {
                continue;
            }
            let Mir65816Terminator::Goto(back) = &b.terminator else {
                unreachable!()
            };
            let Some(update) = back.args.last().and_then(temp) else {
                continue;
            };
            if !unsigned(r, update) {
                continue;
            }
            let updates: Vec<_> = b
                .ops
                .iter()
                .enumerate()
                .filter_map(|(i, op)| match op {
                    Mir65816Op::Binary {
                        dest,
                        width,
                        operation: NirBinaryOp::Add,
                        left,
                        right,
                        signed: false,
                    } if *dest == update
                        && width.get() == 2
                        && temp(left) == Some(param)
                        && immediate(right) == Some(1) =>
                    {
                        Some(i)
                    }
                    _ => None,
                })
                .collect();
            if updates.len() != 1 {
                continue;
            }
            let uses: Vec<_> = r
                .blocks
                .iter()
                .flat_map(|b| {
                    b.ops.iter().enumerate().flat_map(move |(i, op)| {
                        inputs(op).into_iter().map(move |id| (b.id, i, id))
                    })
                })
                .filter(|v| v.2 == param)
                .collect();
            if uses.len() != 2
                || !uses.contains(&(header, 0, param))
                || !uses.contains(&(body, updates[0], param))
            {
                continue;
            }
            let term_use = r.blocks.iter().any(|b| {
                let direct = match &b.terminator {
                    Mir65816Terminator::Return { value, .. } => {
                        value.as_ref().and_then(temp) == Some(param)
                    }
                    Mir65816Terminator::Branch { condition, .. } => temp(condition) == Some(param),
                    _ => false,
                };
                direct
                    || edges(b)
                        .iter()
                        .any(|e| e.args.iter().any(|v| temp(v) == Some(param)))
            });
            if term_use {
                continue;
            }
            let home = frame.temps[&param];
            if !matches!(home,Location::DirectPage(s) if s.width==2 && scalar::word_offset(s.offset))
            {
                continue;
            }
            if [h, b]
                .iter()
                .flat_map(|b| &b.ops)
                .filter_map(output)
                .any(|id| frame.temps[&id].overlaps(home))
            {
                continue;
            }
            let graph = liveness::interference(r)?;
            if !graph[&param].contains(&update) || frame.temps[&update].overlaps(home) {
                return Err("invalid closed X-update interference".into());
            }
            let mut native = true;
            for id in [preheader, body] {
                let Mir65816Terminator::Goto(e) = &byid[&id].terminator else {
                    unreachable!()
                };
                let Some(copies) = frame.word_copies(r, e, 0)? else {
                    native = false;
                    break;
                };
                frame.word_staging(&copies, 0)?;
                if copies.moves.last().map(|m| Location::from(m.1)) != Some(home) {
                    native = false;
                    break;
                }
            }
            if !native {
                continue;
            }
            // Header saves four cycles; refresh costs two. Each body TXA saves
            // two more, paying its own backedge refresh. Zero trips save two.
            // The two TAX bytes fit within the header's two-byte reduction;
            // TXA can only reduce size further. The bounded INX extension saves
            // three additional cycles and bytes when a TXA input was required.
            // Pending X/p equality may not cross internal compare dispatches.
            // Ordinary word loads/stores and ADD/SUB are straight-line and do
            // not consume incoming C/V. Keep the old mirror path otherwise.
            let increment = b.ops[updates[0] + 1..]
                .iter()
                .all(|op| !matches!(op, Mir65816Op::Compare { .. }))
                .then_some(update);
            return Ok(Some(Self {
                header,
                body,
                preheader,
                param,
                condition: *condition,
                increment,
                home,
                threshold,
            }));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn program(optimize: bool) -> Mir65816Program {
        crate::compiler::native65816::prepare_file(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "tools/native65816-runtime-tests/tests/fixtures/code_quality/loop_rotation.act",
            ),
            optimize,
            &Default::default(),
        )
        .unwrap()
        .mir
    }
    fn plan(r: &Mir65816Routine) -> Option<LoopXPlan> {
        LoopXPlan::new(r, &AllocatedFrame::new(r).unwrap()).unwrap()
    }
    #[test]
    fn pending_internal_dispatch_keeps_the_existing_x_mirror_fallback() {
        let p = program(true);
        let mut r = p
            .routines
            .iter()
            .find(|r| plan(r).is_some())
            .unwrap()
            .clone();
        let x = plan(&r).unwrap();
        let q = x.increment.unwrap();
        let mut cmp = r.blocks.iter().find(|b| b.id == x.header).unwrap().ops[0].clone();
        let id = TempId(r.temps.iter().map(|v| v.0.0).max().unwrap() + 1);
        let ty = r
            .temps
            .iter()
            .find(|v| v.0 == x.condition)
            .unwrap()
            .1
            .clone();
        r.temps.push((id, ty));
        let Mir65816Op::Compare {
            dest, left, right, ..
        } = &mut cmp
        else {
            panic!()
        };
        *dest = id;
        *left = Mir65816Value::U16(0);
        *right = Mir65816Value::U16(1);
        r.blocks
            .iter_mut()
            .find(|b| b.id == x.body)
            .unwrap()
            .ops
            .push(cmp);
        assert!(plan(&r).unwrap().increment.is_none());
        let m = select::routine(&r, false).unwrap();
        let i = r
            .blocks
            .iter()
            .find(|b| b.id == x.body)
            .unwrap()
            .ops
            .iter()
            .position(|op| matches!(op,Mir65816Op::Binary{dest,..} if *dest==q))
            .unwrap();
        let span = &m.code.mir_spans[&(x.body, i)];
        assert_eq!(
            &m.code.bytes[span.start..span.start + 5],
            &[0x8a, 0x18, 0x69, 1, 0]
        );
    }
    #[test]
    fn loop_admission_is_typed_bounded_and_preserves_interference() {
        assert!(program(false).routines.iter().all(|r| plan(r).is_none()));
        let p = program(true);
        let r = p.routines.iter().find(|r| plan(r).is_some()).unwrap();
        let x = plan(r).unwrap();
        assert_eq!(x.threshold, 8);
        let b = r.blocks.iter().find(|b| b.id == x.body).unwrap();
        let Mir65816Terminator::Goto(e) = &b.terminator else {
            panic!()
        };
        let q = temp(e.args.last().unwrap()).unwrap();
        let frame = AllocatedFrame::new(r).unwrap();
        assert!(liveness::interference(r).unwrap()[&x.param].contains(&q));
        assert!(!frame.temps[&q].overlaps(x.home));
        for le in [false, true] {
            for k in [0, 1, 0x7fff, 0x8000, 0xfffe, 0xffff] {
                let mut r = r.clone();
                let Mir65816Op::Compare {
                    operation, right, ..
                } = &mut r.blocks.iter_mut().find(|b| b.id == x.header).unwrap().ops[0]
                else {
                    panic!()
                };
                *operation = if le {
                    NirCompareOp::Le
                } else {
                    NirCompareOp::Lt
                };
                *right = Mir65816Value::U16(k);
                let selected = plan(&r);
                if le && k == 0xffff {
                    assert!(selected.is_none());
                    continue;
                }
                let threshold = selected.unwrap().threshold;
                for value in 0..=u16::MAX {
                    assert_eq!(value < threshold, if le { value <= k } else { value < k });
                }
            }
        }
        for variant in 0..7 {
            let mut r = r.clone();
            let h = r.blocks.iter_mut().find(|b| b.id == x.header).unwrap();
            let Mir65816Op::Compare {
                signed,
                operation,
                left,
                right,
                ..
            } = &mut h.ops[0]
            else {
                panic!()
            };
            match variant {
                0 => *signed = true,
                1 => *operation = NirCompareOp::Eq,
                2 => *right = left.clone(),
                3 => std::mem::swap(left, right),
                4 => {
                    let last = h.params.len() - 1;
                    h.params.swap(0, last);
                }
                5 => {
                    let b = r.blocks.iter_mut().find(|b| b.id == x.body).unwrap();
                    let op = b
                        .ops
                        .iter_mut()
                        .find(|o| inputs(o).contains(&x.param))
                        .unwrap();
                    let Mir65816Op::Binary { right, .. } = op else {
                        panic!()
                    };
                    *right = Mir65816Value::U16(2);
                }
                6 => {
                    let b = r.blocks.iter_mut().find(|b| b.id == x.body).unwrap();
                    let Mir65816Op::Binary { operation, .. } = b
                        .ops
                        .iter_mut()
                        .find(|o| matches!(o, Mir65816Op::Binary { .. }))
                        .unwrap()
                    else {
                        panic!()
                    };
                    *operation = NirBinaryOp::Mul;
                }
                _ => unreachable!(),
            }
            assert!(plan(&r).is_none(), "variant {variant}");
        }
    }
}
