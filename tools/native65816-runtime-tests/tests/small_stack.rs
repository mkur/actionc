mod support;
use actionc::mir65816::emit::proof::{self, Value};
use actionc_vm::native65816::{Inputs, Machine, Registers};
use support::*;

#[test]
fn checked_stack_steps_match_ca65_vm_flags_equations_and_cycles() {
    let caller = assemble("jsl $040000\nstp\nnop", 0x030000);
    for subtract in [false, true] {
        for n in 1..=3 {
            let (before, after, snapshots) = proof::small_stack_probe(n, subtract).unwrap();
            let step = if subtract { "dec a\n" } else { "inc a\n" };
            let reverse = if subtract { "clc\nadc" } else { "sec\nsbc" };
            assert_eq!(
                after.bytes,
                assemble(
                    &format!(
                        "tsc\n{}tcs\ntsc\nnop\n{reverse} #{n}\ntcs\nrtl\n",
                        step.repeat(n as usize)
                    ),
                    0x040000
                )
            );
            assert_eq!(before.bytes.len() - after.bytes.len(), usize::from(4 - n));
            let (replayed, observed) = proof::replay_code(&after, true).unwrap();
            proof::compare_replay_output(&after, &replayed).unwrap();
            assert_eq!(snapshots, observed);
            let mut buses: Vec<_> = [&before, &after]
                .into_iter()
                .map(|code| {
                    let mut b = Bus::new();
                    b.map(0, &[0xa5; 65536], true);
                    b.map(0x030000, &caller, false);
                    b.map(0x040000, &code.bytes, false);
                    b
                })
                .collect();
            for s in [
                0u16, 1, 2, 3, 0xff, 0x100, 0x7fff, 0x8000, 0xfffc, 0xfffd, 0xfffe, 0xffff,
            ] {
                for p in (0..=255u8).filter(|p| p & 0x38 == 0) {
                    let initial = Registers {
                        a: 0xabcd,
                        x: 0x1357,
                        y: 0x2468,
                        s,
                        d: 0x2000,
                        dbr: 0,
                        pbr: 3,
                        pc: 0,
                        p,
                        emulation_mode: false,
                    };
                    let mut results = vec![];
                    for (variant, bus) in buses.iter_mut().enumerate() {
                        bus.reads.clear();
                        bus.writes.clear();
                        let mut cpu = Machine::start_at(initial);
                        for _ in 0..100 {
                            if cpu.is_stopped() {
                                break;
                            }
                            if variant == 1 && cpu.is_instruction_boundary() {
                                for fact in snapshots
                                    .iter()
                                    .filter(|f| 0x040000 + f.pc as u32 == cpu.pc())
                                {
                                    let r = cpu.registers();
                                    let entry = s.wrapping_sub(3);
                                    assert_eq!(r.s, entry.wrapping_sub(fact.depth as u16));
                                    if let Value::StackAddress(offset) = fact.a {
                                        assert_eq!(r.a, entry.wrapping_add(offset as u16));
                                    }
                                    if let Some(c) = fact.carry {
                                        assert_eq!(r.p & 1 != 0, c);
                                    }
                                    if let Some(v) = fact.overflow {
                                        assert_eq!(r.p & 0x40 != 0, v);
                                    }
                                }
                            }
                            cpu.tick(bus, Inputs::default()).unwrap();
                        }
                        assert!(cpu.is_stopped());
                        assert_eq!(cpu.registers().s, s);
                        results.push((
                            cpu.registers(),
                            cpu.cycles(),
                            bus.reads.clone(),
                            bus.writes.clone(),
                        ));
                    }
                    assert_eq!(results[0].0, results[1].0);
                    assert_eq!(
                        results[0]
                            .2
                            .iter()
                            .filter(|&&a| a < 0x10000)
                            .collect::<Vec<_>>(),
                        results[1]
                            .2
                            .iter()
                            .filter(|&&a| a < 0x10000)
                            .collect::<Vec<_>>()
                    );
                    assert_eq!(results[0].3, results[1].3);
                    // Three steps trade one extra cycle for one byte; one/two
                    // steps save three/one cycles respectively.
                    assert_eq!(
                        results[0].1 as i64 - results[1].1 as i64,
                        5 - 2 * i64::from(n)
                    );
                }
            }
        }
    }
}

