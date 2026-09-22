mod support;
use actionc::mir65816::{abi, image::AssemblyImport, image::Image};
use actionc::nir::runtime_symbol_id;
use actionc_vm::native65816::Access;
use support::{context::routine, *};

fn word_caller(image: &Image, calls: &[(&str, bool)]) -> Vec<u8> {
    let mut source = String::new();
    for (index, &(name, argument)) in calls.iter().enumerate() {
        let outgoing = if argument { 3 } else { 1 };
        source.push_str(&format!(
            "tsc\nsec\nsbc #{outgoing}\ntcs\nsep #$20\n.a8\nlda #0\nsta {outgoing},s\nrep #$20\n.a16\n"
        ));
        if argument {
            source.push_str("lda f:$007100\nsta 1,s\n");
        }
        // Capture both result registers before cleanup. Only defined lanes are
        // checked below: X is deliberately unspecified for a word result.
        source.push_str(&format!(
            "ldx #$beef\nldy #$dead\njsl ${:06x}\nsta f:${:06x}\ntxa\nsta f:${:06x}\ntsc\nclc\nadc #{outgoing}\ntcs\n",
            routine(image, name), 0x7200 + index * 4, 0x7202 + index * 4,
        ));
    }
    source.push_str("stp\nnop\n");
    assemble(&source, 0x040000)
}

