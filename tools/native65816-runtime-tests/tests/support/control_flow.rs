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

#[derive(Clone, Debug)]
pub struct Dispatch {
    pub routine: actionc::nir::RoutineId,
    pub range: std::ops::Range<u32>,
    pub at: u32,
    pub target: u32,
    pub predicate: u8,
    pub short: bool,
}

pub fn dispatches(
    p: &Mir65816Program,
    machine: &MachineProgram,
    address: impl Fn(actionc::nir::RoutineId) -> u32,
) -> Vec<Dispatch> {
    let mut out = vec![];
    for m in &machine.routines {
        let r = p.routines.iter().find(|r| r.id == m.id).unwrap();
        let base = address(r.id);
        let blocks: Vec<_> = r
            .blocks
            .iter()
            .filter(|b| matches!(b.terminator, Mir65816Terminator::Branch { .. }))
            .collect();
        // Compiler-owned arithmetic bodies have physical control flow without
        // fabricated MIR source spans. Check their branch encodings directly.
        if r.helper.is_some() {
            assert!(m.code.mir_spans.is_empty());
            assert!(m.code.mir_transfers.is_empty());
            for site in &m.code.conditional_branches {
                let target = m.code.labels[&site.target];
                if site.short {
                    assert_eq!(m.code.bytes[site.offset], site.predicate);
                    assert_eq!(
                        site.offset as i64 + 2 + i64::from(m.code.bytes[site.offset + 1] as i8),
                        target as i64
                    );
                } else {
                    assert_eq!(
                        &m.code.bytes[site.offset..site.offset + 3],
                        &[site.predicate ^ 0x20, 4, 0x5c]
                    );
                    assert!(
                        m.code.fixups.iter().any(|f| f.offset == site.offset + 3
                            && f.target == Target::Label(site.target))
                    );
                }
            }
            continue;
        }
        let mut seen = BTreeSet::new();
        for b in blocks {
            let ordinary = m.code.mir_spans.contains_key(&(b.id, b.ops.len()));
            let span = m
                .code
                .mir_spans
                .get(&(b.id, b.ops.len()))
                .unwrap_or_else(|| &m.code.mir_spans[&(b.id, b.ops.len() - 1)]);
            let sites: Vec<_> = m
                .code
                .conditional_branches
                .iter()
                .filter(|s| s.dispatch && span.contains(&s.offset))
                .collect();
            // Fused 24/32-bit inequality can take the true edge after either part.
            // All other selected predicates still have exactly one dispatch.
            let two_part_ne = !ordinary
                && matches!(b.ops.last(), Some(Mir65816Op::Compare {
                width, operation: actionc::nir::NirCompareOp::Ne, ..
            }) if matches!(width.get(), 3 | 4));
            assert_eq!(sites.len(), if two_part_ne { 2 } else { 1 });
            if two_part_ne {
                assert_eq!(sites[0].target, sites[1].target);
                assert!(sites.iter().all(|s| s.predicate == 0xd0));
            }
            for s in sites {
                assert!(seen.insert(s.offset));
                assert!(span.contains(&s.offset));
                let target = m.code.labels[&s.target];
                if s.short {
                    assert_eq!(m.code.bytes[s.offset], s.predicate);
                    assert_eq!(
                        s.offset as i64 + 2 + i64::from(m.code.bytes[s.offset + 1] as i8),
                        target as i64
                    );
                } else {
                    assert_eq!(
                        &m.code.bytes[s.offset..s.offset + 3],
                        &[s.predicate ^ 0x20, 4, 0x5c]
                    );
                    assert!(
                        m.code.fixups.iter().any(
                            |f| f.offset == s.offset + 3 && f.target == Target::Label(s.target)
                        )
                    );
                }
                out.push(Dispatch {
                    routine: r.id,
                    range: base..base + m.code.bytes.len() as u32,
                    at: base + s.offset as u32,
                    target: base + target as u32,
                    predicate: s.predicate,
                    short: s.short,
                });
            }
        }
        // Guard conditionals now share layout metadata, but are not MIR dispatches.
        for fixup in m
            .code
            .fixups
            .iter()
            .filter(|f| f.target == Target::StackOverflow)
        {
            let start = fixup.offset.checked_sub(24).unwrap();
            for (offset, predicate) in [(4, 0x90), (6, 0xf0), (14, 0x90), (18, 0xb0)] {
                let site = m
                    .code
                    .conditional_branches
                    .iter()
                    .find(|s| s.offset == start + offset)
                    .unwrap();
                assert!(site.short);
                assert_eq!(site.predicate, predicate);
                assert!(seen.insert(site.offset));
            }
        }
        assert_eq!(
            seen.len(),
            m.code
                .conditional_branches
                .iter()
                .filter(|s| s.dispatch)
                .count()
        );
    }
    out
}

