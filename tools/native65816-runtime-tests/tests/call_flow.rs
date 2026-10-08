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
