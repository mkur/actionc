//! Read-only inventories from verified MIR and independently checked encodings.
use actionc::mir65816::{
    emit::{Label, MachineProgram, Target},
    image::Image,
    *,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

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
