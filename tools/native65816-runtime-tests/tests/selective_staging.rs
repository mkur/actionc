mod support;
use actionc_vm::native65816::{Access, Inputs, Machine, Registers};
use support::*;

#[test]
fn independent_selective_sequences_match_simultaneous_copies_and_full_staging() {
    // Explicit masks, independent of compiler planning: swap, rotations in both
    // directions, two cycles with an independent immediate, repeated/self source.
    for (sources, saved) in [
        (vec![Some(4), Some(2)], vec![1]),
        (vec![Some(4), Some(6), Some(2)], vec![2]),
        (vec![Some(6), Some(2), Some(4)], vec![1, 2]),
        (vec![Some(4), Some(2), Some(8), Some(6), None], vec![1, 3]),
        (vec![Some(4), Some(2), Some(4), Some(8)], vec![1, 2]),
    ] {
        let assemble_copy = |captures: &[usize]| {
            let mut text = String::new();
            let load = |s: Option<u8>| s.map_or("lda #$a55a\n".into(), |s| format!("lda {s},s\n"));
            for (k, &i) in captures.iter().enumerate() {
                text.push_str(&load(sources[i]));
                text.push_str(&format!("sta {},s\n", 32 + 2 * k));
            }
            for (i, &s) in sources.iter().enumerate() {
                text.push_str(&load(
                    captures
                        .iter()
                        .position(|&j| i == j)
                        .map(|k| 32 + 2 * k as u8)
                        .or(s),
                ));
                text.push_str(&format!("sta {},s\n", 2 + 2 * i));
            }
            text.push_str("stp\nnop");
            assemble(&text, 0x040000)
        };
        let before = assemble_copy(&(0..sources.len()).collect::<Vec<_>>());
        let after = assemble_copy(&saved);
        for value in [0u16, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
            for p in (0..=255u8).filter(|p| p & 0x38 == 0) {
                let initial = Registers {
                    a: 0xabcd,
                    x: 0x5678,
                    y: 0x9abc,
                    s: 0x5f80,
                    d: 0x2000,
                    dbr: 0,
                    pbr: 4,
                    pc: 0,
                    p,
                    emulation_mode: false,
                };
                let run = |code: &[u8], captures: &[usize]| {
                    let mut bus = Bus::new();
                    bus.map(0x040000, code, false);
                    bus.map(0x4000, &[0xa5; 0x2000], true);
                    for i in 0..sources.len() {
                        let at = usize::from(initial.s) + 2 + 2 * i;
                        let v = value.rotate_left(i as u32 * 3) ^ (i as u16 * 0x1111);
                        bus.ram[at..at + 2].copy_from_slice(&v.to_le_bytes());
                    }
                    let original = bus.ram[0x4000..0x6000].to_vec();
                    let values: Vec<_> = sources
                        .iter()
                        .map(|s| {
                            s.map_or(0xa55a, |s| {
                                bus.value(u32::from(initial.s) + u32::from(s), 2) as u16
                            })
                        })
                        .collect();
                    let mut expected = vec![];
                    let read = |trace: &mut Vec<_>, s: u8| {
                        trace.extend([
                            (u32::from(initial.s) + u32::from(s), Access::Read),
                            (u32::from(initial.s) + u32::from(s) + 1, Access::Read),
                        ]);
                    };
                    let write = |trace: &mut Vec<_>, s: u8, v: u16| {
                        trace.extend([
                            (u32::from(initial.s) + u32::from(s), Access::Write(v as u8)),
                            (
                                u32::from(initial.s) + u32::from(s) + 1,
                                Access::Write((v >> 8) as u8),
                            ),
                        ]);
                    };
                    for (k, &i) in captures.iter().enumerate() {
                        if let Some(s) = sources[i] {
                            read(&mut expected, s);
                        }
                        write(&mut expected, 32 + 2 * k as u8, values[i]);
                    }
                    for (i, &s) in sources.iter().enumerate() {
                        if let Some(s) = captures
                            .iter()
                            .position(|&j| i == j)
                            .map(|k| 32 + 2 * k as u8)
                            .or(s)
                        {
                            read(&mut expected, s);
                        }
                        write(&mut expected, 2 + 2 * i as u8, values[i]);
                    }
                    bus.watched = (0x4000..0x6000).chain(0x2000..0x2040).collect();
                    let mut cpu = Machine::start_at(initial);
                    assert!(
                        cpu.run_until(&mut bus, 500, |_| Inputs::default(), |c| c.is_stopped())
                            .unwrap()
                    );
                    assert_eq!(
                        bus.trace
                            .iter()
                            .map(|&(_, a, v)| (a, v))
                            .collect::<Vec<_>>(),
                        expected
                    );
                    for (i, &v) in values.iter().enumerate() {
                        assert_eq!(
                            bus.value(u32::from(initial.s) + 2 + 2 * i as u32, 2),
                            u32::from(v)
                        );
                    }
                    for (i, &v) in original.iter().enumerate() {
                        let a = 0x4000 + i as u32;
                        if !expected
                            .iter()
                            .any(|&(at, op)| at == a && matches!(op, Access::Write(_)))
                        {
                            assert_eq!(bus.ram[a as usize], v);
                        }
                    }
                    assert_eq!(cpu.registers().a, *values.last().unwrap());
                    (cpu.registers(), cpu.cycles())
                };
                let (b, bc) = run(&before, &(0..sources.len()).collect::<Vec<_>>());
                let (a, ac) = run(&after, &saved);
                assert_eq!(
                    (a.a, a.x, a.y, a.s, a.d, a.dbr, a.pbr, a.p, a.emulation_mode),
                    (b.a, b.x, b.y, b.s, b.d, b.dbr, b.pbr, b.p, b.emulation_mode)
                );
                assert_eq!(a.p & 0x7d, p & 0x7d);
                assert_eq!(bc - ac, 10 * (sources.len() - saved.len()) as u64);
            }
        }
    }
}

