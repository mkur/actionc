mod support;
use actionc::mir65816::{Mir65816Terminator, Mir65816Value, emit::Label};
use actionc_vm::native65816::Inputs;
use support::*;

const SOURCE: &str = "BYTE POINTER input=$7100,output=$7200 \
        BYTE POINTER FUNC Work(BYTE POINTER seed) BYTE POINTER p p=seed \
        WHILE p#BYTE POINTER(2) DO p==+1 OD RETURN(p) \
        PROC Main() output=Work(input) RETURN";
const MULTI_SOURCE: &str = "BYTE POINTER input=$7100,output=$7200 \
        BYTE POINTER FUNC Work(BYTE POINTER seed) BYTE POINTER p,q p=seed q=seed \
        WHILE p#BYTE POINTER(2) DO p==+1 q==+1 OD RETURN(q) \
        PROC Main() output=Work(input) RETURN";

const CYCLE_SOURCE: &str = "BYTE POINTER input=$7100,output=$7200 \
        BYTE POINTER FUNC Work(BYTE POINTER seed) BYTE POINTER p,q,t p=seed q=BYTE POINTER(2) \
        WHILE p#BYTE POINTER(2) DO t=p p=q q=t OD RETURN(q) \
        PROC Main() output=Work(input) RETURN";

