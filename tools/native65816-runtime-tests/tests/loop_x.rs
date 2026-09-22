mod support;
use actionc::mir65816::{
    self, Mir65816Op, Mir65816Terminator, Mir65816Value, emit, image::Image, o65 as format,
};
use actionc::nir::NirCompareOp;
use actionc_vm::native65816::{Inputs, Machine};
use support::*;

const SOURCE: &str = "CARD input,result CARD FUNC Rotation(CARD x) CARD a,b,c,i a=x b=x+1 FOR i=0 TO 7 DO c=a a=b b=c+1 OD RETURN(a+b) PROC Main() result=Rotation(input) RETURN";
fn prepared(
    optimize: bool,
    le: bool,
    bound: u16,
    initial: u16,
    reverse: bool,
) -> actionc::compiler::native65816::Prepared {
    let mut p = coalescing::prepared(SOURCE, optimize);
    let r = p
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Rotation")
        .unwrap();
    let header = r
        .blocks
        .iter()
        .find(|b| b.ops.len() == 1 && matches!(b.ops[0], Mir65816Op::Compare { .. }))
        .unwrap()
        .id;
    let body = r
        .blocks
        .iter()
        .find(|b| {
            matches!(&b.terminator,Mir65816Terminator::Goto(e) if e.target==header)
                && !b.ops.is_empty()
                && b.id != r.blocks[0].id
        })
        .unwrap()
        .id;
    for b in &mut r.blocks {
        if b.id == header {
            let Mir65816Op::Compare {
                operation, right, ..
            } = &mut b.ops[0]
            else {
                panic!()
            };
            *operation = if le {
                NirCompareOp::Le
            } else {
                NirCompareOp::Lt
            };
            *right = Mir65816Value::U16(bound);
            if reverse {
                let Mir65816Terminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } = &mut b.terminator
                else {
                    panic!()
                };
                std::mem::swap(then_edge, else_edge);
            }
        } else if b.id != body {
            if let Mir65816Terminator::Goto(e) = &mut b.terminator {
                if e.target == header {
                    *e.args.last_mut().unwrap() = Mir65816Value::U16(initial);
                }
            }
        }
    }
    mir65816::verify_program(&p.mir).unwrap();
    p
}
fn step(h: &mut Harness) {
    loop {
        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
        if h.cpu.is_instruction_boundary() || h.cpu.is_stopped() {
            break;
        }
    }
}
#[test]
fn selected_words_bounds_wrap_and_relocated_images_execute_with_checked_relations() {
    let mut runs = 0;
    for optimize in [false, true] {
        for (le, k, start, reverse, n) in [
            (false, 0, 0, false, 0),
            (false, 1, 0, false, 1),
            (true, 0, 0, false, 1),
            (true, 1, 0, false, 2),
            (false, 0x7fff, 0x7ffe, false, 1),
            (true, 0x7fff, 0x7ffe, false, 2),
            (false, 0x8000, 0x7fff, false, 1),
            (true, 0x8000, 0x7fff, false, 2),
            (false, 0xfffe, 0xfffd, false, 1),
            (true, 0xfffe, 0xfffd, false, 2),
            (false, 0xffff, 0xfffe, false, 1),
            (true, 0, 0xffff, true, 1),
        ] {
            let p = prepared(optimize, le, k, start, reverse);
            let c = p.compile(&layout()).unwrap();
            let image = Image::from_json(&c.image.to_json().unwrap()).unwrap();
            let index = forwarding::compiled(&p, &c);
            assert_eq!(index.x_words.len(), 1);
            let plain = emit::materialize(&p.mir).unwrap();
            let (traced, _) = emit::proof::materialize_with_trace(&p.mir).unwrap();
            for (a, b) in plain.routines.iter().zip(&traced.routines) {
                assert_eq!(a.code.bytes, b.code.bytes);
                assert_eq!(a.frame.temps, b.frame.temps);
            }
            let templates = forwarding::index(&p.mir, &plain, |id| {
                0x10000 * (1 + plain.routines.iter().position(|m| m.id == id).unwrap() as u32)
            });
            let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..3 {
                let loaded = (variant > 0).then(|| {
                    format::relocate(
                        &bytes,
                        &o65::placement(&bytes, variant - 1, vec![o65::fault(variant - 1)]),
                    )
                    .unwrap()
                });
                let entry = loaded.as_ref().map_or(image.entry, |l| l.entry());
                let caller = caller(entry);
                for (input, mask) in [(0u16, 0), (0x7fff, 4), (0x8000, 0), (0xffff, 4)] {
                    let mut h = loaded.as_ref().map_or_else(
                        || Harness::new(&image, &caller, mask),
                        |l| Harness::new_o65(l, &caller, mask),
                    );
                    h.bus.forwarded_words = loaded
                        .as_ref()
                        .map_or_else(|| index.clone(), |l| forwarding::relocated(&templates, l));
                    let symbol = |name| {
                        loaded
                            .as_ref()
                            .map_or_else(|| context::symbol(&image, name), |l| o65::object(l, name))
                    };
                    let at = symbol("input") as usize;
                    h.bus.ram[at..at + 2].copy_from_slice(&input.to_le_bytes());
                    let site = h.bus.forwarded_words.x_words[0].clone();
                    let mut counts = [0; 3];
                    let mut saw_stale = false;
                    for _ in 0..10000 {
                        if h.cpu.is_stopped() {
                            break;
                        }
                        let pc = h.cpu.pc();
                        if pc == site.compare || pc == site.load {
                            site.assert_live(&h.cpu, &h.bus);
                            let r = h.cpu.registers();
                            let cycles = h.cpu.cycles();
                            step(&mut h);
                            let a = h.cpu.registers();
                            if pc == site.compare {
                                counts[0] += 1;
                                assert_eq!(h.cpu.cycles() - cycles, 3);
                                assert_eq!(
                                    (a.a, a.x, a.y, a.p & 0x44),
                                    (r.a, r.x, r.y, r.p & 0x44)
                                );
                                assert_eq!(a.p & 1 != 0, r.x >= site.threshold);
                                assert_eq!(a.p & 2 != 0, r.x == site.threshold);
                                assert_eq!(
                                    a.p & 0x80 != 0,
                                    r.x.wrapping_sub(site.threshold) & 0x8000 != 0
                                );
                            } else {
                                counts[1] += 1;
                                assert_eq!(h.cpu.cycles() - cycles, 2);
                                assert_eq!(
                                    (a.a, a.x, a.y, a.p & 0x45),
                                    (r.x, r.x, r.y, r.p & 0x45)
                                );
                            }
                            continue;
                        }
                        if site.refresh.contains(&pc) {
                            counts[2] += 1;
                            let r = h.cpu.registers();
                            let value = h.bus.value(homes::address(r.s, r.d, site.home), 2) as u16;
                            assert_eq!(r.a, value);
                            saw_stale |= r.x != value;
                            let memory = h.bus.ram[0x2000..0x6000].to_vec();
                            step(&mut h);
                            let a = h.cpu.registers();
                            assert_eq!(
                                (a.a, a.x, a.y, a.s, a.d, a.p),
                                (r.a, value, r.y, r.s, r.d, r.p)
                            );
                            assert_eq!(h.bus.ram[0x2000..0x6000], memory);
                            continue;
                        }
                        step(&mut h);
                    }
                    assert!(h.cpu.is_stopped());
                    h.guards(mask);
                    assert_eq!(counts, [n + 1, n, n + 1]);
                    assert!(saw_stale);
                    assert_eq!(
                        h.bus.value(symbol("result"), 2),
                        u32::from(input.wrapping_mul(2).wrapping_add(1 + n))
                    );
                    runs += 1;
                }
            }
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("loop-x-boundaries.json"),
            format!("{{\"runs\":{runs},\"placements\":3,\"checked_cpu_flags_and_mirror\":true}}\n"),
        )
        .unwrap();
    }
}

