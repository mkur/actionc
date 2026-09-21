mod support;
use actionc_vm::native65816::Inputs;
use std::collections::BTreeSet;
use support::{context::*, *};

fn machine(optimize: bool) -> ContextHarness {
    machine_source(&fixture("preemption.act"), optimize)
}
fn machine_source(source: &str, optimize: bool) -> ContextHarness {
    let mut h = ContextHarness::new(source, optimize, "Task", &[0x7100, 0x7120]);
    for (i, seed) in [13u16, 41].into_iter().enumerate() {
        let at = 0x7100 + i * 0x20;
        h.bus.ram[at..at + 2].copy_from_slice(&seed.to_le_bytes());
        // Native layout: CARD seed/result, BYTE done, three-byte pointer fields.
        h.bus.ram[at + 5..at + 8]
            .copy_from_slice(&(0x7104u32 + (1 - i as u32) * 0x20).to_le_bytes()[..3]);
        let buffer = 0x12fffc + i as u32 * 0x10000;
        h.bus.ram[at + 8..at + 11].copy_from_slice(&buffer.to_le_bytes()[..3]);
        h.bus
            .map(buffer, &[10, 20, 30, 40, 50, 60, 70, 80, 90, 100], true);
    }
    h
}

// Routine, stack/immediate source, frame extent, reached tail instruction PCs.
type ReturnWindow = (u32, bool, u16, Vec<u32>);

fn return_window(h: &ContextHarness) -> Option<ReturnWindow> {
    assert!(h.cpu.is_instruction_boundary());
    let pc = h.cpu.pc();
    let r = h
        .image
        .routines
        .iter()
        .find(|r| r.result_bytes == 2 && (r.address..r.address + r.size).contains(&pc))?;
    if h.cpu.registers().p & 0x30 != 0 {
        return None;
    }
    let opcode = h.bus.ram[pc as usize];
    let size = match opcode {
        0xa3 => 2,
        0xa9 => 3,
        _ => return None,
    };
    let tail = pc + size;
    let mut bytes = vec![];
    let mut addresses = vec![pc];
    if r.fixed_frame != 0 {
        bytes.extend([
            0xa8,
            0x3b,
            0x18,
            0x69,
            r.fixed_frame as u8,
            (r.fixed_frame >> 8) as u8,
            0x1b,
            0x98,
        ]);
        addresses.extend([0, 1, 2, 3, 6, 7].map(|offset| tail + offset));
    }
    addresses.push(tail + bytes.len() as u32);
    bytes.push(0x6b);
    if tail + bytes.len() as u32 > r.address + r.size
        || h.bus.ram[tail as usize..tail as usize + bytes.len()] != bytes
    {
        return None;
    }
    // The candidate starts at a real CPU instruction boundary. The complete
    // mode-aware sequence is checked inside its owning word-result routine.
    Some((r.address, opcode == 0xa3, r.fixed_frame, addresses))
}
fn check(h: &ContextHarness) {
    assert_eq!(h.bus.value(DONE, 2), 1);
    h.guards();
    for (i, seed) in [13u32, 41].into_iter().enumerate() {
        assert_eq!(
            h.bus.value(0x7102 + i as u32 * 0x20, 2),
            (seed + 5 + 3 + 6) * 2 + 1
        );
        assert_eq!(h.bus.value(0x7104 + i as u32 * 0x20, 1), 1);
        let buffer = 0x12fffc + i * 0x10000;
        assert_eq!(
            &h.bus.ram[buffer..buffer + 10],
            &[10, 10, 20, 0, 0, 50, 60, 70, 90, 100]
        );
    }
    assert!(h.bus.value(symbol(&h.image, "dispatches"), 2) >= 2);
}
fn run_injected(h: &mut ContextHarness, mut pending: bool, seed: Option<u64>) {
    let mut rng = seed.unwrap_or(1);
    let mut nmi_cooldown = 0u64;
    let start = h.cpu.cycles();
    for _ in 0..2_000_000 {
        if h.cpu.is_stopped() {
            check(h);
            return;
        }
        let mut nmi = false;
        if seed.is_some() && h.cpu.is_instruction_boundary() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            if rng & 31 == 0 && h.cpu.registers().p & 4 == 0 {
                pending = true;
            }
            if rng & 127 == 1 && h.cpu.cycles() >= nmi_cooldown {
                nmi = true;
                nmi_cooldown = h.cpu.cycles() + 250;
            }
        }
        let before = h.bus.writes.len();
        h.tick(Inputs {
            irq: pending,
            nmi,
            ..Default::default()
        });
        if h.bus.writes[before..].iter().any(|&(a, _)| a == IRQ_ACK) {
            pending = false;
        }
    }
    panic!(
        "interrupt run exceeded budget after {} clocks",
        h.cpu.cycles() - start
    );
}
#[test]
fn two_live_contexts_reenter_recursive_and_memory_helpers_with_seeded_interrupts() {
    for optimize in [false, true] {
        for seed in [0x81620260916, 0x5eedcafe] {
            let mut h = machine(optimize);
            run_injected(&mut h, false, Some(seed));
        }
    }
}
#[test]
fn irq_at_each_reachable_enabled_instruction_preserves_two_context_results() {
    for optimize in [false, true] {
        let mut h = machine(optimize);
        let mut seen = BTreeSet::new();
        let mut word_windows = BTreeSet::new();
        let mut return_windows = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            if h.cpu.is_instruction_boundary()
                && r.p & 4 == 0
                && [0x2000, 0x2100].contains(&r.d)
                && seen.insert(h.cpu.pc())
            {
                let pc = h.cpu.pc();
                if let Some(window) = return_window(&h) {
                    return_windows.insert(window);
                }
                let opcode = h.bus.ram[pc as usize];
                if matches!(opcode, 0x63 | 0xe3) {
                    // Decode only at a reached instruction boundary. These
                    // stack-relative forms are emitted by word arithmetic;
                    // immediate arithmetic in stack guards is not counted.
                    assert_eq!(r.p & 0x20, 0);
                    assert_eq!(
                        h.bus.ram[pc as usize - 1],
                        if opcode == 0x63 { 0x18 } else { 0x38 }
                    );
                    assert_eq!(h.bus.ram[pc as usize + 2], 0x83);
                    word_windows.insert((opcode, pc - 1, pc, pc + 2));
                }
                let saved_cpu = h.cpu.clone();
                let saved_bus = h.bus.clone();
                run_injected(&mut h, true, None);
                h.cpu = saved_cpu;
                h.bus = saved_bus;
            }
            h.tick(Inputs::default());
        }
        check(&h);
        assert_eq!(
            word_windows.iter().map(|w| w.0).collect::<BTreeSet<_>>(),
            BTreeSet::from([0x63, 0xe3])
        );
        for &(_, carry, arithmetic, store) in &word_windows {
            assert!(
                [carry, arithmetic, store]
                    .into_iter()
                    .all(|pc| seen.contains(&pc)),
                "unqualified word arithmetic interruption window"
            );
        }
        assert!(return_windows.iter().any(|w| w.1));
        assert!(return_windows.iter().any(|w| !w.1));
        assert!(
            return_windows
                .iter()
                .all(|w| w.3.iter().all(|pc| seen.contains(pc)))
        );
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            std::fs::write(
                std::path::Path::new(&directory).join(format!("word-preemption-{optimize}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "optimized": optimize, "enabled_instruction_addresses": seen.len(),
                    "word_windows": word_windows,
                    "window_columns": ["opcode", "carry_setup_pc", "arithmetic_pc", "store_pc"],
                    "each_window_address_irq_tested": true,
                }))
                .unwrap(),
            )
            .unwrap();
            std::fs::write(
                std::path::Path::new(&directory).join(format!("return-preemption-{optimize}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "optimized": optimize, "enabled_instruction_addresses": seen.len(),
                    "return_windows": return_windows,
                    "window_columns": ["routine", "stack_source", "frame", "instruction_addresses"],
                    "each_window_address_irq_tested": true,
                }))
                .unwrap(),
            )
            .unwrap();
        }
        assert!(
            seen.len() > 300,
            "only {} instruction boundaries",
            seen.len()
        );
        eprintln!(
            "qualified optimize={optimize}: {} enabled instruction addresses",
            seen.len()
        );
        eprintln!(
            "word arithmetic optimize={optimize}: {} qualified interruption windows",
            word_windows.len()
        );
        eprintln!(
            "word returns optimize={optimize}: {} qualified interruption windows",
            return_windows.len()
        );
    }
}

