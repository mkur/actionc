mod support;
use actionc_vm::native65816::Inputs;
use std::collections::BTreeSet;
use support::{context::*, *};

fn machine(optimize: bool) -> ContextHarness {
    machine_source(&fixture("preemption.act"), optimize)
}
fn machine_source(source: &str, optimize: bool) -> ContextHarness {
    initialize(ContextHarness::new(
        source,
        optimize,
        "Task",
        &[0x7100, 0x7120],
    ))
}
fn initialize(mut h: ContextHarness) -> ContextHarness {
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
    let resident =
        forwarding::reached(&h.cpu, &h.bus).is_some_and(|s| s.kind == forwarding::Kind::Return);
    let size = match opcode {
        0xa8 if resident => 0,
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
    Some((
        r.address,
        opcode == 0xa3 || resident,
        r.fixed_frame,
        addresses,
    ))
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
        let mut comparison_windows = BTreeSet::new();
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
                if let Some(window) = comparison::fused_window(&h.cpu, &h.bus, &h.image.routines) {
                    comparison_windows.insert(window);
                }
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
        assert_eq!(comparison_windows.len(), 3);
        for w in &comparison_windows {
            assert!([w.load, w.cmp, w.branch].iter().all(|pc| seen.contains(pc)));
        }
        // This workload never reaches Odd(0), so its true edge is absent.
        // The targeted task probe below covers both outcomes per predicate.
        assert!(
            comparison_windows
                .iter()
                .all(|w| seen.contains(&w.no) || seen.contains(&w.yes))
        );
        assert!(return_windows.iter().any(|w| !w.1));
        assert!(
            return_windows
                .iter()
                .all(|w| w.3.iter().all(|pc| seen.contains(pc)))
        );
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            std::fs::write(std::path::Path::new(&directory).join(format!("comparison-preemption-{optimize}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({"optimized":optimize,"enabled_instruction_addresses":seen.len(),
                    "windows":comparison_windows.iter().map(|w| serde_json::json!({"load":w.load,"cmp":w.cmp,"predicate":w.predicate,
                        "irq_tested_addresses":w.addresses().into_iter().filter(|pc| seen.contains(pc)).collect::<Vec<_>>()})).collect::<Vec<_>>()})).unwrap()).unwrap();
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
        eprintln!(
            "fused comparisons optimize={optimize}: {} qualified interruption windows",
            comparison_windows.len()
        );
    }
}

