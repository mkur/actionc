mod support;
use actionc::mir65816::{Mir65816Terminator, Mir65816Value, emit::Label};
use actionc_vm::native65816::Inputs;
use support::*;

const SOURCE: &str = "BYTE POINTER input=$7100,output=$7200 \
        BYTE POINTER FUNC Work(BYTE POINTER seed) BYTE POINTER p p=seed \
        WHILE p#BYTE POINTER(2) DO p==+1 OD RETURN(p) \
        PROC Main() output=Work(input) RETURN";
#[test]
fn single_pointer_backedges_preserve_full_register_state_and_live_bindings() {
    let p = prepare(SOURCE, true);
    let c = p.compile(&layout()).unwrap();
    let r = p.mir.routines.iter().find(|r| r.name == "Work").unwrap();
    let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
    let linked = c.image.routines.iter().find(|m| m.id == r.id.0).unwrap();
    assert!(m.frame.edge_copies.iter().any(|s| s.width == 2));
    let mut sites = vec![];
    for (i, block) in r.blocks.iter().enumerate() {
        let Mir65816Terminator::Goto(edge) = &block.terminator else {
            continue;
        };
        let [Mir65816Value::Temp(src, bytes)] = edge.args.as_slice() else {
            continue;
        };
        if bytes.get() != 3 {
            continue;
        }
        let target = r.blocks.iter().find(|b| b.id == edge.target).unwrap();
        let src = m.frame.temps[src].stack().unwrap();
        let dst = m.frame.temps[&target.params[0].0].stack().unwrap();
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
            src.offset,
            dst.offset,
        ));
    }
    assert!(!sites.is_empty());
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
                    && let Some(&(_, target, src, dst)) = sites.iter().find(|s| s.0 == h.cpu.pc())
                {
                    let before = h.cpu.registers();
                    let s = u32::from(before.s);
                    let value = h.bus.value(s + u32::from(src), 3);
                    for _ in 0..1_000 {
                        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                        if h.cpu.is_instruction_boundary() && h.cpu.pc() == target {
                            break;
                        }
                    }
                    assert_eq!(h.cpu.pc(), target);
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
                    assert_eq!(h.bus.value(s + u32::from(dst), 3), value);
                    checked += 1;
                } else {
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                }
            }
            assert!(h.cpu.is_stopped());
            h.guards(mask);
            assert_eq!(h.bus.value(0x7200, 3), 2);
        }
    }
    assert!(checked >= 30);
}

#[test]
fn pointer_edge_state_save_survives_task_switching_and_reentrant_irq_calls() {
    use std::collections::BTreeSet;
    use support::context::*;
    let source = "MODULE TEST PUBLIC EXTERNAL PROC Yield() \
        VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current \
        TYPE Job=[BYTE POINTER item BYTE done BYTE POINTER result,peer] BYTE POINTER irqResult \
        BYTE POINTER FUNC Walk(BYTE POINTER seed) BYTE POINTER p p=seed \
        WHILE p#BYTE POINTER(2) DO p==+1 OD RETURN(p) \
        CARD FUNC Dispatch(CARD saved BYTE reason) irqAck=1 irqResult=Walk(BYTE POINTER($FFFFFF)) \
        IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI taskB=saved current=0 RETURN(taskA) \
        PROC Task(Job POINTER work) work.result=Walk(work.item) work.done=1 \
        WHILE work.peer^=0 DO Yield() OD RETURN PROC Main() RETURN ENDMODULE";
    let check = |h: &ContextHarness| {
        h.guards();
        assert_eq!(h.bus.value(DONE, 2), 1);
        assert_eq!(h.bus.value(0x7103, 1), 1);
        assert_eq!(h.bus.value(0x7123, 1), 1);
        for at in [0x7104, 0x7124, context::symbol(&h.image, "irqResult")] {
            assert_eq!(h.bus.value(at, 3), 2);
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
    for optimize in [false, true] {
        let bytes = o65::compile(SOURCE, optimize, vec![]);
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
                    assert_eq!(h.bus.value(0x7200, 3), 2);
                }
            }
        }
    }
}
