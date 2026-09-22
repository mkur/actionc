//! Test-only decoding. Direct copies require typed edge-site evidence.
use super::*;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub direct: bool,
    pub fallthrough: bool,
    pub sites: Vec<u32>,
    pub moves: Vec<((bool, u16), Option<u8>, u8)>, // source, staging, destination
    pub order: Vec<usize>,
    pub reload: Option<u8>,
    pub target: u32,
    pub end: u32,
}

pub fn decode(bus: &Bus, mut pc: u32, range: Range<u32>) -> Option<Window> {
    if let Some(w) = multi_word_edge::decode(bus, pc, &range) {
        return Some(w);
    }
    // An interior LDA/STA suffix must not masquerade as another edge.
    if bus
        .forwarded_words
        .multi_words
        .iter()
        .any(|s| s.range == range && (s.load..s.jump).contains(&pc))
    {
        return None;
    }
    if let Some(w) = direct(bus, pc, &range) {
        return Some(w);
    }
    let mut sites = vec![];
    let end = range.end;
    if !range.contains(&pc) || pc + 2 > end {
        return None;
    }
    if bus.ram[pc as usize..pc as usize + 2] == [0xc2, 0x20] {
        sites.push(pc);
        pc += 2;
    }
    let mut pairs = vec![];
    while pc + 2 <= end
        && bus.ram[pc as usize] != 0x5c
        && (pairs.is_empty() || control_flow::transfer(bus, pc, &range).is_none())
    {
        let stack = bus.ram[pc as usize] == 0xa3;
        if !stack && bus.ram[pc as usize] != 0xa9 {
            return None;
        }
        let size = if stack { 2 } else { 3 };
        if pc + size + 2 > end || bus.ram[(pc + size) as usize] != 0x83 {
            return None;
        }
        let source = bus.value(pc + 1, (size - 1) as usize) as u16;
        let destination = bus.ram[(pc + size + 1) as usize];
        if !(1..=254).contains(&destination) || (stack && !(1..=254).contains(&source)) {
            return None;
        }
        sites.extend([pc, pc + size]);
        pairs.push(((stack, source), destination));
        pc += size + 2;
    }
    if pairs.is_empty() || pairs.len() % 2 != 0 {
        return None;
    }
    let (target, transfer_end, fallthrough) =
        control_flow::transfer(bus, pc, &range).or_else(|| {
            (pc + 4 <= end && bus.ram[pc as usize] == 0x5c)
                .then(|| (bus.value(pc + 1, 3), pc + 4, false))
        })?;
    if !range.contains(&target) {
        return None;
    }
    let n = pairs.len() / 2;
    let mut moves = vec![];
    let overlap = |a: u8, b: u8| a.abs_diff(b) < 2;
    for i in 0..n {
        let (source, stage) = pairs[i];
        let (staged, dest) = pairs[n + i];
        if staged != (true, u16::from(stage)) {
            return None;
        }
        // Actual staging widths may differ across edges; word starts remain
        // aligned and separated by two or four bytes.
        if stage % 2 != 0
            || (i > 0 && ![2, 4].contains(&(i16::from(stage) - i16::from(pairs[i - 1].1))))
        {
            return None;
        }
        if pairs[..n]
            .iter()
            .any(|&(src, s)| overlap(dest, s) || (src.0 && overlap(src.1 as u8, stage)))
        {
            return None;
        }
        if pairs[n..n + i].iter().any(|&(_, d)| overlap(dest, d)) {
            return None;
        }
        moves.push((source, Some(stage), dest));
    }
    if !fallthrough {
        sites.push(pc);
    }
    Some(Window {
        direct: false,
        fallthrough,
        sites,
        order: (0..moves.len()).collect(),
        reload: None,
        moves,
        target,
        end: transfer_end,
    })
}

