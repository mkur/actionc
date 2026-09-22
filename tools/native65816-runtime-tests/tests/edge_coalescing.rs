mod support;
use actionc::mir65816::{emit, image::Image, o65 as format};
use actionc_vm::native65816::{Inputs, Machine, Registers};
use std::{collections::BTreeSet, path::Path};
use support::*;

fn execute(h: &mut Harness, image: &Image, cv: u8) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    let mut pending: Option<(u32, Registers, Vec<u8>, Vec<u16>, Vec<u16>, bool)> = None;
    for _ in 0..100_000 {
        if h.cpu.is_stopped() {
            assert!(pending.is_none());
            return seen;
        }
        if h.cpu.is_instruction_boundary() {
            if let Some((target, before, stack, values, dests, x_tail)) = pending.take() {
                if h.cpu.pc() == target {
                    let mut expected: Registers = before;
                    expected.pc = target as u16;
                    expected.pbr = (target >> 16) as u8;
                    expected.a = *values.last().unwrap();
                    if x_tail {
                        expected.x = expected.a;
                    }
                    expected.p = (expected.p & !0x82)
                        | if expected.a == 0 { 2 } else { 0 }
                        | if expected.a & 0x8000 != 0 { 0x80 } else { 0 };
                    assert_eq!(h.cpu.registers(), expected);
                    let mut wanted: Vec<u8> = stack;
                    for (&d, &v) in dests.iter().zip(&values) {
                        let at = homes::address(before.s, before.d, d) as usize - 0x2000;
                        wanted[at..at + 2].copy_from_slice(&v.to_le_bytes());
                    }
                    assert_eq!(&h.bus.ram[0x2000..0x6000], wanted);
                } else {
                    pending = Some((target, before, stack, values, dests, x_tail));
                }
            }
            if let Some(w) = word_edge::reached(&h.cpu, &h.bus, &image.routines) {
                if w.form == word_edge::Form::Direct && w.order.len() < w.moves.len() {
                    assert!(pending.is_none());
                    seen.insert(h.cpu.pc());
                    let mut r = h.cpu.registers();
                    r.p = (r.p & !0x41) | cv;
                    h.cpu = Machine::start_at(r);
                    let values: Vec<u16> = w
                        .moves
                        .iter()
                        .map(|&((stack, v), _, _)| {
                            if stack {
                                h.bus.value(homes::address(r.s, r.d, v), 2) as u16
                            } else {
                                v
                            }
                        })
                        .collect();
                    let dests: Vec<u16> = w.moves.iter().map(|m| m.2).collect();
                    pending = Some((
                        w.target,
                        r,
                        h.bus.ram[0x2000..0x6000].to_vec(),
                        values,
                        dests,
                        h.bus
                            .forwarded_words
                            .x_words
                            .iter()
                            .any(|x| x.refresh.iter().any(|pc| w.sites.contains(pc))),
                    ));
                }
            }
        }
        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
    }
    panic!("coalesced execution budget")
}

#[test]
fn compatible_homes_execute_serialized_and_relocated_in_both_frontend_modes() {
    let source = fixture("frame_forwarding.act");
    for optimize in [false, true] {
        let p = coalescing::prepared(&source, optimize);
        let c = p.compile(&layout()).unwrap();
        let image = Image::from_json(&c.image.to_json().unwrap()).unwrap();
        assert_eq!(
            image.to_json().unwrap(),
            coalescing::prepared(&source.replace('\n', "\r\n"), optimize)
                .compile(&layout())
                .unwrap()
                .image
                .to_json()
                .unwrap()
        );
        let index = forwarding::compiled(&p, &c);
        let coalesced: Vec<_> = index
            .multi_words
            .iter()
            .filter(|s| s.form == word_edge::Form::Direct && s.order.len() < s.moves.len())
            .collect();
        assert_eq!(coalesced.len(), 1);
        assert_eq!(coalesced[0].moves.len() - coalesced[0].order.len(), 2);
        let m = emit::materialize(&p.mir).unwrap();
        let templates = forwarding::index(&p.mir, &m, |id| {
            0x10000 * (1 + m.routines.iter().position(|r| r.id == id).unwrap() as u32)
        });
        let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
        for input in [0u16, 1, 0x7fff, 0x8000, 0xffff] {
            for mask in [0, 4] {
                for cv in [0, 1, 0x40, 0x41] {
                    for variant in 0..3 {
                        let loaded = (variant > 0).then(|| {
                            format::relocate(
                                &bytes,
                                &o65::placement(&bytes, variant - 1, vec![o65::fault(variant - 1)]),
                            )
                            .unwrap()
                        });
                        let mut h = if let Some(l) = &loaded {
                            Harness::new_o65(l, &caller(l.entry()), mask)
                        } else {
                            Harness::new(&image, &caller(image.entry), mask)
                        };
                        h.bus.forwarded_words = loaded.as_ref().map_or_else(
                            || index.clone(),
                            |l| forwarding::relocated(&templates, l),
                        );
                        let object = |name| {
                            loaded.as_ref().map_or_else(
                                || context::symbol(&image, name),
                                |l| o65::object(l, name),
                            )
                        };
                        let at = object("input") as usize;
                        h.bus.ram[at..at + 2].copy_from_slice(&input.to_le_bytes());
                        // Decoder ranges use the actually placed routines, never old PCs.
                        let mut placed = image.clone();
                        if let Some(l) = &loaded {
                            for r in &mut placed.routines {
                                r.address = o65::routine(
                                    l,
                                    if r.name.to_ascii_lowercase().contains("rotation") {
                                        "Rotation"
                                    } else if r.name.to_ascii_lowercase().contains("capture") {
                                        "Capture"
                                    } else {
                                        "Main"
                                    },
                                );
                            }
                        }
                        assert_eq!(execute(&mut h, &placed, cv).len(), 1);
                        h.guards(mask);
                        assert_eq!(
                            h.bus.value(object("result"), 2),
                            u32::from(input.wrapping_add(1))
                        );
                        assert_eq!(
                            h.bus.value(object("rotated"), 2),
                            u32::from(input.wrapping_mul(2).wrapping_add(9))
                        );
                    }
                }
            }
        }
        if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
            let stem = Path::new(&dir).join(format!("edge-coalescing-{optimize}"));
            std::fs::write(stem.with_extension("a816.json"), image.to_json().unwrap()).unwrap();
            std::fs::write(stem.with_extension("o65"), bytes).unwrap();
            std::fs::write(stem.with_extension("proof.txt"),format!("{coalesced:#?}\nvalues=0,1,32767,32768,65535; I=0,4; CV=0,1,64,65; placements=flat,o65-0,o65-1\n")).unwrap();
        }
    }
}
