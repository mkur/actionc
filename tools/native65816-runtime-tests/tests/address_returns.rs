mod support;
use actionc_vm::native65816::Inputs;
use std::collections::BTreeSet;
use support::{context::routine, *};

const SOURCE: &str = "TYPE Parcel=[BYTE ARRAY prefix(82) BYTE ARRAY storage(32)]\n\
TYPE Cursor=[BYTE value]\n\
TYPE Shelf=[BYTE tag CARD ARRAY words(16)]\n\
TYPE Crate=[CARD serial Shelf content]\n\
Cursor POINTER FUNC Field(Parcel POINTER item) RETURN(Cursor POINTER(@item.storage))\n\
Cursor POINTER FUNC Chain(Parcel POINTER item) RETURN(Cursor POINTER(@item.storage(0)))\n\
Cursor POINTER FUNC Element(Shelf POINTER item) RETURN(Cursor POINTER(@item.words(3)))\n\
Cursor POINTER FUNC Nested(Crate POINTER item) RETURN(Cursor POINTER(@item.content.words(2)))\n\
Cursor POINTER FUNC Dynamic(Parcel POINTER item BYTE index) RETURN(Cursor POINTER(@item.storage(index)))\n\
Cursor POINTER FUNC Framed(Parcel POINTER item) BYTE ARRAY scratch(4) scratch(0)=7 RETURN(Cursor POINTER(@item.storage))\n\
PROC Main() RETURN\n";

fn caller(address: u32) -> String {
    format!(
        "tsc\nsec\nsbc #3\ntcs\nsep #$20\n.a8\nlda f:$007100\nsta 1,s\nlda f:$007101\nsta 2,s\nlda f:$007102\nsta 3,s\nrep #$20\n.a16\nlda #$d5aa\nldx #$beef\nldy #$cafe\njsl ${address:06x}\n.export returned\nreturned: sta f:$007200\ntxa\nsta f:$007202\ntsc\nclc\nadc #3\ntcs\nstp\nnop\n"
    )
}

fn reach(h: &mut Harness, pc: u32) {
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                100_000,
                |_| Inputs::default(),
                |c| c.is_instruction_boundary() && c.pc() == pc
            )
            .unwrap()
    );
}

#[test]
fn returned_record_addresses_preserve_native_lanes_and_never_read_the_pointee() {
    for optimize in [false, true] {
        let prepared = prepare(SOURCE, optimize);
        for guarded in [false, true] {
            let mut options = layout();
            options.stack_checks = guarded;
            let compiled = prepared.compile(&options).unwrap();
            let image = &compiled.image;
            assert_eq!(
                image.to_json().unwrap(),
                prepare(&SOURCE.replace('\n', "\r\n"), optimize)
                    .compile(&options)
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            // CARD fields are aligned to two bytes by the record layout.
            for (name, offset) in [
                ("Field", 82u32),
                ("Chain", 82),
                ("Element", 8),
                ("Nested", 8),
                ("Framed", 82),
            ] {
                let record = image.routines.iter().find(|r| r.name == name).unwrap();
                if name != "Framed" {
                    assert_eq!(
                        (record.size, record.fixed_frame, record.spill_bytes),
                        (21, 0, 0)
                    );
                    assert!(record.temporaries.is_empty());
                    let machine = compiled
                        .machine
                        .routines
                        .iter()
                        .find(|r| r.id.0 == record.id)
                        .unwrap();
                    assert_eq!(
                        machine.code.bytes,
                        assemble(
                            &format!(
                                "lda 4,s\nclc\nadc #{offset}\ntay\nsep #$20\n.a8\nlda 6,s\nadc #0\nrep #$20\n.a16\nand #$00ff\ntax\ntya\nrtl\n"
                            ),
                            0x050000
                        )
                    );
                }
                if name == "Framed" {
                    assert!(record.fixed_frame >= 4);
                }
                let entry = routine(image, name);
                let assembled = assemble_artifact(&caller(entry), 0x040000);
                for input in [0u32, 1, 0x12ffaf, 0xffffae, 0xffffff, 0x800000, 0x12abcd] {
                    for mask in [0, 4] {
                        let mut h = Harness::new(image, &assembled.bytes, mask);
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&input.to_le_bytes()[..3]);
                        h.bus.ram[0x7103] = 0xb7;
                        h.bus.ram[0x71ff] = 0x39;
                        h.bus.ram[0x7204] = 0x6e;
                        let expected = input.wrapping_add(offset) & 0xffffff;
                        h.bus.watched.insert(input);
                        h.bus.watched.insert(expected);
                        reach(&mut h, entry);
                        let incoming = u32::from(h.cpu.registers().s) + 4;
                        h.bus.reads.clear();
                        h.bus.writes.clear();
                        reach(&mut h, assembled.symbols["returned"]);
                        let r = h.cpu.registers();
                        assert_eq!(
                            u32::from(r.a) | (u32::from(r.x) << 16),
                            expected,
                            "{name}/{input:06x}/{optimize}/{guarded}/{mask}"
                        );
                        assert_eq!(r.p & 0x3c, mask);
                        assert_eq!(r.s, 0x5fed);
                        assert!(h.bus.reads.iter().all(|a| *a != incoming + 3));
                        assert_eq!(
                            h.bus
                                .reads
                                .iter()
                                .filter(|a| (incoming..incoming + 3).contains(*a))
                                .copied()
                                .collect::<Vec<_>>(),
                            [incoming, incoming + 1, incoming + 2]
                        );
                        if name != "Framed" {
                            assert!(h.bus.writes.is_empty());
                        }
                        assert!(
                            h.bus.trace.is_empty(),
                            "forming an address accessed its pointee"
                        );
                        h.run();
                        h.guards(mask);
                        assert_eq!(h.bus.value(0x7200, 4), expected);
                        assert_eq!(
                            &h.bus.ram[0x7100..0x7104],
                            &[input as u8, (input >> 8) as u8, (input >> 16) as u8, 0xb7]
                        );
                        assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7204]), (0x39, 0x6e));
                    }
                }
            }
        }
    }
}

