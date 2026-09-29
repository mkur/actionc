mod support;
use actionc::mir65816::image::{Image, TemporaryHome};
use actionc_vm::native65816::{Access, Inputs};
use std::collections::{BTreeMap, BTreeSet};
use support::{context::*, *};

const INSERT: &str = "TYPE Node=[Node POINTER next,previous]\n\
    TYPE Chain=[Node POINTER head,tail,previous]\n\
    PROC Insert(Chain POINTER chain Node POINTER item)\n\
      LET first=chain.head\n\
      item.previous=Node POINTER(@chain.head) item.next=first\n\
      first.previous=item chain.head=item\n\
    RETURN\n";

fn exercise(image: &Image, chain: u32, item: u32, first: u32, mask: u8) -> u64 {
    let caller = assemble_artifact(
        &format!(
            ".export returned\ntsc\nsec\nsbc #6\ntcs\n\
            lda #{}\nsta 1,s\nsep #$20\n.a8\nlda #{}\nsta 3,s\n\
            rep #$20\n.a16\nlda #{}\nsta 4,s\nsep #$20\n.a8\nlda #{}\nsta 6,s\n\
            rep #$20\n.a16\njsl ${:06x}\nreturned:\ntsc\nclc\nadc #6\ntcs\nstp\nnop",
            chain & 65535,
            chain >> 16,
            item & 65535,
            item >> 16,
            image.entry
        ),
        0x040000,
    );
    let mut h = Harness::new(image, &caller.bytes, mask);
    let mut expected = BTreeMap::new();
    for node in [chain, item, first] {
        for at in node - 1..node + 10 {
            if expected.insert(at, 0xa5u8).is_none() {
                h.bus.map(at, &[0xa5], true);
                h.bus.watched.insert(at);
            }
        }
    }
    for byte in 0..3 {
        let value = (first >> (8 * byte)) as u8;
        h.bus.ram[(chain + byte) as usize] = value;
        expected.insert(chain + byte, value);
    }
    let mut trace: Vec<_> = (chain..chain + 3).map(|at| (at, Access::Read)).collect();
    for (at, value) in [
        (item + 3, chain),
        (item, first),
        (first + 3, item),
        (chain, item),
    ] {
        for byte in 0..3 {
            let value = (value >> (8 * byte)) as u8;
            expected.insert(at + byte, value);
            trace.push((at + byte, Access::Write(value)));
        }
    }
    let mut begin = None;
    let mut cycles = None;
    for _ in 0..10000 {
        if h.cpu.is_instruction_boundary() {
            if h.cpu.pc() == image.entry {
                begin = Some(h.cpu.cycles());
            }
            if h.cpu.pc() == caller.symbols["returned"] {
                cycles = Some(h.cpu.cycles() - begin.unwrap());
                break;
            }
        }
        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
    }
    h.run();
    h.guards(mask);
    for (at, value) in expected {
        assert_eq!(h.bus.ram[at as usize], value);
    }
    assert_eq!(
        h.bus
            .trace
            .iter()
            .map(|(_, at, op)| (*at, *op))
            .collect::<Vec<_>>(),
        trace
    );
    cycles.expect("Insert did not return within its cycle budget")
}

