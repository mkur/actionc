//! Independent ABI/value oracles shared by the call-flow implementation slices.
mod support;
use actionc::mir65816::{abi, image::AssemblyImport};
use actionc::nir::runtime_symbol_id;
use support::*;

const LEAF: u32 = 0x041000;
const ARGUMENTS: u32 = 0x7380;

fn observe(width: u8, value_offset: u8, tail_offset: u8, outgoing: u8) -> Vec<u8> {
    let mut s = String::from("sep #$20\n.a8\n");
    // Copy the complete ABI area independently, including its alignment holes.
    for byte in 0..outgoing {
        s.push_str(&format!(
            "lda {},s\nsta f:${:06x}\n",
            byte + 4,
            ARGUMENTS + u32::from(byte)
        ));
    }
    s.push_str("ldx #63\nlda #$a7\nclobber: sta $80,x\ndex\nbpl clobber\n");
    match width {
        1 => s.push_str(&format!(
            "lda {},s\nrep #$20\n.a16\nand #$00ff\nldx #$beef\n",
            value_offset + 4
        )),
        2 => s.push_str(&format!(
            "rep #$20\n.a16\nlda {},s\nldx #$beef\n",
            value_offset + 4
        )),
        3 => s.push_str(&format!(
            "lda {},s\nrep #$20\n.a16\nand #$00ff\ntax\nlda {},s\n",
            value_offset + 6,
            value_offset + 4
        )),
        4 => s.push_str(&format!(
            "rep #$20\n.a16\nlda {},s\ntax\nlda {},s\n",
            value_offset + 6,
            value_offset + 4
        )),
        _ => unreachable!(),
    }
    // Contradictory N/Z and carry are legal unspecified results; A/X payload,
    // defined extension, I/D, S, D and DBR retain their native ABI obligations.
    s.push_str("ldy #$dead\nsep #$83\nrtl\nnop\n");
    assert!(tail_offset + 2 <= outgoing);
    assemble(&s, LEAF)
}

