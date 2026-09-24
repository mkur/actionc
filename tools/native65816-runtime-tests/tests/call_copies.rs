mod support;
use actionc::mir65816::{Mir65816Op, abi, image::AssemblyImport, o65 as format};
use actionc::nir::runtime_symbol_id;
use actionc_vm::native65816::Inputs;
use support::*;

fn reach(h: &mut Harness, pc: u32) {
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                200_000,
                |_| Inputs::default(),
                |cpu| cpu.is_instruction_boundary() && cpu.pc() == pc
            )
            .unwrap()
    );
}

fn echo(width: u8) -> Vec<u8> {
    let mut source =
        String::from("sep #$20\n.a8\nldx #63\nlda #$a7\nclobber: sta 0,x\ndex\nbpl clobber\n");
    if width == 1 {
        source.push_str("lda 4,s\nrep #$20\n.a16\nand #$00ff\nldx #$beef\n");
    } else {
        if width == 3 {
            source.push_str("lda 6,s\nrep #$20\n.a16\nand #$00ff\ntax\n");
        } else {
            source.push_str("rep #$20\n.a16\nldx #$beef\n");
            if width == 4 {
                source.push_str("lda 6,s\ntax\n");
            }
        }
        source.push_str("lda 4,s\n");
    }
    source.push_str("ldy #$dead\nrtl\nnop\n");
    assemble(&source, 0x041000)
}