fn check_backedges(source: &str, multiple: bool, cyclic: bool) {
    let p = prepare(source, true);
    let c = p.compile(&layout()).unwrap();
    let r = p.mir.routines.iter().find(|r| r.name == "Work").unwrap();
    let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
    let linked = c.image.routines.iter().find(|m| m.id == r.id.0).unwrap();
    assert!(m.frame.edge_copies.iter().any(|s| s.width >= 2));
    if cyclic {
        assert_eq!(
            m.frame
                .edge_copies
                .iter()
                .map(|s| s.width)
                .collect::<Vec<_>>(),
            [3, 3] // The constant-bearing entry also needs complete byte staging.
        );
    }
    let mut cycles = 0;
    let mut sites = vec![];
    for (i, block) in r.blocks.iter().enumerate() {
        let Mir65816Terminator::Goto(edge) = &block.terminator else {
            continue;
        };
        if edge.args.is_empty() || multiple != (edge.args.len() > 1) {
            continue;
        }
        let target = r.blocks.iter().find(|b| b.id == edge.target).unwrap();
        let Some(moves) = edge
            .args
            .iter()
            .zip(&target.params)
            .map(|(arg, &(dest, _))| {
                let Mir65816Value::Temp(src, bytes) = arg else {
                    return None;
                };
                (bytes.get() == 3).then(|| {
                    (
                        m.frame.temps[src].stack().unwrap().offset,
                        m.frame.temps[&dest].stack().unwrap().offset,
                    )
                })
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if moves.len() == 2
            && moves[0].0 == moves[1].1
            && moves[1].0 == moves[0].1
            && moves[0].0 != moves[0].1
        {
            cycles += 1;
        }
        let span = &m.code.mir_spans[&(block.id, block.ops.len())];
        let transfer = m
            .code
            .mir_transfers
            .iter()
            .find(|t| t.source == Label(i as u32))
            .unwrap();
        sites.push((
            linked.address + span.start as u32,
            linked.address + m.code.labels[&transfer.target] as u32,
            moves,
        ));
    }
    assert!(!sites.is_empty());
    if cyclic {
        assert!(cycles > 0);
    }
    let caller = caller(c.image.entry);
    let mut checked = 0;
    for value in [0xfffffdu32, 0xfffffe, 0xffffff, 0, 1, 2] {
        for mask in [0, 4] {
            let mut h = Harness::new(&c.image, &caller, mask);
            h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
            for _ in 0..100_000 {
                if h.cpu.is_stopped() {
                    break;
                }
                if h.cpu.is_instruction_boundary()
                    && let Some((_, target, moves)) = sites.iter().find(|s| s.0 == h.cpu.pc())
                {
                    let before = h.cpu.registers();
                    let s = u32::from(before.s);
                    let values: Vec<_> = moves
                        .iter()
                        .map(|&(src, _)| h.bus.value(s + u32::from(src), 3))
                        .collect();
                    let value = *values.last().unwrap();
                    for _ in 0..1_000 {
                        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                        if h.cpu.is_instruction_boundary() && h.cpu.pc() == *target {
                            break;
                        }
                    }
                    assert_eq!(h.cpu.pc(), *target);
                    let after = h.cpu.registers();
                    assert_eq!(after.a, (before.a & 0xff00) | (value >> 16) as u16);
                    assert_eq!(
                        (after.x, after.y, after.s, after.d, after.dbr),
                        (before.x, before.y, before.s, before.d, before.dbr)
                    );
                    assert_eq!(after.p & 0x7d, before.p & 0x5d); // preserve C/V/I/D/X, finish A16
                    let bank = (value >> 16) as u8;
                    assert_eq!(
                        after.p & 0x82,
                        (bank & 0x80) | if bank == 0 { 2 } else { 0 }
                    );
                    for (&(_, dst), value) in moves.iter().zip(values) {
                        assert_eq!(h.bus.value(s + u32::from(dst), 3), value);
                    }
                    checked += 1;
                } else {
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                }
            }
            assert!(h.cpu.is_stopped());
            h.guards(mask);
            assert_eq!(h.bus.value(0x7200, 3), if cyclic { value } else { 2 });
        }
    }
    assert!(checked >= if cyclic { 10 } else { 30 });
}

#[test]
fn single_pointer_backedges_preserve_full_register_state_and_live_bindings() {
    check_backedges(SOURCE, false, false);
}
#[test]
fn multiple_pointer_backedges_preserve_parallel_values_and_full_register_state() {
    check_backedges(MULTI_SOURCE, true, false);
}

#[test]
fn cyclic_pointer_backedges_preserve_parallel_values_and_full_register_state() {
    check_backedges(CYCLE_SOURCE, true, true);
}

#[test]
fn cyclic_pointer_edge_state_save_survives_task_switching_and_reentrant_irq_calls() {
    use std::collections::BTreeSet;
    use support::context::*;
    let source = "MODULE TEST PUBLIC EXTERNAL PROC Yield() \
        VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current \
        TYPE Job=[BYTE POINTER item BYTE done BYTE POINTER result,peer] BYTE POINTER irqResult \
        BYTE POINTER FUNC Walk(BYTE POINTER seed) BYTE POINTER p,q,t p=seed q=BYTE POINTER(2) \
        WHILE p#BYTE POINTER(2) DO t=p p=q q=t OD RETURN(q) \
        CARD FUNC Dispatch(CARD saved BYTE reason) irqAck=1 irqResult=Walk(BYTE POINTER($FFFFFF)) \
        IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI taskB=saved current=0 RETURN(taskA) \
        PROC Task(Job POINTER work) work.result=Walk(work.item) work.done=1 \
        WHILE work.peer^=0 DO Yield() OD RETURN PROC Main() RETURN ENDMODULE";
    let check = |h: &ContextHarness| {
        h.guards();
        assert_eq!(h.bus.value(DONE, 2), 1);
        assert_eq!(h.bus.value(0x7103, 1), 1);
        assert_eq!(h.bus.value(0x7123, 1), 1);
        for (at, value) in [
            (0x7104, 0xfffffe),
            (0x7124, 0),
            (context::symbol(&h.image, "irqResult"), 0xffffff),
        ] {
            assert_eq!(h.bus.value(at, 3), value);
        }
    };
    for optimize in [false, true] {
        let mut h = ContextHarness::new(source, optimize, "Task", &[0x7100, 0x7120]);
        for (job, value, peer) in [(0x7100usize, 0xfffffeu32, 0x7123u32), (0x7120, 0, 0x7103)] {
            h.bus.ram[job..job + 3].copy_from_slice(&value.to_le_bytes()[..3]);
            h.bus.ram[job + 7..job + 10].copy_from_slice(&peer.to_le_bytes()[..3]);
        }
        let start = context::routine(&h.image, "Walk");
        let end = start
            + h.image
                .routines
                .iter()
                .find(|r| r.address == start)
                .unwrap()
                .size;
        let mut seen = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            if h.cpu.is_instruction_boundary()
                && r.p & 4 == 0
                && [0x2000, 0x2100].contains(&r.d)
                && (start..end).contains(&h.cpu.pc())
                && seen.insert((r.d, h.cpu.pc()))
            {
                let cpu = h.cpu.clone();
                let bus = h.bus.clone();
                let mut pending = true;
                for tick in 0..2_000_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    let writes = h.bus.writes.len();
                    h.tick(Inputs {
                        irq: pending,
                        nmi: tick == 40,
                        ..Default::default()
                    });
                    if h.bus.writes[writes..].iter().any(|&(at, _)| at == IRQ_ACK) {
                        pending = false;
                    }
                }
                check(&h);
                h.cpu = cpu;
                h.bus = bus;
            }
            h.tick(Inputs::default());
        }
        check(&h);
        assert!(
            seen.len() >= 80,
            "only {} pointer loop interrupt sites",
            seen.len()
        );
    }
}

