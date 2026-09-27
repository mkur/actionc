mod support;
use actionc::mir65816::{Mir65816AddressBase, Mir65816Op, emit, o65 as format};
use actionc_vm::native65816::{Access, Inputs, Machine};
use support::context::*;
use support::*;

const SOURCE: &str = r#"
PROC Touch() RETURN
LONGCARD FUNC Read(LONGCARD POINTER p)
 LONGCARD v
 v=p^ Touch()
RETURN(v)
LONGCARD result=$7200
PROC Main() result=Read(LONGCARD POINTER($12fffe)) RETURN
"#;

#[test]
fn final_store_addresses_preserve_exact_width_banked_traffic() {
    for (ty, width, place, rhs, offset) in [
        ("BYTE", 1u32, "p^", "v", 0),
        ("CARD", 2, "p(3)", "v", 6),
        ("ADDRESS", 3, "p(i)", "v", 765),
        ("LONGCARD", 4, "p^", "v", 0),
        ("LONGCARD", 4, "p(i)", "$89abcdef", 1020),
        ("BYTE", 1, "p(3)", "0", 3),
    ] {
        let source = format!(
            "{ty} POINTER base=$7100\n{ty} value=$7104\nBYTE index=$7108\nPROC Touch() RETURN\nPROC Write({ty} POINTER p {ty} v BYTE i) {place}={rhs} Touch() RETURN\nPROC Main() Write(base,value,index) RETURN\n"
        );
        for local in [false, true] {
            let source = if local {
                source.replace(
                    &format!(" {place}="),
                    &format!(
                        " {ty} POINTER local local=p IF i=0 THEN local=p FI {}=",
                        place.replace("p", "local")
                    ),
                )
            } else {
                source.clone()
            };
            for optimize in [false, true] {
                let p = prepare(&source, optimize);
                let c = p.compile(&layout()).unwrap();
                assert_eq!(
                    c.image.to_json().unwrap(),
                    prepare(&source.replace('\n', "\r\n"), optimize)
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
                    .find(|r| r.name == "Write")
                    .unwrap();
                let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                assert!(r.blocks.iter().any(|b| b.ops.iter().enumerate().any(|(i,op)|
                matches!(op,Mir65816Op::Load {width,address,volatile:false,..} if width.get()==3 && (if local { matches!(address.base,Mir65816AddressBase::AutomaticFrame(_)) } else { matches!(address.base,Mir65816AddressBase::Parameter(_)) }))
                    && m.code.mir_spans[&(b.id,i)].is_empty())), "{ty}/{place}/{local}/{optimize}: {:?}",r.blocks);
                let (direct, a) = emit::proof::materialize_reference(&p.mir, true).unwrap();
                let (replayed, b) = emit::proof::materialize_replayed(&p.mir, true).unwrap();
                for ((x, y), (a, b)) in direct
                    .routines
                    .iter()
                    .zip(&replayed.routines)
                    .zip(a.iter().zip(&b))
                {
                    emit::proof::compare_replay_output(&x.code, &y.code).unwrap();
                    assert_eq!(a.snapshots, b.snapshots);
                }
                let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
                for variant in 0..3 {
                    let loaded = (variant > 0).then(|| {
                        format::relocate(
                            &bytes,
                            &o65::placement(&bytes, variant - 1, vec![o65::fault(variant - 1)]),
                        )
                        .unwrap()
                    });
                    for value in [0u32, 0x89abcdef] {
                        for mask in [0, 4] {
                            let mut h = if let Some(l) = &loaded {
                                Harness::new_o65(l, &caller(l.entry()), mask)
                            } else {
                                Harness::new(&c.image, &caller(c.image.entry), mask)
                            };
                            let target = 0x12fffeu32;
                            h.bus.ram[0x7100..0x7103]
                                .copy_from_slice(&(target - offset).to_le_bytes()[..3]);
                            h.bus.ram[0x7104..0x7108].copy_from_slice(&value.to_le_bytes());
                            h.bus.ram[0x7108] = 255;
                            h.bus
                                .map(target - 1, &vec![0xa5; (width + 2) as usize], true);
                            h.bus.watched.extend(target - 1..target + width + 1);
                            h.run();
                            h.guards(mask);
                            let expected = match rhs {
                                "0" => 0,
                                "$89abcdef" => 0x89abcdef,
                                _ => value,
                            } & (u32::MAX >> (8 * (4 - width)));
                            assert_eq!(h.bus.value(target, width as usize), expected);
                            assert_eq!(
                                h.bus
                                    .trace
                                    .iter()
                                    .map(|&(_, a, k)| (a, k))
                                    .collect::<Vec<_>>(),
                                (target..target + width)
                                    .map(|a| (
                                        a,
                                        Access::Write((expected >> (8 * (a - target))) as u8)
                                    ))
                                    .collect::<Vec<_>>()
                            );
                            assert_eq!(
                                (
                                    h.bus.ram[(target - 1) as usize],
                                    h.bus.ram[(target + width) as usize]
                                ),
                                (0xa5, 0xa5)
                            );
                        }
                    }
                }
            }
        }
    }
}

// Exercise a single captured pointer twice, with a separate harmless capture
// between uses. The typed fixture is verified after adding fresh definitions.
fn prepared(source: &str, optimize: bool) -> actionc::compiler::native65816::Prepared {
    let mut p = prepare(source, optimize);
    for r in &mut p.mir.routines {
        if !r.name.to_ascii_uppercase().contains("READ")
            && !r.name.to_ascii_uppercase().contains("FORWARD")
        {
            continue;
        }
        for b in &mut r.blocks {
            let Some(i)=b.ops.windows(2).position(|pair| matches!(&pair[0], Mir65816Op::Load{width,address,volatile:false,..} if width.get()==3 && matches!(address.base,Mir65816AddressBase::Parameter(_) | Mir65816AddressBase::AutomaticFrame(_)))
                && matches!(&pair[1],Mir65816Op::Load{address,volatile:false,..} if matches!(address.base,Mir65816AddressBase::Indirect(_)))) else {continue};
            let next = r.temps.iter().map(|(id, _)| id.0).max().unwrap() + 1;
            for (at, mut op, id) in [
                (i + 2, b.ops[i].clone(), next),
                (i + 3, b.ops[i + 1].clone(), next + 1),
            ] {
                let Mir65816Op::Load { dest, .. } = &mut op else {
                    unreachable!()
                };
                let ty = r.temps.iter().find(|(id, _)| id == dest).unwrap().1.clone();
                *dest = actionc::nir::TempId(id);
                r.temps.push((*dest, ty));
                b.ops.insert(at, op);
            }
            break;
        }
    }
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
}

#[test]
fn borrowed_pointer_reads_preserve_banked_accesses_and_replay() {
    for source in [
        SOURCE.to_string(),
        SOURCE.replace(
            "LONGCARD v\n v=p^",
            "LONGCARD POINTER local\n LONGCARD v\n local=p v=local^",
        ),
    ] {
        for optimize in [false, true] {
            let p = prepared(&source, optimize);
            let compiled = p.compile(&layout()).unwrap();
            assert_eq!(
                compiled.image.to_json().unwrap(),
                prepared(&source.replace('\n', "\r\n"), optimize)
                    .compile(&layout())
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            let mut removed = 0;
            for r in &compiled.machine.prepared.routines {
                let m = compiled
                    .machine
                    .routines
                    .iter()
                    .find(|m| m.id == r.id)
                    .unwrap();
                for b in &r.blocks {
                    for (i, op) in b.ops.iter().enumerate() {
                        if matches!(op,Mir65816Op::Load {width,address,volatile:false,..} if width.get()==3 && matches!(address.base,Mir65816AddressBase::Parameter(_) | Mir65816AddressBase::AutomaticFrame(_)))
                            && m.code.mir_spans[&(b.id, i)].is_empty()
                        {
                            removed += 1;
                        }
                    }
                }
            }
            assert!(removed > 0);
            let (direct, a) = emit::proof::materialize_reference(&p.mir, true).unwrap();
            let (replayed, b) = emit::proof::materialize_replayed(&p.mir, true).unwrap();
            for ((x, y), (a, b)) in direct
                .routines
                .iter()
                .zip(&replayed.routines)
                .zip(a.iter().zip(&b))
            {
                emit::proof::compare_replay_output(&x.code, &y.code).unwrap();
                assert_eq!(a.snapshots, b.snapshots);
            }
            let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..3 {
                let loaded = (variant > 0).then(|| {
                    format::relocate(
                        &bytes,
                        &o65::placement(&bytes, variant - 1, vec![o65::fault(variant - 1)]),
                    )
                    .unwrap()
                });
                for value in [0, 0xffffffff, 0x89abcdef, 0x12345678] {
                    for mask in [0, 4] {
                        let mut h = if let Some(l) = &loaded {
                            Harness::new_o65(l, &caller(l.entry()), mask)
                        } else {
                            Harness::new(&compiled.image, &caller(compiled.image.entry), mask)
                        };
                        h.bus.map(0x12fffd, &[0xa5; 6], true);
                        h.bus.ram[0x12fffe..0x130002].copy_from_slice(&u32::to_le_bytes(value));
                        h.bus.watched.extend(0x12fffd..0x130003);
                        h.run();
                        h.guards(mask);
                        assert_eq!(h.bus.value(0x7200, 4), value);
                        assert_eq!(
                            h.bus
                                .trace
                                .iter()
                                .map(|&(_, a, k)| (a, k))
                                .collect::<Vec<_>>(),
                            (0x12fffe..0x130002)
                                .chain(0x12fffe..0x130002)
                                .map(|a| (a, Access::Read))
                                .collect::<Vec<_>>()
                        );
                        assert_eq!((h.bus.ram[0x12fffd], h.bus.ram[0x130002]), (0xa5, 0xa5));
                    }
                }
            }
        }
    }
}

#[test]
fn borrowed_pointer_consumers_survive_irq_and_nmi_at_each_instruction() {
    for (local, store) in [
        (false, 0),
        (true, 0),
        (false, 1),
        (true, 1),
        (false, 2),
        (true, 2),
    ] {
        for &(ty, width) in &[("LONGCARD", 4u8)] {
            let source = format!(
                "MODULE TEST\nBYTE irqAck=$7800\n{ty} scratch\nPROC Touch() RETURN\n{ty} FUNC Forward({ty} POINTER value) {ty} r r=value^ Touch() RETURN(r)\nCARD FUNC Dispatch(CARD saved BYTE reason) scratch=Forward({ty} POINTER($7140)) irqAck=1 RETURN(saved)\nPROC Task({ty} POINTER argument) argument^=Forward(argument) RETURN\nPROC Main() RETURN\nENDMODULE\n"
            );
            let source = match store {
                1 => source.replace("r=value^ Touch()", "r=value^ value^=r Touch()"),
                2 => source
                    .replace(
                        "PROC Touch()",
                        &format!("{ty} POINTER pointerSink PROC Touch()"),
                    )
                    .replace("r=value^ Touch()", "r=value^ pointerSink=value Touch()"),
                _ => source,
            };
            let source = if local {
                source.replace(
                    &format!("{ty} r r=value^"),
                    &format!("{ty} POINTER local {ty} r local=value IF value=0 THEN local=value FI r=local^"),
                ).replace("pointerSink=value", "pointerSink=local")
            } else {
                source
            };
            for optimize in [false, true] {
                for domain in 0..2 {
                    let mut h = ContextHarness::from_prepared(
                        &source,
                        optimize,
                        "Task",
                        &[0x7100, 0x7120],
                        prepared(&source, optimize),
                    );
                    let mut r = h.cpu.registers();
                    r.a = h.first[domain].saved_s;
                    h.cpu = Machine::start_at(r);
                    let argument = 0x7100 + domain * 0x20;
                    let value = (0x89abcdefu32 >> domain) & (u32::MAX >> (8 * (4 - width)));
                    h.bus.ram[argument..argument + 4].copy_from_slice(&value.to_le_bytes());
                    let address = routine(&h.image, "Forward");
                    let segment = h
                        .image
                        .segments
                        .iter()
                        .find(|s| s.address == address)
                        .unwrap();
                    let jsl = forwarding::instructions(&segment.bytes)
                        .keys()
                        .copied()
                        .find(|&at| segment.bytes[at] == 0x22)
                        .unwrap();
                    let end = address + jsl as u32;
                    let construction = forwarding::instructions(&segment.bytes)
                        .keys()
                        .copied()
                        .filter(|&at| at < jsl && segment.bytes[at] == 0x5c)
                        .next()
                        .unwrap()
                        + 4;
                    assert!(segment.bytes[construction..jsl].contains(&0xb7));
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                100_000,
                                |_| Inputs::default(),
                                |c| c.is_instruction_boundary()
                                    && c.pc() == address + construction as u32
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
                                assert!(
                                    restored,
                                    "{ty}/{optimize}/{domain}/{mask}/{nmi}/{:06x}",
                                    at.pc()
                                );
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
                    assert_eq!(h.bus.value(argument as u32, width.into()), value);
                }
            }
        }
    }
}
