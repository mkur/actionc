//! Native consumers at every reached cleanup/compare/store/return boundary.
mod support;
use actionc::mir65816::{Mir65816Op, Mir65816Value};
use actionc_vm::native65816::{Inputs, Machine};
use support::{context::*, *};

fn prepare_zero_shapes(source: &str, optimize: bool) -> actionc::compiler::native65816::Prepared {
    let mut p = prepare(source, optimize);
    // Raw lowering retains literal-zero widening. Keep raw source fallback
    // covered by call_flow; these verified MIR probes exercise the direct
    // word consumer in both compiler modes, without changing frontend rules.
    for r in &mut p.mir.routines {
        for block in &mut r.blocks {
            let mut i = 0;
            while i + 1 < block.ops.len() {
                let Mir65816Op::Cast {
                    dest,
                    from,
                    to,
                    value: Mir65816Value::U8(0),
                    ..
                } = block.ops[i]
                else {
                    i += 1;
                    continue;
                };
                if from.get() != 1 || to.get() != 2 {
                    i += 1;
                    continue;
                }
                let Mir65816Op::Compare { right, .. } = &mut block.ops[i + 1] else {
                    i += 1;
                    continue;
                };
                if *right != Mir65816Value::Temp(dest, to) {
                    i += 1;
                    continue;
                }
                *right = Mir65816Value::U16(0);
                block.ops.remove(i);
                r.temps.retain(|(id, _)| *id != dest);
            }
        }
    }
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
}

#[test]
fn native_zero_result_consumers_survive_irq_and_nmi_at_each_instruction() {
    result_consumers(false);
}

#[test]
fn native_local_result_consumers_survive_irq_and_nmi_at_each_instruction() {
    result_consumers(true);
}

fn result_consumers(store: bool) {
    // The observable store keeps this an ordinary call: this test targets
    // argument cleanup, while forwarding_wrappers covers terminal jumps.
    for &(ty, width) in &[
        ("BYTE", 1u8),
        ("CARD", 2u8),
        ("ADDRESS", 3u8),
        ("LONGCARD", 4u8),
    ] {
        if !store && width > 2 {
            continue;
        }
        let result = if store { ty } else { "BYTE" };
        let body = if store {
            "local=Echo(value) Echo(value) RETURN(local)"
        } else {
            "RETURN(Echo(value)=0)"
        };
        let locals = if store {
            format!("{ty} local")
        } else {
            String::new()
        };
        let source = format!(
            "MODULE TEST\nBYTE irqAck=$7800,entered\n{ty} scratch\n{ty} FUNC Echo({ty} value) RETURN(value)\n{result} FUNC Forward({ty} value) {locals} entered=1 {body}\nCARD FUNC Dispatch(CARD saved BYTE reason) scratch=Forward({ty}(7)) irqAck=1 RETURN(saved)\nPROC Task({ty} POINTER argument) argument^=Forward(argument^) RETURN\nPROC Main() RETURN\nENDMODULE\n"
        );
        for optimize in [false, true] {
            for domain in 0..2 {
                let mut h = ContextHarness::from_prepared(
                    &source,
                    optimize,
                    "Task",
                    &[0x7100, 0x7120],
                    prepare_zero_shapes(&source, optimize),
                );
                let mut r = h.cpu.registers();
                r.a = h.first[domain].saved_s;
                h.cpu = Machine::start_at(r);
                let argument = 0x7100 + domain * 0x20;
                let value: u32 = if domain == 0 { 0 } else { 1 << (8 * width - 1) };
                h.bus.ram[argument..argument + 4].copy_from_slice(&value.to_le_bytes());
                let address = routine(&h.image, "Forward");
                let segment = h
                    .image
                    .segments
                    .iter()
                    .find(|s| s.address == address)
                    .unwrap()
                    .clone();
                let jsl = forwarding::instructions(&segment.bytes)
                    .keys()
                    .copied()
                    .find(|&at| segment.bytes[at] == 0x22)
                    .unwrap();
                let decoded = forwarding::instructions(&segment.bytes);
                let end = address + segment.bytes.len() as u32;
                let mut saw_consumer = false;
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            100_000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == address + jsl as u32 + 4
                        )
                        .unwrap()
                );
                let mut sites = 0;
                while (address..end).contains(&h.cpu.pc()) {
                    let offset = (h.cpu.pc() - address) as usize;
                    assert!(decoded.contains_key(&offset));
                    saw_consumer |= segment.bytes[offset] == if store { 0x83 } else { 0xc9 };
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
                assert!(sites >= 8 && saw_consumer); // every reached boundary is suspended
                h.run();
                h.guards();
                assert_eq!(
                    h.bus.value(argument as u32, width.into()),
                    if store { value } else { u32::from(value == 0) }
                );
            }
        }
    }
}
