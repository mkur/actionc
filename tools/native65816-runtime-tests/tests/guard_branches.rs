mod support;
use actionc_vm::native65816::{Inputs, Machine, Registers};
use support::*;

const SOURCE: &str = "CARD input,result CARD FUNC Work(CARD a) RETURN(a+1) PROC Empty() RETURN PROC Main() CARD FUNC POINTER cb(CARD a) cb=@Work result=Work(input)+cb(input) Empty() RETURN";

fn reference(amount: u16, origin: u32, fault: u32, short: bool) -> Vec<u8> {
    let branch = |op: &str, inverse: &str, target: &str| {
        if short {
            format!("{op} {target}\n")
        } else {
            format!("{inverse} :+\njml {target}\n:\n")
        }
    };
    assemble(
        &format!(
            "tsc\ntax\ncmp $46\n{}{}{jump} fault\nwithin: sec\nsbc #{amount}\n{}cmp $44\n{}fault: lda #{amount}\njml ${fault:06x}\ndone: nop\n",
            branch("bcc", "bcs", "within"),
            branch("beq", "bne", "within"),
            branch("bcc", "bcs", "fault"),
            branch("bcs", "bcc", "done"),
            jump = if short { "bra" } else { "jml" }
        ),
        origin,
    )
}

fn run(
    code: &[u8],
    start: u32,
    done: u32,
    fault: u32,
    s: u16,
    floor: u16,
    ceiling: u16,
    p: u8,
) -> (Registers, Vec<u32>, u64, bool) {
    let mut bus = Bus::new();
    bus.map(start, code, false);
    bus.map(0x2000, &[0; 256], true);
    bus.map(fault, &[0xdb, 0xea], false);
    bus.ram[0x2044..0x2046].copy_from_slice(&floor.to_le_bytes());
    bus.ram[0x2046..0x2048].copy_from_slice(&ceiling.to_le_bytes());
    let mut cpu = Machine::start_at(Registers {
        a: 0xabcd,
        x: 0x1234,
        y: 0x5678,
        s,
        d: 0x2000,
        dbr: 0,
        pbr: (start >> 16) as u8,
        pc: start as u16,
        p,
        emulation_mode: false,
    });
    assert!(
        cpu.run_until(
            &mut bus,
            100,
            |_| Inputs::default(),
            |c| c.is_instruction_boundary() && [done, fault].contains(&c.pc())
        )
        .unwrap()
    );
    assert!(bus.writes.is_empty());
    let failed = cpu.pc() == fault;
    let mut r = cpu.registers();
    r.pc = 0;
    r.pbr = 0; // compare corresponding logical exits, not relocated PCs
    (
        r,
        bus.reads
            .into_iter()
            .filter(|a| (0x2000..0x2100).contains(a))
            .collect(),
        cpu.cycles(),
        failed,
    )
}

