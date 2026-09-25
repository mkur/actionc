mod support;
use actionc_vm::native65816::{Inputs, Machine};
use support::{context::*, *};

#[test]
fn incremental_argument_pushes_survive_irq_and_nmi_at_each_instruction() {
    for &(ty, width) in &[("CARD", 2u8)] {
        let source = format!(
            "MODULE TEST\nBYTE irqAck=$7800\n{ty} scratch\nCARD FUNC Echo(BYTE a CARD b BYTE c) RETURN(CARD(a)+b+CARD(c))\nCARD FUNC Forward(CARD value) RETURN(Echo(BYTE(value),value,BYTE(value)))\nCARD FUNC Dispatch(CARD saved BYTE reason) scratch=Forward({ty}(7)) irqAck=1 RETURN(saved)\nPROC Task({ty} POINTER argument) argument^=Forward(argument^) RETURN\nPROC Main() RETURN\nENDMODULE\n"
        );
        for optimize in [false, true] {
            for domain in 0..2 {
                let mut h = ContextHarness::from_prepared(
                    &source,
                    optimize,
                    "Task",
                    &[0x7100, 0x7120],
                    prepare(&source, optimize),
                );
                let mut r = h.cpu.registers();
                r.a = h.first[domain].saved_s;
                h.cpu = Machine::start_at(r);
                let argument = 0x7100 + domain * 0x20;
                let value = (0x89abcdefu32 >> domain) & (u32::MAX >> (8 * (4 - width)));
                h.bus.ram[argument..argument + 4].copy_from_slice(&value.to_le_bytes());
                let address = routine(&h.image, "Forward");
                let segment = h
                    .image
                    .segments
                    .iter()
                    .find(|s| s.address == address)
                    .unwrap();
                let jsl = forwarding::instructions(&segment.bytes)
                    .keys()
                    .copied()
                    .find(|&at| segment.bytes[at] == 0x22)
                    .unwrap();
                let end = address + jsl as u32;
                let construction = forwarding::instructions(&segment.bytes)
                    .keys()
                    .copied()
                    .filter(|&at| at < jsl && segment.bytes[at] == 0x5c)
                    .last()
                    .unwrap()
                    + 4;
                assert!(segment.bytes[construction..jsl].contains(&0x48));
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            100_000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary()
                                && c.pc() == address + construction as u32
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
                            assert!(
                                restored,
                                "{ty}/{optimize}/{domain}/{mask}/{nmi}/{:06x}",
                                at.pc()
                            );
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
                assert_eq!(
                    h.bus.value(argument as u32, width.into()),
                    (value + 2 * (value & 255)) & 65535
                );
            }
        }
    }
}
