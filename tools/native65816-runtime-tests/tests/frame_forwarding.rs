mod support;
use actionc::mir65816::o65 as format;
use actionc_vm::native65816::Inputs;
use std::{collections::BTreeSet, path::Path};
use support::*;

fn execute(h: &mut Harness) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    for _ in 0..100_000 {
        if h.cpu.is_stopped() {
            return seen;
        }
        if let Some(site) = frame_forwarding::reached(&h.cpu, &h.bus) {
            site.assert_live(&h.cpu, &h.bus);
            seen.insert(site.consumer);
        }
        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
    }
    panic!("frame forwarding budget exhausted")
}

#[test]
fn raw_and_optimized_frame_captures_execute_flat_and_relocated_with_both_irq_masks() {
    let source = fixture("frame_forwarding.act");
    for optimize in [false, true] {
        let (image, index) = forwarding::compile(&source, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&source.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        assert!(!index.frame_words.is_empty());
        let (bytes, templates) = forwarding::o65(&source, optimize);
        assert_eq!(
            bytes,
            forwarding::o65(&source.replace('\n', "\r\n"), optimize).0
        );
        let mut seen = BTreeSet::new();
        let mut nz = BTreeSet::new();
        for input in [0u16, 1, 13, 0x7fff, 0x8000, 0xffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                h.bus.forwarded_words = index.clone();
                let at = context::symbol(&image, "input") as usize;
                h.bus.ram[at..at + 2].copy_from_slice(&input.to_le_bytes());
                // Mutating every nonrelocatable proof byte must invalidate it.
                for s in &index.frame_words {
                    assert!(s.valid(&h.bus));
                    for (i, b) in s.bytes.iter().enumerate() {
                        if b.is_some() {
                            h.bus.ram[s.producer as usize + i] ^= 0x80;
                            assert!(!s.valid(&h.bus));
                            h.bus.ram[s.producer as usize + i] ^= 0x80;
                        }
                    }
                }
                for _ in 0..100_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    if let Some(s) = frame_forwarding::reached(&h.cpu, &h.bus) {
                        s.assert_live(&h.cpu, &h.bus);
                        seen.insert(s.consumer);
                        nz.insert(h.cpu.registers().p & 0x82);
                    }
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                }
                assert!(h.cpu.is_stopped());
                h.guards(mask);
                assert_eq!(
                    h.global(&image, "result", 2),
                    u32::from(input.wrapping_add(1))
                );
                assert_eq!(
                    h.global(&image, "rotated", 2),
                    u32::from(input.wrapping_mul(2).wrapping_add(9))
                );
                for variant in 0..2 {
                    let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
                    let loaded = format::relocate(&bytes, &placement).unwrap();
                    let mut h = Harness::new_o65(&loaded, &caller(loaded.entry()), mask);
                    h.bus.forwarded_words = forwarding::relocated(&templates, &loaded);
                    let at = o65::object(&loaded, "input") as usize;
                    h.bus.ram[at..at + 2].copy_from_slice(&input.to_le_bytes());
                    let reached = execute(&mut h);
                    assert_eq!(
                        reached,
                        h.bus
                            .forwarded_words
                            .frame_words
                            .iter()
                            .map(|s| s.consumer)
                            .collect()
                    );
                    h.guards(mask);
                    assert_eq!(
                        h.bus.value(o65::object(&loaded, "result"), 2),
                        u32::from(input.wrapping_add(1))
                    );
                    assert_eq!(
                        h.bus.value(o65::object(&loaded, "rotated"), 2),
                        u32::from(input.wrapping_mul(2).wrapping_add(9))
                    );
                    o65::record(
                        "frame-forwarding",
                        optimize,
                        &bytes,
                        &placement,
                        &loaded,
                        h.cpu.cycles(),
                        None,
                    );
                }
            }
        }
        assert_eq!(seen, index.frame_words.iter().map(|s| s.consumer).collect());
        assert_eq!(nz, BTreeSet::from([0, 2, 0x80]));
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            let stem = Path::new(&directory).join(format!("frame-forwarding-{optimize}"));
            std::fs::write(stem.with_extension("act"), &source).unwrap();
            std::fs::write(stem.with_extension("a816.json"), image.to_json().unwrap()).unwrap();
            std::fs::write(stem.with_extension("proof.json"),serde_json::to_vec_pretty(&serde_json::json!({"sites":seen,"nz":nz,"inputs":[0,1,13,32767,32768,65535],"irq_masks":[0,4],"o65_placements":2})).unwrap()).unwrap();
        }
    }
}