#[test]
fn returned_addresses_match_independent_callers_after_o65_relocation() {
    use actionc::mir65816::o65 as format;
    for optimize in [false, true] {
        let p = prepare(SOURCE, optimize);
        let object = p.compile_o65(&Default::default()).unwrap().bytes;
        for variant in 0..2 {
            let placement = o65::placement(&object, variant, vec![o65::fault(variant)]);
            let image = format::relocate(&object, &placement).unwrap();
            for (name, offset) in [
                ("Field", 82u32),
                ("Chain", 82),
                ("Nested", 8),
                ("Framed", 82),
            ] {
                let address = o65::routine(&image, name);
                let code = assemble(&caller(address), 0x040000);
                for input in [0x12ffafu32, 0xffffff, 0x800000] {
                    for mask in [0, 4] {
                        let mut h = Harness::new_o65(&image, &code, mask);
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&input.to_le_bytes()[..3]);
                        h.bus
                            .watched
                            .extend([input, input.wrapping_add(offset) & 0xffffff]);
                        h.run();
                        h.guards(mask);
                        assert_eq!(
                            h.bus.value(0x7200, 4),
                            input.wrapping_add(offset) & 0xffffff
                        );
                        assert!(h.bus.trace.is_empty());
                    }
                }
            }
        }
    }
}

#[test]
fn computed_address_results_survive_recursive_frames() {
    let source = "TYPE Parcel=[BYTE ARRAY pad(82) BYTE ARRAY storage(32)] \
        TYPE Cursor=[BYTE value] Cursor POINTER FUNC Recurse(Parcel POINTER item CARD depth) \
        IF depth=0 THEN RETURN(Cursor POINTER(@item.storage(0))) FI \
        RETURN(Recurse(item,depth-1)) PROC Main() RETURN";
    for optimize in [false, true] {
        for guarded in [false, true] {
            let mut options = layout();
            options.stack_checks = guarded;
            let image = prepare(source, optimize).compile(&options).unwrap().image;
            let entry = routine(&image, "Recurse");
            let record = image.routines.iter().find(|r| r.address == entry).unwrap();
            let bytes = record.outgoing_bytes;
            let depth = record.arguments[1].offset + 1;
            let asm = format!(
                "tsc\nsec\nsbc #{bytes}\ntcs\nsep #$20\n.a8\nlda #$ff\nsta 1,s\nsta 2,s\nsta 3,s\nlda #3\nsta {depth},s\nlda #0\nsta {},s\nrep #$20\n.a16\nldx #$beef\njsl ${entry:06x}\nsta f:$007200\ntxa\nsta f:$007202\ntsc\nclc\nadc #{bytes}\ntcs\nstp\nnop\n",
                depth + 1
            );
            let caller = assemble(&asm, 0x040000);
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.watched.extend([0xffffff, 0x51]);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(0x7200, 4), 0x51);
                assert!(h.bus.trace.is_empty());
            }
        }
    }
}

