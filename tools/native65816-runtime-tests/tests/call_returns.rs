mod support;
use actionc::mir65816::{Mir65816Op, abi, emit::proof, image::AssemblyImport, o65 as format};
use actionc::nir::runtime_symbol_id;
use actionc_vm::native65816::{Inputs, Machine};
use support::{context::*, *};

const TYPES: &[(&str, u8)] = &[("BYTE", 1), ("CARD", 2), ("INT", 2)];
const VALUES: &[u32] = &[0, 1, 0x7f, 0x80, 0xff, 0x100, 0x7fff, 0x8000, 0xffff];

fn reach(h: &mut Harness, pc: u32) {
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                200_000,
                |_| Inputs::default(),
                |c| c.is_instruction_boundary() && c.pc() == pc
            )
            .unwrap()
    );
}

fn caller_for(address: u32, width: u8) -> Assembly {
    let outgoing = width | 1;
    let mut s =
        format!("tsc\nsec\nsbc #{outgoing}\ntcs\nsep #$20\n.a8\nlda #0\nsta {outgoing},s\n");
    for i in 0..width {
        s.push_str(&format!(
            "lda f:${:06x}\nsta {},s\n",
            0x7100 + u32::from(i),
            i + 1
        ));
    }
    s.push_str(&format!("rep #$20\n.a16\njsl ${address:06x}\n.export returned\nreturned: sta f:$007200\ntxa\nsta f:$007202\ntsc\nclc\nadc #{outgoing}\ntcs\nstp\nnop\n"));
    assemble_artifact(&s, 0x040000)
}

fn echo(width: u8) -> Vec<u8> {
    let mut s =
        String::from("sep #$20\n.a8\nldx #63\nlda #$a7\nclobber: sta 0,x\ndex\nbpl clobber\n");
    if width == 1 {
        s.push_str("lda 4,s\nrep #$20\n.a16\nand #$00ff\nldx #$beef\n");
    } else {
        s.push_str("rep #$20\n.a16\nldx #$beef\nlda 4,s\n");
    }
    s.push_str("ldy #$dead\nrtl\nnop\n");
    assemble(&s, 0x041000)
}

#[test]
fn forwarded_results_match_ca65_without_private_spills_or_reloads() {
    for &(ty, width) in TYPES {
        let source = format!(
            "MODULE TEST\nPUBLIC EXTERNAL {ty} FUNC Observe({ty} value)\n{ty} FUNC Forward({ty} value) RETURN(Observe(value))\nPROC Main() RETURN\nENDMODULE\n"
        );
        let leaf = echo(width);
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            let external = p
                .mir
                .routines
                .iter()
                .find(|r| r.entry.external_symbol == Some(runtime_symbol_id("TEST.Observe")))
                .unwrap();
            let mut options = layout();
            options.imports.push(AssemblyImport {
                symbol: runtime_symbol_id("TEST.Observe").0,
                signature: external.signature.0,
                abi: abi::generated::ABI_NAME.into(),
                address: 0x041000,
                size: leaf.len() as u32,
                stack_peak: 0,
                checks_stack: true,
                irq_effect: Default::default(),
            });
            let c = p.compile(&options).unwrap();
            assert_eq!(
                c.image.to_json().unwrap(),
                prepare(&source.replace('\n', "\r\n"), optimize)
                    .compile(&options)
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            let r = p
                .mir
                .routines
                .iter()
                .find(|r| !r.entry.external && r.result_home.is_some())
                .unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let ir = c.image.routines.iter().find(|m| m.id == r.id.0).unwrap();
            let block = &r.blocks[0];
            let i = block.ops.len() - 1;
            let Mir65816Op::Call {
                plan,
                result: Some((id, _)),
                ..
            } = &block.ops[i]
            else {
                panic!()
            };
            assert!(m.frame.temps.contains_key(id));
            let call_span = &m.code.mir_spans[&(block.id, i)];
            let tail = &m.code.mir_spans[&(block.id, i + 1)];
            let jsl = forwarding::instructions(&m.code.bytes)
                .keys()
                .copied()
                .find(|&at| call_span.contains(&at) && m.code.bytes[at] == 0x22)
                .unwrap();
            let cleanup = assemble(
                &format!(
                    "tay\ntsc\nclc\nadc #{}\ntcs\ntya\n",
                    plan.outgoing_bytes.get()
                ),
                0x050000,
            );
            assert_eq!(&m.code.bytes[jsl + 4..call_span.end], cleanup);
            let teardown = assemble(
                &format!("tay\ntsc\nclc\nadc #{}\ntcs\ntya\nrtl\n", m.frame.extent),
                0x050000,
            );
            assert_eq!(&m.code.bytes[tail.clone()], teardown);
            let (reference, rt) = proof::materialize_reference(&p.mir, true).unwrap();
            let (replayed, pt) = proof::materialize_replayed(&p.mir, true).unwrap();
            for ((a, b), (x, y)) in reference
                .routines
                .iter()
                .zip(&replayed.routines)
                .zip(rt.iter().zip(&pt))
            {
                proof::compare_replay_output(&a.code, &b.code).unwrap();
                assert_eq!(x.snapshots, y.snapshots);
            }
            let caller = caller_for(ir.address, width);
            for &value in VALUES {
                let expected = value & (u32::MAX >> (8 * (4 - width)));
                for mask in [0, 4] {
                    let mut h = Harness::new(&c.image, &caller.bytes, mask);
                    h.bus.map(0x041000, &leaf, false);
                    h.bus.ram[0x7100..0x7104].copy_from_slice(&value.to_le_bytes());
                    reach(&mut h, ir.address + jsl as u32 + 4);
                    let before = h.cpu.registers();
                    assert_eq!(before.a, expected as u16);
                    assert_eq!(before.x, 0xbeef);
                    let reads = h.bus.reads.len();
                    let writes = h.bus.writes.len();
                    reach(&mut h, caller.symbols["returned"]);
                    let after = h.cpu.registers();
                    assert_eq!(
                        (
                            after.a,
                            after.x,
                            after.s,
                            after.d,
                            after.dbr,
                            after.p & 0x3c
                        ),
                        (
                            before.a,
                            before.x,
                            0x5ff0 - u16::from(width | 1),
                            before.d,
                            before.dbr,
                            mask
                        )
                    );
                    assert_eq!(h.bus.writes.len(), writes);
                    let stack_reads: Vec<_> = h.bus.reads[reads..]
                        .iter()
                        .copied()
                        .filter(|a| (0x4000..0x6000).contains(a))
                        .collect();
                    let return_s =
                        u32::from(before.s) + plan.outgoing_bytes.get() + u32::from(m.frame.extent);
                    assert_eq!(
                        stack_reads,
                        (1..=3).map(|i| return_s + i).collect::<Vec<_>>()
                    );
                    assert!(
                        !h.bus.reads[reads..]
                            .iter()
                            .any(|a| (0x2000..0x2040).contains(a))
                    );
                    h.run();
                    h.guards(mask);
                }
            }
        }
    }
}

