//! Several independently borrowed private inputs share one checked native call.
mod support;
use actionc::mir65816::{abi, emit::proof, image::AssemblyImport};
use actionc::nir::runtime_symbol_id;
use support::*;
const LEAF: u32 = 0x041000;

#[test]
fn bounded_private_inputs_preserve_complete_arguments_padding_and_fresh_results() {
    // Independent ABI offsets: byte 0, words 2/4, long 6, pointer 10; O=13.
    let mut asm = String::from("sep #$20\n.a8\n");
    for i in 0..13 {
        asm.push_str(&format!("lda {},s\nsta f:${:06x}\n", i + 4, 0x7380 + i));
    }
    asm.push_str("lda #$a7\nldx #63\nclobber: sta $80,x\ndex\nbpl clobber\nrep #$20\n.a16\nlda 6,s\nldx #$beef\nldy #$dead\nsep #$83\nrtl\nnop\n");
    let leaf = assemble(&asm, LEAF);
    let source = "MODULE TEST PUBLIC EXTERNAL CARD FUNC Observe(BYTE a CARD b CARD c LONGCARD wide BYTE POINTER symbol) BYTE symbol,entered CARD output=$7200 CARD FUNC Forward(BYTE a CARD b CARD c LONGCARD wide) entered=1 RETURN(Observe(a,b,c,wide,@symbol)) PROC Main() output=Forward($81,$89ab,$cdef,LONGCARD($87654321)) RETURN ENDMODULE";
    for optimize in [false, true] {
        for extension in [false, true] {
            let mut p = prepare(source, optimize);
            if extension {
                // A verifier-clean narrow literal in a word slot selects the
                // complete reservation/store strategy; the other inputs remain
                // borrowed and share its preflight.
                let r = p
                    .mir
                    .routines
                    .iter_mut()
                    .find(|r| r.name.to_ascii_uppercase().contains("_FORWARD_"))
                    .unwrap();
                for op in r.blocks.iter_mut().flat_map(|b| &mut b.ops) {
                    if let actionc::mir65816::Mir65816Op::Call { args, .. } = op {
                        args[2] = actionc::mir65816::Mir65816Value::U8(0x7b);
                    }
                }
                actionc::mir65816::verify_program(&p.mir).unwrap();
            }
            let external = p
                .mir
                .routines
                .iter()
                .find(|r| r.entry.external_symbol == Some(runtime_symbol_id("TEST.Observe")))
                .unwrap();
            for guarded in [false, true] {
                let mut options = layout();
                options.stack_checks = guarded;
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
                    .find(|r| r.name.to_ascii_uppercase().contains("_FORWARD_"))
                    .unwrap();
                let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                let reads = proof::scalar_reads(r, &m.frame).unwrap();
                let narrow: Vec<_> = reads.iter().filter(|r| r.bytes <= 2).collect();
                assert_eq!(narrow.len(), if extension { 2 } else { 3 });
                let call = r
                    .blocks
                    .iter()
                    .find_map(|b| {
                        b.ops.iter().enumerate().find_map(|(i, op)| {
                            matches!(op, actionc::mir65816::Mir65816Op::Call { .. })
                                .then_some((b.id, i))
                        })
                    })
                    .unwrap();
                let span = &m.code.mir_spans[&call];
                let bytes = &m.code.bytes[span.clone()];
                let decoded = forwarding::instructions(bytes);
                let jsl = *decoded.keys().find(|&&i| bytes[i] == 0x22).unwrap();
                assert_eq!(
                    decoded.keys().any(|&i| i < jsl && bytes[i] == 0x1b),
                    extension,
                    "complete construction strategy"
                );
                for read in narrow {
                    assert!(!m.frame.temps.contains_key(&read.temp));
                    assert!(m.code.mir_spans[&(read.block, read.producer)].is_empty());
                }
                if !extension {
                    assert_eq!(
                        c.image.to_json().unwrap(),
                        prepare(&source.replace('\n', "\r\n"), optimize)
                            .compile(&options)
                            .unwrap()
                            .image
                            .to_json()
                            .unwrap()
                    );
                }
                let symbol = c
                    .image
                    .data
                    .iter()
                    .find(|d| d.name.to_ascii_uppercase().contains("SYMBOL"))
                    .unwrap()
                    .address;
                for mask in [0, 4] {
                    let mut h = Harness::new(&c.image, &caller(c.image.entry), mask);
                    h.bus.map(LEAF, &leaf, false);
                    h.bus.ram[0x71ff..0x7204].fill(0x6d);
                    h.bus.watched.extend(0x71ff..0x7204);
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.bus.value(0x7200, 2), 0x89ab);
                    assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7202]), (0x6d, 0x6d));
                    let mut expected =
                        vec![0x81, 0, 0xab, 0x89, 0xef, 0xcd, 0x21, 0x43, 0x65, 0x87];
                    if extension {
                        expected[4] = 0x7b;
                        expected[5] = 0;
                    }
                    expected.extend_from_slice(&symbol.to_le_bytes()[..3]);
                    assert_eq!(&h.bus.ram[0x7380..0x738d], expected);
                }
            }
        }
    }
}

#[test]
fn terminal_wrappers_have_no_ordinary_scalar_call_read_observations() {
    let source = "MODULE TEST CARD output=$7200 CARD FUNC Echo(BYTE a CARD b BYTE c CARD d) RETURN(b) CARD FUNC Forward(BYTE a CARD b BYTE c CARD d) RETURN(Echo(a,b,c,d)) PROC Main() output=Forward($81,$89ab,$ef,$cdef) RETURN ENDMODULE";
    for optimize in [false, true] {
        let c = prepare(source, optimize).compile(&layout()).unwrap();
        let r = c
            .machine
            .prepared
            .routines
            .iter()
            .find(|r| r.name.to_ascii_uppercase().contains("_FORWARD_"))
            .unwrap();
        let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
        assert_eq!(
            (
                m.frame.extent,
                m.frame.spill_bytes,
                m.frame.peak_below_entry
            ),
            (0, 0, 0)
        );
        assert!(m.frame.temps.is_empty());
        assert!(proof::scalar_reads(r, &m.frame).unwrap().is_empty());
        let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
        h.run();
        h.guards(0);
        assert_eq!(h.bus.value(0x7200, 2), 0x89ab);
    }
}