fn comparison_source(source: &str) -> String {
    source
        .replace("\r\n", "\n")
        .replace(
            "BYTE POINTER other,buffer]",
            "BYTE POINTER other,buffer CARD low,high,equal]",
        )
        .replace(
            "CARD FUNC Read",
            r#"
CARD FUNC Compare(CARD a,b)
 BYTE eq,ne,lt,ge
 eq=(a=b) ne=(a#b) lt=(a<b) ge=(a>=b)
RETURN(CARD(eq)+(CARD(ne) LSH 1)+(CARD(lt) LSH 2)+(CARD(ge) LSH 3))
CARD FUNC Read"#,
        )
        .replace(
            "  work.done=1",
            r#"
  work.low=Compare(work.seed,work.seed+1)
  work.high=Compare(work.seed+1,work.seed)
  work.equal=Compare(work.seed,work.seed)
  work.done=1"#,
        )
}
fn check_comparison_results(h: &ContextHarness) {
    check(h);
    for job in [0x7100, 0x7120] {
        // CARD fields align to two bytes after the final three-byte pointer.
        for (offset, expected) in [(12, 6), (14, 10), (16, 9)] {
            assert_eq!(h.bus.value(job + offset, 2), expected);
        }
    }
}

#[test]
fn both_comparison_outcomes_survive_each_task_irq_boundary_and_seeded_nmi() {
    let fixture = fixture("preemption.act");
    let source = comparison_source(&fixture);
    assert_eq!(source, comparison_source(&fixture.replace('\n', "\r\n")));
    for optimize in [false, true] {
        let mut h = machine_source(&source, optimize);
        let address = routine(&h.image, "Compare");
        let map = h
            .image
            .routines
            .iter()
            .find(|r| r.address == address)
            .unwrap();
        let end = address + map.size;
        let mut windows = BTreeSet::new();
        let mut targets = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut outcomes = BTreeSet::new();
        let mut flag_outcomes = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            let pc = h.cpu.pc();
            if h.cpu.is_instruction_boundary() && r.p & 4 == 0 && [0x2000, 0x2100].contains(&r.d) {
                if (address..end).contains(&pc) {
                    if let Some(w) = comparison::window(&h.cpu, &h.bus, &h.image.routines) {
                        targets.extend(w.addresses().into_iter().map(|pc| (r.d, pc)));
                        windows.insert(w);
                    }
                }
                let mut new_flags = false;
                for w in &windows {
                    if pc == w.done + 2 {
                        assert!(r.a & 255 <= 1);
                        outcomes.insert((r.d, w.predicate, r.a as u8));
                    }
                    if pc == w.branch {
                        let truth = match w.predicate {
                            0xf0 => r.p & 2 != 0,
                            0xd0 => r.p & 2 == 0,
                            0x90 => r.p & 1 == 0,
                            0xb0 => r.p & 1 != 0,
                            _ => unreachable!(),
                        };
                        new_flags |= flag_outcomes.insert((r.d, w.predicate, u8::from(truth)));
                    }
                }
                let site = (r.d, pc);
                if targets.contains(&site) && (seen.insert(site) || new_flags) {
                    let cpu = h.cpu.clone();
                    let bus = h.bus.clone();
                    run_injected(&mut h, true, None);
                    check_comparison_results(&h);
                    h.cpu = cpu;
                    h.bus = bus;
                }
            }
            h.tick(Inputs::default());
        }
        check_comparison_results(&h);
        assert_eq!(windows.len(), 4);
        assert_eq!(targets, seen);
        let expected = [0x2000u16, 0x2100]
            .into_iter()
            .flat_map(|d| {
                [0xf0u8, 0xd0, 0x90, 0xb0]
                    .into_iter()
                    .flat_map(move |predicate| {
                        [0u8, 1].into_iter().map(move |value| (d, predicate, value))
                    })
            })
            .collect();
        assert_eq!(outcomes, expected);
        assert_eq!(flag_outcomes, expected);
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            std::fs::write(std::path::Path::new(&directory).join(format!("comparison-task-preemption-{optimize}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({"optimized":optimize,"irq_sites":seen,"site_columns":["task_domain","pc"],
                    "outcomes":outcomes,"irq_with_live_flag_outcomes":flag_outcomes,"outcome_columns":["task_domain","predicate_opcode","boolean"],"every_window_boundary_tested":true})).unwrap()).unwrap();
        }
        eprintln!(
            "word comparisons optimize={optimize}: {} task/PC sites, {} predicate/outcome/domain combinations",
            seen.len(),
            outcomes.len()
        );
        for seed in [0x81620260916, 0x5eedcafe] {
            let mut h = machine_source(&source, optimize);
            run_injected(&mut h, false, Some(seed));
            check_comparison_results(&h);
        }
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

