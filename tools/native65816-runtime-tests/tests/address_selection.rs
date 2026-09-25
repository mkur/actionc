mod support;
use actionc::mir65816::Mir65816Op;
use actionc::mir65816::o65 as format;
use actionc_vm::native65816::Access;
use support::*;

#[test]
fn symbolic_addresses_preserve_all_lanes_in_fixed_and_relocated_images() {
    let source = "TYPE Pair=[BYTE first BYTE last] Pair object BYTE initialized=[42] ADDRESS a=$7100,b=$7103,c=$7106 PROC Main() a=ADDRESS(@initialized) b=ADDRESS(@object.last) c=ADDRESS(@object) RETURN";
    for optimize in [false, true] {
        let p = prepare(source, optimize);
        let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
        for variant in 0..2 {
            let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
            let relocated = format::relocate(&bytes, &placement).unwrap();
            let mut fixed_layout = layout();
            fixed_layout.data_origin = placement.bases[1];
            fixed_layout.zero_fill_origin = Some(placement.bases[2]);
            let fixed = p.compile(&fixed_layout).unwrap().image;
            for mask in [0, 4] {
                for use_o65 in [false, true] {
                    let (mut h, initialized, object) = if use_o65 {
                        (
                            Harness::new_o65(&relocated, &caller(relocated.entry()), mask),
                            o65::object(&relocated, "initialized"),
                            o65::object(&relocated, "object"),
                        )
                    } else {
                        (
                            Harness::new(&fixed, &caller(fixed.entry), mask),
                            context::symbol(&fixed, "initialized"),
                            context::symbol(&fixed, "object"),
                        )
                    };
                    h.bus.ram[0x70ff..0x710a].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.bus.value(0x7100, 3), initialized);
                    assert_eq!(h.bus.value(0x7103, 3), object + 1);
                    assert_eq!(h.bus.value(0x7106, 3), object);
                    assert_eq!((h.bus.ram[0x70ff], h.bus.ram[0x7109]), (0xa5, 0xa5));
                    assert_eq!(h.bus.ram[initialized as usize], 42);
                }
            }
        }
    }
}

#[test]
fn constant_address_chains_relocate_with_bank_carry_and_one_past_fallback() {
    for offset in [0, 1, 255, 256, 299, 300, 65535] {
        let source = format!(
            "BYTE ARRAY bytes(300) ADDRESS result=$7100 PROC Main() result=ADDRESS(@bytes({offset})) RETURN"
        );
        for optimize in [false, true] {
            let bytes = o65::compile(&source, optimize, vec![]);
            for variant in 0..3 {
                let mut placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
                if variant == 2 {
                    placement.bases[2] = 0x1000000 - 300;
                }
                let image = format::relocate(&bytes, &placement).unwrap();
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                    h.bus.ram[0x70ff..0x7104].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        h.bus.value(0x7100, 3),
                        (o65::object(&image, "bytes") + offset) & 0xffffff
                    );
                    assert_eq!((h.bus.ram[0x70ff], h.bus.ram[0x7103]), (0xa5, 0xa5));
                }
            }
        }
    }
}

#[test]
fn constant_byte_accesses_keep_mutation_across_calls_and_exact_access_order() {
    let source = "BYTE ARRAY bytes(300) BYTE before=$7100,after=$7101 PROC Change() bytes(0)=7 RETURN PROC Main() bytes(0)=3 before=bytes(0) Change() after=bytes(0) bytes(299)=after RETURN";
    for optimize in [false, true] {
        for volatile in [false, true] {
            let mut p = prepare(source, optimize);
            if volatile {
                for op in p
                    .mir
                    .routines
                    .iter_mut()
                    .flat_map(|r| &mut r.blocks)
                    .flat_map(|b| &mut b.ops)
                {
                    match op {
                        Mir65816Op::Load {
                            width, volatile, ..
                        }
                        | Mir65816Op::Store {
                            width, volatile, ..
                        } if width.get() == 1 => *volatile = true,
                        _ => {}
                    }
                }
            }
            let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..2 {
                let image = format::relocate(
                    &bytes,
                    &o65::placement(&bytes, variant, vec![o65::fault(variant)]),
                )
                .unwrap();
                let start = o65::object(&image, "bytes");
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                    h.bus.ram[start as usize..start as usize + 300].fill(0xa5);
                    h.bus.watched.extend(start..start + 300);
                    h.run();
                    h.guards(mask);
                    assert_eq!(&h.bus.ram[0x7100..0x7102], &[3, 7]);
                    assert_eq!(h.bus.ram[start as usize + 299], 7);
                    assert!(
                        h.bus.ram[start as usize + 1..start as usize + 299]
                            .iter()
                            .all(|&b| b == 0xa5)
                    );
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|&(_, at, op)| (at, op))
                            .collect::<Vec<_>>(),
                        vec![
                            (start, Access::Write(3)),
                            (start, Access::Read),
                            (start, Access::Write(7)),
                            (start, Access::Read),
                            (start + 299, Access::Write(7))
                        ]
                    );
                }
            }
        }
    }
}

