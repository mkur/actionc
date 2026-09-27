mod support;
use actionc::mir65816::{
    Mir65816Terminator, Mir65816Value, abi,
    image::{AssemblyImport, Image},
    o65 as format,
};
use actionc::nir::runtime_symbol_id;
use actionc_vm::native65816::{Access, Inputs, Machine};
use support::{context::routine, o65 as native, *};

const SOURCE: &str = "BYTE FUNC Echo(BYTE value) RETURN(value)\nBYTE FUNC Incoming(BYTE value) RETURN(value)\nBYTE FUNC Mutate(BYTE value) value==+1 RETURN(value)\nBYTE FUNC Direct(BYTE value) RETURN(Echo(value))\nBYTE FUNC Indirect(BYTE value) BYTE FUNC POINTER cb(BYTE arg) cb=@Echo RETURN(cb(value))\nPROC Main() RETURN\n";

fn prepared(source: &str, optimize: bool) -> actionc::compiler::native65816::Prepared {
    let mut p = prepare(source, optimize);
    // Exercise the legal direct parameter operand and real zero-frame emission;
    // ordinary source lowering currently captures BYTE parameters in temps.
    let r = p
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Incoming")
        .unwrap();
    assert_eq!(r.blocks.len(), 1);
    assert!(r.frame.parameters[0].frame_object.is_none());
    r.blocks[0].ops.clear();
    r.temps.clear();
    let Mir65816Terminator::Return { value, .. } = &mut r.blocks[0].terminator else {
        panic!()
    };
    *value = Some(Mir65816Value::Param(r.frame.parameters[0].param));
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
}

fn all_values_caller(address: u32) -> Vec<u8> {
    let mut source = String::new();
    for value in 0..=255 {
        source.push_str(&format!("tsc\nsec\nsbc #1\ntcs\nsep #$20\n.a8\nlda #{value}\nsta 1,s\nrep #$20\n.a16\nlda #$a5ff\nldx #$beef\njsl ${address:06x}\nsta f:${:06x}\ntsc\nclc\nadc #1\ntcs\n",0x7200+value*2));
    }
    source.push_str("stp\nnop\n");
    assemble(&source, 0x040000)
}

fn check_values(h: &mut Harness, mask: u8, mutate: bool) {
    h.run();
    h.guards(mask);
    for value in 0..=255 {
        assert_eq!(
            h.bus.value(0x7200 + value * 2, 2),
            (value + u32::from(mutate)) & 255
        );
    }
}

#[test]
fn all_byte_values_survive_direct_indirect_mutable_and_relocated_returns() {
    for optimize in [false, true] {
        let p = prepared(SOURCE, optimize);
        let image =
            Image::from_json(&p.compile(&layout()).unwrap().image.to_json().unwrap()).unwrap();
        assert_eq!(
            image.to_json().unwrap(),
            prepared(&SOURCE.replace('\n', "\r\n"), optimize)
                .compile(&layout())
                .unwrap()
                .image
                .to_json()
                .unwrap()
        );
        let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
        for name in ["Echo", "Incoming", "Mutate", "Direct", "Indirect"] {
            let caller = all_values_caller(routine(&image, name));
            for mask in [0, 4] {
                check_values(
                    &mut Harness::new(&image, &caller, mask),
                    mask,
                    name == "Mutate",
                );
            }
            for variant in 0..2 {
                let placement = native::placement(&bytes, variant, vec![native::fault(variant)]);
                let moved = format::relocate(&bytes, &placement).unwrap();
                let caller = all_values_caller(native::routine(&moved, name));
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&moved, &caller, mask);
                    check_values(&mut h, mask, name == "Mutate");
                    native::record(
                        &format!("captured-byte-{name}"),
                        optimize,
                        &bytes,
                        &placement,
                        &moved,
                        h.cpu.cycles(),
                        None,
                    );
                }
            }
        }
    }
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
fn captured_return_tails_match_ca65_clear_hidden_b_and_read_no_neighbor_or_scratch() {
    for optimize in [false, true] {
        let p = prepared(SOURCE, optimize);
        let c = p.compile(&layout()).unwrap();
        let image = Image::from_json(&c.image.to_json().unwrap()).unwrap();
        // Direct's immediate result now stays in A; call_returns checks that
        // path's exact tail and absence of private result traffic.
        for name in ["Echo", "Incoming", "Mutate", "Indirect"] {
            let r = p.mir.routines.iter().find(|r| r.name == name).unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let ir = image.routines.iter().find(|r| r.name == name).unwrap();
            let block = r.blocks.last().unwrap();
            let span = &m.code.mir_spans[&(block.id, block.ops.len())];
            let Mir65816Terminator::Return {
                value: Some(value), ..
            } = &block.terminator
            else {
                panic!()
            };
            let source = match value {
                Mir65816Value::Temp(id, _) => m.frame.temps[id].slot().offset,
                Mir65816Value::Param(_) => {
                    assert_eq!(m.frame.extent, 0);
                    4
                }
                _ => panic!(),
            };
            // Retain the existing MIR terminal-boundary A16 restoration, then
            // select A8 for the exact-byte read. Only return preparation changes.
            let restore = m.code.bytes[span.start..].starts_with(&[0xc2, 0x20]);
            let mut asm = if restore {
                String::from("rep #$20\n")
            } else {
                String::new()
            };
            asm.push_str(&format!(
                "sep #$20\n.a8\nlda {source},s\nrep #$20\n.a16\nand #$00ff\n"
            ));
            if m.frame.extent != 0 {
                asm.push_str(&format!(
                    "tay\ntsc\nclc\nadc #{}\ntcs\ntya\n",
                    m.frame.extent
                ));
            }
            asm.push_str("rtl\n");
            assert_eq!(&m.code.bytes[span.clone()], assemble(&asm, 0x050000));
            let tail = ir.address + span.start as u32 + if restore { 4 } else { 2 };
            let caller = assemble_artifact(
                &format!(
                    "tsc\nsec\nsbc #1\ntcs\nsep #$20\n.a8\nlda f:$007100\nsta 1,s\nrep #$20\n.a16\njsl ${:06x}\n.export returned\nreturned: sta f:$007200\ntsc\nclc\nadc #1\ntcs\nstp\nnop",
                    ir.address
                ),
                0x040000,
            );
            for value in [0u8, 1, 127, 128, 255] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller.bytes, mask);
                    h.bus.ram[0x7100] = value;
                    reach(&mut h, tail);
                    let mut before = h.cpu.registers();
                    assert_eq!(before.p & 0x3c, mask | 0x20);
                    before.a = 0xd5aa;
                    before.x = 0xbeef;
                    h.cpu = Machine::start_at(before);
                    let reads = h.bus.reads.len();
                    let writes = h.bus.writes.len();
                    reach(&mut h, caller.symbols["returned"]);
                    let after = h.cpu.registers();
                    let expected = if name == "Mutate" {
                        value.wrapping_add(1)
                    } else {
                        value
                    };
                    assert_eq!(
                        (
                            after.a,
                            after.x,
                            after.s,
                            after.d,
                            after.dbr,
                            after.p & 0x3c
                        ),
                        (u16::from(expected), 0xbeef, 0x5fef, 0x2000, 0, mask)
                    );
                    assert_eq!(h.bus.writes.len(), writes);
                    let accessed = &h.bus.reads[reads..];
                    assert!(!accessed.iter().any(|a| (0x2000..0x2040).contains(a)));
                    let stack: Vec<_> = accessed
                        .iter()
                        .copied()
                        .filter(|a| (0x4000..0x6000).contains(a))
                        .collect();
                    let mut expected_reads = vec![u32::from(before.s) + u32::from(source)];
                    expected_reads.extend(
                        (1..=3).map(|i| u32::from(before.s) + u32::from(m.frame.extent) + i),
                    );
                    assert_eq!(stack, expected_reads);
                    h.run();
                    h.guards(mask);
                }
            }
        }
    }
}

