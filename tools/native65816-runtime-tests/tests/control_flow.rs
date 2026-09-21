mod support;
use actionc_vm::native65816::{Inputs, Machine, Registers};
use support::*;

#[test]
fn redundant_native_rep_omission_preserves_hidden_lane_flags_and_memory() {
    let before = assemble("rep #$20\nsta 2,s\nstp\nnop", 0x040000);
    let after = assemble("sta 2,s\nstp\nnop", 0x040000);
    assert_eq!(&before[..2], &[0xc2, 0x20]);
    assert_eq!(&before[2..], after);
    equivalent_without_transfer(&before, &after, 3);
}

#[test]
fn adjacent_jml_omission_preserves_live_a_flags_and_memory() {
    let before = assemble("jml next\nnext: sta 2,s\nstp\nnop", 0x040000);
    let after = assemble("sta 2,s\nstp\nnop", 0x040000);
    assert_eq!(&before[..4], &[0x5c, 4, 0, 4]);
    assert_eq!(&before[4..], after);
    equivalent_without_transfer(&before, &after, 4);
}

fn equivalent_without_transfer(before: &[u8], after: &[u8], saved: u64) {
    for a in [0u16, 0xff, 0x100, 0x8000, 0xabcd, 0xffff] {
        for p in (0..=255u8).filter(|p| p & 0x38 == 0) {
            let initial = Registers {
                a,
                x: 0x5678,
                y: 0x9abc,
                s: 0x5fe0,
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
                let mut cpu = Machine::start_at(initial);
                assert!(
                    cpu.run_until(&mut bus, 100, |_| Inputs::default(), |cpu| cpu.is_stopped())
                        .unwrap()
                );
                let r = cpu.registers();
                assert_eq!(
                    (r.a, r.x, r.y, r.s, r.d, r.dbr, r.p, r.emulation_mode),
                    (
                        initial.a,
                        initial.x,
                        initial.y,
                        initial.s,
                        initial.d,
                        initial.dbr,
                        initial.p,
                        initial.emulation_mode
                    )
                );
                (cpu.cycles(), bus.writes)
            };
            let (bc, bw) = run(before);
            let (ac, aw) = run(after);
            assert_eq!(bc - ac, saved);
            assert_eq!(bw, aw);
        }
    }
}
