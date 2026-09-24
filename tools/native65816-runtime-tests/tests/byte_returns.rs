mod support;
use actionc::mir65816::o65 as format;
use actionc_vm::native65816::Inputs;
use support::{context::routine, o65 as native, *};

fn constant_source() -> String {
    let mut source = String::new();
    for value in 0..=255 {
        source.push_str(&format!("BYTE FUNC Constant{value}() RETURN({value})\n"));
    }
    source.push_str("PROC Main() RETURN\n");
    source
}

fn constant_caller(address: impl Fn(&str) -> u32) -> Vec<u8> {
    let mut source = String::new();
    for value in 0..=255 {
        // Ordinary zero-argument ABI padding. Read both A bytes before cleanup;
        // X is deliberately poisoned but is not part of the BYTE result.
        source.push_str(&format!(
            "tsc\nsec\nsbc #1\ntcs\nsep #$20\n.a8\nlda #0\nsta 1,s\nrep #$20\n.a16\nlda #$a5ff\nldx #$beef\njsl ${:06x}\nsta f:${:06x}\ntsc\nclc\nadc #1\ntcs\n",
            address(&format!("Constant{value}")), 0x7200 + value * 2,
        ));
    }
    source.push_str("stp\nnop\n");
    assemble(&source, 0x040000)
}

fn check_constants(h: &mut Harness, mask: u8) {
    h.run();
    h.guards(mask);
    for value in 0..=255 {
        assert_eq!(h.bus.value(0x7200 + value * 2, 2), value);
    }
    assert!(
        h.bus
            .writes
            .iter()
            .all(|&(a, _)| !(0x2000..0x2040).contains(&a))
    );
}

#[test]
fn every_byte_constant_is_zero_extended_in_fixed_and_relocated_images() {
    let source = constant_source();
    for optimize in [false, true] {
        let image = compile(&source, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&source.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let caller = constant_caller(|name| routine(&image, name));
        for mask in [0, 4] {
            check_constants(&mut Harness::new(&image, &caller, mask), mask);
        }
        let bytes = native::compile(&source, optimize, vec![]);
        for variant in 0..2 {
            let placement = native::placement(&bytes, variant, vec![native::fault(variant)]);
            let moved = format::relocate(&bytes, &placement).unwrap();
            let caller = constant_caller(|name| native::routine(&moved, name));
            for mask in [0, 4] {
                let mut h = Harness::new_o65(&moved, &caller, mask);
                check_constants(&mut h, mask);
                native::record(
                    "byte-constants",
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

#[test]
fn byte_constant_tails_preserve_frames_and_captured_returns_keep_their_values() {
    let source = r#"
BYTE marker
BYTE FUNC ZeroFrame() RETURN(0)
BYTE FUNC Framed(CARD value) marker=BYTE(value) RETURN(255)
BYTE FUNC Pick(CARD value)
  marker=BYTE(value)
  IF marker=0 THEN RETURN(0) FI
RETURN(128)
BYTE FUNC Captured(CARD value) RETURN(BYTE(value))
BYTE FUNC Chain(CARD value) RETURN(Framed(value))
BYTE FUNC Indirect(CARD value)
  BYTE FUNC POINTER cb(CARD arg)
  cb=@Framed
RETURN(cb(value))
PROC Main() RETURN
"#;
    for optimize in [false, true] {
        let image = compile(source, optimize);
        for (name, constant) in [
            ("ZeroFrame", Some(0)),
            ("Framed", Some(255)),
            ("Pick", Some(128)),
            ("Captured", None),
            ("Chain", None),
            ("Indirect", None),
        ] {
            let r = image.routines.iter().find(|r| r.name == name).unwrap();
            let outgoing = if name == "ZeroFrame" { 1 } else { 3 };
            if constant.is_some() {
                assert_eq!(r.fixed_frame == 0, name == "ZeroFrame");
            }
            let caller = assemble_artifact(
                &format!(
                    "tsc\nsec\nsbc #{outgoing}\ntcs\nsep #$20\n.a8\nlda #0\nsta {outgoing},s\nrep #$20\n.a16\n{}ldx #$beef\njsl ${:06x}\n.export returned\nreturned:\nsta f:$007200\ntsc\nclc\nadc #{outgoing}\ntcs\nstp\nnop\n",
                    if outgoing == 3 {
                        "lda f:$007100\nsta 1,s\n"
                    } else {
                        ""
                    },
                    r.address,
                ),
                0x040000,
            );
            for value in [0u16, 1, 127, 128, 255, 0xffff] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller.bytes, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&value.to_le_bytes());
                    if let Some(constant) = constant.filter(|_| name != "Pick" || value != 0) {
                        let mut tail = format!("lda #{constant}\n");
                        if r.fixed_frame != 0 {
                            tail.push_str(&format!(
                                "tay\ntsc\nclc\nadc #{}\ntcs\ntya\n",
                                r.fixed_frame
                            ));
                        }
                        tail.push_str("rtl\n");
                        let expected = assemble(&tail, 0x050000);
                        let tail_pc = r.address + r.size - expected.len() as u32;
                        assert!(
                            h.cpu
                                .run_until(
                                    &mut h.bus,
                                    100_000,
                                    |_| Inputs::default(),
                                    |cpu| cpu.is_instruction_boundary() && cpu.pc() == tail_pc
                                )
                                .unwrap()
                        );
                        assert_eq!(
                            &h.bus.ram[tail_pc as usize..(r.address + r.size) as usize],
                            expected
                        );
                        let before = h.cpu.registers();
                        assert_eq!(before.p & 0x3c, mask);
                        let read_start = h.bus.reads.len();
                        let write_start = h.bus.writes.len();
                        assert!(
                            h.cpu
                                .run_until(
                                    &mut h.bus,
                                    1_000,
                                    |_| Inputs::default(),
                                    |cpu| cpu.is_instruction_boundary()
                                        && cpu.pc() == caller.symbols["returned"]
                                )
                                .unwrap()
                        );
                        let after = h.cpu.registers();
                        assert_eq!(
                            (after.a, after.s, after.d, after.dbr, after.p & 0x3c),
                            (constant, 0x5ff0 - outgoing, 0x2000, 0, mask)
                        );
                        assert_eq!(after.x, before.x);
                        assert_eq!(h.bus.writes.len(), write_start);
                        let reads = &h.bus.reads[read_start..];
                        assert!(reads.iter().all(|a| !(0x2000..0x2040).contains(a)));
                        let stack_reads: Vec<_> = reads
                            .iter()
                            .copied()
                            .filter(|a| (0x4000..0x6000).contains(a))
                            .collect();
                        assert_eq!(
                            stack_reads,
                            (1..=3)
                                .map(|i| u32::from(before.s) + u32::from(r.fixed_frame) + i)
                                .collect::<Vec<_>>()
                        );
                    }
                    h.run();
                    h.guards(mask);
                    let expected = match name {
                        "ZeroFrame" => 0,
                        "Pick" => {
                            if value == 0 {
                                0
                            } else {
                                128
                            }
                        }
                        "Captured" => value & 255,
                        _ => 255,
                    };
                    assert_eq!(
                        h.bus.value(0x7200, 2),
                        u32::from(expected),
                        "{optimize}/{name}/{value}/{mask}"
                    );
                }
            }
        }
    }
}
