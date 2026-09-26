mod support;
use actionc::mir65816::{Mir65816Op, o65 as format};
use actionc::target::{ByteOffset, ByteSize};
use actionc_vm::native65816::Access;
use support::*;

fn prepared(
    source: &str,
    optimize: bool,
    displacement: u32,
) -> actionc::compiler::native65816::Prepared {
    shaped(source, optimize, displacement, None)
}

fn shaped(
    source: &str,
    optimize: bool,
    displacement: u32,
    stride: Option<u32>,
) -> actionc::compiler::native65816::Prepared {
    let mut p = prepare(source, optimize);
    for op in p
        .mir
        .routines
        .iter_mut()
        .flat_map(|r| &mut r.blocks)
        .flat_map(|b| &mut b.ops)
    {
        match op {
            Mir65816Op::Load { address, .. } | Mir65816Op::Store { address, .. }
                if address.index.is_some() =>
            {
                address.displacement = ByteOffset::new(displacement);
                if let Some(stride) = stride {
                    address.index.as_mut().unwrap().stride = ByteSize::new(stride);
                }
            }
            _ => {}
        }
    }
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
}

#[test]
fn byte_index_reads_exactly_one_byte_and_preserves_banked_load_store_order() {
    let source = "BYTE POINTER base=$7100 BYTE index=$7104,value=$7106,result=$7107 BYTE FUNC Exchange(BYTE POINTER p BYTE i BYTE v) BYTE old old=p(i) p(i)=v RETURN(old) PROC Main() result=Exchange(base,index,value) RETURN";
    for optimize in [false, true] {
        for displacement in [0, 17, 65280, 65281] {
            let p = prepared(source, optimize, displacement);
            let c = p.compile(&layout()).unwrap();
            assert_eq!(
                c.image.to_json().unwrap(),
                prepared(&source.replace('\n', "\r\n"), optimize, displacement)
                    .compile(&layout())
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            let r = c
                .machine
                .prepared
                .routines
                .iter()
                .find(|r| r.name == "Exchange")
                .unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let mut selected = 0;
            for b in &r.blocks {
                for (i, op) in b.ops.iter().enumerate() {
                    if matches!(op,Mir65816Op::Load{address,..}|Mir65816Op::Store{address,..} if address.index.is_some())
                    {
                        let bytes = &m.code.bytes[m.code.mir_spans[&(b.id, i)].clone()];
                        selected += usize::from(bytes.windows(3).any(|w| w == [0x29, 0xff, 0]));
                    }
                }
            }
            assert_eq!(selected, if displacement <= 65280 { 2 } else { 0 });
            let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..3 {
                let loaded = (variant > 0).then(|| {
                    format::relocate(
                        &bytes,
                        &o65::placement(&bytes, variant - 1, vec![o65::fault(variant - 1)]),
                    )
                    .unwrap()
                });
                let indexes: Vec<_> = if displacement == 0 && variant == 0 {
                    (0..=255u8).collect()
                } else {
                    vec![0, 1, 127, 128, 255]
                };
                for index in indexes {
                    for base in [0x22fff0u32, 0xfffff0] {
                        let target = (base + u32::from(index) + displacement) & 0xffffff;
                        let mask = if index & 1 == 0 { 0 } else { 4 };
                        let mut h = if let Some(l) = &loaded {
                            Harness::new_o65(l, &caller(l.entry()), mask)
                        } else {
                            Harness::new(&c.image, &caller(c.image.entry), mask)
                        };
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                        h.bus.ram[0x7104..0x7108].copy_from_slice(&[index, 0xee, 231, 0]);
                        for (offset, value) in [(0xffffff, 0xa5), (0, 117), (1, 0xa5)] {
                            let at = (target + offset) & 0xffffff;
                            h.bus.map(at, &[value], true);
                            h.bus.watched.insert(at);
                        }
                        h.run();
                        h.guards(mask);
                        assert_eq!(h.bus.ram[0x7107], 117);
                        assert_eq!(h.bus.ram[target as usize], 231);
                        assert_eq!(
                            h.bus
                                .trace
                                .iter()
                                .map(|&(_, a, k)| (a, k))
                                .collect::<Vec<_>>(),
                            [(target, Access::Read), (target, Access::Write(231))]
                        );
                        assert_eq!(h.bus.ram[((target + 0xffffff) & 0xffffff) as usize], 0xa5);
                        assert_eq!(h.bus.ram[((target + 1) & 0xffffff) as usize], 0xa5);
                    }
                }
            }
        }
    }
}

#[test]
fn byte_indexes_preserve_relocated_symbol_bases() {
    let source = "BYTE ARRAY bytes(300) BYTE index=$7100,result=$7102 PROC Main() bytes(index)=217 result=bytes(index) RETURN";
    for optimize in [false, true] {
        let object = o65::compile(source, optimize, vec![]);
        for variant in 0..2 {
            let l = format::relocate(
                &object,
                &o65::placement(&object, variant, vec![o65::fault(variant)]),
            )
            .unwrap();
            for index in [0, 1, 127, 255] {
                let mut h = Harness::new_o65(&l, &caller(l.entry()), 0);
                h.bus.ram[0x7100] = index;
                h.bus.ram[0x7101] = 0xee;
                let target = o65::object(&l, "bytes") + u32::from(index);
                h.bus.watched.insert(target);
                h.run();
                h.guards(0);
                assert_eq!(h.bus.ram[0x7102], 217);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, a, k)| (a, k))
                        .collect::<Vec<_>>(),
                    [(target, Access::Write(217)), (target, Access::Read)]
                );
            }
        }
    }
}