#[test]
fn pointer_edge_frames_and_transfers_survive_o65_relocation() {
    for (source, optimize) in [SOURCE, MULTI_SOURCE, CYCLE_SOURCE]
        .into_iter()
        .flat_map(|s| [(s, false), (s, true)])
    {
        let bytes = o65::compile(source, optimize, vec![]);
        for variant in 0..2 {
            let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
            let image = actionc::mir65816::o65::relocate(&bytes, &placement).unwrap();
            let caller = caller(image.entry());
            for value in [0xfffffeu32, 0xffffff, 0, 1, 2] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        h.bus.value(0x7200, 3),
                        if source == CYCLE_SOURCE { value } else { 2 }
                    );
                }
            }
        }
    }
}

#[test]
fn independent_pointer_schedules_match_byte_staging_and_preserve_hidden_b() {
    use actionc_vm::native65816::{Machine, Registers};
    // Independent explicit orders include a chain requiring reverse order,
    // repeated sources, identities, a swap, a rotation and cycle fan-out.
    for (sources, transfers) in [
        ([16, 20, 24], vec![(16, 4), (20, 8), (24, 12)]),
        ([16, 4, 8], vec![(8, 12), (4, 8), (16, 4)]),
        ([16, 16, 16], vec![(16, 4), (16, 8), (16, 12)]),
        ([16, 20, 12], vec![(16, 4), (20, 8)]),
        ([4, 8, 12], vec![]),
        ([8, 4, 12], vec![(4, 68), (8, 4), (68, 8)]),
        ([8, 12, 4], vec![(4, 68), (8, 4), (12, 8), (68, 12)]),
        ([8, 4, 4], vec![(4, 12), (4, 68), (8, 4), (68, 8)]),
    ] {
        let destinations = [4, 8, 12];
        let mut old = String::from("sep #$20\n.a8\n");
        for i in 0..3 {
            for byte in 0..3 {
                old.push_str(&format!(
                    "lda {},s\nsta {},s\n",
                    sources[i] + byte,
                    64 + 4 * i + byte
                ));
            }
        }
        for i in 0..3 {
            for byte in 0..3 {
                old.push_str(&format!(
                    "lda {},s\nsta {},s\n",
                    64 + 4 * i + byte,
                    destinations[i] + byte
                ));
            }
        }
        old.push_str("rep #$20\n.a16\nstp\nnop");
        let old = assemble(&old, 0x040000);
        let mut new = String::from("rep #$20\n.a16\n");
        if !transfers.is_empty() {
            new.push_str("sta 64,s\n");
            for (source, destination) in transfers {
                for byte in [0, 1] {
                    new.push_str(&format!(
                        "lda {},s\nsta {},s\n",
                        source + byte,
                        destination + byte
                    ));
                }
            }
            new.push_str("lda 64,s\n");
        }
        new.push_str("sep #$20\n.a8\nlda 14,s\nrep #$20\n.a16\nstp\nnop");
        let new = assemble(&new, 0x040000);
        for value in [0u32, 0xff, 0x100, 0xffff, 0x10000, 0x800000, 0xffffff] {
            for p in (0..=255u8).filter(|p| p & 0x18 == 0) {
                let initial = Registers {
                    a: 0xabcd,
                    x: 0x5678,
                    y: 0x9abc,
                    s: 0x5f80,
                    d: 0x2000,
                    dbr: 0,
                    pbr: 4,
                    pc: 0,
                    p,
                    emulation_mode: false,
                };
                let run = |code: &[u8]| {
                    let mut bus = Bus::new();
                    bus.map(0x040000, code, false);
                    bus.map(0x4000, &[0xa5; 0x2000], true);
                    for (i, at) in [4, 8, 12, 16, 20, 24].into_iter().enumerate() {
                        let v = value ^ (0x12345 * i as u32);
                        let at = usize::from(initial.s) + at;
                        bus.ram[at..at + 3].copy_from_slice(&v.to_le_bytes()[..3]);
                    }
                    let mut cpu = Machine::start_at(initial);
                    assert!(
                        cpu.run_until(&mut bus, 500, |_| Inputs::default(), |c| c.is_stopped())
                            .unwrap()
                    );
                    (
                        cpu.registers(),
                        cpu.cycles(),
                        bus.ram[0x4000..0x6000].to_vec(),
                    )
                };
                let (a, ac, amem) = run(&new);
                let (b, bc, bmem) = run(&old);
                assert_eq!(
                    (a.a, a.x, a.y, a.s, a.d, a.dbr, a.p, a.emulation_mode),
                    (b.a, b.x, b.y, b.s, b.d, b.dbr, b.p, b.emulation_mode)
                );
                assert!(ac < bc);
                for i in 0..amem.len() {
                    if !(0x1fc0..0x1fcb).contains(&i) {
                        assert_eq!(amem[i], bmem[i]);
                    }
                }
                assert_eq!(amem[0x1fc2], 0xa5); // no third staging byte
            }
        }
    }
}