fn fused_source(source: &str) -> String {
    source
        .replace("\r\n", "\n")
        .replace(
            "BYTE POINTER other,buffer]",
            "BYTE POINTER other,buffer CARD low,high,equal,immLow,immHigh,immEqual]",
        )
        .replace(
            "CARD FUNC Read",
            r#"
CARD FUNC BranchEq(CARD a,b) IF a=b THEN RETURN($A001) FI RETURN($A000)
CARD FUNC BranchNe(CARD a,b) IF a#b THEN RETURN($A002) FI RETURN($A000)
CARD FUNC BranchLt(CARD a,b) IF a<b THEN RETURN($A004) FI RETURN($A000)
CARD FUNC BranchGe(CARD a,b) IF a>=b THEN RETURN($A008) FI RETURN($A000)
CARD FUNC BranchImm(CARD a) IF a<CARD($8000) THEN RETURN($1234) FI RETURN($ABCD)
CARD FUNC BranchImmEq(CARD a) IF a=CARD($8000) THEN RETURN($5678) FI RETURN($9876)
CARD FUNC Compare(CARD a,b) RETURN(BranchEq(a,b)+BranchNe(a,b)+BranchLt(a,b)+BranchGe(a,b))
CARD FUNC Read"#,
        )
        .replace(
            "  work.done=1",
            r#"
  work.low=Compare(work.seed,work.seed+1)
  work.high=Compare(work.seed+1,work.seed)
  work.equal=Compare(work.seed,work.seed)
  work.immLow=BranchImm(work.seed)+BranchImmEq(work.seed)
  work.immHigh=BranchImm(work.seed+32768)+BranchImmEq(work.seed+32768)
  work.immEqual=BranchImm(32768)+BranchImmEq(32768)
  work.done=1"#,
        )
}
fn fused_machine(source: &str, optimize: bool) -> ContextHarness {
    use actionc::{mir65816::*, nir::TempId, target::ByteSize};
    let mut prepared = prepare(source, optimize);
    // Preserve nonempty cyclic edge copies in both modes. Each successor rotates
    // three words and returns its independently chosen word argument. This is a
    // verified target-IR fixture, not an optimization of the source program.
    for r in prepared.mir.routines.iter_mut().filter(|r| {
        r.name.eq_ignore_ascii_case("BranchEq")
            || r.name.to_ascii_uppercase().contains("_BRANCHEQ_")
    }) {
        let ty = r.temps[0].1.clone();
        let word = ByteSize::new(2);
        let mut next = r.temps.iter().map(|(id, _)| id.0).max().unwrap() + 1;
        let mut next_block = r.blocks.iter().map(|b| b.id.0).max().unwrap() + 1;
        let mut returns = std::collections::BTreeMap::new();
        let mut extra = vec![];
        for block in &mut r.blocks {
            if let Mir65816Terminator::Return {
                value: Some(original),
                ..
            } = &block.terminator
            {
                assert!(block.ops.is_empty() && matches!(original, Mir65816Value::U16(_)));
                returns.insert(block.id, original.clone());
                let ret = block.terminator.clone();
                let mut ids = [TempId(next), TempId(next + 1), TempId(next + 2)];
                next += 3;
                for id in ids {
                    r.temps.push((id, ty.clone()));
                    block.params.push((id, word));
                }
                let val = |id| Mir65816Value::Temp(id, word);
                // Three cyclic assignments return the original word. All words
                // have nonzero high bytes; allocator reuse creates overlapping
                // source/destination homes, protected by the staging phase.
                block.terminator = Mir65816Terminator::Goto(Mir65816Edge {
                    target: actionc::nir::BlockId(next_block),
                    args: vec![val(ids[1]), val(ids[2]), val(ids[0])],
                });
                for step in 0..3 {
                    ids = [TempId(next), TempId(next + 1), TempId(next + 2)];
                    next += 3;
                    for id in ids {
                        r.temps.push((id, ty.clone()));
                    }
                    let id = actionc::nir::BlockId(next_block);
                    next_block += 1;
                    let terminator = if step == 2 {
                        let mut t = ret.clone();
                        if let Mir65816Terminator::Return { value, .. } = &mut t {
                            *value = Some(val(ids[0]));
                        }
                        t
                    } else {
                        Mir65816Terminator::Goto(Mir65816Edge {
                            target: actionc::nir::BlockId(next_block),
                            args: vec![val(ids[1]), val(ids[2]), val(ids[0])],
                        })
                    };
                    extra.push(Mir65816Block {
                        id,
                        params: ids.into_iter().map(|id| (id, word)).collect(),
                        ops: vec![],
                        terminator,
                    });
                }
            }
        }
        r.blocks.extend(extra);
        assert_eq!(returns.len(), 2);
        for block in &mut r.blocks {
            if let Mir65816Terminator::Branch {
                then_edge,
                else_edge,
                ..
            } = &mut block.terminator
            {
                for edge in [then_edge, else_edge] {
                    edge.args.extend([
                        returns[&edge.target].clone(),
                        Mir65816Value::U16(0x8001),
                        Mir65816Value::U16(0xffff),
                    ]);
                }
            }
        }
    }
    actionc::mir65816::verify_program(&prepared.mir).unwrap();
    if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&directory).join(format!("fused-preemption-mir-{optimize}.txt")),
            format!("{:#?}", prepared.mir),
        )
        .unwrap();
    }
    initialize(ContextHarness::from_prepared(
        source,
        optimize,
        "Task",
        &[0x7100, 0x7120],
        prepared,
    ))
}
fn check_fused_results(h: &ContextHarness) {
    check(h);
    for job in [0x7100, 0x7120] {
        for (offset, expected) in [
            (12, 0x8006),
            (14, 0x800a),
            (16, 0x8009),
            (18, 0x1234u16.wrapping_add(0x9876)),
            (20, 0xabcdu16.wrapping_add(0x9876)),
            (22, 0xabcdu16.wrapping_add(0x5678)),
        ] {
            assert_eq!(
                h.bus.value(job + offset, 2),
                u32::from(expected),
                "job {job:x} field {offset}"
            );
        }
    }
}
fn run_checked_fused_irq(h: &mut ContextHarness) -> (u32, u8) {
    // IRQ is sampled at the end of the instruction whose boundary arms it.
    // Independently execute that instruction without IRQ to obtain the exact
    // A/P (and other registers) which the bridge must restore at its saved PC.
    let mut reference = h.cpu.clone();
    let mut bus = h.bus.clone();
    reference.tick(&mut bus, Inputs::default()).unwrap();
    while !reference.is_instruction_boundary() {
        reference.tick(&mut bus, Inputs::default()).unwrap();
    }
    let before = reference.registers();
    let pc = reference.pc();
    let mut acknowledged = false;
    for _ in 0..2_000_000 {
        let writes = h.bus.writes.len();
        h.tick(Inputs {
            irq: !acknowledged,
            ..Default::default()
        });
        acknowledged |= h.bus.writes[writes..].iter().any(|&(a, _)| a == IRQ_ACK);
        if acknowledged
            && h.cpu.is_instruction_boundary()
            && h.cpu.pc() == pc
            && h.cpu.registers().d == before.d
        {
            assert_eq!(
                h.cpu.registers(),
                before,
                "IRQ changed live fused comparison state"
            );
            let ceiling = if before.d == 0x2000 { 0x5000 } else { 0x6000 };
            assert_eq!(
                &h.bus.ram[usize::from(before.s) + 1..ceiling],
                &bus.ram[usize::from(before.s) + 1..ceiling],
                "IRQ changed invocation-owned frame/staging"
            );
            run_injected(h, false, None);
            return (pc, before.p);
        }
        assert!(
            !h.cpu.is_stopped(),
            "task never resumed its interrupted instruction"
        );
    }
    panic!("fused IRQ restoration budget exhausted");
}