fn tasks(optimize: bool) -> context::ContextHarness {
    let mut h = context::ContextHarness::new(
        &fixture("address_return_preemption.act"),
        optimize,
        "Task",
        &[0x7100, 0x7120],
    );
    for (i, input) in [0x12ffafu32, 0xffffff].into_iter().enumerate() {
        let job = 0x7100 + i * 0x20;
        h.bus.ram[job..job + 3].copy_from_slice(&input.to_le_bytes()[..3]);
        h.bus.ram[job + 7..job + 10]
            .copy_from_slice(&(0x7106u32 + (1 - i as u32) * 0x20).to_le_bytes()[..3]);
        h.bus
            .watched
            .extend([input, input.wrapping_add(82) & 0xffffff]);
    }
    let irq = context::symbol(&h.image, "irqItem") as usize;
    h.bus.ram[irq..irq + 3].copy_from_slice(&0x800000u32.to_le_bytes()[..3]);
    h.bus.watched.extend([0x800000, 0x800052]);
    h
}

fn check_tasks(h: &context::ContextHarness) {
    h.guards();
    assert_eq!(h.bus.value(context::DONE, 2), 1);
    assert_eq!(h.bus.value(0x7103, 3), 0x130001);
    assert_eq!(h.bus.value(0x7123, 3), 0x51);
    assert_eq!(
        h.bus.value(context::symbol(&h.image, "irqResult"), 3),
        0x800052
    );
    assert!(h.bus.trace.is_empty());
}

fn run_tasks(h: &mut context::ContextHarness, mut pending: bool, seed: Option<u64>) {
    let mut rng = seed.unwrap_or(1);
    let mut nmi_after = 0;
    for _ in 0..2_000_000 {
        if h.cpu.is_stopped() {
            check_tasks(h);
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
            if rng & 127 == 1 && h.cpu.cycles() >= nmi_after {
                nmi = true;
                nmi_after = h.cpu.cycles() + 250;
            }
        }
        let writes = h.bus.writes.len();
        h.tick(Inputs {
            irq: pending,
            nmi,
            ..Default::default()
        });
        if h.bus.writes[writes..]
            .iter()
            .any(|&(at, _)| at == context::IRQ_ACK)
        {
            pending = false;
        }
    }
    panic!("address Return preemption exceeded cycle budget");
}

#[test]
fn every_address_return_boundary_survives_task_irq_and_nmi_contexts() {
    for optimize in [false, true] {
        let mut h = tasks(optimize);
        let ranges: Vec<_> = ["Chain", "Framed"]
            .into_iter()
            .map(|name| {
                let start = routine(&h.image, name);
                let size = h
                    .image
                    .routines
                    .iter()
                    .find(|r| r.address == start)
                    .unwrap()
                    .size;
                start..start + size
            })
            .collect();
        let mut seen = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            if h.cpu.is_instruction_boundary()
                && r.p & 4 == 0
                && [0x2000, 0x2100].contains(&r.d)
                && ranges.iter().any(|range| range.contains(&h.cpu.pc()))
                && seen.insert((r.d, h.cpu.pc()))
            {
                let cpu = h.cpu.clone();
                let bus = h.bus.clone();
                run_tasks(&mut h, true, None);
                h.cpu = cpu;
                h.bus = bus;
            }
            h.tick(Inputs::default());
        }
        check_tasks(&h);
        let sites = |domain| {
            seen.iter()
                .filter_map(|&(d, pc)| (d == domain).then_some(pc))
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(sites(0x2000), sites(0x2100));
        for range in ranges {
            assert!(sites(0x2000).contains(&range.start));
            assert!(sites(0x2000).contains(&(range.end - 1)));
        }
        for seed in [0x81620260916, 0x5eedcafe] {
            run_tasks(&mut tasks(optimize), false, Some(seed));
        }
    }
}