fn zero_frame_source(source: &str) -> String {
    source
        .replace("\r\n", "\n")
        .replace(
            "CARD FUNC Read",
            "CARD FUNC ZeroFrame() RETURN(32768)\nCARD FUNC Read",
        )
        .replace(
            "  work.done=1",
            // Keep every returned bit observable: the earlier Shift and final
            // doubling would otherwise discard a corrupted high bit.
            "  work.result=work.result+ZeroFrame()-32768\n  work.done=1",
        )
}

#[test]
fn zero_frame_word_return_survives_both_task_irq_sites_and_seeded_nmi() {
    let fixture = fixture("preemption.act");
    let source = zero_frame_source(&fixture);
    assert_eq!(source, zero_frame_source(&fixture.replace('\n', "\r\n")));
    for optimize in [false, true] {
        let mut h = machine_source(&source, optimize);
        let address = routine(&h.image, "ZeroFrame");
        let map = h
            .image
            .routines
            .iter()
            .find(|r| r.address == address)
            .unwrap();
        assert_eq!(map.fixed_frame, 0);
        let mut targets = BTreeSet::new();
        let mut seen = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            if h.cpu.is_instruction_boundary() && r.p & 4 == 0 && [0x2000, 0x2100].contains(&r.d) {
                if let Some(window) = return_window(&h) {
                    if window.0 == address {
                        assert!(!window.1 && window.2 == 0);
                        targets.extend(window.3.into_iter().map(|pc| (r.d, pc)));
                    }
                }
                let site = (r.d, h.cpu.pc());
                if targets.contains(&site) && seen.insert(site) {
                    let cpu = h.cpu.clone();
                    let bus = h.bus.clone();
                    run_injected(&mut h, true, None);
                    h.cpu = cpu;
                    h.bus = bus;
                }
            }
            h.tick(Inputs::default());
        }
        check(&h);
        assert_eq!(targets, seen);
        assert_eq!(seen.len(), 4); // LDA / RTL in each task, with A live before RTL.
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            std::fs::write(std::path::Path::new(&directory).join(format!("return-zero-preemption-{optimize}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({"optimized":optimize,"irq_sites":seen,"columns":["task_domain","pc"]})).unwrap()).unwrap();
        }
        for seed in [0x81620260916, 0x5eedcafe] {
            run_injected(&mut machine_source(&source, optimize), false, Some(seed));
        }
    }
}
