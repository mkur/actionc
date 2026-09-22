mod support;
use actionc_vm::native65816::{Inputs, Machine, Registers};
use support::*;

#[test]
fn independent_acyclic_encodings_preserve_words_full_a_flags_and_staging() {
    // Explicit schedules: independent, forward chain, reordered chain, and
    // a self-copy plus a repeated source. Reordering retains original final A.
    for (sources, destinations, order, reload) in [
        ([6, 8], [2, 4], [0, 1], None),
        ([4, 6], [2, 4], [0, 1], None),
        ([6, 2], [2, 4], [1, 0], Some(4)),
        ([2, 2], [2, 4], [1, 0], Some(4)),
    ] {
        let before = assemble(
            &format!(
                "lda {},s\nsta 32,s\nlda {},s\nsta 36,s\nlda 32,s\nsta {},s\nlda 36,s\nsta {},s\nstp\nnop",
                sources[0], sources[1], destinations[0], destinations[1]
            ),
            0x040000,
        );
        let mut text = String::new();
        for i in order {
            text.push_str(&format!(
                "lda {},s\nsta {},s\n",
                sources[i], destinations[i]
            ));
        }
        if let Some(d) = reload {
            text.push_str(&format!("lda {d},s\n"));
        }
        text.push_str("stp\nnop");
        let after = assemble(&text, 0x040000);
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
                let run = |code: &[u8]| {
                    let mut bus = Bus::new();
                    bus.map(0x040000, code, false);
                    bus.map(0x4000, &[0xa5; 0x2000], true);
                    for (i, d) in [2, 4, 6, 8].into_iter().enumerate() {
                        let v = value.rotate_left(i as u32 * 3) ^ ((i as u16) * 0x1111);
                        bus.ram[usize::from(initial.s) + d..usize::from(initial.s) + d + 2]
                            .copy_from_slice(&v.to_le_bytes());
                    }
                    let mut cpu = Machine::start_at(initial);
                    assert!(
                        cpu.run_until(&mut bus, 300, |_| Inputs::default(), |c| c.is_stopped())
                            .unwrap()
                    );
                    (
                        cpu.registers(),
                        cpu.cycles(),
                        bus.ram[0x4000..0x6000].to_vec(),
                    )
                };
                let (b, bc, bmem) = run(&before);
                let (a, ac, amem) = run(&after);
                assert_eq!(
                    (a.a, a.x, a.y, a.s, a.d, a.dbr, a.p, a.emulation_mode),
                    (b.a, b.x, b.y, b.s, b.d, b.dbr, b.p, b.emulation_mode)
                );
                assert_eq!(a.p & 0x7d, p & 0x7d);
                assert_eq!(bc - ac, 20 - if reload.is_some() { 5 } else { 0 });
                for i in 0..amem.len() {
                    if [0x1fa0, 0x1fa1, 0x1fa4, 0x1fa5].contains(&i) {
                        assert_eq!(amem[i], 0xa5);
                    } else {
                        assert_eq!(amem[i], bmem[i]);
                    }
                }
            }
        }
    }
}

#[test]
fn direct_multi_copy_evidence_rejects_unsafe_orders_and_stale_restore() {
    let moves = [((true, 6), 32, 2), ((true, 2), 36, 4)];
    let valid = [0xa3, 2, 0x83, 4, 0xa3, 6, 0x83, 2, 0xa3, 4];
    assert_eq!(
        multi_word_edge::schedule(&valid, &moves, true),
        Some(vec![1, 0])
    );
    for bytes in [
        vec![0xa3, 6, 0x83, 2, 0xa3, 2, 0x83, 4],
        vec![0xa3, 2, 0x83, 4, 0xa3, 6, 0x83, 2, 0xa3, 2],
        valid[..8].to_vec(),
    ] {
        assert!(multi_word_edge::schedule(&bytes, &moves, true).is_none());
    }
    let cyclic = [((true, 4), 32, 2), ((true, 2), 36, 4)];
    assert!(multi_word_edge::schedule(&valid, &cyclic, true).is_none());
}
