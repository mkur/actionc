mod support;
use actionc::mir65816::{Mir65816Terminator, emit, o65 as format};
use support::*;

const SOURCE: &str = r#"
BYTE input=$7100, result=$7200
PROC Pick(BYTE value)
 BYTE local
 local=value
 IF local=0 THEN result=11 RETURN FI
 IF local=1 THEN result=22 RETURN FI
 result=33
RETURN
PROC Empty() RETURN
PROC Main() Pick(input) Empty() RETURN
"#;

#[test]
fn shared_void_tails_restore_frames_and_relocate() {
    for optimize in [false, true] {
        let p = prepare(SOURCE, optimize);
        let c = p.compile(&layout()).unwrap();
        assert_eq!(
            c.image.to_json().unwrap(),
            compile(&SOURCE.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let r = c
            .machine
            .prepared
            .routines
            .iter()
            .find(|r| r.name == "Pick")
            .unwrap();
        let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
        assert!(m.frame.extent > 0);
        let tails: Vec<_> = r
            .blocks
            .iter()
            .filter(|b| matches!(b.terminator, Mir65816Terminator::Return { .. }))
            .map(|b| &m.code.bytes[m.code.mir_spans[&(b.id, b.ops.len())].clone()])
            .collect();
        assert_eq!(tails.len(), 3);
        assert_eq!(tails.iter().filter(|t| t.last() == Some(&0x6b)).count(), 1);
        assert_eq!(
            r.blocks
                .iter()
                .filter(|b| matches!(b.terminator, Mir65816Terminator::Return { .. }))
                .filter(|b| {
                    let span = &m.code.mir_spans[&(b.id, b.ops.len())];
                    m.code.local_jumps.iter().any(|j| span.contains(&j.offset))
                })
                .count(),
            2
        );
        let (a, _) = emit::proof::materialize_reference(&p.mir, true).unwrap();
        let (b, _) = emit::proof::materialize_replayed(&p.mir, true).unwrap();
        for (x, y) in a.routines.iter().zip(&b.routines) {
            emit::proof::compare_replay_output(&x.code, &y.code).unwrap();
        }
        let object = p.compile_o65(&Default::default()).unwrap().bytes;
        for variant in 0..3 {
            let loaded = (variant > 0).then(|| {
                format::relocate(
                    &object,
                    &o65::placement(&object, variant - 1, vec![o65::fault(variant - 1)]),
                )
                .unwrap()
            });
            for (value, expected) in [(0, 11), (1, 22), (2, 33), (255, 33)] {
                for mask in [0, 4] {
                    let mut h = if let Some(l) = &loaded {
                        Harness::new_o65(l, &caller(l.entry()), mask)
                    } else {
                        Harness::new(&c.image, &caller(c.image.entry), mask)
                    };
                    h.bus.ram[0x7100] = value;
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.bus.ram[0x7200], expected);
                }
            }
        }
    }
}

#[test]
fn shared_return_join_survives_interrupt_reentry() {
    let source = "MODULE TEST BYTE irqAck=$7800 PROC Work(BYTE v) BYTE local local=v IF local=0 THEN RETURN FI IF local=1 THEN RETURN FI RETURN CARD FUNC Dispatch(CARD saved BYTE reason) Work(0) irqAck=1 RETURN(saved) PROC Task(CARD POINTER argument) Work(BYTE(argument^)) RETURN PROC Main() RETURN ENDMODULE";
    check_interrupt_reentry(source);
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

#[test]
fn native_value_tails_preserve_every_result_lane_and_call_forwarding() {
    for (ty, mask) in [
        ("BYTE", 255u32),
        ("CARD", 65535),
        ("ADDRESS", 0xffffff),
        ("LONGCARD", u32::MAX),
    ] {
        let source = format!(
            "MODULE TEST BYTE irqAck=$7800 {ty} scratch {ty} FUNC Echo({ty} v) RETURN(v) {ty} FUNC Work(BYTE v) IF v=0 THEN RETURN({ty}($89abcdef)) FI IF v=1 THEN RETURN(Echo({ty}($fedcba98))) FI RETURN({ty}(0)) CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(0) irqAck=1 RETURN(saved) PROC Task(LONGCARD POINTER argument) argument^=LONGCARD(Work(BYTE(argument^))) RETURN PROC Main() RETURN ENDMODULE"
        );
        check_interrupt_reentry(&source);
        let source = format!(
            "BYTE input=$7100 LONGCARD result=$7200 {ty} FUNC Echo({ty} v) RETURN(v) {ty} FUNC Work(BYTE v) IF v=0 THEN RETURN({ty}($89abcdef)) FI IF v=1 THEN RETURN(Echo({ty}($fedcba98))) FI RETURN({ty}(0)) PROC Main() result=LONGCARD(Work(input)) RETURN"
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            let image = p.compile(&layout()).unwrap().image;
            assert_eq!(
                image.to_json().unwrap(),
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
                for (input, expected) in [(0, 0x89abcdefu32), (1, 0xfedcba98), (255, 0)] {
                    let mut h = if let Some(l) = &loaded {
                        Harness::new_o65(l, &caller(l.entry()), 0)
                    } else {
                        Harness::new(&image, &caller(image.entry), 0)
                    };
                    h.bus.ram[0x7100] = input;
                    h.run();
                    h.guards(0);
                    assert_eq!(h.bus.value(0x7200, 4), expected & mask, "{ty}/{input}");
                }
            }
        }
    }
}