#[test]
fn resident_insertion_matches_independent_reference_size_cycles_and_memory_order() {
    for newline in ["\n", "\r\n"] {
        let mut reference = compile("PROC Insert(BYTE POINTER chain,item) RETURN", false);
        let code = assemble(
            &fixture("resident_addhead.s").replace('\n', newline),
            reference.entry,
        );
        assert_eq!(code.len(), 96);
        reference.routines[0].size = code.len() as u32;
        reference
            .segments
            .iter_mut()
            .find(|s| s.executable)
            .unwrap()
            .bytes = code;
        for optimize in [false, true] {
            let image = compile(&INSERT.replace('\n', newline), optimize);
            let r = &image.routines[0];
            if optimize {
                assert!(r.size <= 96);
                assert_eq!(
                    (r.fixed_frame, r.spill_bytes, r.local_stack_peak),
                    (0, 0, 0)
                );
                let slots: BTreeSet<_> = r
                    .temporaries
                    .iter()
                    .map(|t| match t.home {
                        TemporaryHome::DirectPage { offset } => offset,
                        _ => panic!("resident pointer has a stack home"),
                    })
                    .collect();
                assert_eq!(slots.len(), 3);
            } else {
                assert_eq!((r.size, r.fixed_frame), (146, 4));
            }
            for (chain, item, first) in [
                (0x12fffe, 0x32fffc, 0x43fffe),
                (0x12fffe, 0x32fffc, 0x130001),
                (0x12fffe, 0x32fffc, 0x32fffc),
                (0x12fffe, 0x12fffe, 0x43fffe),
                (0x12fffe, 0x130000, 0x130001),
            ] {
                for mask in [0, 4] {
                    let expected = exercise(&reference, chain, item, first, mask);
                    let cycles = exercise(&image, chain, item, first, mask);
                    if optimize {
                        assert!(cycles <= expected, "{cycles} > {expected}");
                    }
                    if newline == "\n"
                        && chain == 0x12fffe
                        && item == 0x32fffc
                        && first == 0x43fffe
                        && mask == 0
                    {
                        eprintln!(
                            "resident Insert optimize={optimize}: {} bytes, {cycles} cycles; reference 96 bytes, {expected} cycles",
                            r.size
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn all_three_resident_pointers_survive_task_irq_boundaries_and_dispatcher_nmi() {
    let source = format!(
        "MODULE TEST PUBLIC EXTERNAL PROC Yield()\n\
        VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current\n\
        {INSERT}\n\
        TYPE Job=[Chain POINTER list Node POINTER item BYTE done BYTE POINTER peer]\n\
        Chain POINTER irqList=$7140 Node POINTER irqItem=$7144\n\
        CARD FUNC Dispatch(CARD saved BYTE reason)\n\
          irqAck=1 irqList.head=Node POINTER($83fffe) Insert(irqList,irqItem)\n\
          IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI\n\
          taskB=saved current=0 RETURN(taskA)\n\
        PROC Task(Job POINTER work)\n\
          Insert(work.list,work.item) work.done=1\n\
          WHILE work.peer^=0 DO Yield() OD\n\
        RETURN PROC Main() RETURN ENDMODULE"
    );
    let nodes = [
        (0x12fffeu32, 0x22fffc, 0x33fffe),
        (0x42fffe, 0x52fffc, 0x63fffe),
        (0x72fffe, 0x82fffc, 0x83fffe),
    ];
    let check = |h: &ContextHarness| {
        h.guards();
        assert_eq!(h.bus.value(DONE, 2), 1);
        for (chain, item, first) in nodes {
            assert_eq!(h.bus.value(chain, 3), item);
            assert_eq!(h.bus.value(item, 3), first);
            assert_eq!(h.bus.value(item + 3, 3), chain);
            assert_eq!(h.bus.value(first + 3, 3), item);
        }
    };
    for optimize in [false, true] {
        let mut h = ContextHarness::new(&source, optimize, "Task", &[0x7100, 0x7120]);
        for (i, &(chain, item, first)) in nodes.iter().enumerate() {
            for node in [chain, item, first] {
                h.bus.map(node - 1, &[0xa5; 11], true);
            }
            h.bus.ram[chain as usize..chain as usize + 3]
                .copy_from_slice(&first.to_le_bytes()[..3]);
            if i < 2 {
                let job = 0x7100 + i * 0x20;
                h.bus.ram[job..job + 3].copy_from_slice(&chain.to_le_bytes()[..3]);
                h.bus.ram[job + 3..job + 6].copy_from_slice(&item.to_le_bytes()[..3]);
                let peer = if i == 0 { 0x7126u32 } else { 0x7106u32 };
                h.bus.ram[job + 7..job + 10].copy_from_slice(&peer.to_le_bytes()[..3]);
            } else {
                h.bus.ram[0x7140..0x7143].copy_from_slice(&chain.to_le_bytes()[..3]);
                h.bus.ram[0x7144..0x7147].copy_from_slice(&item.to_le_bytes()[..3]);
            }
        }
        let start = routine(&h.image, "Insert");
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
            seen.len() >= 80,
            "only {} task/instruction sites",
            seen.len()
        );
        eprintln!(
            "resident Insert optimize={optimize}: {} task/instruction IRQ sites",
            seen.len()
        );
    }
}