#[test]
fn forwarded_calls_and_recursive_returns_execute_at_two_o65_placements() {
    for &(ty, width) in TYPES {
        let source = format!(
            "{ty} FUNC Echo({ty} value) RETURN(value)\n{ty} FUNC Rec({ty} value CARD depth) IF depth=0 THEN RETURN(Echo(value)) FI RETURN(Rec(value,depth-1))\n{ty} FUNC Forward({ty} value) RETURN(Rec(value,3))\nPROC Main() RETURN\n"
        );
        for optimize in [false, true] {
            let object = o65::compile(&source, optimize, vec![]);
            for variant in 0..2 {
                let placed = format::relocate(
                    &object,
                    &o65::placement(&object, variant, vec![o65::fault(variant)]),
                )
                .unwrap();
                let caller = caller_for(o65::routine(&placed, "Forward"), width);
                for &value in VALUES {
                    for mask in [0, 4] {
                        let mut h = Harness::new_o65(&placed, &caller.bytes, mask);
                        h.bus.ram[0x7100..0x7104].copy_from_slice(&value.to_le_bytes());
                        h.run();
                        h.guards(mask);
                        assert_eq!(
                            h.bus.value(0x7200, usize::from(width.max(2))),
                            value & (u32::MAX >> (8 * (4 - width)))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn forwarded_result_cleanup_survives_irq_and_nmi_at_each_instruction() {
    for &(ty, width) in TYPES {
        let source = format!(
            "MODULE TEST\nBYTE irqAck=$7800\n{ty} scratch\n{ty} FUNC Echo({ty} value) RETURN(value)\n{ty} FUNC Forward({ty} value) RETURN(Echo(value))\nCARD FUNC Dispatch(CARD saved BYTE reason) scratch=Forward({ty}(7)) irqAck=1 RETURN(saved)\nPROC Task({ty} POINTER argument) argument^=Forward(argument^) RETURN\nPROC Main() RETURN\nENDMODULE\n"
        );
        for optimize in [false, true] {
            for domain in 0..2 {
                let mut h = ContextHarness::new(&source, optimize, "Task", &[0x7100, 0x7120]);
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
                let end = address + segment.bytes.len() as u32;
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            100_000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == address + jsl as u32 + 4
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
                assert_eq!(sites, 13); // six cleanup + six frame release + RTL
                h.run();
                h.guards();
                assert_eq!(h.bus.value(argument as u32, width.into()), value);
            }
        }
    }
}