#[test]
fn accumulator_steps_preserve_hidden_b_and_carry_overflow_at_both_widths() {
    let caller = assemble("jsl $040000\nstp\nnop", 0x030000);
    for byte in [false, true] {
        for subtract in [false, true] {
            for value in [0u16, 1, 0x7f, 0xff, 0x100, 0xabff, 0x7fff, 0xffff] {
                let code = proof::accumulator_step_probe(value, byte, subtract);
                for cv in [0, 1, 0x40, 0x41] {
                    let mut bus = Bus::new();
                    bus.map(0x4000, &[0xa5; 0x2000], true);
                    bus.map(0x030000, &caller, false);
                    bus.map(0x040000, &code.bytes, false);
                    let mut cpu = Machine::start_at(Registers {
                        a: 0,
                        x: 0x1234,
                        y: 0x5678,
                        s: 0x5fe0,
                        d: 0x2000,
                        dbr: 0,
                        pbr: 3,
                        pc: 0,
                        p: cv,
                        emulation_mode: false,
                    });
                    assert!(
                        cpu.run_until(&mut bus, 100, |_| Inputs::default(), |c| c.is_stopped())
                            .unwrap()
                    );
                    let mask = if byte { 0xff } else { 0xffff };
                    let low = if subtract {
                        value.wrapping_sub(1)
                    } else {
                        value.wrapping_add(1)
                    } & mask;
                    let r = cpu.registers();
                    assert_eq!(r.a, (value & !mask) | low);
                    assert_eq!((r.x, r.y, r.s, r.p & 0x79), (0x1234, 0x5678, 0x5fe0, cv));
                    assert_eq!(
                        r.p & 0x82,
                        if low == 0 { 2 } else { 0 }
                            | if low & ((mask >> 1) + 1) != 0 {
                                0x80
                            } else {
                                0
                            }
                    );
                }
            }
        }
    }
}

#[test]
fn small_frames_and_call_cleanup_preserve_every_native_result_width() {
    let mut frames = std::collections::BTreeSet::new();
    let mut steps = 0;
    for (ty, width, constant) in [
        ("BYTE", 1, 0x80u32),
        ("CARD", 2, 0x8000),
        ("ADDRESS", 3, 0x810000),
        ("LONGCARD", 4, 0x81230000),
    ] {
        // A mutable byte parameter keeps a two-byte frame; constant returns
        // isolate teardown/result preservation from arithmetic temporary homes.
        let value = constant + 2;
        let source = format!(
            "{ty} result\n{ty} FUNC Work(BYTE x) x==+1 RETURN({ty}(${value:X}))\nPROC Main() result=Work(1) RETURN"
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            for guards in [false, true] {
                let mut options = layout();
                options.stack_checks = guards;
                let c = p.compile(&options).unwrap();
                frames.insert(
                    c.image
                        .routines
                        .iter()
                        .find(|r| r.name == "Work")
                        .unwrap()
                        .fixed_frame,
                );
                for m in &c.machine.routines {
                    steps += proof::selected_actions(&m.code)
                        .unwrap()
                        .iter()
                        .filter(|a| {
                            a.kind == "instruction"
                                && a.encoded.len() == 1
                                && m.code.bytes[a.encoded.start] == 0x1a
                        })
                        .count();
                }
                for mask in [0, 4] {
                    let mut h = Harness::new(&c.image, &caller(c.image.entry), mask);
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.global(&c.image, "result", width), constant + 2);
                }
            }
        }
    }
    assert!(frames.contains(&2), "frames: {frames:?}");
    assert!(steps > 0);
}

#[test]
fn compact_frame_release_restores_irq_nmi_state_and_wide_results() {
    let source = "MODULE TEST VOLATILE BYTE irqAck=$7800 LONGCARD scratch
      LONGCARD FUNC Work(BYTE x) x==+1 RETURN(LONGCARD($81230002))
      CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(3) irqAck=1 RETURN(saved)
      PROC Task(LONGCARD POINTER argument) argument^=Work(BYTE(argument^)) RETURN
      PROC Main() RETURN ENDMODULE";
    for optimize in [false, true] {
        let c = prepare(source, optimize).compile(&layout()).unwrap();
        assert_eq!(
            c.image
                .routines
                .iter()
                .find(|r| r.name.ends_with("Work") || r.name.to_uppercase().contains("_WORK_"))
                .unwrap()
                .fixed_frame,
            2
        );
    }
    windows::check_work_interrupts(source);
}