#[test]
fn independent_caller_checks_word_bits_frames_and_other_result_lanes() {
    for ty in ["CARD", "INT"] {
        let source = format!(
            r#"
BYTE marker
{ty} FUNC Echo({ty} value) RETURN(value)
{ty} FUNC Changed({ty} value) value==+1 RETURN(value)
{ty} FUNC Pick({ty} value)
  marker=BYTE(value)
  IF marker=0 THEN RETURN(value+1) FI
RETURN(value-1)
{ty} FUNC Narrow({ty} value) RETURN({ty}(BYTE(value)))
{ty} FUNC Constant() RETURN({ty}($8000))
{ty} FUNC Small() RETURN({ty}(255))
CARD FUNC ZeroFrame() RETURN(32768)
{ty} FUNC Chain({ty} value)
  {ty} FUNC POINTER cb({ty} x)
  cb=@Echo
RETURN(cb(Changed(value)))
{ty} FUNC Rec({ty} value CARD depth)
  IF depth=0 THEN RETURN(value) FI
RETURN(Rec(value+1,depth-1))
{ty} FUNC Recursive({ty} value) RETURN(Rec(value,3))
BYTE FUNC ByteResult(CARD value) RETURN(BYTE(value))
ADDRESS FUNC AddressResult() RETURN(ADDRESS($ABFFFF))
LONGCARD FUNC LongResult() RETURN(LONGCARD($89ABCDEF))
PROC Main() RETURN
"#
        );
        let calls = [
            ("Echo", true),
            ("Changed", true),
            ("Pick", true),
            ("Narrow", true),
            ("Constant", false),
            ("Small", false),
            ("Chain", true),
            ("Recursive", true),
            ("ByteResult", true),
            ("AddressResult", false),
            ("LongResult", false),
            ("Echo", true),
            ("ZeroFrame", false),
        ];
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            // Exercise the actual parser/lowering with both host newline forms.
            assert_eq!(
                image.to_json().unwrap(),
                compile(&source.replace('\n', "\r\n"), optimize)
                    .to_json()
                    .unwrap()
            );
            assert_eq!(
                image
                    .routines
                    .iter()
                    .find(|r| r.name == "ZeroFrame")
                    .unwrap()
                    .fixed_frame,
                0
            );
            assert!(
                image
                    .routines
                    .iter()
                    .find(|r| r.name == "Echo")
                    .unwrap()
                    .fixed_frame
                    == 0
            );
            let caller = word_caller(&image, &calls);
            for value in [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&value.to_le_bytes());
                    h.run();
                    h.guards(mask);
                    let expected = [
                        u32::from(value),
                        u32::from(value.wrapping_add(1)),
                        u32::from(if value & 255 == 0 {
                            value.wrapping_add(1)
                        } else {
                            value.wrapping_sub(1)
                        }),
                        u32::from(value & 255),
                        0x8000,
                        255,
                        u32::from(value.wrapping_add(1)),
                        u32::from(value.wrapping_add(3)),
                        u32::from(value & 255),
                        0x00abffff,
                        0x89abcdef,
                        u32::from(value),
                        0x8000,
                    ];
                    for (index, expected) in expected.into_iter().enumerate() {
                        assert_eq!(
                            h.bus.value(
                                0x7200 + index as u32 * 4,
                                if matches!(index, 9 | 10) { 4 } else { 2 }
                            ),
                            expected,
                            "{ty}/{optimize}/{value:04x}/{mask}/{}",
                            calls[index].0
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn captured_and_reloaded_words_respect_volatile_aliases_and_clobbering_calls() {
    let source = r#"
MODULE TEST
PUBLIC EXTERNAL PROC Smash()
VOLATILE CARD io=$D000
CARD FUNC VolatileWord() RETURN(io)
CARD FUNC Captured()
  CARD POINTER p
  CARD saved
  p=CARD POINTER($12FFFF) saved=p^
  Smash()
RETURN(saved)
CARD FUNC Reloaded()
  CARD POINTER p
  PROC POINTER cb
  p=CARD POINTER($12FFFF) cb=@Smash
  cb()
RETURN(p^)
PROC Main() RETURN
ENDMODULE
"#;
    let smash = assemble(
        r#"
        sep #$20
        .a8
        lda #$34
        sta f:$12ffff
        lda #$12
        sta f:$130000
        ldx #63
        lda #$a7
    again:
        sta 0,x
        dex
        bpl again
        rep #$20
        .a16
        lda #$9876
        ldx #$beef
        ldy #$dead
        rtl
    "#,
        0x041000,
    );
    for optimize in [false, true] {
        let prepared = prepare(source, optimize);
        let symbol = runtime_symbol_id("TEST.Smash");
        let signature = prepared
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
        let image = Image::from_json(&prepared.compile(&options).unwrap().image.to_json().unwrap())
            .unwrap();
        let caller = word_caller(
            &image,
            &[
                ("VolatileWord", false),
                ("Captured", false),
                ("Reloaded", false),
            ],
        );
        for value in [0u16, 0x8000, 0xffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.map(0x041000, &smash, false);
                h.bus.map(0xd000, &value.to_le_bytes(), true);
                h.bus.watched.extend(0xd000..0xd002);
                let [low, high] = value.to_le_bytes();
                h.bus.map(0x12fffe, &[0xa5, low, high, 0x5a], true);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(0x7200, 2), u32::from(value));
                assert_eq!(h.bus.value(0x7204, 2), u32::from(value));
                assert_eq!(h.bus.value(0x7208, 2), 0x1234);
                assert_eq!(&h.bus.ram[0x12fffe..0x130002], &[0xa5, 0x34, 0x12, 0x5a]);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, address, access)| (address, access))
                        .collect::<Vec<_>>(),
                    [(0xd000, Access::Read), (0xd001, Access::Read)]
                );
                assert!((0x2000..0x2040).all(|address| h.bus.writes.contains(&(address, 0xa7))));
            }
        }
    }
}

