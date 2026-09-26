mod support;
use actionc::mir65816::{Mir65816Terminator, Mir65816Value, emit::Label};
use actionc_vm::native65816::Inputs;
use support::*;

const SOURCE: &str = "LONGCARD input=$7100,output=$7200 CARD limit=$7104
 LONGCARD FUNC Work(LONGCARD seed CARD count) LONGCARD a,b,c,t CARD n BYTE tag
 a=seed b=LONGCARD($89ABCDEF) c=LONGCARD($01020304) n=0 tag=7
 WHILE n<count DO IF n AND 1 THEN t=a a=b b=c c=t FI tag==+1 n==+1 OD
 RETURN(a XOR b XOR (c LSH 1) XOR LONGCARD(tag))
 PROC Main() output=Work(input,limit) RETURN";

#[test]
fn mixed_edges_preserve_simultaneous_values_hidden_b_and_flags() {
    let mut checked = 0;
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
            .find(|r| r.name == "Work")
            .unwrap();
        let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
        let linked = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
        let mut sites = vec![];
        for (i, block) in r.blocks.iter().enumerate() {
            let Mir65816Terminator::Goto(edge) = &block.terminator else {
                continue;
            };
            if edge.args.is_empty() {
                continue;
            }
            let target = r.blocks.iter().find(|b| b.id == edge.target).unwrap();
            let Some(moves) = edge
                .args
                .iter()
                .zip(&target.params)
                .map(|(arg, &(dest, w))| {
                    let Mir65816Value::Temp(src, _) = arg else {
                        return None;
                    };
                    Some((
                        m.frame.temps[src].stack().ok()?.offset,
                        m.frame.temps[&dest].stack().ok()?.offset,
                        w.get() as usize,
                    ))
                })
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            if !moves.iter().any(|m| m.2 == 4) {
                continue;
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
        if optimize {
            assert!(!sites.is_empty());
        }
        for seed in [0u32, 0x8000_0000, 0xffff_ffff, 0x1234_5678] {
            for count in [0u16, 1, 2, 3, 7] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&c.image, &caller(c.image.entry), mask);
                    h.bus.ram[0x7100..0x7104].copy_from_slice(&seed.to_le_bytes());
                    h.bus.ram[0x7104..0x7106].copy_from_slice(&count.to_le_bytes());
                    for _ in 0..200_000 {
                        if h.cpu.is_stopped() {
                            break;
                        }
                        if h.cpu.is_instruction_boundary()
                            && let Some((_, target, moves)) =
                                sites.iter().find(|s| s.0 == h.cpu.pc())
                        {
                            let before = h.cpu.registers();
                            let s = u32::from(before.s);
                            let values: Vec<_> = moves
                                .iter()
                                .map(|&(src, _, bytes)| h.bus.value(s + u32::from(src), bytes))
                                .collect();
                            for _ in 0..2_000 {
                                h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                                if h.cpu.is_instruction_boundary() && h.cpu.pc() == *target {
                                    break;
                                }
                            }
                            assert_eq!(h.cpu.pc(), *target);
                            let after = h.cpu.registers();
                            let byte = (values.last().unwrap()
                                >> (8 * (moves.last().unwrap().2 - 1)))
                                as u8;
                            assert_eq!(after.a, (before.a & 0xff00) | u16::from(byte));
                            assert_eq!(
                                (after.x, after.y, after.s, after.d, after.dbr),
                                (before.x, before.y, before.s, before.d, before.dbr)
                            );
                            assert_eq!(after.p & 0x7d, before.p & 0x5d);
                            assert_eq!(
                                after.p & 0x82,
                                (byte & 0x80) | if byte == 0 { 2 } else { 0 }
                            );
                            for (&(_, dst, bytes), value) in moves.iter().zip(values) {
                                assert_eq!(h.bus.value(s + u32::from(dst), bytes), value);
                            }
                            checked += 1;
                        } else {
                            h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                        }
                    }
                    assert!(h.cpu.is_stopped());
                    h.guards(mask);
                    let (mut a, mut b, mut c) = (seed, 0x89ab_cdefu32, 0x0102_0304u32);
                    for n in 0..count {
                        if n & 1 != 0 {
                            (a, b, c) = (b, c, a);
                        }
                    }
                    assert_eq!(
                        h.bus.value(0x7200, 4),
                        a ^ b ^ (c << 1) ^ (7 + u32::from(count))
                    );
                }
            }
        }
    }
    assert!(checked > 100);
}

#[test]
fn mixed_long_edge_state_save_survives_task_switching_and_reentrant_irq_calls() {
    use std::collections::BTreeSet;
    use support::context::*;
    let source = "MODULE TEST PUBLIC EXTERNAL PROC Yield() \
        VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current \
        TYPE Job=[BYTE POINTER item BYTE done BYTE POINTER result,peer] BYTE POINTER irqResult \
        BYTE POINTER FUNC Walk(BYTE POINTER seed) LONGCARD a,b,t BYTE n a=LONGCARD(ADDRESS(seed)) b=2 n=0 \
        WHILE n<2 DO t=a a=b b=t n==+1 OD RETURN(BYTE POINTER(ADDRESS(a))) \
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
            "only {} mixed loop interrupt sites",
            seen.len()
        );
    }
}
