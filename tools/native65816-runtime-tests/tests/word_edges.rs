mod support;
use actionc::mir65816::{self, image::Image, *};
use actionc::target::ByteSize;
use support::*;

#[test]
fn rotations_repeated_sources_live_ins_unused_and_mixed_parameters_execute() {
    for optimize in [false, true] {
        for mixed in [false, true] {
            for ordinary in [false, true] {
                let p = edges::rotation(optimize, mixed, ordinary);
                let r = p.mir.routines.iter().find(|r| r.name == "Work").unwrap();
                assert_eq!(r.blocks[1].params.len(), if mixed { 9 } else { 6 });
                let compiled = p.compile(&layout()).unwrap();
                let image = Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
                let sites = word_edge::index(&p.mir, &compiled.machine, |id| {
                    image
                        .routines
                        .iter()
                        .find(|r| r.id == id.0)
                        .unwrap()
                        .address
                });
                let caller = caller(image.entry);
                for a in [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
                    let b = a.rotate_left(8) ^ 0x5aa5;
                    for mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller, mask);
                        h.bus.single_word_edges = sites.clone();
                        h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                        h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                        assert_eq!(
                            run_edges(&mut h, &image),
                            if mixed { (1, 2) } else { (5, 26) }
                        );
                        h.guards(mask);
                        assert_eq!(
                            h.bus.value(0x7200, 2),
                            u32::from(a.wrapping_sub(b).wrapping_add(a))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn mutable_parameter_edges_use_current_home_and_preserve_full_words() {
    for optimize in [false, true] {
        let source = "CARD out=$7200 CARD FUNC Work(CARD x) x=x+1 RETURN(x) PROC Main() out=Work($FFFF) RETURN";
        let mut p = prepare(source, optimize);
        let r = p
            .mir
            .routines
            .iter_mut()
            .find(|r| r.name == "Work")
            .unwrap();
        let parameter = r.frame.parameters[0].clone();
        assert!(parameter.frame_object.is_some());
        let last = r
            .blocks
            .iter_mut()
            .find(|b| matches!(b.terminator, Mir65816Terminator::Return { .. }))
            .unwrap();
        let mut ret = last.terminator.clone();
        let Mir65816Terminator::Return {
            value: Some(Mir65816Value::Temp(id, w)),
            ..
        } = &ret
        else {
            panic!()
        };
        let (old_id, w) = (*id, *w);
        let id = actionc::nir::TempId(99);
        if let Mir65816Terminator::Return { value, .. } = &mut ret {
            *value = Some(Mir65816Value::Temp(id, w));
        }
        last.terminator = Mir65816Terminator::Goto(Mir65816Edge {
            target: actionc::nir::BlockId(99),
            args: vec![Mir65816Value::Param(parameter.param)],
        });
        r.blocks.push(Mir65816Block {
            id: actionc::nir::BlockId(99),
            params: vec![(id, w)],
            ops: vec![],
            terminator: ret,
        });
        let ty = r
            .temps
            .iter()
            .find(|(id, _)| *id == old_id)
            .unwrap()
            .1
            .clone();
        r.temps.push((id, ty));
        assert_eq!(w, ByteSize::new(2));
        mir65816::verify_program(&p.mir).unwrap();
        let compiled = p.compile(&layout()).unwrap();
        let image = Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
        let sites = word_edge::index(&p.mir, &compiled.machine, |id| {
            image
                .routines
                .iter()
                .find(|r| r.id == id.0)
                .unwrap()
                .address
        });
        for mask in [0, 4] {
            let mut h = Harness::new(&image, &caller(image.entry), mask);
            h.bus.single_word_edges = sites.clone();
            assert_eq!(run_edges(&mut h, &image), (1, 1));
            h.guards(mask);
            assert_eq!(h.bus.value(0x7200, 2), 0);
        }
    }
}

pub fn run_edges(h: &mut Harness, image: &Image) -> (usize, usize) {
    use actionc_vm::native65816::{Access, Inputs};
    let mut counts = (0, 0);
    for _ in 0..100000 {
        if h.cpu.is_stopped() {
            return counts;
        }
        if h.cpu.is_instruction_boundary() {
            if let Some(w) = word_edge::reached(&h.cpu, &h.bus, &image.routines) {
                counts.0 += 1;
                counts.1 += w.moves.len();
                let before = h.cpu.registers();
                let s = u32::from(before.s);
                let mut expected = vec![];
                let mut values = vec![];
                for &((stack, source), stage, _) in &w.moves {
                    let value = if stack {
                        h.bus.value(s + u32::from(source), 2) as u16
                    } else {
                        source
                    };
                    values.push(value);
                    if stack {
                        expected.extend([
                            (s + u32::from(source), Access::Read),
                            (s + u32::from(source) + 1, Access::Read),
                        ]);
                    }
                    if !w.direct {
                        expected.extend([
                            (s + u32::from(stage), Access::Write(value as u8)),
                            (s + u32::from(stage) + 1, Access::Write((value >> 8) as u8)),
                        ]);
                    }
                    if w.direct {
                        h.bus.ram
                            [(s + u32::from(stage)) as usize..(s + u32::from(stage) + 2) as usize]
                            .copy_from_slice(&[0xbe, 0xef]);
                    }
                    // All four bytes belong to staging; the upper two must stay untouched.
                    h.bus.ram
                        [(s + u32::from(stage) + 2) as usize..(s + u32::from(stage) + 4) as usize]
                        .copy_from_slice(&[0xde, 0xad]);
                }
                for (&(_, stage, dest), &value) in w.moves.iter().zip(&values) {
                    if !w.direct {
                        expected.extend([
                            (s + u32::from(stage), Access::Read),
                            (s + u32::from(stage) + 1, Access::Read),
                        ]);
                    }
                    expected.extend([
                        (s + u32::from(dest), Access::Write(value as u8)),
                        (s + u32::from(dest) + 1, Access::Write((value >> 8) as u8)),
                    ]);
                }
                h.bus.watched = (0x4000..0x6000).chain(0x2000..0x2040).collect();
                h.bus.trace.clear();
                for _ in 0..10000 {
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    if h.cpu.is_instruction_boundary() && h.cpu.pc() == w.target {
                        break;
                    }
                }
                assert_eq!(h.cpu.pc(), w.target);
                let actual: Vec<_> = h.bus.trace.iter().map(|&(_, a, v)| (a, v)).collect();
                assert_eq!(actual, expected);
                h.bus.watched.clear();
                for (&(_, stage, dest), &value) in w.moves.iter().zip(&values) {
                    assert_eq!(h.bus.value(s + u32::from(dest), 2), u32::from(value));
                    assert_eq!(h.bus.value(s + u32::from(stage) + 2, 2), 0xadde);
                    if w.direct {
                        assert_eq!(h.bus.value(s + u32::from(stage), 2), 0xefbe);
                    }
                }
                let after = h.cpu.registers();
                assert_eq!(
                    (
                        after.s,
                        after.d,
                        after.dbr,
                        after.x,
                        after.y,
                        after.p & 0x3c
                    ),
                    (
                        before.s,
                        before.d,
                        before.dbr,
                        before.x,
                        before.y,
                        before.p & 0x0c
                    )
                );
                continue;
            }
        }
        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
    }
    panic!("edge execution exceeded budget");
}

#[test]
fn independent_assembler_and_decoder_check_complete_word_copy_shape() {
    let code = assemble(
        "rep #$20\nlda 4,s\nsta 8,s\nlda #$a55a\nsta 12,s\nlda 8,s\nsta 2,s\nlda 12,s\nsta 4,s\njml $040017\nstp\nnop",
        0x040000,
    );
    assert_eq!(
        code,
        [
            0xc2, 0x20, 0xa3, 4, 0x83, 8, 0xa9, 0x5a, 0xa5, 0x83, 12, 0xa3, 8, 0x83, 2, 0xa3, 12,
            0x83, 4, 0x5c, 0x17, 0, 4, 0xdb, 0xea
        ]
    );
    let start = 0x040000;
    let end = start + code.len() as u32;
    // Target must be inside the owning routine, including after relocation.
    let mut bus = Bus::new();
    bus.map(start, &code, false);
    let w = word_edge::decode(&bus, start, start..end).unwrap();
    assert_eq!(w.moves, vec![((true, 4), 8, 2), ((false, 0xa55a), 12, 4)]);
    assert_eq!(
        word_edge::decode(&bus, start + 2, start..end)
            .unwrap()
            .moves,
        w.moves
    );
    for at in [2, 4, 9, 12, 16, 19, 20] {
        let mut bad = bus.clone();
        bad.ram[start as usize + at] = 0xea;
        assert!(word_edge::decode(&bad, start, start..end).is_none(), "{at}");
    }
    for cutoff in 0..23 {
        assert!(word_edge::decode(&bus, start, start..start + cutoff).is_none());
    }
}

// Changed with selection, after the same semantic probes pass on the baseline.
const EXPECT_DIRECT_SINGLE_WORD: bool = false;

#[test]
fn single_word_edges_cover_immediates_parameters_backedges_and_both_branch_arms() {
    for optimize in [false, true] {
        for ordinary in [false, true] {
            let p = edges::single(optimize, ordinary);
            let c = p.compile(&layout()).unwrap();
            let image = Image::from_json(&c.image.to_json().unwrap()).unwrap();
            let sites = word_edge::index(&p.mir, &c.machine, |id| {
                image
                    .routines
                    .iter()
                    .find(|r| r.id == id.0)
                    .unwrap()
                    .address
            });
            assert_eq!(sites.len(), 5);
            assert!(
                sites
                    .values()
                    .all(|s| s.direct == EXPECT_DIRECT_SINGLE_WORD)
            );
            let mut reached = std::collections::BTreeSet::new();
            for a in [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
                let b = a ^ 0xa55a;
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), mask);
                    h.bus.single_word_edges = sites.clone();
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                    assert_eq!(run_edges(&mut h, &image), (6, 6));
                    for &pc in &h.bus.reads {
                        if sites.contains_key(&pc) {
                            reached.insert(pc);
                        }
                    }
                    h.guards(mask);
                    assert_eq!(h.bus.value(0x7200, 2), u32::from(a.min(b).wrapping_add(a)));
                }
            }
            assert_eq!(reached.len(), sites.len());
            if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
                std::fs::write(
                    std::path::Path::new(&directory)
                        .join(format!("single-word-edges-{optimize}-{ordinary}.json")),
                    image.to_json().unwrap(),
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn independent_direct_word_copies_preserve_flags_even_with_overlapping_homes() {
    use actionc_vm::native65816::{Access, Inputs, Machine, Registers};
    for immediate in [false, true] {
        for a8 in [false, true] {
            for (source, destination) in [(2u8, 4u8), (2, 2), (2, 3), (3, 2), (254, 2), (2, 254)] {
                if immediate && (source, destination) != (2, 4) {
                    continue;
                }
                let prefix = if a8 { "rep #$20\n" } else { "" };
                let load = if immediate {
                    "lda #$a55a".to_string()
                } else {
                    format!("lda {source},s")
                };
                let code = assemble(
                    &format!("{prefix}{load}\nsta {destination},s\njml done\ndone: stp\nnop"),
                    0x040000,
                );
                let target = 0x040000 + code.len() as u32 - 2;
                let load_pc = 0x040000 + if a8 { 2 } else { 0 };
                let mut expected = if a8 { vec![0xc2, 0x20] } else { vec![] };
                expected.extend(if immediate {
                    vec![0xa9, 0x5a, 0xa5]
                } else {
                    vec![0xa3, source]
                });
                expected.extend([0x83, destination, 0x5c]);
                expected.extend(&target.to_le_bytes()[..3]);
                expected.extend([0xdb, 0xea]);
                assert_eq!(code, expected);
                for value in [0u16, 1, 0x00ff, 0x0100, 0x7fff, 0x8000, 0xffff] {
                    let mut bus = Bus::new();
                    bus.map(0x040000, &code, false);
                    bus.map(0x4000, &[0xcc; 0x2000], true);
                    bus.map(0x2000, &[0xa7; 256], true);
                    bus.ram[0x5000 + source as usize..0x5002 + source as usize]
                        .copy_from_slice(&value.to_le_bytes());
                    let value = if immediate { 0xa55a } else { value };
                    let before = Registers {
                        a: 0x1234,
                        x: 0xabcd,
                        y: 0x7654,
                        s: 0x5000,
                        d: 0x2000,
                        dbr: 0x12,
                        pbr: 4,
                        pc: 0,
                        p: 0x45 | if a8 { 0x20 } else { 0 },
                        emulation_mode: false,
                    };
                    let mut cpu = Machine::start_at(before);
                    bus.watched = (0x4000..0x6000).chain(0x2000..0x2100).collect();
                    while cpu.pc() != target || !cpu.is_instruction_boundary() {
                        cpu.tick(&mut bus, Inputs::default()).unwrap();
                    }
                    let mut after = before;
                    after.a = value;
                    after.pc = target as u16;
                    after.p = 0x45
                        | if value == 0 { 2 } else { 0 }
                        | if value & 0x8000 != 0 { 0x80 } else { 0 };
                    assert_eq!(cpu.registers(), after);
                    assert_eq!(
                        cpu.cycles(),
                        (if immediate { 12 } else { 14 }) + if a8 { 3 } else { 0 }
                    );
                    let mut trace = vec![];
                    if !immediate {
                        trace.extend([
                            (0x5000 + u32::from(source), Access::Read),
                            (0x5001 + u32::from(source), Access::Read),
                        ]);
                    }
                    trace.extend([
                        (0x5000 + u32::from(destination), Access::Write(value as u8)),
                        (
                            0x5001 + u32::from(destination),
                            Access::Write((value >> 8) as u8),
                        ),
                    ]);
                    assert_eq!(
                        bus.trace
                            .iter()
                            .map(|&(_, a, op)| (a, op))
                            .collect::<Vec<_>>(),
                        trace
                    );
                    // The same bytes without verified site evidence are not a direct edge.
                    assert!(word_edge::decode(&bus, load_pc, 0x040000..target + 2).is_none());
                    bus.single_word_edges.insert(
                        load_pc,
                        word_edge::Site {
                            range: 0x040000..target + 2,
                            load: load_pc,
                            jump: target - 4,
                            source: (!immediate, if immediate { 0xa55a } else { source.into() }),
                            staging: 16,
                            destination,
                            target,
                            direct: true,
                        },
                    );
                    let w = word_edge::decode(&bus, 0x040000, 0x040000..target + 2).unwrap();
                    assert!(w.direct);
                    assert_eq!(w.sites.len(), if a8 { 4 } else { 3 });
                    for at in [load_pc, target - 6, target - 5, target - 4, target - 3] {
                        let mut bad = bus.clone();
                        bad.ram[at as usize] ^= 1;
                        assert!(word_edge::decode(&bad, load_pc, 0x040000..target + 2).is_none());
                    }
                    for end in load_pc..target {
                        assert!(word_edge::decode(&bus, load_pc, 0x040000..end).is_none());
                    }
                }
            }
        }
    }
}
