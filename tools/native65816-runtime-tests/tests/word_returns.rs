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
                    > 0
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