#[test]
fn emitted_entry_and_call_guards_match_independent_reference_on_all_boundary_paths() {
    let mut records = vec![];
    for optimize in [false, true] {
        let compiled = prepare(SOURCE, optimize).compile(&layout()).unwrap();
        let sites = guard::sites(&compiled);
        assert!(sites.iter().any(|s| s.amount == 0));
        assert!(sites.iter().any(|s| s.amount == 6)); // word call: O=3 + JSL=3
        assert!(sites.iter().any(|s| s.amount == 9)); // word indirect: O=3 + transfer=6
        for site in sites {
            let segment = compiled
                .image
                .segments
                .iter()
                .find(|s| s.address <= site.start && site.end <= s.address + s.bytes.len() as u32)
                .unwrap();
            let bytes = &segment.bytes
                [(site.start - segment.address) as usize..(site.end - segment.address) as usize];
            let fault = compiled.image.stack_overflow;
            let old = reference(site.amount, site.start, fault, false);
            let short = reference(site.amount, site.start, fault, true);
            assert_eq!(old.len() - short.len(), 18);
            assert_eq!(bytes.len(), 27);
            assert_eq!(bytes, &short[..short.len() - 1]);
            let candidate = 0x5f00 - site.amount;
            let mut cases = vec![
                (0x5f00, candidate, 0x5f01),
                (0x5f00, candidate, 0x5f00),
                (0x5f00, candidate, 0x5eff),
                (0x5f00, candidate + 1, 0x5f01),
                (0x5f00, candidate - 1, 0x5f01),
                (0x8000, 1, 0xffff),
                (0xffff, 0, 0xffff),
            ];
            if site.amount > 0 {
                cases.push((site.amount - 1, 0, 0xffff));
            } else {
                cases.push((0, 0, 0));
            }
            for (s, floor, ceiling) in cases {
                for p in (0..=255u8).filter(|p| p & 0x38 == 0) {
                    let actual = run(bytes, site.start, site.end, fault, s, floor, ceiling, p);
                    let before = run(
                        &old,
                        site.start,
                        site.start + 45,
                        fault,
                        s,
                        floor,
                        ceiling,
                        p,
                    );
                    let compact = run(
                        &short,
                        site.start,
                        site.start + 27,
                        fault,
                        s,
                        floor,
                        ceiling,
                        p,
                    );
                    assert_eq!(
                        (&actual.0, &actual.1, actual.3),
                        (&before.0, &before.1, before.3)
                    );
                    assert_eq!(
                        (&compact.0, &compact.1, compact.3),
                        (&before.0, &before.1, before.3)
                    );
                    assert!(compact.2 < before.2);
                    let failed =
                        s > ceiling || s.checked_sub(site.amount).is_none_or(|n| n < floor);
                    assert_eq!(actual.3, failed);
                    assert_eq!(
                        (actual.0.a, actual.0.x, actual.0.y, actual.0.s),
                        (
                            if failed { site.amount } else { s - site.amount },
                            s,
                            0x5678,
                            s
                        )
                    );
                    if p == 0 || p == 4 {
                        records.push(serde_json::json!({"optimize":optimize,"routine":site.routine,
                            "amount":site.amount,"bytes":bytes.len(),"s":s,"floor":floor,"ceiling":ceiling,
                            "p":p,"failed":failed,"cycles":actual.2,"legacy_cycles":before.2,
                            "compact_cycles":compact.2,"exit_p":actual.0.p,"reads":actual.1}));
                    }
                }
            }
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("guard-observations.json"),
            serde_json::to_vec_pretty(&records).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn relocated_guards_reach_both_success_and_moved_fault_exits() {
    use actionc::mir65816::o65 as format;
    for optimize in [false, true] {
        let bytes = o65::compile(SOURCE, optimize, vec![]);
        for variant in 0..2 {
            let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
            let image = format::relocate(&bytes, &placement).unwrap();
            let entry = image.entry();
            let main = image
                .profile()
                .routines
                .iter()
                .find(|r| r.name == "Main")
                .unwrap();
            let amount = main.frame;
            for mask in [0, 4] {
                let mut h = Harness::new_o65(&image, &caller(entry), mask);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(o65::object(&image, "result"), 2), 2);
                for failed in [false, true] {
                    let mut h = Harness::new_o65(&image, &caller(entry), mask);
                    let mut r = h.cpu.registers();
                    r.pc = entry as u16;
                    r.pbr = (entry >> 16) as u8;
                    h.cpu = Machine::start_at(r);
                    let floor = r.s - amount + u16::from(failed);
                    h.bus.ram[0x2044..0x2046].copy_from_slice(&floor.to_le_bytes());
                    let dest = if failed {
                        image.stack_overflow()
                    } else {
                        entry + 27
                    };
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                100,
                                |_| Inputs::default(),
                                |c| c.is_instruction_boundary() && c.pc() == dest
                            )
                            .unwrap()
                    );
                    let out = h.cpu.registers();
                    assert_eq!(
                        (out.a, out.x, out.s),
                        (if failed { amount } else { floor }, r.s, r.s)
                    );
                    assert_eq!(out.p & 0x3c, mask);
                    assert!(h.bus.writes.is_empty());
                }
            }
        }
    }
}