#[test]
fn card_byte_loads_use_full_y_and_preserve_24_bit_carry_and_wrap() {
    let source = "BYTE POINTER base=$7100 CARD index=$7104 BYTE result=$7106 BYTE FUNC Read(BYTE POINTER p CARD i) RETURN(p(i)) PROC Main() result=Read(base,index) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        for base in [0x22fffeu32, 0x23ffff, 0x320001, 0xff1234] {
            for index in [0u16, 1, 255, 256, 32767, 32768, 65534, 65535] {
                let target = (base + u32::from(index)) & 0xffffff;
                let value = (index as u8).wrapping_add(53);
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), mask);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                    h.bus.ram[0x7104..0x7106].copy_from_slice(&index.to_le_bytes());
                    h.bus.map(target - 1, &[0xa5, value, 0xa5], true);
                    h.bus.watched.extend(target - 1..target + 2);
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.bus.ram[0x7106], value, "{base:x}/{index}/{optimize}");
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|&(_, at, op)| (at, op))
                            .collect::<Vec<_>>(),
                        vec![(target, Access::Read)]
                    );
                    assert_eq!(
                        (
                            h.bus.ram[target as usize - 1],
                            h.bus.ram[target as usize + 1]
                        ),
                        (0xa5, 0xa5)
                    );
                }
            }
        }
    }
}

#[test]
fn card_indices_load_relocated_symbols_without_losing_the_bank_carry() {
    let source = "BYTE ARRAY bytes(300) CARD index=$7100 BYTE result=$7102 PROC Main() result=bytes(index) RETURN";
    for optimize in [false, true] {
        let bytes = o65::compile(source, optimize, vec![]);
        for variant in 0..2 {
            let image = format::relocate(
                &bytes,
                &o65::placement(&bytes, variant, vec![o65::fault(variant)]),
            )
            .unwrap();
            for index in [0u16, 1, 255, 256, 299] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                    let target = o65::object(&image, "bytes") + u32::from(index);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&index.to_le_bytes());
                    h.bus.ram[target as usize] = 217;
                    h.bus.watched.insert(target);
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.bus.ram[0x7102], 217);
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|&(_, at, op)| (at, op))
                            .collect::<Vec<_>>(),
                        vec![(target, Access::Read)]
                    );
                }
            }
        }
    }
}

#[test]
fn card_byte_stores_preserve_values_alias_order_and_neighbor_bytes() {
    let source = "BYTE POINTER base=$7100 CARD index=$7104 BYTE value=$7106,result=$7107 BYTE FUNC Exchange(BYTE POINTER p CARD i BYTE v) BYTE old old=p(i) p(i)=v RETURN(old) PROC Main() result=Exchange(base,index,value) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let boundary = [0x22fffeu32, 0x23ffff, 0x320001, 0xff1234]
            .into_iter()
            .flat_map(|base| {
                [0u16, 1, 255, 256, 32767, 32768, 65534, 65535]
                    .into_iter()
                    .map(move |index| (base, index, 231u8))
            });
        for (base, index, value) in boundary.chain((0..=255).map(|value| (0x22ffff, 1, value))) {
            let target = (base + u32::from(index)) & 0xffffff;
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                h.bus.ram[0x7104..0x7106].copy_from_slice(&index.to_le_bytes());
                h.bus.ram[0x7106] = value;
                h.bus.map(target - 1, &[0xa5, 117, 0xa5], true);
                h.bus.watched.extend(target - 1..target + 2);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.ram[0x7107], 117);
                assert_eq!(h.bus.ram[target as usize], value);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, at, op)| (at, op))
                        .collect::<Vec<_>>(),
                    vec![(target, Access::Read), (target, Access::Write(value))]
                );
                assert_eq!(
                    (
                        h.bus.ram[target as usize - 1],
                        h.bus.ram[target as usize + 1]
                    ),
                    (0xa5, 0xa5)
                );
            }
        }
    }
}