#[test]
fn scaled_byte_indexes_keep_wide_payloads_exact_and_bounded() {
    for (ty, width) in [("BYTE", 1u32), ("CARD", 2), ("SIZE", 3), ("LONGCARD", 4)] {
        let source = format!(
            "{ty} POINTER base=$7100 BYTE index=$7104 {ty} value=$7110,result=$7120 {ty} FUNC Exchange({ty} POINTER p BYTE i {ty} v) {ty} old old=p(i) p(i)=v RETURN(old) PROC Main() result=Exchange(base,index,value) RETURN"
        );
        for optimize in [false, true] {
            for (stride, displacement) in [
                (1, 0),
                (2, 0),
                (4, 17),
                (3, 0),
                (5, 17),
                (44, 0),
                (255, 0),
                (257, 0),
                (128, 0),
                (256, 256 - width),
                (256, 257 - width),
            ] {
                let p = shaped(&source, optimize, displacement, Some(stride));
                let c = p.compile(&layout()).unwrap();
                let object = p.compile_o65(&Default::default()).unwrap().bytes;
                for variant in 0..3 {
                    let loaded = (variant > 0).then(|| {
                        format::relocate(
                            &object,
                            &o65::placement(&object, variant - 1, vec![o65::fault(variant - 1)]),
                        )
                        .unwrap()
                    });
                    let indexes: Vec<_> = if matches!(stride, 2 | 3 | 44) && variant == 0 {
                        (0..=255u8).collect()
                    } else {
                        vec![0, 1, 127, 128, 255]
                    };
                    for index in indexes {
                        let base = if index < 128 || stride > 4 {
                            0x22fff0u32
                        } else {
                            0xfffff0
                        };
                        let target = (base + u32::from(index) * stride + displacement) & 0xffffff;
                        let mut h = if let Some(l) = &loaded {
                            Harness::new_o65(l, &caller(l.entry()), 0)
                        } else {
                            Harness::new(&c.image, &caller(c.image.entry), 0)
                        };
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                        h.bus.ram[0x7104] = index;
                        h.bus.ram[0x7105] = 0xee;
                        h.bus.ram[0x7110..0x7114].copy_from_slice(&0x89abcdefu32.to_le_bytes());
                        for i in 0..width + 2 {
                            let at = (target + 0xffffff + i) & 0xffffff;
                            h.bus.map(at, &[0xa5], true);
                            h.bus.watched.insert(at);
                        }
                        for i in 0..width {
                            h.bus.ram[((target + i) & 0xffffff) as usize] =
                                (0xfedcba98u32 >> (8 * i)) as u8;
                        }
                        h.run();
                        h.guards(0);
                        assert_eq!(
                            h.bus.value(0x7120, width as usize),
                            0xfedcba98u32 & (u32::MAX >> (8 * (4 - width)))
                        );
                        let trace: Vec<_> = (0..width)
                            .map(|i| ((target + i) & 0xffffff, Access::Read))
                            .chain((0..width).map(|i| {
                                (
                                    (target + i) & 0xffffff,
                                    Access::Write((0x89abcdefu32 >> (8 * i)) as u8),
                                )
                            }))
                            .collect();
                        assert_eq!(
                            h.bus
                                .trace
                                .iter()
                                .map(|&(_, a, k)| (a, k))
                                .collect::<Vec<_>>(),
                            trace,
                            "{ty}/{stride}/{displacement}/{index}"
                        );
                        assert_eq!(h.bus.ram[((target + 0xffffff) & 0xffffff) as usize], 0xa5);
                        assert_eq!(h.bus.ram[((target + width) & 0xffffff) as usize], 0xa5);
                    }
                }
            }
        }
    }
}

#[test]
fn scaled_index_windows_survive_irq_nmi_reentry() {
    let source = "MODULE TEST BYTE irqAck=$7800 LONGCARD scratch LONGCARD FUNC Work(LONGCARD POINTER p BYTE i) LONGCARD old old=p(i) p(i)=old RETURN(old) CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work(LONGCARD POINTER($7140),1) irqAck=1 RETURN(saved) PROC Task(LONGCARD POINTER argument) argument^=Work(argument,1) RETURN PROC Main() RETURN ENDMODULE";
    for stride in [4, 3, 44] {
        check_scaled_interrupts(source, stride);
    }
}

