mod support;
use actionc::mir65816::{
    abi,
    image::{AssemblyImport, Image},
};
use actionc::nir::runtime_symbol_id;
use actionc_vm::native65816::{Inputs, Machine};
use support::*;

#[test]
fn indirect_targets_at_zero_and_last_offset_keep_the_target_bank() {
    let source = r#"
MODULE TEST
PUBLIC EXTERNAL PROC EdgeZero()
PUBLIC EXTERNAL PROC EdgeLast()
PROC POINTER cb
CARD count
PROC Main()
 cb=@EdgeZero cb() count==+1
 cb=@EdgeLast cb() count==+1
RETURN
ENDMODULE
"#;
    for optimize in [false, true] {
        let prepared = prepare(source, optimize);
        let mut options = layout();
        for (name, address) in [("TEST.EdgeZero", 0x050000), ("TEST.EdgeLast", 0x06ffff)] {
            let symbol = runtime_symbol_id(name);
            let r = prepared
                .mir
                .routines
                .iter()
                .find(|r| r.entry.external_symbol == Some(symbol))
                .unwrap();
            options.imports.push(AssemblyImport {
                symbol: symbol.0,
                signature: r.signature.0,
                abi: abi::generated::ABI_NAME.into(),
                address,
                size: 1,
                stack_peak: 0,
                checks_stack: true,
                irq_effect: Default::default(),
            });
        }
        let image = Image::from_json(&prepared.compile(&options).unwrap().image.to_json().unwrap())
            .unwrap();
        for mask in [0, 4] {
            let mut h = Harness::new(&image, &caller(image.entry), mask);
            h.bus.map(0x050000, &assemble("rtl\nnop", 0x050000), false);
            h.bus.map(0x06ffff, &[0x6b], false); // one-byte RTL at the bank's final address
            h.bus.map(0x060000, &[0xea], false); // next fetch stays in the same program bank
            h.run();
            h.guards(mask);
            assert_eq!(h.global(&image, "count", 2), 2);
            assert_eq!(h.global(&image, "cb", 3), 0x06ffff);
            assert!(h.bus.reads.contains(&0x050000));
            assert!(h.bus.reads.contains(&0x06ffff));
        }
    }
}

#[test]
fn typed_indirect_results_and_mixed_assembly_arguments_match_direct_calls() {
    for optimize in [false, true] {
        for (ty, value, width) in [
            ("BYTE", "$B7", 1),
            ("INT", "-300", 2),
            ("ADDRESS", "$834567", 3),
            ("SIZE", "$EFABCD", 3),
            ("LONGINT", "LONGINT($89ABCDEF)", 4),
            ("BYTE POINTER", "BYTE POINTER($AB789A)", 3),
        ] {
            let source = format!(
                "{ty} direct,indirect {ty} FUNC POINTER cb({ty} value) {ty} FUNC Echo({ty} value) RETURN(value) PROC Main() direct=Echo({value}) cb=@Echo indirect=cb({value}) RETURN"
            );
            let prepared = prepare(&source, optimize);
            let machine = actionc::mir65816::emit::materialize(&prepared.mir).unwrap();
            let mut options = layout();
            options.code_origin = 0x020000 - machine.routines[0].code.bytes.len() as u32 - 1;
            let image = prepared.compile(&options).unwrap().image;
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                h.run();
                h.guards(mask);
                assert_eq!(
                    h.global(&image, "direct", width),
                    h.global(&image, "indirect", width),
                    "{ty}"
                );
            }
        }
        // The existing independent assembly leaf records literal incoming bytes.
        let source=fixture("interop.act").replace("LONGINT importedResult","LONGINT FUNC POINTER callback(BYTE a CARD b BYTE POINTER p LONGINT c)\nLONGINT importedResult").replace("importedResult=AsmMixed(","callback=@AsmMixed\n  importedResult=callback(");
        let prepared = prepare(&source, optimize);
        let mut options = layout();
        for (name, address, size) in [
            ("TEST.AsmMixed", 0x041000, 0x100),
            ("TEST.AsmEmpty", 0x041100, 0x30),
        ] {
            let symbol = runtime_symbol_id(name);
            let r = prepared
                .mir
                .routines
                .iter()
                .find(|r| r.entry.external_symbol == Some(symbol))
                .unwrap();
            options.imports.push(AssemblyImport {
                symbol: symbol.0,
                signature: r.signature.0,
                abi: abi::generated::ABI_NAME.into(),
                address,
                size,
                stack_peak: 0,
                checks_stack: true,
                irq_effect: Default::default(),
            });
        }
        let image = prepared.compile(&options).unwrap().image;
        let mut leaf_source = String::from("sep #$20\n.a8\n");
        for byte in 0..13 {
            leaf_source.push_str(&format!("lda {},s\nsta ${:06x}\n", 4 + byte, 0x7100 + byte));
        }
        leaf_source.push_str("ldx #63\nlda #$AA\nclobber: sta 0,x\ndex\nbpl clobber\nrep #$20\n.a16\nlda #$CDEF\nldx #$89AB\nrtl\nnop");
        let leaf = assemble(&leaf_source, 0x041000);
        for mask in [0, 4] {
            let mut h = Harness::new(&image, &caller(image.entry), mask);
            h.bus.map(0x041000, &leaf, false);
            h.bus.map(0x041100, &[0x6b, 0xea], false);
            h.run();
            h.guards(mask);
            assert_eq!(h.global(&image, "importedResult", 4), 0x89abcdef);
            assert_eq!(
                &h.bus.ram[0x7100..0x710d],
                &[
                    0x12, 0, 0x56, 0x34, 0x9a, 0x78, 0xab, 0, 0x12, 0xf0, 0xde, 0xbc, 0
                ]
            );
        }
    }
}

#[test]
fn indirect_reservation_faults_before_any_outgoing_or_transfer_write() {
    let image = compile(
        "PROC POINTER cb PROC Empty() RETURN PROC Main() cb=@Empty cb() RETURN",
        false,
    );
    let main = image
        .routines
        .iter()
        .find(|r| r.address == image.entry)
        .unwrap();
    assert_eq!(main.local_stack_peak, main.fixed_frame + 7);
    let mut h = Harness::new(&image, &caller(image.entry), 0);
    let mut r = h.cpu.registers();
    r.s = 0x4019 + main.fixed_frame + 6;
    r.pc = image.entry as u16;
    r.pbr = (image.entry >> 16) as u8;
    h.cpu = Machine::start_at(r);
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                2000,
                |_| Inputs::default(),
                |c| c.is_instruction_boundary() && c.pc() == image.stack_overflow
            )
            .unwrap()
    );
    let r = h.cpu.registers();
    assert_eq!((r.a, r.x, r.s), (7, 0x401f, 0x401f));
    assert!(h.bus.writes.iter().all(|&(a, _)| a > 0x401f || a < 0x4000));
}