#[test]
fn captured_bytes_preserve_volatile_alias_order_across_full_scratch_clobbers() {
    let source = "MODULE TEST\nPUBLIC EXTERNAL PROC Smash()\nVOLATILE BYTE io=$D000\nBYTE FUNC VolatileByte() RETURN(io)\nBYTE FUNC Captured() BYTE POINTER p BYTE saved p=BYTE POINTER($12FFFF) saved=p^ Smash() RETURN(saved)\nBYTE FUNC Reloaded() BYTE POINTER p PROC POINTER cb p=BYTE POINTER($12FFFF) cb=@Smash cb() RETURN(p^)\nPROC Main() RETURN\nENDMODULE\n";
    let smash = assemble(
        "sep #$20\n.a8\nlda #$34\nsta f:$12ffff\nldx #63\nlda #$a7\nagain: sta 0,x\ndex\nbpl again\nrep #$20\n.a16\nlda #$9876\nldx #$beef\nldy #$dead\nrtl\nnop",
        0x041000,
    );
    for optimize in [false, true] {
        let p = prepare(source, optimize);
        let symbol = runtime_symbol_id("TEST.Smash");
        let signature = p
            .mir
            .routines
            .iter()
            .find(|r| r.entry.external_symbol == Some(symbol))
            .unwrap()
            .signature
            .0;
        let mut options = layout();
        options.imports.push(AssemblyImport {
            symbol: symbol.0,
            signature,
            abi: abi::generated::ABI_NAME.into(),
            address: 0x041000,
            size: smash.len() as u32,
            stack_peak: 0,
            checks_stack: true,
            irq_effect: Default::default(),
        });
        let image =
            Image::from_json(&p.compile(&options).unwrap().image.to_json().unwrap()).unwrap();
        let mut asm = String::new();
        for (i, name) in ["VolatileByte", "Captured", "Reloaded"].iter().enumerate() {
            asm.push_str(&format!("tsc\nsec\nsbc #1\ntcs\nsep #$20\n.a8\nlda #0\nsta 1,s\nrep #$20\n.a16\njsl ${:06x}\nsta f:${:06x}\ntsc\nclc\nadc #1\ntcs\n",routine(&image,name),0x7200+2*i));
        }
        asm.push_str("stp\nnop");
        let caller = assemble(&asm, 0x040000);
        for value in [0u8, 128, 255] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.map(0x041000, &smash, false);
                h.bus.map(0xd000, &[value], true);
                h.bus.watched.insert(0xd000);
                h.bus.map(0x12fffe, &[0xa5, value, 0x5a], true);
                h.run();
                h.guards(mask);
                assert_eq!(
                    (
                        h.bus.value(0x7200, 2),
                        h.bus.value(0x7202, 2),
                        h.bus.value(0x7204, 2)
                    ),
                    (u32::from(value), u32::from(value), 0x34)
                );
                assert_eq!(&h.bus.ram[0x12fffe..0x130001], &[0xa5, 0x34, 0x5a]);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, a, op)| (a, op))
                        .collect::<Vec<_>>(),
                    [(0xd000, Access::Read)]
                );
                assert!((0x2000..0x2040).all(|a| h.bus.writes.contains(&(a, 0xa7))));
            }
        }
    }
}
