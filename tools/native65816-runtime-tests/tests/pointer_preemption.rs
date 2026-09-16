mod support;
use actionc::mir65816::image::TemporaryHome;
use actionc_vm::native65816::Inputs;
use std::collections::BTreeSet;
use support::{context::*, *};

const LISTS: [[u32; 3]; 3] = [
    [0x21ffff, 0x32fffc, 0x43fffe],
    [0x51ffff, 0x62fffc, 0x73fffe],
    [0x81ffff, 0x92fffc, 0xa3fffe],
];
fn machine(optimize: bool) -> ContextHarness {
    let mut h = ContextHarness::new(
        &fixture("pointer_preemption.act"),
        optimize,
        "Task",
        &[0x7100, 0x7120],
    );
    for (i, [left, item, right]) in LISTS.into_iter().enumerate() {
        for (node, next, prev) in [
            (left, item, 0xabcdef),
            (item, right, left),
            (right, 0xfedcba, item),
        ] {
            let mut data = [0xa5; 8];
            data[1..4].copy_from_slice(&next.to_le_bytes()[..3]);
            data[4..7].copy_from_slice(&prev.to_le_bytes()[..3]);
            h.bus.map(node - 1, &data, true);
        }
        if i < 2 {
            let job = 0x7100 + i * 0x20;
            h.bus.ram[job..job + 3].copy_from_slice(&item.to_le_bytes()[..3]);
            h.bus.ram[job + 4..job + 7]
                .copy_from_slice(&(0x7103u32 + (1 - i as u32) * 0x20).to_le_bytes()[..3]);
        } else {
            let irq = symbol(&h.image, "irqItem") as usize;
            h.bus.ram[irq..irq + 3].copy_from_slice(&item.to_le_bytes()[..3]);
        }
    }
    if optimize {
        let address = routine(&h.image, "Unlink");
        let map = h
            .image
            .routines
            .iter()
            .find(|r| r.address == address)
            .unwrap();
        assert_eq!(map.fixed_frame, 0);
        assert_eq!(map.temporaries.len(), 3);
        assert!(
            map.temporaries
                .iter()
                .all(|t| matches!(t.home, TemporaryHome::DirectPage { .. }))
        );
    }
    h
}
fn check(h: &ContextHarness) {
    h.guards();
    assert_eq!(h.bus.value(DONE, 2), 1);
    assert_eq!(h.bus.value(0x7103, 1), 1);
    assert_eq!(h.bus.value(0x7123, 1), 1);
    for [left, item, right] in LISTS {
        for (node, next, prev) in [
            (left, right, 0xabcdef),
            (item, right, left),
            (right, 0xfedcba, left),
        ] {
            assert_eq!(h.bus.value(node, 3), next);
            assert_eq!(h.bus.value(node + 3, 3), prev);
            assert_eq!(h.bus.value(node - 1, 1), 0xa5);
            assert_eq!(h.bus.value(node + 6, 1), 0xa5);
        }
    }
}
fn run(h: &mut ContextHarness, mut pending: bool, seed: Option<u64>) {
    let mut rng = seed.unwrap_or(1);
    let mut nmi_after = 0;
    for _ in 0..2_000_000 {
        if h.cpu.is_stopped() {
            check(h);
            return;
        }
        let mut nmi = false;
        if seed.is_some() && h.cpu.is_instruction_boundary() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            if rng & 31 == 0 && h.cpu.registers().p & 4 == 0 {
                pending = true;
            }
            if rng & 127 == 1 && h.cpu.cycles() >= nmi_after {
                nmi = true;
                nmi_after = h.cpu.cycles() + 250;
            }
        }
        let writes = h.bus.writes.len();
        h.tick(Inputs {
            irq: pending,
            nmi,
            ..Default::default()
        });
        if h.bus.writes[writes..].iter().any(|&(at, _)| at == IRQ_ACK) {
            pending = false;
        }
    }
    panic!("pointer preemption exceeded cycle budget");
}
#[test]
fn every_enabled_pointer_leaf_instruction_preserves_both_tasks_and_irq_scratch() {
    for optimize in [false, true] {
        let mut h = machine(optimize);
        let address = routine(&h.image, "Unlink");
        let end = address
            + h.image
                .routines
                .iter()
                .find(|r| r.address == address)
                .unwrap()
                .size;
        let mut seen = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            if h.cpu.is_instruction_boundary()
                && r.p & 4 == 0
                && [0x2000, 0x2100].contains(&r.d)
                && (address..end).contains(&h.cpu.pc())
                && seen.insert((r.d, h.cpu.pc()))
            {
                let cpu = h.cpu.clone();
                let bus = h.bus.clone();
                run(&mut h, true, None);
                h.cpu = cpu;
                h.bus = bus;
            }
            h.tick(Inputs::default());
        }
        check(&h);
        assert!(
            seen.len() >= 80,
            "only {} task/instruction sites",
            seen.len()
        );
        eprintln!(
            "pointer leaf optimize={optimize}: {} task/instruction IRQ sites",
            seen.len()
        );
    }
}
#[test]
fn pointer_leaf_survives_seeded_irq_and_nmi_schedules() {
    for optimize in [false, true] {
        for seed in [0x81620260916, 0x5eedcafe] {
            run(&mut machine(optimize), false, Some(seed));
        }
    }
}
