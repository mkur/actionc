mod support;
use actionc::mir65816::{emit, image::Image, o65 as format};
use actionc_vm::native65816::{Inputs, Machine};
use std::{collections::BTreeSet, path::Path};
use support::*;

fn execute(h: &mut Harness, cv: u8) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    for _ in 0..100_000 {
        if h.cpu.is_stopped() {
            return seen;
        }
        if h.cpu.is_instruction_boundary()
            && h.bus
                .forwarded_words
                .parameter_words
                .iter()
                .any(|s| s.producer == h.cpu.pc())
        {
            let mut r = h.cpu.registers();
            r.p = (r.p & !0x41) | cv;
            h.cpu = Machine::start_at(r);
        }
        if let Some(site) = parameter_forwarding::reached(&h.cpu, &h.bus) {
            site.assert_live(&h.cpu, &h.bus);
            assert_eq!(h.cpu.registers().p & 0x41, cv);
            seen.insert(site.consumer);
        }
        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
    }
    panic!("parameter forwarding budget exhausted")
}

#[test]
fn typed_parameter_probes_execute_both_shapes_flat_and_at_two_o65_placements() {
    let source = fixture("parameter_forwarding.act");
    for optimize in [false, true] {
        // Ordinary source paths execute too; optimized source need not retain a repeated load.
        let ordinary = compile(&source, optimize);
        let mut h = Harness::new(&ordinary, &caller(ordinary.entry), 0);
        h.bus.ram[context::symbol(&ordinary, "input") as usize] = 13;
        h.run();
        h.guards(0);
        assert_eq!(h.global(&ordinary, "pair", 2), 26);
        assert_eq!(h.global(&ordinary, "bridge", 2), 26);
        let p = parameter_forwarding::prepared(&source, optimize);
        let c = p.compile(&layout()).unwrap();
        let image = Image::from_json(&c.image.to_json().unwrap()).unwrap();
        assert_eq!(
            image.to_json().unwrap(),
            parameter_forwarding::prepared(&source.replace('\n', "\r\n"), optimize)
                .compile(&layout())
                .unwrap()
                .image
                .to_json()
                .unwrap()
        );
        let index = forwarding::compiled(&p, &c);
        assert_eq!(index.parameter_words.len(), 2);
        assert_eq!(
            index
                .parameter_words
                .iter()
                .map(|s| s.bridge)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([false, true])
        );
        for s in &index.parameter_words {
            let r = p.mir.routines.iter().find(|r| r.id == s.routine).unwrap();
            assert_eq!(s.parameter, r.frame.parameters[1].param);
        }
        let m = emit::materialize(&p.mir).unwrap();
        let templates = forwarding::index(&p.mir, &m, |id| {
            0x10000 * (1 + m.routines.iter().position(|r| r.id == id).unwrap() as u32)
        });
        let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
        assert_eq!(
            bytes,
            parameter_forwarding::prepared(&source.replace('\n', "\r\n"), optimize)
                .compile_o65(&Default::default())
                .unwrap()
                .bytes
        );
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
                        for s in &h.bus.forwarded_words.parameter_words {
                            for i in 0..s.bytes.len() {
                                h.bus.ram[s.producer as usize + i] ^= 0x80;
                                assert!(!s.valid(&h.bus));
                                h.bus.ram[s.producer as usize + i] ^= 0x80;
                            }
                        }
                        let seen = execute(&mut h, cv);
                        assert_eq!(
                            seen,
                            h.bus
                                .forwarded_words
                                .parameter_words
                                .iter()
                                .map(|s| s.consumer)
                                .collect()
                        );
                        h.guards(mask);
                        for name in ["pair", "bridge"] {
                            assert_eq!(
                                h.bus.value(object(name), 2),
                                u32::from(input.wrapping_mul(2))
                            );
                        }
                    }
                }
            }
        }
        if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
            let stem = Path::new(&dir).join(format!("parameter-forwarding-{optimize}"));
            std::fs::write(stem.with_extension("a816.json"), image.to_json().unwrap()).unwrap();
            std::fs::write(stem.with_extension("o65"), bytes).unwrap();
            std::fs::write(stem.with_extension("proof.txt"),format!("{:?}\ninputs=0,1,32767,32768,65535; I=0,4; CV=0,1,64,65; placements=flat,o65-0,o65-1\n",index.parameter_words)).unwrap();
        }
    }
}
