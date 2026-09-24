mod support;
use actionc::mir65816::{
    Mir65816Terminator, Mir65816Value, abi,
    image::{AssemblyImport, Image},
    o65 as format,
};
use actionc::nir::runtime_symbol_id;
use actionc_vm::native65816::{Access, Inputs, Machine};
use support::{context::routine, o65 as native, *};

fn source(ty: &str) -> String {
    format!(
        "{ty} FUNC Echo({ty} value) RETURN(value)\n{ty} FUNC Incoming({ty} value) RETURN(value)\n{ty} FUNC Mutate({ty} value) value={ty}(LONGCARD(value)+1) RETURN(value)\n{ty} FUNC Direct({ty} value) RETURN(Echo(value))\n{ty} FUNC Indirect({ty} value) {ty} FUNC POINTER cb({ty} arg) cb=@Echo RETURN(cb(value))\n{ty} FUNC Constant() RETURN({ty}($89ABCDEF))\n{ty} FUNC Zero() RETURN({ty}(0))\n{ty} FUNC Narrow(BYTE value) RETURN({ty}(value))\n{ty} FUNC Rec({ty} value CARD depth) IF depth=0 THEN RETURN(value) FI RETURN(Rec({ty}(LONGCARD(value)+1),depth-1))\n{ty} FUNC Recursive({ty} value) RETURN(Rec(value,3))\nPROC Main() RETURN\n"
    )
}
fn prepared(source: &str, optimize: bool) -> actionc::compiler::native65816::Prepared {
    let mut p = prepare(source, optimize);
    // Direct parameter operands are legal MIR. Exercise a real zero-frame return.
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
fn caller(address: u32, bytes: u8) -> String {
    let outgoing = if bytes % 2 == 0 { bytes + 1 } else { bytes };
    let mut s =
        format!("tsc\nsec\nsbc #{outgoing}\ntcs\nsep #$20\n.a8\nlda #0\nsta {outgoing},s\n");
    for i in 0..bytes {
        s.push_str(&format!(
            "lda f:${:06x}\nsta {},s\n",
            0x7100 + u32::from(i),
            i + 1
        ));
    }
    s.push_str(&format!("rep #$20\n.a16\nlda #$d5aa\nldx #$beef\njsl ${address:06x}\n.export returned\nreturned: sta f:$007200\ntxa\nsta f:$007202\ntsc\nclc\nadc #{outgoing}\ntcs\nstp\nnop\n"));
    s
}
const VALUES: &[u32] = &[
    0, 1, 255, 256, 0x7fff, 0x8000, 0xffff, 0x10000, 0x7fffff, 0x800000, 0xffffff, 0x1000000,
    0x7fffffff, 0x80000000, 0x89abcdef, 0xffffffff,
];
const NAMES: &[&str] = &[
    "Echo",
    "Incoming",
    "Mutate",
    "Direct",
    "Indirect",
    "Constant",
    "Zero",
    "Narrow",
    "Recursive",
];
fn expected(name: &str, value: u32, mask: u32) -> u32 {
    (match name {
        "Mutate" => value.wrapping_add(1),
        "Constant" => 0x89abcdef,
        "Zero" => 0,
        "Narrow" => value & 255,
        "Recursive" => value.wrapping_add(3),
        _ => value,
    }) & mask
}
#[test]
fn wide_result_lanes_survive_calls_recursion_and_relocation() {
    for (ty, bytes) in [("ADDRESS", 3), ("LONGCARD", 4), ("LONGINT", 4)] {
        let value_mask = if bytes == 3 { 0xffffff } else { u32::MAX };
        let source = source(ty).replace("ADDRESS($89ABCDEF)", "ADDRESS($ABCDEF)");
        for optimize in [false, true] {
            let p = prepared(&source, optimize);
            let image = p.compile(&layout()).unwrap().image;
            assert_eq!(
                image.to_json().unwrap(),
                prepared(&source.replace('\n', "\r\n"), optimize)
                    .compile(&layout())
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            let object = p.compile_o65(&Default::default()).unwrap().bytes;
            for &name in NAMES {
                let argument = match name {
                    "Constant" | "Zero" => 0,
                    "Narrow" => 1,
                    _ => bytes,
                };
                let fixed = assemble(&caller(routine(&image, name), argument), 0x040000);
                for variant in 0..3 {
                    let placement =
                        native::placement(&object, variant % 2, vec![native::fault(variant % 2)]);
                    let moved = format::relocate(&object, &placement).unwrap();
                    let relocated =
                        assemble(&caller(native::routine(&moved, name), argument), 0x040000);
                    for &value in VALUES {
                        for mask in [0, 4] {
                            let mut h = if variant == 0 {
                                Harness::new(&image, &fixed, mask)
                            } else {
                                Harness::new_o65(&moved, &relocated, mask)
                            };
                            let input = value & value_mask;
                            h.bus.ram[0x7100..0x7104].copy_from_slice(&input.to_le_bytes());
                            h.run();
                            h.guards(mask);
                            assert_eq!(
                                h.bus.value(0x7200, 4),
                                expected(name, input, value_mask),
                                "{ty}/{name}/{value:08x}/{optimize}/{variant}/{mask}"
                            );
                            if variant != 0 && value == 0x89abcdef {
                                native::record(
                                    &format!("wide-return-{ty}-{name}"),
                                    optimize,
                                    &object,
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
fn wide_return_tails_match_ca65_touch_only_private_bytes_and_restore_both_lanes() {
    for (ty, bytes) in [("ADDRESS", 3), ("LONGCARD", 4)] {
        let source = source(ty).replace("ADDRESS($89ABCDEF)", "ADDRESS($ABCDEF)");
        for optimize in [false, true] {
            let p = prepared(&source, optimize);
            let c = p.compile(&layout()).unwrap();
            let image = c.image;
            for &name in &NAMES[..5] {
                let r = p.mir.routines.iter().find(|r| r.name == name).unwrap();
                let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                let block = r.blocks.last().unwrap();
                let span = &m.code.mir_spans[&(block.id, block.ops.len())];
                let Mir65816Terminator::Return {
                    value: Some(value), ..
                } = &block.terminator
                else {
                    panic!()
                };
                let at = match value {
                    Mir65816Value::Temp(id, _) => m.frame.temps[id].slot().offset,
                    Mir65816Value::Param(_) => {
                        assert_eq!(m.frame.extent, 0);
                        4
                    }
                    _ => panic!(),
                };
                let restore = m.code.bytes[span.start..].starts_with(&[0xc2, 0x20]);
                let mut asm = if restore {
                    String::from("rep #$20\n")
                } else {
                    String::new()
                };
                asm.push_str(&format!(
                    "lda {},s\n{}tax\nlda {at},s\n",
                    at + u16::from(bytes) - 2,
                    if bytes == 3 { "xba\nand #$00ff\n" } else { "" }
                ));
                if m.frame.extent != 0 {
                    asm.push_str(&format!(
                        "tay\ntsc\nclc\nadc #{}\ntcs\ntya\n",
                        m.frame.extent
                    ));
                }
                asm.push_str("rtl\n");
                assert_eq!(&m.code.bytes[span.clone()], assemble(&asm, 0x050000));
                let addr = routine(&image, name);
                let caller = assemble_artifact(&caller(addr, bytes), 0x040000);
                for value in [0, 0x8000, 0xabcdef, 0xffffffff] {
                    let value = value & if bytes == 3 { 0xffffff } else { u32::MAX };
                    for mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller.bytes, mask);
                        h.bus.ram[0x7100..0x7104].copy_from_slice(&value.to_le_bytes());
                        reach(
                            &mut h,
                            addr + span.start as u32 + if restore { 2 } else { 0 },
                        );
                        let mut before = h.cpu.registers();
                        assert_eq!(before.p & 0x3c, mask);
                        before.a = 0xd5aa;
                        before.x = 0xbeef;
                        h.cpu = Machine::start_at(before);
                        let reads = h.bus.reads.len();
                        let writes = h.bus.writes.len();
                        reach(&mut h, caller.symbols["returned"]);
                        let after = h.cpu.registers();
                        let value =
                            expected(name, value, if bytes == 3 { 0xffffff } else { u32::MAX });
                        let outgoing = if bytes == 3 { 3 } else { 5 };
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
                                value as u16,
                                (value >> 16) as u16,
                                0x5ff0 - outgoing,
                                0x2000,
                                0,
                                mask
                            )
                        );
                        assert_eq!(writes, h.bus.writes.len());
                        let accesses = &h.bus.reads[reads..];
                        assert!(!accesses.iter().any(|a| (0x2000..0x2040).contains(a)));
                        let stack: Vec<_> = accesses
                            .iter()
                            .copied()
                            .filter(|a| (0x4000..0x6000).contains(a))
                            .collect();
                        let base = u32::from(before.s) + u32::from(at);
                        let mut expected = vec![
                            base + u32::from(bytes) - 2,
                            base + u32::from(bytes) - 1,
                            base,
                            base + 1,
                        ];
                        expected.extend(
                            (1..=3).map(|i| u32::from(before.s) + u32::from(m.frame.extent) + i),
                        );
                        assert_eq!(stack, expected);
                        h.run();
                        h.guards(mask);
                    }
                }
            }
        }
    }
}
#[test]
fn captured_wide_values_preserve_volatile_reads_and_alias_order_across_clobbers() {
    for (ty, bytes) in [("ADDRESS", 3), ("LONGCARD", 4)] {
        let source = format!(
            "MODULE TEST\nPUBLIC EXTERNAL PROC Smash()\nVOLATILE {ty} io=$D000\n{ty} FUNC VolatileValue() RETURN(io)\n{ty} FUNC Captured() {ty} POINTER p {ty} saved p={ty} POINTER($12FFFF) saved=p^ Smash() RETURN(saved)\n{ty} FUNC Reloaded() {ty} POINTER p PROC POINTER cb p={ty} POINTER($12FFFF) cb=@Smash cb() RETURN(p^)\nPROC Main() RETURN\nENDMODULE\n"
        );
        let smash = assemble(
            "sep #$20\n.a8\nlda #$78\nsta f:$12ffff\nlda #$56\nsta f:$130000\nlda #$34\nsta f:$130001\nlda #$12\nsta f:$130002\nldx #63\nlda #$a7\nagain: sta 0,x\ndex\nbpl again\nrep #$20\n.a16\nlda #$9876\nldx #$beef\nldy #$dead\nrtl\nnop",
            0x041000,
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
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
            for (name, result) in [
                ("VolatileValue", 0x89abcdef),
                ("Captured", 0x89abcdef),
                ("Reloaded", 0x12345678),
            ] {
                let caller = assemble(&caller(routine(&image, name), 0), 0x040000);
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.map(0x041000, &smash, false);
                    h.bus
                        .map(0xd000, &0x89abcdefu32.to_le_bytes()[..bytes], true);
                    h.bus.watched.extend(0xd000..0xd000 + bytes as u32);
                    h.bus
                        .map(0x12fffe, &[0xa5, 0xef, 0xcd, 0xab, 0x89, 0x5a], true);
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        h.bus.value(0x7200, 4),
                        result & if bytes == 3 { 0xffffff } else { u32::MAX }
                    );
                    let accesses: Vec<_> = h.bus.trace.iter().map(|&(_, a, op)| (a, op)).collect();
                    let expected: Vec<_> = if name == "VolatileValue" {
                        (0xd000..0xd000 + bytes as u32)
                            .map(|a| (a, Access::Read))
                            .collect()
                    } else {
                        vec![]
                    };
                    assert_eq!(accesses, expected);
                    if name != "VolatileValue" {
                        assert!((0x2000..0x2040).all(|a| h.bus.writes.contains(&(a, 0xa7))));
                    }
                }
            }
        }
    }
}
