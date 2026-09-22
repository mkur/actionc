//! Independent typed multi-word edge identity and final-byte validation.
use super::*;
use actionc::mir65816::{emit::*, *};
use std::{collections::BTreeSet, ops::Range};

#[derive(Clone, Debug)]
pub struct Site {
    pub routine: actionc::nir::RoutineId,
    pub range: Range<u32>,
    pub load: u32,
    pub jump: u32,
    pub target: u32,
    pub fallthrough: bool,
    pub moves: Vec<((bool, u16), u8, u8)>,
    pub order: Vec<usize>,
    pub reload: Option<u8>,
    pub bytes: Vec<u8>,
}

fn source(v: &Mir65816Value, r: &Mir65816Routine, m: &MachineRoutine) -> Option<(bool, u16)> {
    match v {
        Mir65816Value::U16(v) => Some((false, *v)),
        Mir65816Value::Temp(id, w) if w.get() == 2 => match m.frame.temps[id] {
            Location::Stack(s) if s.width == 2 => Some((true, s.offset)),
            _ => None,
        },
        Mir65816Value::Param(id) => {
            let p = r.frame.parameters.iter().find(|p| p.param == *id).unwrap();
            let Mir65816AbiHome::StackArgument { offset, size, .. } = p.incoming else {
                panic!()
            };
            if size.get() != 2 {
                return None;
            }
            let at = if let Some(id) = p.frame_object {
                r.frame
                    .objects
                    .iter()
                    .find(|o| o.id == id)
                    .unwrap()
                    .stack_offset
                    .get()
            } else {
                abi::stack::incoming_displacement(
                    actionc::target::ByteSize::new(m.frame.extent.into()),
                    offset,
                    size,
                )
                .unwrap()
                .get()
            };
            Some((true, at.try_into().unwrap()))
        }
        _ => None,
    }
}
fn load(v: (bool, u16)) -> Vec<u8> {
    if v.0 {
        vec![0xa3, v.1.try_into().unwrap()]
    } else {
        vec![0xa9, v.1 as u8, (v.1 >> 8) as u8]
    }
}

/// Decode a direct schedule, then check the parallel-copy dependency rule.
/// No compiler scheduling decision is used as the expected result.
pub fn schedule(bytes: &[u8], moves: &[((bool, u16), u8, u8)], reload: bool) -> Option<Vec<usize>> {
    let mut order = vec![];
    let mut pc = 0;
    let mut pending: BTreeSet<_> = (0..moves.len()).collect();
    while !pending.is_empty() {
        let op = *bytes.get(pc)?;
        let n = if op == 0xa3 {
            2
        } else if op == 0xa9 {
            3
        } else {
            return None;
        };
        if bytes.get(pc + n) != Some(&0x83) {
            return None;
        }
        let dest = *bytes.get(pc + n + 1)?;
        let i = *pending.iter().find(|&&i| moves[i].2 == dest)?;
        if bytes.get(pc..pc + n)? != load(moves[i].0) {
            return None;
        }
        if pending
            .iter()
            .any(|&j| j != i && moves[j].0.0 && moves[j].0.1.abs_diff(dest.into()) < 2)
        {
            return None;
        }
        if moves
            .iter()
            .any(|m| m.0.0 && moves.iter().any(|d| m.0.1.abs_diff(d.2.into()) == 1))
        {
            return None;
        }
        pending.remove(&i);
        order.push(i);
        pc += n + 2;
    }
    let need = order.last().copied() != Some(moves.len() - 1);
    if need != reload {
        return None;
    }
    if reload {
        if bytes.get(pc..pc + 2)? != [0xa3, moves.last()?.2] {
            return None;
        }
        pc += 2;
    }
    (pc == bytes.len()).then_some(order)
}