#[test]
fn fused_flags_and_edge_copies_survive_both_task_irq_outcomes_and_seeded_nmi() {
    let fixture = fixture("preemption.act");
    let source = fused_source(&fixture);
    assert_eq!(source, fused_source(&fixture.replace('\n', "\r\n")));
    for optimize in [false, true] {
        let mut h = fused_machine(&source, optimize);
        let ranges: Vec<_> = h
            .image
            .routines
            .iter()
            .filter(|r| r.name.to_ascii_uppercase().contains("BRANCH"))
            .map(|r| r.address..r.address + r.size)
            .collect();
        assert_eq!(ranges.len(), 6);
        let mut windows = BTreeSet::new();
        let mut word_sites = BTreeSet::new();
        let mut overlapping_domains = BTreeSet::new();
        let mut targets = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut flags = BTreeSet::new();
        let mut cmp_outcomes = BTreeSet::new();
        let mut restorations = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            let pc = h.cpu.pc();
            if h.cpu.is_instruction_boundary() && r.p & 4 == 0 && [0x2000, 0x2100].contains(&r.d) {
                if ranges.iter().any(|range| range.contains(&pc)) {
                    if let Some(w) = comparison::fused_window(&h.cpu, &h.bus, &h.image.routines) {
                        targets.extend(w.addresses().into_iter().map(|pc| (r.d, pc)));
                        windows.insert(w);
                    }
                }
                if ranges.iter().any(|range| range.contains(&pc)) && r.p & 0x30 == 0 {
                    let range = ranges.iter().find(|range| range.contains(&pc)).unwrap();
                    if let Some(w) = word_edge::decode(&h.bus, pc, range.clone()) {
                        if w.moves.iter().any(|&((stack, source), _, dest)| {
                            stack
                                && source != u16::from(dest)
                                && w.moves.iter().any(|&(_, _, d)| u16::from(d) == source)
                        }) {
                            overlapping_domains.insert(r.d);
                        }
                        for site in w.sites {
                            targets.insert((r.d, site));
                            word_sites.insert((r.d, site));
                        }
                    }
                }
                let mut new_flags = false;
                for w in &windows {
                    if pc == w.cmp {
                        let right = if w.sources[1].0 {
                            h.bus.value(u32::from(r.s) + u32::from(w.sources[1].1), 2) as u16
                        } else {
                            w.sources[1].1
                        };
                        let truth = match w.predicate {
                            0xf0 => r.a == right,
                            0xd0 => r.a != right,
                            0x90 => r.a < right,
                            0xb0 => r.a >= right,
                            _ => unreachable!(),
                        };
                        // Re-arm CMP for both outcomes: its completion leaves live
                        // flags which must survive IRQ before the branch executes.
                        new_flags |= cmp_outcomes.insert((r.d, w.load, truth));
                    }
                    if pc == w.branch {
                        let truth = match w.predicate {
                            0xf0 => r.p & 2 != 0,
                            0xd0 => r.p & 2 == 0,
                            0x90 => r.p & 1 == 0,
                            0xb0 => r.p & 1 != 0,
                            _ => unreachable!(),
                        };
                        new_flags |= flags.insert((r.d, w.load, truth));
                    }
                }
                let site = (r.d, pc);
                if targets.contains(&site) && (seen.insert(site) || new_flags) {
                    let cpu = h.cpu.clone();
                    let bus = h.bus.clone();
                    let restored = run_checked_fused_irq(&mut h);
                    restorations.insert((r.d, pc, restored.0, restored.1));
                    check_fused_results(&h);
                    h.cpu = cpu;
                    h.bus = bus;
                }
            }
            h.tick(Inputs::default());
        }
        check_fused_results(&h);
        assert_eq!(windows.len(), 6);
        assert_eq!(
            windows
                .iter()
                .filter(|w| w.edges[0].len() == 1 && w.edges[1].len() == 2)
                .count(),
            5,
            "all empty false/true transfers must remain in IRQ coverage"
        );
        assert!(!word_sites.is_empty());
        assert_eq!(overlapping_domains, BTreeSet::from([0x2000, 0x2100]));
        assert_eq!(targets, seen);
        assert!(windows.iter().any(|w| w.edges.iter().any(|e| e.len() > 3)));
        let expected: BTreeSet<_> = [0x2000u16, 0x2100]
            .into_iter()
            .flat_map(|d| {
                windows
                    .iter()
                    .flat_map(move |w| [false, true].map(|truth| (d, w.load, truth)))
            })
            .collect();
        assert_eq!(flags, expected);
        assert_eq!(cmp_outcomes, expected);
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            std::fs::write(std::path::Path::new(&directory).join(format!("fused-task-preemption-{optimize}.json")),serde_json::to_vec_pretty(&serde_json::json!({"irq_sites":seen,"word_edge_sites":word_sites,"overlapping_domains":overlapping_domains,"site_columns":["task_domain","pc"],"live_flag_outcomes":flags,"irq_after_cmp_outcomes":cmp_outcomes,"register_restorations":restorations,"restoration_columns":["domain","armed_pc","restored_pc","restored_p"],"outcome_columns":["task_domain","load_pc","truth"],"windows":windows.iter().map(|w|serde_json::json!({"load":w.load,"cmp":w.cmp,"branch":w.branch,"predicate":w.predicate,"sources":w.sources,"edges":w.edges})).collect::<Vec<_>>()})).unwrap()).unwrap();
        }
        eprintln!(
            "fused comparisons optimize={optimize}: {} task/PC sites, {} flag outcomes",
            seen.len(),
            flags.len()
        );
        for seed in [0x81620260916, 0x5eedcafe] {
            let mut h = fused_machine(&source, optimize);
            run_injected(&mut h, false, Some(seed));
            check_fused_results(&h);
        }
    }
}

