//! Read-only inventories from verified MIR and independently checked encodings.
use actionc::mir65816::{
    emit::{Label, MachineProgram, Target},
    image::Image,
    *,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct Transfer {
    pub routine: actionc::nir::RoutineId,
    pub range: std::ops::Range<u32>,
    pub at: u32,
    pub target: u32,
    pub fallthrough: bool,
}
pub type Index = Vec<Transfer>;

pub fn index(
    p: &Mir65816Program,
    machine: &MachineProgram,
    address: impl Fn(actionc::nir::RoutineId) -> u32,
) -> Index {
    let mut out = vec![];
    for m in &machine.routines {
        let r = p.routines.iter().find(|r| r.id == m.id).unwrap();
        let base = address(r.id);
        let mut expected = vec![];
        for (i, b) in r.blocks.iter().enumerate() {
            let targets = match &b.terminator {
                Mir65816Terminator::Goto(e) => vec![e.target],
                Mir65816Terminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![else_edge.target, then_edge.target],
                Mir65816Terminator::Fallthrough => vec![r.blocks[i + 1].id],
                _ => vec![],
            };
            expected.extend(targets.into_iter().map(|id| {
                (
                    Label(i as u32),
                    Label(r.blocks.iter().position(|b| b.id == id).unwrap() as u32),
                )
            }));
        }
        assert_eq!(
            expected,
            m.code
                .mir_transfers
                .iter()
                .map(|e| (e.source, e.target))
                .collect::<Vec<_>>()
        );
        for e in &m.code.mir_transfers {
            let target = m.code.labels[&e.target];
            if e.fallthrough {
                assert_eq!(e.offset, target);
                // The next block may itself start with a relocated instruction;
                // a zero-byte transfer owns no operand range to exclude.
            } else {
                assert_eq!(m.code.bytes[e.offset], 0x5c);
                assert!(m.code.fixups.iter().any(|f| f.offset == e.offset + 1
                    && f.target == Target::Label(e.target)
                    && f.addend == 0
                    && f.byte.is_none()));
            }
            out.push(Transfer {
                routine: r.id,
                range: base..base + m.code.bytes.len() as u32,
                at: base + e.offset as u32,
                target: base + target as u32,
                fallthrough: e.fallthrough,
            });
        }
    }
    out
}

pub fn relocated(templates: &Index, image: &actionc::mir65816::o65::RelocatedImage) -> Index {
    templates
        .iter()
        .map(|s| {
            let r = image
                .profile()
                .routines
                .iter()
                .find(|r| r.id == s.routine.0)
                .unwrap();
            let base = image.routine_address(r);
            Transfer {
                range: base..base + (s.range.end - s.range.start),
                at: base + s.at - s.range.start,
                target: base + s.target - s.range.start,
                ..s.clone()
            }
        })
        .collect()
}

/// Metadata locates a transfer; the final bytes still establish its encoding.
pub fn transfer(
    bus: &super::Bus,
    at: u32,
    range: &std::ops::Range<u32>,
) -> Option<(u32, u32, bool)> {
    let site = bus
        .forwarded_words
        .control
        .iter()
        .find(|s| s.at == at && &s.range == range)?;
    if !range.contains(&site.target) {
        return None;
    }
    if site.fallthrough {
        (site.target == at).then_some((at, at, true))
    } else {
        (at + 4 <= range.end && bus.ram[at as usize] == 0x5c && bus.value(at + 1, 3) == site.target)
            .then_some((site.target, at + 4, false))
    }
}

pub fn inventory(p: &Mir65816Program, machine: &MachineProgram, image: &Image) -> Vec<Value> {
    verify_program(p).unwrap();
    let mut out = vec![];
    for m in &machine.routines {
        let r = p.routines.iter().find(|r| r.id == m.id).unwrap();
        let base = image
            .routines
            .iter()
            .find(|r| r.id == m.id.0)
            .unwrap()
            .address;
        let successors = |i: usize| -> Vec<_> {
            match &r.blocks[i].terminator {
                Mir65816Terminator::Goto(e) => vec![e.target],
                Mir65816Terminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![else_edge.target, then_edge.target],
                Mir65816Terminator::Fallthrough => vec![r.blocks[i + 1].id],
                _ => vec![],
            }
        };
        let mut reachable = BTreeSet::new();
        let mut pending = vec![r.blocks[0].id];
        while let Some(id) = pending.pop() {
            if reachable.insert(id) {
                pending.extend(successors(
                    r.blocks.iter().position(|b| b.id == id).unwrap(),
                ));
            }
        }
        for (i, b) in r.blocks.iter().enumerate() {
            // Current emitter allocates MIR labels in block order before internal
            // labels. Check the association against its independent span boundary.
            let at = m.code.labels[&Label(i as u32)];
            assert_eq!(at, m.code.mir_spans[&(b.id, 0)].start);
            let end = if i + 1 < r.blocks.len() {
                m.code.labels[&Label(i as u32 + 1)]
            } else {
                m.code.bytes.len()
            };
            let candidate =
                reachable.contains(&b.id) && m.code.bytes.get(at..at + 2) == Some(&[0xc2, 0x20]);
            out.push(json!({"slice":"3a", "routine":r.id.0,"block":b.id.0,"pc":base + at as u32,
                "selected":candidate,"reason":if !reachable.contains(&b.id) {"unreachable"} else if candidate {"reachable MIR A16 entry REP"} else {"no entry REP"}}));
            let successors = successors(i);
            if let Some(next) = r.blocks.get(i + 1) {
                if successors.last() == Some(&next.id) {
                    let jump = end.checked_sub(4).unwrap();
                    if let Some(f) = m.code.fixups.iter().find(|f| {
                        f.offset == jump + 1 && f.target == Target::Label(Label(i as u32 + 1))
                    }) {
                        assert_eq!((m.code.bytes[jump], f.addend, f.byte), (0x5c, 0, None));
                        out.push(json!({"slice":"3b","routine":r.id.0,"block":b.id.0,"pc":base + jump as u32,"selected":true,"target":base + end as u32}));
                    }
                }
            }
            if matches!(b.terminator, Mir65816Terminator::Branch { .. }) {
                let span = m
                    .code
                    .mir_spans
                    .get(&(b.id, b.ops.len()))
                    .unwrap_or_else(|| &m.code.mir_spans[&(b.id, b.ops.len() - 1)]);
                let mut found = 0;
                for f in &m.code.fixups {
                    if f.offset < 3 {
                        continue;
                    }
                    let pc = f.offset - 3;
                    if !span.contains(&pc) || !matches!(f.target, Target::Label(_)) {
                        continue;
                    }
                    let code = &m.code.bytes[pc..f.offset];
                    if matches!(code[0], 0x10 | 0x30 | 0x90 | 0xb0 | 0xd0 | 0xf0)
                        && code[1..] == [4, 0x5c]
                    {
                        let Target::Label(label) = f.target else {
                            unreachable!()
                        };
                        let target = m.code.labels[&label];
                        // The candidate's own removal also moves a forward target.
                        let delta =
                            target as i64 - if target >= pc + 6 { 4 } else { 0 } - (pc + 2) as i64;
                        out.push(json!({"slice":"3c","routine":r.id.0,"block":b.id.0,"pc":base+pc as u32,"target":base+target as u32,
                            "predicate":code[0]^0x20,"selected":(-128..=127).contains(&delta)}));
                        found += 1;
                    }
                }
                assert_eq!(found, 1, "one MIR conditional dispatch per terminator");
            }
        }
    }
    out
}
