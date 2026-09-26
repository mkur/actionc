mod support;
use actionc::mir65816::o65 as format;
use actionc_vm::native65816::Access;
use support::*;

#[test]
fn native_unsigned_casts_and_signed_fallbacks_preserve_bits_and_exact_extents() {
    for (from, from_width, signed) in [
        ("CARD", 2usize, false),
        ("SIZE", 3, false),
        ("LONGCARD", 4, false),
        ("INT", 2, true),
        ("LONGINT", 4, true),
    ] {
        for (to, to_width) in [
            ("BYTE", 1usize),
            ("CARD", 2),
            ("SIZE", 3),
            ("LONGCARD", 4),
            ("LONGINT", 4),
        ] {
            let source = format!(
                "{from} input=$7100,original=$7130 {to} result=$7120 {to} FUNC Convert({from} v) {from} saved {to} out saved=v out={to}(saved) original=saved RETURN(out) PROC Main() result=Convert(input) RETURN"
            );
            for optimize in [false, true] {
                let p = prepare(&source, optimize);
                let c = p.compile(&layout()).unwrap();
                assert_eq!(
                    c.image.to_json().unwrap(),
                    compile(&source.replace('\n', "\r\n"), optimize)
                        .to_json()
                        .unwrap()
                );
                let object = p.compile_o65(&Default::default()).unwrap().bytes;
                for variant in 0..3 {
                    let loaded = (variant > 0).then(|| {
                        format::relocate(
                            &object,
                            &o65::placement(&object, variant - 1, vec![o65::fault(variant - 1)]),
                        )
                        .unwrap()
                    });
                    for value in [
                        0u32, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff, 0xffffff, 0x80000000,
                        0xffffffff,
                    ] {
                        let source_mask = u32::MAX >> (8 * (4 - from_width));
                        let destination_mask = u32::MAX >> (8 * (4 - to_width));
                        let bits = value & source_mask;
                        let extended = if signed && bits & (1 << (8 * from_width - 1)) != 0 {
                            bits | !source_mask
                        } else {
                            bits
                        };
                        let mask = if value & 1 == 0 { 0 } else { 4 };
                        let mut h = if let Some(l) = &loaded {
                            Harness::new_o65(l, &caller(l.entry()), mask)
                        } else {
                            Harness::new(&c.image, &caller(c.image.entry), mask)
                        };
                        for at in [0x7100, 0x7120, 0x7130] {
                            h.bus.ram[at - 1..at + 5].fill(0xa5);
                        }
                        h.bus.ram[0x7100..0x7100 + from_width]
                            .copy_from_slice(&value.to_le_bytes()[..from_width]);
                        h.bus.watched.extend(0x70ff..0x7100 + from_width as u32 + 1);
                        h.run();
                        h.guards(mask);
                        assert_eq!(
                            h.bus.value(0x7120, to_width),
                            extended & destination_mask,
                            "{from}->{to}/{value:x}/{optimize}"
                        );
                        assert_eq!(h.bus.value(0x7130, from_width), bits);
                        for (at, width) in [
                            (0x7100, from_width),
                            (0x7120, to_width),
                            (0x7130, from_width),
                        ] {
                            assert_eq!((h.bus.ram[at - 1], h.bus.ram[at + width]), (0xa5, 0xa5));
                        }
                        assert_eq!(
                            h.bus
                                .trace
                                .iter()
                                .map(|&(_, a, k)| (a, k))
                                .collect::<Vec<_>>(),
                            (0x7100..0x7100 + from_width as u32)
                                .map(|a| (a, Access::Read))
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn unsigned_cast_windows_survive_irq_nmi_and_reentrant_calls() {
    for (from, to) in [
        ("CARD", "LONGCARD"),
        ("SIZE", "LONGCARD"),
        ("LONGCARD", "SIZE"),
        ("LONGCARD", "CARD"),
    ] {
        let source = format!(
            "MODULE TEST BYTE irqAck=$7800 {to} scratch {to} FUNC Work({from} v) RETURN({to}(v)) CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work({from}($89abcdef)) irqAck=1 RETURN(saved) PROC Task(LONGCARD POINTER argument) argument^=LONGCARD(Work({from}(argument^))) RETURN PROC Main() RETURN ENDMODULE"
        );
        check_interrupt_reentry(&source);
    }
}

fn check_interrupt_reentry(source: &str) {
    use actionc_vm::native65816::{Inputs, Machine};
    use support::context::*;
    for optimize in [false, true] {
        for domain in 0..2 {
            let mut h = ContextHarness::from_prepared(
                source,
                optimize,
                "Task",
                &[0x7100, 0x7120],
                prepare(source, optimize),
            );
            let mut r = h.cpu.registers();
            r.a = h.first[domain].saved_s;
            h.cpu = Machine::start_at(r);
            h.bus.ram[0x7100 + domain * 0x20] = 1;
            let address = routine(&h.image, "Work");
            let end = address
                + h.image
                    .routines
                    .iter()
                    .find(|r| r.address == address)
                    .unwrap()
                    .size;
            assert!(
                h.cpu
                    .run_until(
                        &mut h.bus,
                        100_000,
                        |_| Inputs::default(),
                        |c| c.is_instruction_boundary() && c.pc() == address
                    )
                    .unwrap()
            );
            let mut sites = 0;
            while (address..end).contains(&h.cpu.pc()) {
                let checkpoint = h.cpu.clone();
                let memory = h.bus.clone();
                for mask in [0, 4] {
                    let mut r = checkpoint.registers();
                    r.p = (r.p & !4) | mask;
                    let at = Machine::start_at(r);
                    let mut reference = at.clone();
                    let mut rb = memory.clone();
                    reference.tick(&mut rb, Inputs::default()).unwrap();
                    while !reference.is_instruction_boundary() {
                        reference.tick(&mut rb, Inputs::default()).unwrap();
                    }
                    for nmi in [false, true] {
                        h.cpu = at.clone();
                        h.bus = memory.clone();
                        let mut ack = false;
                        let mut restored = false;
                        for _ in 0..100_000 {
                            let writes = h.bus.writes.len();
                            h.tick(Inputs {
                                irq: !nmi && !ack,
                                nmi: nmi && !ack,
                                ..Default::default()
                            });
                            ack |= h.bus.writes[writes..]
                                .iter()
                                .any(|&(a, _)| a == if nmi { NMI_ACK } else { IRQ_ACK });
                            if h.cpu.is_instruction_boundary()
                                && h.cpu.pc() == reference.pc()
                                && h.cpu.registers().d == r.d
                                && (ack || !nmi && mask == 4)
                            {
                                assert_eq!(h.cpu.registers(), reference.registers());
                                assert_eq!(
                                    // Sampling completes TCS/RTL first. Released
                                    // bytes may then hold the interrupt frame.
                                    &h.bus.ram[usize::from(reference.registers().s) + 1
                                        ..0x5000 + domain * 0x1000],
                                    &rb.ram[usize::from(reference.registers().s) + 1
                                        ..0x5000 + domain * 0x1000]
                                );
                                let dp = usize::from(r.d);
                                assert_eq!(&h.bus.ram[dp..dp + 256], &rb.ram[dp..dp + 256]);
                                assert_eq!(ack, nmi || mask == 0);
                                restored = true;
                                break;
                            }
                            assert!(!h.cpu.is_stopped());
                        }
                        assert!(restored, "{optimize}/{domain}/{mask}/{nmi}/{:06x}", at.pc());
                    }
                }
                h.cpu = checkpoint;
                h.bus = memory;
                h.tick(Inputs::default());
                while !h.cpu.is_instruction_boundary() {
                    h.tick(Inputs::default());
                }
                sites += 1;
            }

            assert!(sites >= 8);
            h.run();
            h.guards();
        }
    }
}
