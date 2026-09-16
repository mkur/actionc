mod support;
use actionc::mir65816::context::{Domain, DomainKind, FirstTask};
use actionc_vm::native65816::{Inputs, Machine};
use support::{context::*, *};
const SOURCE: &str = r#"
MODULE TEST
PUBLIC EXTERNAL PROC Yield()
CARD dispatches,lastReason
CARD FUNC Dispatch(CARD saved BYTE reason)
 dispatches==+1 lastReason=CARD(reason)
RETURN(saved)
PROC Task(BYTE POINTER argument)
 argument^=argument^+1
 Yield()
 argument^=argument^+1
RETURN
PROC Main() RETURN
ENDMODULE
"#;
#[test]
fn published_first_task_bytes_and_layout_rejections() {
    let d = Domain {
        direct_page: 0x2200,
        owner: 0x120000,
        kind: DomainKind::Task,
        stack_low: 0x5000,
        stack_high: 0x60ff,
        body_s: 0x6000,
        nmi_extra: 0,
    };
    let t = FirstTask::new(&d, 0x128000, 0x345678, 0x009000, 0, false).unwrap();
    assert_eq!(t.saved_s, 0x5fed);
    assert_eq!(
        t.bytes,
        [
            0, 0, 0x22, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x12, 0xff, 0x8f, 0, 0x78, 0x56, 0x34
        ]
    );
    assert!(FirstTask::new(&d, 0x1000000, 0, 0, 0, false).is_err());
    assert!(FirstTask::new(&d, 0, 0, 0, 0xffff, false).is_err());
    assert!(
        Domain {
            direct_page: 0x2201,
            ..d
        }
        .bytes()
        .is_err()
    );
    assert!(
        Domain {
            direct_page: 0x5000,
            ..d
        }
        .bytes()
        .is_err()
    );
    let t = FirstTask::new(&d, 0x128000, 0, 0x120000, 0, true).unwrap();
    assert_eq!(&t.bytes[13..16], &[0xff, 0xff, 0x12]);
    assert_eq!(t.bytes[9], 4);
}
#[test]
fn fabricated_task_yields_and_returns_through_the_exit_binding() {
    for optimize in [false, true] {
        let mut h = ContextHarness::new(SOURCE, optimize, "Task", &[0x7100]);
        h.run();
        h.guards();
        assert_eq!(h.bus.value(0x7100, 1), 2);
        assert_eq!(h.bus.value(symbol(&h.image, "dispatches"), 2), 1);
        assert_eq!(h.bus.value(symbol(&h.image, "lastReason"), 2), 1);
        let r = h.cpu.registers();
        assert_eq!((r.s, r.d, r.dbr, r.p & 0x3c), (0x4fec, 0x2000, 0, 0));
    }
}
#[test]
fn irq_and_nmi_restore_full_registers_for_every_interrupted_width() {
    for status in [0, 0x20, 0x10, 0x30, 0x28, 0x38, 0xc3, 0xf3, 0xf7] {
        for nmi in [false, true] {
            if status & 4 != 0 && !nmi {
                continue;
            }
            let mut h = ContextHarness::new(SOURCE, false, "Task", &[0x7100]);
            h.bus
                .map(0x060100, &assemble("nop\nbra *-1", 0x060100), false);
            let r = actionc_vm::native65816::Registers {
                a: 0xabcd,
                x: if status & 0x10 == 0 { 0x5678 } else { 0x78 },
                y: if status & 0x10 == 0 { 0x9abc } else { 0xbc },
                s: 0x4ff0,
                d: 0x2000,
                dbr: 0x12,
                pbr: 6,
                pc: 0x100,
                p: status,
                emulation_mode: false,
            };
            h.cpu = Machine::start_at(r);
            let mut entered = false;
            let mut resumed = false;
            for _ in 0..20000 {
                let pc = h.cpu.pc();
                if pc
                    == h.symbols[if nmi {
                        "__a816_nmi_v1"
                    } else {
                        "__a816_irq_v1"
                    }]
                    && h.cpu.is_instruction_boundary()
                {
                    entered = true;
                }
                if entered && h.cpu.is_instruction_boundary() && h.cpu.registers().pbr == 6 {
                    resumed = true;
                    break;
                }
                h.tick(Inputs {
                    irq: !nmi && !entered,
                    nmi: nmi && !entered,
                    ..Inputs::default()
                });
            }
            assert!(resumed, "status={status:02x} nmi={nmi}");
            let actual = h.cpu.registers();
            assert_eq!(
                (
                    actual.a, actual.x, actual.y, actual.s, actual.d, actual.dbr, actual.pbr,
                    actual.p
                ),
                (r.a, r.x, r.y, r.s, r.d, r.dbr, r.pbr, r.p)
            );
            assert!([0x100, 0x101].contains(&actual.pc));
            h.guards();
        }
    }
}
#[test]
fn invalid_yield_domains_and_masked_yield_fault_without_dispatch() {
    for kind in [0, 1, 2] {
        let mut h = ContextHarness::new(SOURCE, false, "Task", &[0x7100]);
        let mut r = h.cpu.registers();
        r.pc = h.symbols["__a816_yield_v1"] as u16;
        r.a = 0;
        r.s = 0x4ff0;
        r.d = 0x2000;
        r.p = if kind == 0 { 4 } else { 0 };
        h.bus.ram[0x2043] = kind;
        h.cpu = Machine::start_at(r);
        assert!(
            h.cpu
                .run_until(&mut h.bus, 2000, |_| Inputs::default(), |c| c.is_stopped())
                .unwrap()
        );
        assert_eq!(h.bus.value(DONE, 2), 0xee);
        assert_eq!(h.bus.value(symbol(&h.image, "dispatches"), 2), 0);
    }
}