#[test]
fn every_reached_guard_boundary_restores_flags_under_irq_and_nmi_in_both_domains() {
    use support::context::*;
    // The existing bridge owns saving/restoring both domains. This dispatch
    // resumes the interrupted task; the full preemption suite covers switching.
    let source = "MODULE TEST VOLATILE BYTE irqAck=$7800 CARD result CARD FUNC Dispatch(CARD saved BYTE reason) irqAck=1 RETURN(saved) CARD FUNC Work(CARD n) RETURN(n+1) PROC Task(BYTE POINTER ignored) CARD FUNC POINTER cb(CARD n) cb=@Work result=Work(7)+cb(9) RETURN PROC Main() RETURN ENDMODULE";
    let mut records = vec![];
    for optimize in [false, true] {
        let prepared = prepare(source, optimize);
        let compiled = prepared.compile(&layout()).unwrap();
        let entry = routine(&compiled.image, "Task");
        let task = compiled
            .image
            .routines
            .iter()
            .find(|r| r.address == entry)
            .unwrap();
        let sites: Vec<_> = guard::sites(&compiled)
            .into_iter()
            .filter(|s| s.routine == task.name)
            .collect();
        assert_eq!(sites.len(), 3); // entry, direct call, indirect call
        for domain in 0..2 {
            let mut h =
                ContextHarness::from_prepared(source, optimize, "Task", &[0, 0], prepared.clone());
            let mut r = h.cpu.registers();
            r.a = h.first[domain].saved_s;
            h.cpu = Machine::start_at(r);
            for site in &sites {
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            10000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == site.start
                        )
                        .unwrap()
                );
                assert_eq!(h.cpu.registers().d, 0x2000 + domain as u16 * 0x100);
                let checkpoint = h.cpu.clone();
                let memory = h.bus.clone();
                for scenario in 0..4 {
                    for mask in [0, 4] {
                        h.cpu = checkpoint.clone();
                        h.bus = memory.clone();
                        let mut r = h.cpu.registers();
                        r.p = (r.p & !4) | mask;
                        h.cpu = Machine::start_at(r);
                        let floor = r.s - site.amount + u16::from(scenario == 2);
                        let ceiling = if scenario == 3 {
                            r.s - 1
                        } else {
                            r.s + u16::from(scenario != 0)
                        };
                        let dp = r.d as usize;
                        h.bus.ram[dp + 0x44..dp + 0x46].copy_from_slice(&floor.to_le_bytes());
                        h.bus.ram[dp + 0x46..dp + 0x48].copy_from_slice(&ceiling.to_le_bytes());
                        let exit = if scenario >= 2 {
                            h.image.stack_overflow
                        } else {
                            site.end
                        };
                        let mut final_cpu = h.cpu.clone();
                        let mut final_bus = h.bus.clone();
                        assert!(
                            final_cpu
                                .run_until(
                                    &mut final_bus,
                                    100,
                                    |_| Inputs::default(),
                                    |c| c.is_instruction_boundary() && c.pc() == exit
                                )
                                .unwrap()
                        );
                        loop {
                            if h.cpu.pc() == exit && h.cpu.is_instruction_boundary() {
                                break;
                            }
                            assert!(h.cpu.is_instruction_boundary());
                            let at = h.cpu.clone();
                            let bus = h.bus.clone();
                            // Interrupt sampling finishes this instruction first.
                            let mut reference = at.clone();
                            let mut rb = bus.clone();
                            reference.tick(&mut rb, Inputs::default()).unwrap();
                            while !reference.is_instruction_boundary() {
                                reference.tick(&mut rb, Inputs::default()).unwrap();
                            }
                            for nmi in [false, true] {
                                h.cpu = at.clone();
                                h.bus = bus.clone();
                                let mut acknowledged = false;
                                let mut restored = false;
                                let ack = if nmi { NMI_ACK } else { IRQ_ACK };
                                for _ in 0..10000 {
                                    let writes = h.bus.writes.len();
                                    h.tick(Inputs {
                                        irq: !nmi && !acknowledged,
                                        nmi: nmi && !acknowledged,
                                        ..Default::default()
                                    });
                                    acknowledged |=
                                        h.bus.writes[writes..].iter().any(|&(a, _)| a == ack);
                                    if h.cpu.is_instruction_boundary()
                                        && h.cpu.pc() == reference.pc()
                                        && h.cpu.registers().d == r.d
                                        && (acknowledged || !nmi && mask == 4)
                                    {
                                        assert_eq!(h.cpu.registers(), reference.registers());
                                        assert_eq!(
                                            &h.bus.ram
                                                [usize::from(r.s) + 1..0x5000 + domain * 0x1000],
                                            &rb.ram[usize::from(r.s) + 1..0x5000 + domain * 0x1000]
                                        );
                                        assert_eq!(&h.bus.ram[dp..dp + 256], &rb.ram[dp..dp + 256]);
                                        assert_eq!(acknowledged, nmi || mask == 0);
                                        restored = true;
                                        break;
                                    }
                                    assert!(!h.cpu.is_stopped());
                                }
                                assert!(restored, "interrupt restoration at {:06x}", at.pc());
                                assert!(
                                    h.cpu
                                        .run_until(
                                            &mut h.bus,
                                            100,
                                            |_| Inputs::default(),
                                            |c| c.is_instruction_boundary() && c.pc() == exit
                                        )
                                        .unwrap()
                                );
                                assert_eq!(h.cpu.registers(), final_cpu.registers());
                                records.push(serde_json::json!({"optimize":optimize,"domain":domain,"guard":site.start,
                                    "scenario":scenario,"mask":mask,"pc":at.pc(),"nmi":nmi,"delivered":acknowledged}));
                            }
                            h.cpu = reference;
                            h.bus = rb;
                        }
                    }
                }
                h.cpu = checkpoint;
                h.bus = memory;
                // Leave this guard so the next search cannot rediscover it.
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            100,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == site.end
                        )
                        .unwrap()
                );
            }
            h.run();
            assert_eq!(h.bus.value(symbol(&h.image, "result"), 2), 18);
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("guard-interrupts.json"),
            serde_json::to_vec_pretty(&records).unwrap(),
        )
        .unwrap();
    }
}