pub fn index(
    p: &Mir65816Program,
    machine: &MachineProgram,
    address: impl Fn(actionc::nir::RoutineId) -> u32,
) -> Vec<Site> {
    let mut out = vec![];
    for m in &machine.routines {
        let r = p.routines.iter().find(|r| r.id == m.id).unwrap();
        let base = address(m.id);
        let mut transfers = m.code.mir_transfers.iter();
        for (bi, b) in r.blocks.iter().enumerate() {
            let edges = match &b.terminator {
                Mir65816Terminator::Goto(e) => vec![e.clone()],
                Mir65816Terminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![else_edge.clone(), then_edge.clone()],
                Mir65816Terminator::Fallthrough => vec![Mir65816Edge {
                    target: r.blocks[bi + 1].id,
                    args: vec![],
                }],
                _ => vec![],
            };
            for e in edges {
                let t = transfers.next().unwrap();
                let ti = r.blocks.iter().position(|b| b.id == e.target).unwrap();
                assert_eq!((t.source, t.target), (Label(bi as u32), Label(ti as u32)));
                let params = &r.blocks[ti].params;
                if params.len() < 2 || params.iter().any(|p| p.1.get() != 2) {
                    continue;
                }
                let moves: Option<Vec<_>> = e
                    .args
                    .iter()
                    .zip(params)
                    .enumerate()
                    .map(|(i, (v, (id, _)))| {
                        let Location::Stack(d) = m.frame.temps[id] else {
                            return None;
                        };
                        Some((
                            source(v, r, m)?,
                            m.frame.edge_copies[i].offset.try_into().unwrap(),
                            d.offset.try_into().unwrap(),
                        ))
                    })
                    .collect();
                let Some(moves) = moves else { continue };
                let mut staged = vec![];
                for &(s, stage, _) in &moves {
                    staged.extend(load(s));
                    staged.extend([0x83, stage]);
                }
                for &(_, stage, d) in &moves {
                    staged.extend([0xa3, stage, 0x83, d]);
                }
                let lo = m.code.labels[&t.source];
                if t.offset >= lo + staged.len()
                    && m.code.bytes[t.offset - staged.len()..t.offset] == staged
                {
                    continue;
                }
                let length: usize = moves.iter().map(|m| load(m.0).len() + 2).sum();
                let mut found = None;
                for reload in [false, true] {
                    let length = length + if reload { 2 } else { 0 };
                    if t.offset < lo + length {
                        continue;
                    }
                    let start = t.offset - length;
                    let bytes = &m.code.bytes[start..t.offset];
                    if let Some(order) = schedule(bytes, &moves, reload) {
                        found = Some(Site {
                            routine: m.id,
                            range: base..base + m.code.bytes.len() as u32,
                            load: base + start as u32,
                            jump: base + t.offset as u32,
                            target: base + m.code.labels[&t.target] as u32,
                            fallthrough: t.fallthrough,
                            reload: reload.then_some(moves.last().unwrap().2),
                            moves: moves.clone(),
                            order,
                            bytes: bytes.to_vec(),
                        });
                        break;
                    }
                }
                out.push(found.expect("typed multi-word edge has an unsafe or unknown encoding"));
            }
        }
        assert!(transfers.next().is_none());
    }
    out
}

impl Site {
    pub fn rebase(&self, base: u32) -> Self {
        let mut s = self.clone();
        let old = self.range.start;
        s.range = base..base + self.range.end - old;
        s.load = base + self.load - old;
        s.jump = base + self.jump - old;
        s.target = base + self.target - old;
        s
    }
}

pub fn decode(bus: &Bus, pc: u32, range: &Range<u32>) -> Option<word_edge::Window> {
    let prefix = range.contains(&pc)
        && pc + 2 <= range.end
        && bus.ram[pc as usize..pc as usize + 2] == [0xc2, 0x20];
    let start = pc + if prefix { 2 } else { 0 };
    let s = bus
        .forwarded_words
        .multi_words
        .iter()
        .find(|s| s.load == start && &s.range == range)?;
    if s.jump != s.load + s.bytes.len() as u32
        || !range.contains(&s.target)
        || s.jump + if s.fallthrough { 0 } else { 4 } > range.end
        || bus.ram[s.load as usize..s.jump as usize] != s.bytes
        || schedule(&s.bytes, &s.moves, s.reload.is_some())? != s.order
    {
        return None;
    }
    if s.fallthrough {
        if s.jump != s.target {
            return None;
        }
    } else if bus.ram[s.jump as usize] != 0x5c || bus.value(s.jump + 1, 3) != s.target {
        return None;
    }
    let mut sites = if prefix { vec![pc] } else { vec![] };
    let mut at = s.load;
    for &i in &s.order {
        sites.push(at);
        at += load(s.moves[i].0).len() as u32;
        sites.push(at);
        at += 2;
    }
    if s.reload.is_some() {
        sites.push(at);
        at += 2;
    }
    assert_eq!(at, s.jump);
    if !s.fallthrough {
        sites.push(s.jump);
    }
    Some(word_edge::Window {
        direct: true,
        fallthrough: s.fallthrough,
        sites,
        moves: s.moves.clone(),
        order: s.order.clone(),
        reload: s.reload,
        target: s.target,
        end: s.jump + if s.fallthrough { 0 } else { 4 },
    })
}