#[test]
fn nmi_is_safe_at_each_irq_bridge_instruction_including_stack_domain_transitions() {
    let mut baseline = ContextHarness::new(SOURCE, false, "Task", &[0x7100]);
    baseline
        .bus
        .map(0x060100, &assemble("nop\nbra *-1", 0x060100), false);
    let r = actionc_vm::native65816::Registers {
        a: 0xabcd,
        x: 0x5678,
        y: 0x9abc,
        s: 0x4ff0,
        d: 0x2000,
        dbr: 0x12,
        pbr: 6,
        pc: 0x100,
        p: 0x28,
        emulation_mode: false,
    };
    baseline.cpu = Machine::start_at(r);
    let mut entered = false;
    let mut points = Vec::new();
    for _ in 0..20000 {
        if baseline.cpu.is_instruction_boundary() {
            if baseline.cpu.pc() == baseline.symbols["__a816_irq_v1"] {
                entered = true;
            }
            if entered && baseline.cpu.registers().pbr == 6 {
                break;
            }
            if entered && (0x8000..0x9000).contains(&baseline.cpu.pc()) {
                points.push((baseline.cpu.clone(), baseline.bus.clone()));
            }
        }
        baseline.tick(Inputs {
            irq: !entered,
            ..Inputs::default()
        });
    }
    assert!(points.len() > 25);
    for (mut cpu, mut bus) in points {
        let injected_pc = cpu.pc();
        let mut nmi_entered = false;
        let mut complete = false;
        for _ in 0..20000 {
            if cpu.is_instruction_boundary() {
                if cpu.pc() == baseline.symbols["__a816_nmi_v1"] {
                    nmi_entered = true;
                }
                if nmi_entered && cpu.registers().pbr == 6 {
                    complete = true;
                    break;
                }
            }
            cpu.tick(
                &mut bus,
                Inputs {
                    nmi: !nmi_entered,
                    ..Inputs::default()
                },
            )
            .unwrap();
        }
        assert!(complete, "NMI at {injected_pc:06x}");
        let actual = cpu.registers();
        assert_eq!(
            (
                actual.a, actual.x, actual.y, actual.s, actual.d, actual.dbr, actual.p
            ),
            (r.a, r.x, r.y, r.s, r.d, r.dbr, r.p),
            "NMI at {injected_pc:06x}"
        );
        assert_eq!(bus.value(NMI_ACK, 1), 1);
        assert_eq!(&bus.ram[0x2000..0x2100], &baseline.bus.ram[0x2000..0x2100]);
    }
}

#[test]
fn unknown_cop_and_returning_task_exit_reach_terminal_faults() {
    for bad_cop in [true, false] {
        let mut h = ContextHarness::new(SOURCE, false, "Task", &[0x7100]);
        if bad_cop {
            h.bus
                .map(0x060200, &assemble("cop $01\nnop", 0x060200), false);
            h.cpu = Machine::start_at(actionc_vm::native65816::Registers {
                s: 0x4ff0,
                d: 0x2000,
                pbr: 6,
                pc: 0x200,
                ..Default::default()
            });
        } else {
            // A deliberately invalid platform binding returns from task exit.
            h.bus.ram[h.symbols["test_exit"] as usize] = 0x6b;
        }
        assert!(
            h.cpu
                .run_until(&mut h.bus, 20000, |_| Inputs::default(), |c| c.is_stopped())
                .unwrap()
        );
        assert_eq!(h.bus.value(DONE, 2), 0xee);
        if bad_cop {
            assert_eq!(h.bus.value(symbol(&h.image, "dispatches"), 2), 0);
        }
    }
}