#[test]
fn card_indices_store_captured_bytes_and_zero_to_relocated_symbols() {
    let source = "BYTE ARRAY bytes(300) CARD index=$7100 BYTE value=$7102 PROC Main() bytes(index)=value bytes(index)=0 RETURN";
    for optimize in [false, true] {
        let bytes = o65::compile(source, optimize, vec![]);
        for variant in 0..2 {
            let image = format::relocate(
                &bytes,
                &o65::placement(&bytes, variant, vec![o65::fault(variant)]),
            )
            .unwrap();
            let base = o65::object(&image, "bytes");
            for index in [0u16, 1, 255, 256, 299] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                    let target = base + u32::from(index);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&index.to_le_bytes());
                    h.bus.ram[0x7102] = 217;
                    h.bus.ram[base as usize..base as usize + 300].fill(0xa5);
                    h.bus.watched.extend(base..base + 300);
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.bus.ram[target as usize], 0);
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|&(_, at, op)| (at, op))
                            .collect::<Vec<_>>(),
                        vec![(target, Access::Write(217)), (target, Access::Write(0))]
                    );
                    assert!(
                        (base..base + 300)
                            .filter(|&a| a != target)
                            .all(|a| h.bus.ram[a as usize] == 0xa5)
                    );
                }
            }
        }
    }
}

#[test]
fn indexed_byte_accesses_survive_irq_nmi_and_reentrant_dispatch() {
    use actionc_vm::native65816::Inputs;
    use std::collections::BTreeSet;
    use support::context::*;
    let source = "MODULE TEST PUBLIC EXTERNAL PROC Yield() VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current,irqResult TYPE Job=[BYTE POINTER item BYTE done BYTE result BYTE POINTER peer] BYTE FUNC Exchange(BYTE POINTER p CARD i BYTE v) BYTE old old=p(i) p(i)=v RETURN(old) CARD FUNC Dispatch(CARD saved BYTE reason) irqAck=1 irqResult=Exchange(BYTE POINTER($43ff00),$100,7) IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI taskB=saved current=0 RETURN(taskA) PROC Task(Job POINTER work) work.result=Exchange(work.item,$100,21) work.done=1 WHILE work.peer^=0 DO Yield() OD RETURN PROC Main() RETURN ENDMODULE";
    let check = |h: &ContextHarness| {
        h.guards();
        assert_eq!(h.bus.value(DONE, 2), 1);
        assert_eq!((h.bus.value(0x7103, 1), h.bus.value(0x7123, 1)), (1, 1));
        assert_eq!((h.bus.value(0x7104, 1), h.bus.value(0x7124, 1)), (11, 12));
        assert_eq!(h.bus.value(symbol(&h.image, "irqResult"), 1), 7);
        for (target, value) in [(0x220000, 21), (0x330000, 21), (0x440000, 7)] {
            assert_eq!(h.bus.value(target, 1), value);
            assert_eq!(
                (h.bus.value(target - 1, 1), h.bus.value(target + 1, 1)),
                (0xa5, 0xa5)
            );
        }
    };
    for optimize in [false, true] {
        let mut h = ContextHarness::new(source, optimize, "Task", &[0x7100, 0x7120]);
        for (job, base, peer) in [
            (0x7100usize, 0x21ff00u32, 0x7123u32),
            (0x7120, 0x32ff00, 0x7103),
        ] {
            h.bus.ram[job..job + 3].copy_from_slice(&base.to_le_bytes()[..3]);
            h.bus.ram[job + 5..job + 8].copy_from_slice(&peer.to_le_bytes()[..3]);
        }
        for (target, value) in [(0x220000, 11), (0x330000, 12), (0x440000, 7)] {
            h.bus.map(target - 1, &[0xa5, value, 0xa5], true);
        }
        let start = routine(&h.image, "Exchange");
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
            seen.len() >= 40,
            "too few indexed access injection sites: {}",
            seen.len()
        );
    }
}