#[test]
fn selective_evidence_rejects_missing_late_reused_and_forged_captures() {
    let moves = [
        ((true, 6), None, 2),
        ((true, 2), Some(10), 4),
        ((true, 4), Some(12), 6),
    ];
    let valid = [
        0xa3, 2, 0x83, 10, 0xa3, 4, 0x83, 12, 0xa3, 6, 0x83, 2, 0xa3, 10, 0x83, 4, 0xa3, 12, 0x83,
        6,
    ];
    assert!(multi_word_edge::selective(&valid, &moves).is_some());
    for n in 0..valid.len() {
        assert!(multi_word_edge::selective(&valid[..n], &moves).is_none());
    }
    for i in 0..valid.len() {
        let mut wrong = valid;
        wrong[i] ^= 1;
        assert!(multi_word_edge::selective(&wrong, &moves).is_none());
    }
    for i in [1, 2] {
        for slot in [None, Some(0), Some(2), Some(10), Some(254), Some(255)] {
            if slot == moves[i].1 {
                continue;
            }
            let mut wrong = moves;
            wrong[i].1 = slot;
            assert!(multi_word_edge::selective(&valid, &wrong).is_none());
        }
    }
    let mut late = valid;
    late[..4].copy_from_slice(&valid[8..12]);
    late[8..12].copy_from_slice(&valid[..4]);
    assert!(multi_word_edge::selective(&late, &moves).is_none());
}

#[test]
fn typed_selective_windows_reject_suffixes_stale_targets_and_bytes() {
    for optimize in [false, true] {
        let p = edges::rotation(optimize, false, false);
        let compiled = p.compile(&layout()).unwrap();
        let image = &compiled.image;
        let mut h = Harness::new(image, &caller(image.entry), 0);
        h.bus.forwarded_words = forwarding::compiled(&p, &compiled);
        let sites: Vec<_> = h
            .bus
            .forwarded_words
            .multi_words
            .iter()
            .filter(|s| s.form == word_edge::Form::Selective)
            .cloned()
            .collect();
        assert!(!sites.is_empty());
        for s in sites {
            let w = word_edge::decode(&h.bus, s.load, s.range.clone()).unwrap();
            assert_eq!(w.form, word_edge::Form::Selective);
            for pc in s.load + 1..s.jump {
                assert!(word_edge::decode(&h.bus, pc, s.range.clone()).is_none());
            }
            for pc in s.load..s.jump {
                h.bus.ram[pc as usize] ^= 1;
                assert!(word_edge::decode(&h.bus, s.load, s.range.clone()).is_none());
                h.bus.ram[pc as usize] ^= 1;
            }
            let i = h
                .bus
                .forwarded_words
                .multi_words
                .iter()
                .position(|v| v.load == s.load)
                .unwrap();
            h.bus.forwarded_words.multi_words[i].target += 1;
            assert!(word_edge::decode(&h.bus, s.load, s.range.clone()).is_none());
            h.bus.forwarded_words.multi_words[i] = s.clone();
            h.bus.forwarded_words.multi_words[i].jump -= 1;
            assert!(word_edge::decode(&h.bus, s.load, s.range.clone()).is_none());
            h.bus.forwarded_words.multi_words[i] = s;
        }
    }
}
