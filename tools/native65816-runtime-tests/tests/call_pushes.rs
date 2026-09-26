mod support;
use actionc_vm::native65816::{Inputs, Machine};
use support::{context::*, *};

#[test]
fn incremental_argument_pushes_survive_irq_and_nmi_at_each_instruction() {
    for &(ty, width) in &[("CARD", 2u8), ("ADDRESS", 3), ("LONGCARD", 4)] {
        for immediate in [false, true] {
            let source = format!(
                "MODULE TEST\nBYTE irqAck=$7800\n{ty} scratch\n{ty} FUNC Echo(BYTE a {ty} b BYTE c) RETURN({ty}(LONGCARD(a)+LONGCARD(b)+LONGCARD(c)))\n{ty} FUNC Forward({ty} value) RETURN(Echo(BYTE(value),value,BYTE(value)))\nCARD FUNC Dispatch(CARD saved BYTE reason) scratch=Forward({ty}(7)) irqAck=1 RETURN(saved)\nPROC Task({ty} POINTER argument) argument^=Forward(argument^) RETURN\nPROC Main() RETURN\nENDMODULE\n"
            );
            let source = if immediate {
                source.replace(
                    "Echo(BYTE(value),value,BYTE(value))",
                    &format!("Echo(BYTE(value),{ty}($1234),BYTE(value))"),
                )
            } else {
                source
            };
            for optimize in [false, true] {
                for domain in 0..2 {
                    let mut prepared = prepare(&source, optimize);
                    if immediate {
                        // Raw explicit casts retain a capture. Exercise the
                        // verifier-clean immediate ABI operand independently
                        // of whether the frontend folds that private cast.
                        let forward = prepared
                            .mir
                            .routines
                            .iter_mut()
                            .find(|r| r.name.to_ascii_uppercase().contains("FORWARD"))
                            .unwrap();
                        for op in forward.blocks.iter_mut().flat_map(|b| &mut b.ops) {
                            if let actionc::mir65816::Mir65816Op::Call { args, .. } = op {
                                args[1] = match width {
                                    2 => actionc::mir65816::Mir65816Value::U16(0x1234),
                                    3 => actionc::mir65816::Mir65816Value::U24(0x1234),
                                    _ => actionc::mir65816::Mir65816Value::U32(0x1234),
                                };
                            }
                        }
                        actionc::mir65816::verify_program(&prepared.mir).unwrap();
                    }
                    let mut h = ContextHarness::from_prepared(
                        &source,
                        optimize,
                        "Task",
                        &[0x7100, 0x7120],
                        prepared,
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
                    if immediate {
                        assert!(forwarding::instructions(&segment.bytes).keys().any(|&at| {
                            construction <= at && at < jsl && segment.bytes[at] == 0xf4
                        }));
                    }
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
                        (if immediate { 0x1234u32 } else { value }).wrapping_add(2 * (value & 255))
                            & (u32::MAX >> (8 * (4 - width)))
                    );
                }
            }
        }
    }
}

#[test]
fn symbolic_arguments_keep_relocations_and_bank_carry_at_two_placements() {
    let source = r#"BYTE ARRAY bytes(300) ADDRESS result=$7100 BYTE letter=$7103 ADDRESS FUNC Echo(ADDRESS p BYTE b LONGCARD n) RETURN(p) BYTE FUNC First(BYTE POINTER p) RETURN(p^) PROC Main() result=Echo(ADDRESS(@bytes(255)),7,LONGCARD($89abcdef)) letter=First("abc") RETURN"#;
    for optimize in [false, true] {
        let p = prepare(source, optimize);
        let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
        assert_eq!(
            bytes,
            prepare(&source.replace('\n', "\r\n"), optimize)
                .compile_o65(&Default::default())
                .unwrap()
                .bytes
        );
        for variant in 0..2 {
            let mut placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
            placement.bases[2] = if variant == 0 { 0x12ff80 } else { 0xabff80 };
            let image = actionc::mir65816::o65::relocate(&bytes, &placement).unwrap();
            for mask in [0, 4] {
                let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                h.bus.ram[0x70ff..0x7105].fill(0xa5);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(0x7100, 3), o65::object(&image, "bytes") + 255);
                assert_eq!(h.bus.ram[0x7103], 3); // Action! counted string.
                assert_eq!((h.bus.ram[0x70ff], h.bus.ram[0x7104]), (0xa5, 0xa5));
            }
        }
    }
}
