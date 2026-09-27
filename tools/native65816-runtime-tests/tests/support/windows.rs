use super::*;
pub fn check_work_interrupts(source: &str) {
    use super::context::*;
    use actionc_vm::native65816::{Inputs, Machine};
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