#[test]
fn native_result_consumers_and_private_multi_arguments_match_independent_abi() {
    for (ty, width, value_offset, tail_offset, outgoing) in [
        ("BYTE", 1u8, 1u8, 2u8, 5u8),
        ("CARD", 2, 2, 4, 7),
        ("ADDRESS", 3, 2, 6, 9),
        ("LONGCARD", 4, 2, 6, 9),
    ] {
        let source = format!(
            "MODULE TEST\nPUBLIC EXTERNAL {ty} FUNC Observe(BYTE tag {ty} value CARD tail)\n\
             {ty} input=$7100,stored=$7200\nBYTE equal=$7230,unequal=$7231\n\
             {ty} FUNC Forward({ty} value) RETURN(Observe(5,value,$9a7b))\n\
             PROC Main() {ty} local\nlocal=Observe(5,input,$9a7b)\n\
             stored=Observe(5,local,$9a7b)\nequal=Observe(5,input,$9a7b)=0\n\
             IF Observe(5,input,$9a7b)<>0 THEN unequal=1 ELSE unequal=0 FI\n\
             Forward(input) RETURN\nENDMODULE\n"
        );
        let leaf = observe(width, value_offset, tail_offset, outgoing);
        let mask = u32::MAX >> (32 - u32::from(width) * 8);
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            let external = p
                .mir
                .routines
                .iter()
                .find(|r| r.entry.external_symbol == Some(runtime_symbol_id("TEST.Observe")))
                .unwrap();
            for guards in [false, true] {
                let mut options = layout();
                options.stack_checks = guards;
                options.imports.push(AssemblyImport {
                    symbol: runtime_symbol_id("TEST.Observe").0,
                    signature: external.signature.0,
                    abi: abi::generated::ABI_NAME.into(),
                    address: LEAF,
                    size: leaf.len() as u32,
                    stack_peak: 0,
                    checks_stack: true,
                    irq_effect: Default::default(),
                });
                let c = p.compile(&options).unwrap();
                let main = c
                    .machine
                    .prepared
                    .routines
                    .iter()
                    .find(|r| r.name.to_ascii_uppercase().contains("_MAIN_"))
                    .unwrap();
                let machine = c.machine.routines.iter().find(|m| m.id == main.id).unwrap();
                let mut local_stores = 0;
                for block in &main.blocks {
                    for (index, pair) in block.ops.windows(2).enumerate() {
                        if let [
                            actionc::mir65816::Mir65816Op::Call {
                                result: Some((temp, _)),
                                ..
                            },
                            actionc::mir65816::Mir65816Op::Store { address, .. },
                        ] = pair
                            && matches!(
                                address.base,
                                actionc::mir65816::Mir65816AddressBase::AutomaticFrame(_)
                            )
                        {
                            assert!(!machine.frame.temps.contains_key(temp));
                            assert!(!machine.code.mir_spans[&(block.id, index + 1)].is_empty());
                            local_stores += 1;
                        }
                    }
                }
                assert!(local_stores <= 1, "final private Store coverage");
                if width == 1 || width == 2 && optimize {
                    let r = c
                        .machine
                        .prepared
                        .routines
                        .iter()
                        .find(|r| !r.entry.external && r.result_home.is_none())
                        .unwrap();
                    let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                    let mut admitted = 0;
                    for block in &r.blocks {
                        for pair in block.ops.windows(2) {
                            if let [
                                actionc::mir65816::Mir65816Op::Call {
                                    result: Some((temp, _)),
                                    ..
                                },
                                actionc::mir65816::Mir65816Op::Compare { .. },
                            ] = pair
                            {
                                assert!(!m.frame.temps.contains_key(temp));
                                admitted += 1;
                            }
                        }
                    }
                    assert_eq!(admitted, 2, "byte/word zero-test coverage");
                }

                // Use the actual frontend/compile path for both host newlines.
                assert_eq!(
                    c.image.to_json().unwrap(),
                    prepare(&source.replace('\n', "\r\n"), optimize)
                        .compile(&options)
                        .unwrap()
                        .image
                        .to_json()
                        .unwrap()
                );
                let image = actionc::mir65816::image::Image::from_json(&c.image.to_json().unwrap())
                    .unwrap();
                let caller = caller(image.entry);
                for value in [
                    0,
                    1,
                    mask,
                    1 << (u32::from(width) * 8 - 1),
                    0x89abcdef & mask,
                ] {
                    for irq_mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller, irq_mask);
                        h.bus.map(LEAF, &leaf, false);
                        h.bus.ram[0x7100..0x7110].fill(0x6d);
                        h.bus.ram[0x7200..0x7210].fill(0x6d);
                        h.bus.ram[0x7100..0x7100 + usize::from(width)]
                            .copy_from_slice(&value.to_le_bytes()[..usize::from(width)]);
                        h.bus.watched.extend(0x7100..0x7110);
                        h.bus.watched.extend(0x7200..0x7210);
                        h.run();
                        h.guards(irq_mask);
                        assert_eq!(
                            h.bus.value(0x7200, width.into()),
                            value,
                            "{ty}/{optimize}/{guards}"
                        );
                        assert_eq!(h.bus.value(0x7230, 1), u32::from(value == 0));
                        assert_eq!(h.bus.value(0x7231, 1), u32::from(value != 0));
                        assert!(
                            h.bus.ram[0x7200 + usize::from(width)..0x7210]
                                .iter()
                                .all(|&b| b == 0x6d)
                        );
                        assert!(
                            h.bus.trace.iter().all(|(_, address, _)| {
                                (0x7100..0x7100 + u32::from(width)).contains(address)
                                    || (0x7200..0x7200 + u32::from(width)).contains(address)
                            }),
                            "neighbor access: {ty}"
                        );
                        let mut expected = vec![0; usize::from(outgoing)];
                        expected[0] = 5;
                        expected[usize::from(value_offset)..usize::from(value_offset + width)]
                            .copy_from_slice(&value.to_le_bytes()[..usize::from(width)]);
                        expected[usize::from(tail_offset)..usize::from(tail_offset + 2)]
                            .copy_from_slice(&0x9a7bu16.to_le_bytes());
                        assert_eq!(
                            &h.bus.ram[ARGUMENTS as usize..ARGUMENTS as usize + expected.len()],
                            expected
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn private_record_field_results_write_exactly_the_owned_range() {
    use actionc::mir65816::{Mir65816AddressBase, Mir65816Op};
    use actionc_vm::native65816::{Access, Inputs};
    for (ty, width, value_offset, tail_offset, outgoing) in [
        ("BYTE", 1u8, 1u8, 2u8, 5u8),
        ("CARD", 2, 2, 4, 7),
        ("ADDRESS", 3, 2, 6, 9),
        ("LONGCARD", 4, 2, 6, 9),
    ] {
        let source = format!(
            "MODULE TEST PUBLIC EXTERNAL {ty} FUNC Observe(BYTE tag {ty} value CARD tail) TYPE Packet=[BYTE before {ty} payload BYTE after] {ty} input=$7100,stored=$7200 BYTE before=$7240,after=$7241 PROC Main() Packet local local.before=$5a local.after=$a5 local.payload=Observe(5,input,$9a7b) stored=local.payload before=local.before after=local.after RETURN ENDMODULE"
        );
        let leaf = observe(width, value_offset, tail_offset, outgoing);
        for optimize in [false, true] {
            let mut p = prepare(&source, optimize);
            // Record lowering currently leaves the whole object's mutable fact
            // unset despite its payload Stores. Preserve that source fallback;
            // this verified typed fixture supplies the stronger ownership fact
            // to qualify resolved, nonzero-offset Store destinations.
            let r = p
                .mir
                .routines
                .iter_mut()
                .find(|r| r.name.to_ascii_uppercase().contains("_MAIN_"))
                .unwrap();
            for object in &mut r.frame.objects {
                object.mutable = true;
            }
            actionc::mir65816::verify_program(&p.mir).unwrap();
            let external = p.mir.routines.iter().find(|r| r.entry.external).unwrap();
            for guards in [false, true] {
                let mut options = layout();
                options.stack_checks = guards;
                options.imports.push(AssemblyImport {
                    symbol: external.entry.external_symbol.unwrap().0,
                    signature: external.signature.0,
                    abi: abi::generated::ABI_NAME.into(),
                    address: LEAF,
                    size: leaf.len() as u32,
                    stack_peak: 0,
                    checks_stack: true,
                    irq_effect: Default::default(),
                });
                let c = p.compile(&options).unwrap();
                let r = c
                    .machine
                    .prepared
                    .routines
                    .iter()
                    .find(|r| r.name.to_ascii_uppercase().contains("_MAIN_"))
                    .unwrap();
                let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                let (block, index, temp, address) = r
                    .blocks
                    .iter()
                    .find_map(|b| {
                        b.ops.windows(2).enumerate().find_map(|(i, ops)| {
                            if let [
                                Mir65816Op::Call {
                                    result: Some((temp, _)),
                                    ..
                                },
                                Mir65816Op::Store { address, .. },
                            ] = ops
                            {
                                Some((b.id, i + 1, *temp, address))
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap();
                assert!(
                    !m.frame.temps.contains_key(&temp),
                    "{ty}/{optimize}/{guards} {r:#?}"
                );
                let Mir65816AddressBase::AutomaticFrame(id) = address.base else {
                    panic!()
                };
                let object = r.frame.objects.iter().find(|o| o.id == id).unwrap();
                assert!(address.displacement.get() > 0);
                let span = &m.code.mir_spans[&(block, index)];
                let entry = context::routine(&c.image, "Main");
                let caller = caller(c.image.entry);
                let mut h = Harness::new(&c.image, &caller, 0);
                h.bus.map(LEAF, &leaf, false);
                let value = 0x89abcdefu32 & (u32::MAX >> (32 - u32::from(width) * 8));
                h.bus.ram[0x7100..0x7100 + usize::from(width)]
                    .copy_from_slice(&value.to_le_bytes()[..usize::from(width)]);
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            100_000,
                            |_| Inputs::default(),
                            |cpu| cpu.is_instruction_boundary()
                                && cpu.pc() == entry + span.start as u32
                        )
                        .unwrap()
                );
                let base = u32::from(h.cpu.registers().s) + object.stack_offset.get();
                let dest = base + address.displacement.get();
                h.bus.watched.extend(base..base + object.size.get());
                h.bus.trace.clear();
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            100_000,
                            |_| Inputs::default(),
                            |cpu| cpu.is_instruction_boundary()
                                && cpu.pc() == entry + span.end as u32
                        )
                        .unwrap()
                );
                let writes: Vec<_> = h
                    .bus
                    .trace
                    .iter()
                    .filter_map(|(_, a, access)| {
                        if let Access::Write(v) = access {
                            Some((*a, *v))
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(
                    writes,
                    (0..u32::from(width))
                        .map(|i| (dest + i, value.to_le_bytes()[i as usize]))
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    h.bus.trace.len(),
                    usize::from(width),
                    "no neighboring reads or writes"
                );
                h.run();
                h.guards(0);
                assert_eq!(h.bus.value(0x7200, width.into()), value);
                assert_eq!(h.bus.value(0x7240, 1), 0x5a);
                assert_eq!(h.bus.value(0x7241, 1), 0xa5);
            }
        }
    }
}