pub fn relocated_dispatches(
    templates: &[Dispatch],
    image: &actionc::mir65816::o65::RelocatedImage,
) -> Vec<Dispatch> {
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
            Dispatch {
                range: base..base + s.range.end - s.range.start,
                at: base + s.at - s.range.start,
                target: base + s.target - s.range.start,
                ..s.clone()
            }
        })
        .collect()
}

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
                check_jump(&m.code, e.offset, e.target);
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
        let (target, end) = jump(bus, at, range)?;
        (target == site.target).then_some((target, end, false))
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
                for branch in &m.code.conditional_branches {
                    let pc = branch.offset;
                    if !branch.dispatch || !span.contains(&pc) {
                        continue;
                    }
                    let target = m.code.labels[&branch.target];
                    let delta = target as i64
                        - if !branch.short && target >= pc + 6 {
                            4
                        } else {
                            0
                        }
                        - (pc + 2) as i64;
                    out.push(json!({"slice":"3c","routine":r.id.0,"block":b.id.0,"pc":base+pc as u32,"target":base+target as u32,
                        "predicate":branch.predicate,"selected":(-128..=127).contains(&delta)}));
                    found += 1;
                }
                assert_eq!(found, 1, "one MIR conditional dispatch per terminator");
            }
        }
    }
    out
}

/// Independent decoding at a known transfer boundary, with checked bank/range.
pub fn jump(bus: &super::Bus, at: u32, range: &std::ops::Range<u32>) -> Option<(u32, u32)> {
    if !range.contains(&at) {
        return None;
    }
    let size = match bus.ram[at as usize] {
        0x80 => 2,
        0x82 => 3,
        0x5c => 4,
        _ => return None,
    };
    let end = at.checked_add(size)?;
    if end > range.end || at >> 16 != end >> 16 {
        return None;
    }
    let target = match size {
        2 => i64::from(end) + i64::from(bus.ram[at as usize + 1] as i8),
        3 => i64::from(end) + i64::from(bus.value(at + 1, 2) as u16 as i16),
        _ => i64::from(bus.value(at + 1, 3)),
    };
    let target = u32::try_from(target).ok()?;
    (range.contains(&target) && target >> 16 == at >> 16).then_some((target, end))
}
pub fn check_jump(code: &actionc::mir65816::emit::Code, at: usize, label: Label) {
    use actionc::mir65816::emit::JumpEncoding;
    let site = code.local_jumps.iter().find(|s| s.offset == at).unwrap();
    assert_eq!(site.target, label);
    let end = at + site.encoding.size();
    let delta = code.labels[&label] as i64 - end as i64;
    let expected = match site.encoding {
        JumpEncoding::Relative8 => vec![0x80, i8::try_from(delta).unwrap() as u8],
        JumpEncoding::Relative16 => {
            let [lo, hi] = i16::try_from(delta).unwrap().to_le_bytes();
            vec![0x82, lo, hi]
        }
        JumpEncoding::Long => {
            assert!(code.fixups.iter().any(|f| f.offset == at + 1
                && f.target == Target::Label(label)
                && f.addend == 0
                && f.byte.is_none()));
            vec![0x5c, 0, 0, 0]
        }
    };
    assert_eq!(&code.bytes[at..end], expected);
}