#[test]
fn direct_and_indirect_result_captures_match_ca65_and_preserve_neighbor_bytes() {
    for (ty, width) in [("BYTE", 1u8), ("INT", 2), ("ADDRESS", 3), ("LONGINT", 4)] {
        let source = format!(
            "MODULE TEST\nPUBLIC EXTERNAL {ty} FUNC Observe({ty} value)\n{ty} input=$7100,first=$7200,second=$7210\n{ty} FUNC POINTER cb({ty} value)\nPROC Main() first=Observe(input) cb=@Observe second=cb(input) Observe(input) RETURN\nENDMODULE\n"
        );
        let leaf = echo(width);
        for optimize in [false, true] {
            let prepare_calls = |source: &str| {
                let mut p = prepare(source, optimize);
                // Raw NIR may retain an unused result temp. Exercise the legal
                // MIR discard explicitly in both frontend modes.
                let main = p
                    .mir
                    .routines
                    .iter_mut()
                    .find(|r| !r.entry.external)
                    .unwrap();
                let last = main
                    .blocks
                    .iter_mut()
                    .flat_map(|b| &mut b.ops)
                    .rev()
                    .find(|op| matches!(op, Mir65816Op::Call { .. }))
                    .unwrap();
                let Mir65816Op::Call { result, .. } = last else {
                    unreachable!()
                };
                if let Some((id, _)) = result.take() {
                    main.temps.retain(|(temp, _)| *temp != id);
                }
                actionc::mir65816::verify_program(&p.mir).unwrap();
                p
            };
            let p = prepare_calls(&source);
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
            let compiled = p.compile(&options).unwrap();
            assert_eq!(
                compiled.image.to_json().unwrap(),
                prepare_calls(&source.replace('\n', "\r\n"))
                    .compile(&options)
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            let main = p.mir.routines.iter().find(|r| !r.entry.external).unwrap();
            let m = compiled
                .machine
                .routines
                .iter()
                .find(|r| r.id == main.id)
                .unwrap();
            let base = compiled.image.entry;
            let mut captures = vec![];
            let mut discarded = 0;
            for block in &main.blocks {
                for (i, op) in block.ops.iter().enumerate() {
                    if let Mir65816Op::Call { result, .. } = op {
                        let Some((id, _)) = result else {
                            discarded += 1;
                            continue;
                        };
                        let home = m.frame.temps[id].stack().unwrap().offset;
                        let suffix = match width {
                            1 => format!("sep #$20\n.a8\nsta {home},s\nrep #$20\n.a16\n"),
                            2 => format!("sta {home},s\n"),
                            3 => format!(
                                "sta {home},s\ntxa\nsep #$20\n.a8\nsta {},s\nrep #$20\n.a16\n",
                                home + 2
                            ),
                            _ => format!("sta {home},s\ntxa\nsta {},s\n", home + 2),
                        };
                        let end = base + m.code.mir_spans[&(block.id, i)].end as u32;
                        let expected = assemble(&suffix, 0x041000);
                        let start = end - expected.len() as u32;
                        let image = &compiled.image;
                        let segment = image.segments.iter().find(|s| s.address == base).unwrap();
                        assert_eq!(
                            &segment.bytes[(start - base) as usize..(end - base) as usize],
                            expected
                        );
                        captures.push((start, end, home));
                    }
                }
            }
            assert_eq!(captures.len(), 2);
            assert_eq!(discarded, 1);
            for value in [
                0u32, 1, 0x80, 0xff, 0x100, 0x8000, 0xffff, 0x10000, 0x800000, 0xffffff,
                0x80000000, 0xffffffff,
            ] {
                let expected = value & (u32::MAX >> (8 * (4 - width)));
                for mask in [0, 4] {
                    let mut h = Harness::new(&compiled.image, &caller(base), mask);
                    h.bus.map(0x041000, &leaf, false);
                    h.bus.ram[0x7100..0x7104].copy_from_slice(&value.to_le_bytes());
                    h.bus.ram[0x7200..0x7220].fill(0xa5);
                    for &(start, end, home) in &captures {
                        reach(&mut h, start);
                        let r = h.cpu.registers();
                        let at = usize::from(r.s) + usize::from(home);
                        let mut stack = h.bus.ram[0x4000..0x6000].to_vec();
                        stack[at - 0x4000..at - 0x4000 + usize::from(width)]
                            .copy_from_slice(&expected.to_le_bytes()[..usize::from(width)]);
                        let reads = h.bus.reads.len();
                        let writes = h.bus.writes.len();
                        reach(&mut h, end);
                        assert_eq!(&h.bus.ram[0x4000..0x6000], stack);
                        let touched: Vec<_> =
                            h.bus.writes[writes..].iter().map(|&(a, _)| a).collect();
                        // Word stores retain ascending byte order on the bus.
                        assert_eq!(
                            touched,
                            (at as u32..at as u32 + u32::from(width)).collect::<Vec<_>>()
                        );
                        assert!(
                            !h.bus.reads[reads..]
                                .iter()
                                .any(|a| (0x2000..0x2040).contains(a))
                        );
                        let after = h.cpu.registers();
                        assert_eq!(
                            (
                                after.s,
                                after.d,
                                after.dbr,
                                after.x,
                                after.y,
                                after.p & 0x3c
                            ),
                            (r.s, r.d, r.dbr, r.x, r.y, r.p & 4)
                        );
                    }
                    h.run();
                    h.guards(mask);
                    for at in [0x7200usize, 0x7210] {
                        assert_eq!(h.bus.value(at as u32, width.into()), expected);
                        assert!(
                            h.bus.ram[at + usize::from(width)..at + 0x10]
                                .iter()
                                .all(|&v| v == 0xa5)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn all_native_widths_round_trip_through_mutable_parameters_and_relocated_calls() {
    for (ty, width) in [
        ("BYTE", 1u8),
        ("CARD", 2),
        ("INT", 2),
        ("ADDRESS", 3),
        ("SIZE", 3),
        ("BYTE POINTER", 3),
        ("LONGCARD", 4),
        ("LONGINT", 4),
    ] {
        let source = format!(
            "{ty} input,first,second\n{ty} FUNC POINTER cb({ty} value)\n{ty} FUNC Echo({ty} value) {ty} saved saved=value value={ty}(0) RETURN(saved)\nPROC Main() first=Echo(input) cb=@Echo second=cb(input) RETURN\n"
        );
        for optimize in [false, true] {
            let bytes = o65::compile(&source, optimize, vec![]);
            for variant in 0..2 {
                let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
                let image = format::relocate(&bytes, &placement).unwrap();
                for value in [0u32, 0x100, 0x8000, 0x800000, 0x89abcdef, u32::MAX] {
                    for mask in [0, 4] {
                        let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                        let at = o65::object(&image, "input") as usize;
                        h.bus.ram[at..at + usize::from(width)]
                            .copy_from_slice(&value.to_le_bytes()[..usize::from(width)]);
                        h.run();
                        h.guards(mask);
                        for name in ["first", "second"] {
                            assert_eq!(
                                h.bus.value(o65::object(&image, name), width.into()),
                                value & (u32::MAX >> (8 * (4 - width)))
                            );
                        }
                        if value == u32::MAX && mask == 0 {
                            o65::record(
                                &format!("call-copies-{}", ty.replace(' ', "-")),
                                optimize,
                                &bytes,
                                &placement,
                                &image,
                                h.cpu.cycles(),
                                None,
                            );
                        }
                    }
                }
            }
        }
    }
}