/// Count at the first LDA only, so an optional REP does not count twice.
pub fn reached(
    cpu: &Machine,
    bus: &Bus,
    routines: &[actionc::mir65816::image::Routine],
) -> Option<Window> {
    assert!(cpu.is_instruction_boundary());
    if cpu.registers().p & 0x30 != 0 || !matches!(bus.ram[cpu.pc() as usize], 0xa3 | 0xa9) {
        return None;
    }
    let r = routines
        .iter()
        .find(|r| (r.address..r.address + r.size).contains(&cpu.pc()))?;
    decode(bus, cpu.pc(), r.address..r.address + r.size)
}

/// Test-only evidence derived from a verified MIR edge and its typed JML fixup.
/// Staging is retained in the frame even when the machine transfer is direct.
#[derive(Clone, Debug)]
pub struct Site {
    pub range: Range<u32>,
    pub load: u32,
    pub jump: u32,
    pub source: (bool, u16),
    pub staging: Option<u8>,
    pub destination: u8,
    pub target: u32,
    pub direct: bool,
    pub fallthrough: bool,
}
pub type Index = std::collections::BTreeMap<u32, Site>;

pub fn index(
    mir: &actionc::mir65816::Mir65816Program,
    machine: &actionc::mir65816::emit::MachineProgram,
    address: impl Fn(actionc::nir::RoutineId) -> u32,
) -> Index {
    use actionc::mir65816::{
        emit::{Label, Location, Target},
        *,
    };
    actionc::mir65816::verify_program(mir).unwrap();
    let mut result = Index::new();
    for m in &machine.routines {
        let r = mir.routines.iter().find(|r| r.id == m.id).unwrap();
        let base = address(r.id);
        let range = base..base + m.code.bytes.len() as u32;
        let stack = |id| match m.frame.temps[&id] {
            Location::Stack(slot) if slot.width == 2 => Some(slot.offset as u8),
            _ => None,
        };
        for (bi, b) in r.blocks.iter().enumerate() {
            let lo = m.code.labels[&Label(bi as u32)];
            let hi = if bi + 1 < r.blocks.len() {
                m.code.labels[&Label(bi as u32 + 1)]
            } else {
                m.code.bytes.len()
            };
            let edges: Vec<_> = match &b.terminator {
                Mir65816Terminator::Goto(e) => vec![e],
                Mir65816Terminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![else_edge, then_edge],
                _ => vec![],
            };
            let mut expected = vec![];
            for e in edges {
                let target = r.blocks.iter().position(|b| b.id == e.target).unwrap();
                let params = &r.blocks[target].params;
                if e.args.len() != 1 || params.len() != 1 || params[0].1.get() != 2 {
                    continue;
                }
                let Some(destination) = stack(params[0].0) else {
                    continue;
                };
                let source = match &e.args[0] {
                    Mir65816Value::U16(v) => (false, *v),
                    Mir65816Value::Temp(id, w) if w.get() == 2 => {
                        let Some(s) = stack(*id) else {
                            continue;
                        };
                        (true, u16::from(s))
                    }
                    Mir65816Value::Param(id) => {
                        let p = r.frame.parameters.iter().find(|p| p.param == *id).unwrap();
                        let Mir65816AbiHome::StackArgument { offset, size, .. } = p.incoming else {
                            panic!()
                        };
                        if size.get() != 2 {
                            continue;
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
                            actionc::mir65816::abi::stack::incoming_displacement(
                                actionc::target::ByteSize::new(m.frame.extent.into()),
                                offset,
                                size,
                            )
                            .unwrap()
                            .get()
                        };
                        (true, at.try_into().unwrap())
                    }
                    _ => continue,
                };
                expected.push((target as u32, source, destination));
            }
            let mut found = 0;
            for transfer in &m.code.mir_transfers {
                let label = transfer.target;
                if transfer.source != Label(bi as u32) || !expected.iter().any(|e| e.0 == label.0) {
                    continue;
                }
                assert!((lo..=hi).contains(&transfer.offset));
                if transfer.fallthrough {
                    assert_eq!(m.code.labels[&label], transfer.offset);
                } else {
                    assert_eq!(m.code.bytes[transfer.offset], 0x5c);
                    assert!(m.code.fixups.iter().any(|f| f.offset == transfer.offset + 1
                        && f.target == Target::Label(label)
                        && f.addend == 0
                        && f.byte.is_none()));
                }
                let staging: Option<u8> = m
                    .frame
                    .edge_copies
                    .first()
                    .filter(|s| s.width >= 2)
                    .map(|s| s.offset.try_into().unwrap());
                let jump = transfer.offset;
                let mut matched = None;
                for &(_, source, destination) in expected.iter().filter(|e| e.0 == label.0) {
                    let load = if source.0 {
                        vec![0xa3, source.1.try_into().unwrap()]
                    } else {
                        vec![0xa9, source.1 as u8, (source.1 >> 8) as u8]
                    };
                    // Try the complete staged shape first: its final LDA/STA
                    // suffix must never become a second direct edge.
                    for direct in [false, true] {
                        let mut bytes = load.clone();
                        if !direct {
                            let Some(stage) = staging else { continue };
                            bytes.extend([0x83, stage, 0xa3, stage]);
                        }
                        bytes.extend([0x83, destination]);
                        if jump >= lo + bytes.len()
                            && m.code.bytes[jump - bytes.len()..jump] == bytes
                        {
                            matched = Some(Site {
                                range: range.clone(),
                                load: base + (jump - bytes.len()) as u32,
                                jump: base + jump as u32,
                                source,
                                staging,
                                destination,
                                target: base + m.code.labels[&label] as u32,
                                direct,
                                fallthrough: transfer.fallthrough,
                            });
                            break;
                        }
                    }
                    if matched.is_some() {
                        break;
                    }
                }
                let site = matched.expect("typed single-word edge has an unexpected encoding");
                assert!(result.insert(site.load, site).is_none());
                found += 1;
            }
            assert_eq!(
                found,
                expected.len(),
                "missing typed single-word edge in {}",
                r.name
            );
        }
    }
    result
}