#[test]
fn executed_word_return_tails_read_only_the_source_and_restore_the_stack() {
    use actionc_vm::native65816::Inputs;
    let source = "BYTE marker CARD FUNC Echo(CARD value) RETURN(value) CARD FUNC ZeroFrame() RETURN(32768) CARD FUNC FramedLiteral(CARD value) marker=BYTE(value) RETURN(32768) PROC Main() RETURN";
    for optimize in [false, true] {
        let (image, forwarded) = forwarding::compile(source, optimize);
        for name in ["Echo", "ZeroFrame", "FramedLiteral"] {
            let r = image.routines.iter().find(|r| r.name == name).unwrap();
            let stack_source = name == "Echo";
            let load_size = if stack_source { 0 } else { 3 };
            let release_size = if r.fixed_frame == 0 { 1 } else { 9 };
            let load_pc = r.address + r.size - load_size - release_size;
            let outgoing = if name == "ZeroFrame" { 1 } else { 3 };
            let caller = assemble_artifact(
                &format!(
                    "tsc\nsec\nsbc #{outgoing}\ntcs\nsep #$20\n.a8\nlda #0\nsta {outgoing},s\nrep #$20\n.a16\n{}jsl ${:06x}\n.export returned\nreturned:\nsta f:$007200\ntsc\nclc\nadc #{outgoing}\ntcs\nstp\nnop",
                    if outgoing == 3 {
                        "lda f:$007100\nsta 1,s\n"
                    } else {
                        ""
                    },
                    r.address,
                ),
                0x040000,
            );
            let returned = caller.symbols["returned"];
            for value in [0u16, 0x8000, 0xffff] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller.bytes, mask);
                    h.bus.forwarded_words = forwarded.clone();
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&value.to_le_bytes());
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                10_000,
                                |_| Inputs::default(),
                                |cpu| cpu.is_instruction_boundary() && cpu.pc() == r.address
                            )
                            .unwrap()
                    );
                    let entry_cycles = h.cpu.cycles();
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                10_000,
                                |_| Inputs::default(),
                                |cpu| cpu.is_instruction_boundary() && cpu.pc() == load_pc
                            )
                            .unwrap()
                    );
                    let before = h.cpu.registers();
                    assert_eq!(before.p & 0x3c, mask);
                    // Anchor the known tail at an actually reached instruction,
                    // then verify the complete sequence, including its one RTL.
                    let mut expected = if stack_source {
                        let site =
                            forwarding::reached(&h.cpu, &h.bus).expect("typed forwarded return");
                        assert_eq!(site.kind, forwarding::Kind::Return);
                        assert_eq!(before.a, value);
                        vec![]
                    } else {
                        vec![0xa9, 0, 0x80]
                    };
                    if r.fixed_frame != 0 {
                        expected.extend([
                            0xa8,
                            0x3b,
                            0x18,
                            0x69,
                            r.fixed_frame as u8,
                            (r.fixed_frame >> 8) as u8,
                            0x1b,
                            0x98,
                        ]);
                    }
                    expected.push(0x6b);
                    assert_eq!(
                        &h.bus.ram[load_pc as usize..(r.address + r.size) as usize],
                        expected
                    );
                    let read_start = h.bus.reads.len();
                    let write_start = h.bus.writes.len();
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                1_000,
                                |_| Inputs::default(),
                                |cpu| cpu.is_instruction_boundary() && cpu.pc() == returned
                            )
                            .unwrap()
                    );
                    let cycles = h.cpu.cycles() - entry_cycles;
                    let after = h.cpu.registers();
                    assert_eq!(after.a, if stack_source { value } else { 0x8000 });
                    assert_eq!(
                        (after.s, after.d, after.dbr, after.p & 0x3c),
                        (0x5ff0 - outgoing, 0x2000, 0, mask)
                    );
                    assert_eq!(after.x, before.x, "word-return tail need not clear X");
                    assert_eq!(h.bus.writes.len(), write_start);
                    let reads = &h.bus.reads[read_start..];
                    assert!(!reads.iter().any(|a| (0x2000..0x2040).contains(a)));
                    let stack_reads: Vec<_> = reads
                        .iter()
                        .copied()
                        .filter(|a| (0x4000..0x6000).contains(a))
                        .collect();
                    let mut expected_reads = vec![];
                    expected_reads.extend(
                        (1..=3).map(|i| u32::from(before.s) + u32::from(r.fixed_frame) + i),
                    );
                    assert_eq!(stack_reads, expected_reads);
                    if name == "Echo" {
                        assert!(
                            r.size <= 70 && cycles <= 80,
                            "{optimize}: {} bytes, {cycles} cycles",
                            r.size
                        );
                    }
                    if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
                        std::fs::write(std::path::Path::new(&directory).join(format!("word-return-tail-{optimize}-{name}.json")),
                            serde_json::to_vec_pretty(&serde_json::json!({"optimized":optimize,"routine":name,"code_bytes":r.size,"cycles":cycles,"frame":r.fixed_frame,"tail_pc":load_pc,"tail_stack_reads":stack_reads,"tail_dp_reads":0,"tail_writes":0})).unwrap()).unwrap();
                    }
                    h.run();
                    h.guards(mask);
                }
            }
        }
    }
}