fn check_scaled_interrupts(source: &str, stride: u32) {
    use actionc_vm::native65816::{Inputs, Machine};
    use support::context::*;
    for optimize in [false, true] {
        for domain in 0..2 {
            let mut h = ContextHarness::from_prepared(
                source,
                optimize,
                "Task",
                &[0x7100, 0x7120],
                shaped(source, optimize, 0, Some(stride)),
            );
            let mut r = h.cpu.registers();
            r.a = h.first[domain].saved_s;
            h.cpu = Machine::start_at(r);
            h.bus.ram[0x7100 + domain * 0x20] = 1;
            let address = routine(&h.image, "Work");
            let end = address
                + h.image
                    .routines
                    .iter()
                    .find(|r| r.address == address)
                    .unwrap()
                    .size;
            assert!(
                h.cpu
                    .run_until(
                        &mut h.bus,
                        100_000,
                        |_| Inputs::default(),
                        |c| c.is_instruction_boundary() && c.pc() == address
                    )
                    .unwrap()
            );
            let mut sites = 0;
            while (address..end).contains(&h.cpu.pc()) {
                let checkpoint = h.cpu.clone();
                let memory = h.bus.clone();
                for mask in [0, 4] {
                    let mut r = checkpoint.registers();
                    r.p = (r.p & !4) | mask;
                    let at = Machine::start_at(r);
                    let mut reference = at.clone();
                    let mut rb = memory.clone();
                    reference.tick(&mut rb, Inputs::default()).unwrap();
                    while !reference.is_instruction_boundary() {
                        reference.tick(&mut rb, Inputs::default()).unwrap();
                    }
                    for nmi in [false, true] {
                        h.cpu = at.clone();
                        h.bus = memory.clone();
                        let mut ack = false;
                        let mut restored = false;
                        for _ in 0..100_000 {
                            let writes = h.bus.writes.len();
                            h.tick(Inputs {
                                irq: !nmi && !ack,
                                nmi: nmi && !ack,
                                ..Default::default()
                            });
                            ack |= h.bus.writes[writes..]
                                .iter()
                                .any(|&(a, _)| a == if nmi { NMI_ACK } else { IRQ_ACK });
                            if h.cpu.is_instruction_boundary()
                                && h.cpu.pc() == reference.pc()
                                && h.cpu.registers().d == r.d
                                && (ack || !nmi && mask == 4)
                            {
                                assert_eq!(h.cpu.registers(), reference.registers());
                                assert_eq!(
                                    // Sampling completes TCS/RTL first. Released
                                    // bytes may then hold the interrupt frame.
                                    &h.bus.ram[usize::from(reference.registers().s) + 1
                                        ..0x5000 + domain * 0x1000],
                                    &rb.ram[usize::from(reference.registers().s) + 1
                                        ..0x5000 + domain * 0x1000]
                                );
                                let dp = usize::from(r.d);
                                assert_eq!(&h.bus.ram[dp..dp + 256], &rb.ram[dp..dp + 256]);
                                assert_eq!(ack, nmi || mask == 0);
                                restored = true;
                                break;
                            }
                            assert!(!h.cpu.is_stopped());
                        }
                        assert!(restored, "{optimize}/{domain}/{mask}/{nmi}/{:06x}", at.pc());
                    }
                }
                h.cpu = checkpoint;
                h.bus = memory;
                h.tick(Inputs::default());
                while !h.cpu.is_instruction_boundary() {
                    h.tick(Inputs::default());
                }
                sites += 1;
            }

            assert!(sites >= 8);
            h.run();
            h.guards();
        }
    }
}

#[test]
fn indexed_wide_zero_stores_zero_extend_narrow_constants_without_neighbor_writes() {
    for ty in ["CARD", "SIZE", "LONGCARD"] {
        let width = match ty {
            "CARD" => 2,
            "SIZE" => 3,
            _ => 4,
        };
        let source = format!(
            "{ty} POINTER base=$7100 BYTE index=$7104 PROC Work({ty} POINTER p BYTE i) p(i)=0 RETURN PROC Main() Work(base,index) RETURN"
        );
        let p = shaped(&source, true, 7, Some(16));
        let image = p.compile(&layout()).unwrap().image;
        for index in [0, 255] {
            let mut h = Harness::new(&image, &caller(image.entry), 0);
            let base = 0x22ffffu32;
            let at = base + index as u32 * 16 + 7;
            h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
            h.bus.ram[0x7104] = index;
            h.bus.map(at - 1, &[0xa5; 6], true);
            h.bus.watched.extend(at - 1..at + width + 1);
            h.run();
            h.guards(0);
            assert_eq!(
                h.bus
                    .trace
                    .iter()
                    .map(|&(_, a, k)| (a, k))
                    .collect::<Vec<_>>(),
                (at..at + width)
                    .map(|a| (a, Access::Write(0)))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                (
                    h.bus.ram[(at - 1) as usize],
                    h.bus.ram[(at + width) as usize]
                ),
                (0xa5, 0xa5)
            );
        }
    }
}