fn single_word_machine(source: &str, optimize: bool) -> ContextHarness {
    let mut p = prepare(source, optimize);
    edges::single_returns(&mut p, "BranchEq");
    initialize(ContextHarness::from_prepared(
        source,
        optimize,
        "Task",
        &[0x7100, 0x7120],
        p,
    ))
}

#[test]
fn direct_word_edges_preserve_live_a_and_frame_at_every_transfer_boundary() {
    let source = fused_source(&fixture("preemption.act"));
    for optimize in [false, true] {
        let mut h = single_word_machine(&source, optimize);
        assert_eq!(h.bus.single_word_edges.len(), 4);
        assert!(h.bus.single_word_edges.values().all(|s| s.direct));
        let mut targets = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut forms = BTreeSet::new();
        let mut restored = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            let pc = h.cpu.pc();
            if h.cpu.is_instruction_boundary() && r.p & 0x34 == 0 && [0x2000, 0x2100].contains(&r.d)
            {
                for routine in &h.image.routines {
                    if let Some(w) = word_edge::decode(
                        &h.bus,
                        pc,
                        routine.address..routine.address + routine.size,
                    ) {
                        if w.direct {
                            forms.insert((r.d, w.moves[0].0.0, w.moves[0].0.1));
                            targets
                                .extend(w.sites.into_iter().chain([w.target]).map(|pc| (r.d, pc)));
                        }
                    }
                }
                let site = (r.d, pc);
                if targets.contains(&site) && seen.insert(site) {
                    let cpu = h.cpu.clone();
                    let bus = h.bus.clone();
                    let after = run_checked_fused_irq(&mut h);
                    restored.insert((r.d, pc, after.0, after.1));
                    check_fused_results(&h);
                    h.cpu = cpu;
                    h.bus = bus;
                }
            }
            h.tick(Inputs::default());
        }
        check_fused_results(&h);
        assert_eq!(targets, seen);
        for d in [0x2000, 0x2100] {
            for immediate in [0xa000, 0xa001] {
                assert!(forms.contains(&(d, false, immediate)));
            }
            assert!(forms.iter().any(|&(domain, stack, _)| domain == d && stack));
        }
        assert_eq!(restored.len(), seen.len());
        assert!(seen.len() >= 24);
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            std::fs::write(
                std::path::Path::new(&directory)
                    .join(format!("single-word-preemption-{optimize}.json")),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"sites":seen,"restorations":restored,"forms":forms}),
                )
                .unwrap(),
            )
            .unwrap();
        }
        eprintln!(
            "single-word edges optimize={optimize}: {} IRQ sites",
            seen.len()
        );
        for seed in [0x81620260916, 0x5eedcafe] {
            let mut h = single_word_machine(&source, optimize);
            run_injected(&mut h, false, Some(seed));
            check_fused_results(&h);
        }
    }
}