fn direct(bus: &Bus, pc: u32, range: &Range<u32>) -> Option<Window> {
    let prefix = range.contains(&pc)
        && pc + 2 <= range.end
        && bus.ram[pc as usize..pc as usize + 2] == [0xc2, 0x20];
    let load = pc + if prefix { 2 } else { 0 };
    let s = bus.single_word_edges.get(&load)?;
    if !s.direct
        || &s.range != range
        || s.load != load
        || !range.contains(&s.target)
        || !range.contains(&pc)
        || s.jump + if s.fallthrough { 0 } else { 4 } > range.end
        || (s.fallthrough && s.target != s.jump)
        || !(1..=254).contains(&s.destination)
        || (s.source.0 && !(1..=254).contains(&s.source.1))
    {
        return None;
    }
    let mut bytes = if s.source.0 {
        vec![0xa3, s.source.1 as u8]
    } else {
        vec![0xa9, s.source.1 as u8, (s.source.1 >> 8) as u8]
    };
    let store = load + bytes.len() as u32;
    bytes.extend([0x83, s.destination]);
    if !s.fallthrough {
        bytes.push(0x5c);
        bytes.extend(&s.target.to_le_bytes()[..3]);
    }
    if store + 2 != s.jump
        || load + bytes.len() as u32 > range.end
        || bus.ram[load as usize..load as usize + bytes.len()] != bytes
    {
        return None;
    }
    let mut sites = if prefix { vec![pc] } else { vec![] };
    sites.extend([load, store]);
    if !s.fallthrough {
        sites.push(s.jump);
    }
    Some(Window {
        direct: true,
        fallthrough: s.fallthrough,
        sites,
        moves: vec![(s.source, s.staging, s.destination)],
        order: vec![0],
        reload: None,
        target: s.target,
        end: s.jump + if s.fallthrough { 0 } else { 4 },
    })
}
