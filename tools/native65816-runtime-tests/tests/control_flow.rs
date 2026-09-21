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

#[test]
fn short_dispatch_encodings_preserve_flags_and_exact_taken_costs() {
    for (mnemonic, inverse, opcode, mask, set) in [
        ("bne", "beq", 0xd0, 2, false),
        ("beq", "bne", 0xf0, 2, true),
        ("bcc", "bcs", 0x90, 1, false),
        ("bcs", "bcc", 0xb0, 1, true),
    ] {
        // Force taken branches across a page while remaining in the same bank.
        let base = 0x0400fc;
        let before = assemble(
            &format!("{inverse} no\njml yes\nno: sta 2,s\nstp\nnop\nyes: sta 4,s\nstp\nnop"),
            base,
        );
        let after = assemble(
            &format!("{mnemonic} yes\nno: sta 2,s\nstp\nnop\nyes: sta 4,s\nstp\nnop"),
            base,
        );
        assert_eq!(&after[..2], &[opcode, 4]);
        for p in (0..=255u8).filter(|p| p & 0x38 == 0) {
            let truth = (p & mask != 0) == set;
            let run = |code: &[u8]| {
                let mut bus = Bus::new();
                bus.map(base, code, false);
                bus.map(0x4000, &[0xa5; 0x2000], true);
                let initial = Registers {
                    a: 0xab80,
                    x: 0x5678,
                    y: 0x9abc,
                    s: 0x5fe0,
                    d: 0x2000,
                    dbr: 0,
                    pbr: 4,
                    pc: base as u16,
                    p,
                    emulation_mode: false,
                };
                let mut cpu = Machine::start_at(initial);
                assert!(
                    cpu.run_until(&mut bus, 100, |_| Inputs::default(), |c| c.is_stopped())
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
                        false
                    )
                );
                assert_eq!(bus.value(0x5fe0 + if truth { 4 } else { 2 }, 2), 0xab80);
                (cpu.cycles(), bus.writes)
            };
            let (bc, bw) = run(&before);
            let (ac, aw) = run(&after);
            assert_eq!(bc - ac, if truth { 3 } else { 1 });
            assert_eq!(bw, aw);
        }
    }
}

#[test]
fn generated_short_and_long_dispatch_execute_copies_at_banked_image_and_o65_placements() {
    use actionc::{
        mir65816::{self, Mir65816Terminator, Mir65816Value, image::Image},
        nir::TempId,
        target::ByteSize,
    };
    for optimize in [false, true] {
        for count in [2u32, 16] {
            let mut p = edges::program(optimize, false);
            let r = p
                .mir
                .routines
                .iter_mut()
                .find(|r| r.name == "Work")
                .unwrap();
            let ty = r
                .temps
                .iter()
                .find(|(id, _)| *id == TempId(6))
                .unwrap()
                .1
                .clone();
            for n in 0..count - 2 {
                let id = TempId(10 + n);
                r.temps.push((id, ty.clone()));
                r.blocks[1].params.push((id, ByteSize::new(2)));
                let Mir65816Terminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } = &mut r.blocks[0].terminator
                else {
                    panic!()
                };
                then_edge.args.push(Mir65816Value::U16(0xa500 + n as u16));
                else_edge.args.push(Mir65816Value::U16(0x5a00 + n as u16));
            }
            mir65816::verify_program(&p.mir).unwrap();
            let mut options = layout();
            options.code_origin = 0x01fff0;
            let c = p.compile(&options).unwrap();
            let image = Image::from_json(&c.image.to_json().unwrap()).unwrap();
            let work = image.routines.iter().find(|r| r.name == "Work").unwrap();
            assert_eq!(work.address & 65535, 0);
            let m = c
                .machine
                .routines
                .iter()
                .find(|m| m.id.0 == work.id)
                .unwrap();
            assert_eq!(m.code.conditional_branches.len(), 1);
            assert_eq!(m.code.conditional_branches[0].short, count == 2);
            let templates = forwarding::compiled(&p, &c);
            let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..2 {
                let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
                let moved = mir65816::o65::relocate(&bytes, &placement).unwrap();
                for (a, b) in [(0u16, 0xffffu16), (0xffff, 0), (0x8000, 0x7fff)] {
                    for mask in [0, 4] {
                        for relocated in [false, true] {
                            let mut h = if relocated {
                                Harness::new_o65(&moved, &caller(moved.entry()), mask)
                            } else {
                                Harness::new(&image, &caller(image.entry), mask)
                            };
                            h.bus.forwarded_words = if relocated {
                                forwarding::relocated(&templates, &moved)
                            } else {
                                templates.clone()
                            };
                            h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                            h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                            for s in &h.bus.forwarded_words.dispatches {
                                let encoded = &h.bus.ram[s.at as usize..s.at as usize + 2];
                                if s.short {
                                    assert_eq!(
                                        encoded,
                                        &[
                                            s.predicate,
                                            (i64::from(s.target) - i64::from(s.at) - 2) as i8 as u8
                                        ]
                                    );
                                } else {
                                    assert_eq!(encoded, &[s.predicate ^ 0x20, 4]);
                                    assert_eq!(h.bus.value(s.at + 3, 3), s.target);
                                }
                            }
                            h.run();
                            h.guards(mask);
                            assert_eq!(h.bus.value(0x7200, 2), u32::from(a.abs_diff(b)));
                        }
                    }
                }
            }
        }
    }
}
