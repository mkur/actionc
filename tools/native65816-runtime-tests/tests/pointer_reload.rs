mod support;
use actionc::mir65816::{Mir65816Op, image::TemporaryHome};
use actionc_vm::native65816::{Access, Inputs};
use std::collections::{BTreeMap, BTreeSet};
use support::{context::*, *};

const TRANSFORM: &str = "TYPE Node=[Node POINTER a,b,c] \
    PROC Transform(Node POINTER p) Node POINTER a,b,c \
    a=p.a b=p.b c=p.c a.a=b b.a=c c.a=a RETURN";
const NODES: [u32; 4] = [0x21fffau32, 0x32ffff, 0x43fffe, 0x54fffd];

fn map_nodes(bus: &mut Bus, nodes: [u32; 4], watch: bool) -> BTreeMap<u32, [u8; 11]> {
    let mut data: BTreeMap<_, _> = nodes.into_iter().map(|n| (n, [0xa5; 11])).collect();
    for (offset, pointer) in [0, 3, 6].into_iter().zip(nodes[1..].iter()) {
        data.get_mut(&nodes[0]).unwrap()[offset + 1..offset + 4]
            .copy_from_slice(&pointer.to_le_bytes()[..3]);
    }
    for (&node, bytes) in &data {
        bus.map(node - 1, bytes, true);
        if watch {
            bus.watched.extend(node - 1..node + 10);
        }
    }
    data
}
fn update_expected(data: &mut BTreeMap<u32, [u8; 11]>, nodes: [u32; 4]) {
    for (node, value) in [
        (nodes[1], nodes[2]),
        (nodes[2], nodes[3]),
        (nodes[3], nodes[1]),
    ] {
        data.get_mut(&node).unwrap()[1..4].copy_from_slice(&value.to_le_bytes()[..3]);
    }
}
fn check_nodes(bus: &Bus, data: &BTreeMap<u32, [u8; 11]>) {
    for (&node, bytes) in data {
        assert_eq!(&bus.ram[node as usize - 1..node as usize + 10], bytes);
    }
}
fn exercise(mut h: Harness, nodes: [u32; 4], mask: u8) -> u64 {
    h.bus.ram[0x7100..0x7103].copy_from_slice(&nodes[0].to_le_bytes()[..3]);
    let mut data = map_nodes(&mut h.bus, nodes, true);
    update_expected(&mut data, nodes);
    h.run();
    h.guards(mask);
    check_nodes(&h.bus, &data);
    let mut expected: Vec<_> = (0..9)
        .map(|offset| (nodes[0] + offset, Access::Read))
        .collect();
    for (node, value) in [
        (nodes[1], nodes[2]),
        (nodes[2], nodes[3]),
        (nodes[3], nodes[1]),
    ] {
        expected.extend(
            (0..3).map(|offset| (node + offset, Access::Write((value >> (8 * offset)) as u8))),
        );
    }
    assert_eq!(
        h.bus
            .trace
            .iter()
            .map(|(_, at, op)| (*at, *op))
            .collect::<Vec<_>>(),
        expected
    );
    h.cpu.cycles()
}