fn forwarding_source(source: &str) -> String {
    source
        .replace("\r\n", "\n")
        .replace(
            "BYTE POINTER other,buffer]",
            "BYTE POINTER other,buffer CARD fadd,fsub,flow,fhigh,fzero]",
        )
        .replace(
            "CARD FUNC Read",
            r#"
CARD FUNC ForwardAdd(CARD x) RETURN(x+1+2)
CARD FUNC ForwardSub(CARD x) x=x-1 RETURN(x)
CARD FUNC ForwardGt(CARD x,y) IF x>y THEN RETURN(x) FI RETURN(y)
CARD FUNC Read"#,
        )
        .replace(
            "  work.done=1",
            r#"
  work.fadd=ForwardAdd(work.seed)
  work.fsub=ForwardSub(0)
  work.flow=ForwardGt(work.seed,work.seed+1)
  work.fhigh=ForwardGt(work.seed+1,work.seed)
  work.fzero=ForwardAdd(65533)
  work.done=1"#,
        )
}
fn check_forwarding_results(h: &ContextHarness) {
    check(h);
    for (job, seed) in [(0x7100, 13u32), (0x7120, 41)] {
        for (offset, value) in [
            (12, seed + 3),
            (14, 65535),
            (16, seed + 1),
            (18, seed + 1),
            (20, 0),
        ] {
            assert_eq!(
                h.bus.value(job + offset, 2),
                value,
                "forwarded task field {offset}"
            );
        }
    }
}
#[test]
fn forwarded_values_flags_and_return_teardown_survive_both_task_irq_domains() {
    let original = fixture("preemption.act");
    let source = forwarding_source(&original);
    assert_eq!(source, forwarding_source(&original.replace('\n', "\r\n")));
    for optimize in [false, true] {
        let mut h = machine_source(&source, optimize);
        let selected: BTreeSet<_> = h
            .image
            .routines
            .iter()
            .filter(|r| r.name.to_ascii_uppercase().contains("FORWARD"))
            .map(|r| r.id)
            .collect();
        let sites: Vec<_> = h
            .bus
            .forwarded_words
            .values()
            .filter(|s| selected.contains(&s.routine.0))
            .cloned()
            .collect();
        assert!(sites.iter().all(|s| s.forwarded()));
        let mut boundaries = BTreeSet::new();
        for s in &sites {
            let bytes = &h.bus.ram[s.range.start as usize..s.range.end as usize];
            let ins = forwarding::instructions(bytes);
            boundaries.insert(s.store);
            let count = if s.kind == forwarding::Kind::Return {
                8
            } else if s.kind == forwarding::Kind::Store {
                2
            } else {
                3
            };
            boundaries.extend(
                ins.range((s.consumer - s.range.start) as usize..)
                    .take(count)
                    .map(|(&at, _)| s.range.start + at as u32),
            );
        }
        let targets: BTreeSet<_> = [0x2000, 0x2100]
            .into_iter()
            .flat_map(|d| boundaries.iter().map(move |&pc| (d, pc)))
            .collect();
        let mut seen = BTreeSet::new();
        let mut forms = BTreeSet::new();
        let mut restored = BTreeSet::new();
        let mut nz = BTreeSet::new();
        let mut truth = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            let pc = h.cpu.pc();
            if h.cpu.is_instruction_boundary() && r.p & 0x34 == 0 && [0x2000, 0x2100].contains(&r.d)
            {
                if let Some(s) = sites.iter().find(|s| s.consumer == pc) {
                    assert!(s.valid(&h.bus));
                    assert_eq!(
                        u32::from(r.a),
                        h.bus.value(u32::from(r.s) + u32::from(s.slot), 2)
                    );
                    forms.insert((r.d, s.kind));
                    nz.insert(r.p & 0x82);
                }
                if let Some(s) = sites
                    .iter()
                    .find(|s| s.kind == forwarding::Kind::Compare && s.consumer + 2 == pc)
                {
                    let _ = s;
                    truth.insert((r.d, r.p & 1 != 0));
                }
                if targets.contains(&(r.d, pc)) && seen.insert((r.d, pc)) {
                    let cpu = h.cpu.clone();
                    let bus = h.bus.clone();
                    let after = run_checked_fused_irq(&mut h);
                    restored.insert((r.d, pc, after.0, after.1));
                    check_forwarding_results(&h);
                    h.cpu = cpu;
                    h.bus = bus;
                }
            }
            h.tick(Inputs::default());
        }
        check_forwarding_results(&h);
        assert_eq!(seen, targets);
        assert_eq!(restored.len(), seen.len());
        for d in [0x2000, 0x2100] {
            for k in [
                forwarding::Kind::Arithmetic,
                forwarding::Kind::Compare,
                forwarding::Kind::Store,
                forwarding::Kind::Return,
            ] {
                assert!(forms.contains(&(d, k)));
            }
            for b in [false, true] {
                assert!(truth.contains(&(d, b)));
            }
        }
        assert_eq!(nz, BTreeSet::from([0, 2, 0x80]));
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            std::fs::write(std::path::Path::new(&directory).join(format!("accumulator-preemption-{optimize}.json")),serde_json::to_vec_pretty(&serde_json::json!({"sites":seen,"restorations":restored,"forms":forms.iter().map(|(d,k)|(d,format!("{k:?}"))).collect::<Vec<_>>(),"nz":nz,"cmp_carry_outcomes":truth})).unwrap()).unwrap();
        }
        eprintln!(
            "accumulator forwarding optimize={optimize}: {} IRQ sites, {} forms",
            seen.len(),
            forms.len()
        );
        for seed in [0x81620260916, 0x5eedcafe] {
            let mut h = machine_source(&source, optimize);
            run_injected(&mut h, false, Some(seed));
            check_forwarding_results(&h);
        }
    }
}