#[test]
fn observers_reject_mutated_encodings_tails_thresholds_and_branch_polarity() {
    let p = prepared(true, true, 7, 0, false);
    let c = p.compile(&layout()).unwrap();
    let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
    h.bus.forwarded_words = forwarding::compiled(&p, &c);
    let site = h.bus.forwarded_words.x_words[0].clone();
    while h.cpu.pc() != site.compare {
        step(&mut h);
        assert!(!h.cpu.is_stopped());
    }
    assert!(comparison::fused_window(&h.cpu, &h.bus, &c.image.routines).is_some());
    for pc in [
        site.compare,
        site.compare + 1,
        site.compare + 2,
        site.load,
        site.load + 3,
        site.refresh[0],
        site.refresh[1],
        site.refresh[1] - 1,
    ] {
        let mut bus = h.bus.clone();
        bus.ram[pc as usize] ^= 1;
        assert!(!site.valid(&bus));
        assert!(comparison::fused_window(&h.cpu, &bus, &c.image.routines).is_none());
    }
    let mut bus = h.bus.clone();
    bus.ram[site.compare as usize + 3] ^= 0x20;
    assert!(comparison::fused_window(&h.cpu, &bus, &c.image.routines).is_none());
    let mut bad = site.clone();
    bad.home += 2;
    assert!(!bad.valid(&h.bus));
    let mut bad = site.clone();
    bad.threshold += 1;
    assert!(!bad.valid(&h.bus));
    let mut r = h.cpu.registers();
    r.x ^= 1;
    let cpu = Machine::start_at(r);
    assert!(std::panic::catch_unwind(|| site.assert_live(&cpu, &h.bus)).is_err());
}