#[test]
fn dying_base_reuse_matches_stack_fallback_with_exact_banked_and_alias_traces() {
    let source =
        format!("{TRANSFORM} Node POINTER input=$7100 PROC Main() Transform(input) RETURN");
    for optimize in [false, true] {
        let prepared = prepare(&source, optimize);
        let image = prepared.compile(&layout()).unwrap().image;
        let transform = image
            .routines
            .iter()
            .find(|r| r.address == routine(&image, "Transform"))
            .unwrap();
        if optimize {
            assert_eq!(transform.fixed_frame, 0);
            assert_eq!(transform.temporaries.len(), 4);
            let homes: BTreeSet<_> = transform
                .temporaries
                .iter()
                .map(|t| match t.home {
                    TemporaryHome::DirectPage { offset } => offset,
                    _ => panic!("stack fallback"),
                })
                .collect();
            assert_eq!(homes.len(), 3);
        }
        let mut stack = prepared;
        for op in stack
            .mir
            .routines
            .iter_mut()
            .find(|r| r.name == "Transform")
            .unwrap()
            .blocks
            .iter_mut()
            .flat_map(|b| &mut b.ops)
        {
            match op {
                Mir65816Op::Load { volatile, .. } | Mir65816Op::Store { volatile, .. } => {
                    *volatile = true
                }
                _ => {}
            }
        }
        let stack = stack.compile(&layout()).unwrap().image;
        for nodes in [
            NODES,
            [0x54fffd, 0x43fffe, 0x32ffff, 0x21fffa],
            [NODES[0], NODES[1], NODES[1], NODES[3]],
            [NODES[0], NODES[0], NODES[2], NODES[3]],
            [NODES[0]; 4],
        ] {
            for mask in [0, 4] {
                let cycles = exercise(
                    Harness::new(&image, &caller(image.entry), mask),
                    nodes,
                    mask,
                );
                let old_cycles = exercise(
                    Harness::new(&stack, &caller(stack.entry), mask),
                    nodes,
                    mask,
                );
                if optimize {
                    assert!(cycles < old_cycles);
                }
                if nodes == NODES && mask == 0 {
                    eprintln!(
                        "pointer reload optimized={optimize}: {} bytes, {cycles} caller cycles; bytewise stack {} bytes, {old_cycles} cycles",
                        transform.size,
                        stack
                            .routines
                            .iter()
                            .find(|r| r.address == routine(&stack, "Transform"))
                            .unwrap()
                            .size
                    );
                }
            }
        }
    }
}

#[test]
fn dying_base_reuse_survives_both_o65_placements_and_masks() {
    let source =
        format!("{TRANSFORM} Node POINTER input=$7100 PROC Main() Transform(input) RETURN");
    for optimize in [false, true] {
        let bytes = o65::compile(&source, optimize, vec![]);
        for variant in 0..2 {
            let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
            let image = actionc::mir65816::o65::relocate(&bytes, &placement).unwrap();
            for mask in [0, 4] {
                exercise(
                    Harness::new_o65(&image, &caller(image.entry()), mask),
                    NODES,
                    mask,
                );
            }
        }
    }
}

#[test]
fn dying_base_x_capture_survives_every_task_irq_boundary_and_dispatcher_nmi() {
    let source = format!(
        "MODULE TEST PUBLIC EXTERNAL PROC Yield() \
        VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current \
        {TRANSFORM} TYPE Job=[Node POINTER item BYTE done BYTE POINTER peer] \
        CARD FUNC Dispatch(CARD saved BYTE reason) irqAck=1 Transform(Node POINTER($81FFFA)) \
        IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI taskB=saved current=0 RETURN(taskA) \
        PROC Task(Job POINTER work) Transform(work.item) work.done=1 \
        WHILE work.peer^=0 DO Yield() OD RETURN PROC Main() RETURN ENDMODULE"
    );
    // Each invocation reads a stable root and updates disjoint children, so
    // repeated dispatcher entry has a deterministic result.
    let sets = [
        NODES,
        NODES.map(|n| n + 0x300000),
        NODES.map(|n| n + 0x600000),
    ];
    for optimize in [false, true] {
        let mut h = ContextHarness::new(&source, optimize, "Task", &[0x7100, 0x7120]);
        let mut expected = BTreeMap::new();
        for nodes in sets {
            let mut data = map_nodes(&mut h.bus, nodes, false);
            update_expected(&mut data, nodes);
            expected.extend(data);
        }
        for (job, nodes, peer) in [(0x7100usize, sets[0], 0x7123u32), (0x7120, sets[1], 0x7103)] {
            h.bus.ram[job..job + 3].copy_from_slice(&nodes[0].to_le_bytes()[..3]);
            h.bus.ram[job + 4..job + 7].copy_from_slice(&peer.to_le_bytes()[..3]);
        }
        let check = |h: &ContextHarness| {
            h.guards();
            assert_eq!(h.bus.value(DONE, 2), 1);
            assert_eq!(h.bus.value(0x7103, 1), 1);
            assert_eq!(h.bus.value(0x7123, 1), 1);
            check_nodes(&h.bus, &expected);
        };
        let start = routine(&h.image, "Transform");
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
                let saved_cpu = h.cpu.clone();
                let saved_bus = h.bus.clone();
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
                h.cpu = saved_cpu;
                h.bus = saved_bus;
            }
            h.tick(Inputs::default());
        }
        check(&h);
        assert!(
            seen.len() >= 60,
            "only {} instruction/domain sites",
            seen.len()
        );
    }
}
