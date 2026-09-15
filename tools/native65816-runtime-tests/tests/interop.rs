mod support;
use actionc::{
    mir65816::{
        abi,
        image::{AssemblyImport, Image},
    },
    nir::runtime_symbol_id,
};
use support::*;

#[test]
fn independent_assembly_calls_action_and_action_calls_assembly() {
    for optimize in [false, true] {
        let prepared = prepare(&fixture("interop.act"), optimize);
        let mut options = layout();
        for (name, address, size) in [
            ("TEST.AsmMixed", 0x041000, 0x100),
            ("TEST.AsmEmpty", 0x041100, 0x30),
        ] {
            let symbol = runtime_symbol_id(name);
            let routine = prepared
                .mir
                .routines
                .iter()
                .find(|r| r.entry.external_symbol == Some(symbol))
                .unwrap_or_else(|| panic!("interface {name}: {:?}", prepared.mir.routines));
            options.imports.push(AssemblyImport {
                symbol: symbol.0,
                signature: routine.signature.0,
                abi: abi::generated::ABI_NAME.into(),
                address,
                size,
                stack_peak: 0,
                checks_stack: true,
            });
        }
        let compiled = prepared.compile(&options).unwrap();
        let image = Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
        let mut defines = format!("ACTION_MAIN = ${:06x}\n", image.entry);
        for (name, label) in [
            ("ActionMixed", "ACTION_MIXED"),
            ("EchoByte", "ECHO_BYTE"),
            ("EchoWord", "ECHO_WORD"),
            ("EchoAddress", "ECHO_ADDRESS"),
            ("EchoSize", "ECHO_SIZE"),
            ("EchoLong", "ECHO_LONG"),
            ("EchoPointer", "ECHO_POINTER"),
        ] {
            let routine = image
                .routines
                .iter()
                .find(|r| {
                    r.name
                        .to_ascii_uppercase()
                        .contains(&name.to_ascii_uppercase())
                })
                .unwrap();
            defines.push_str(&format!("{label} = ${:06x}\n", routine.address));
        }
        let caller = assemble(&(defines + &fixture("interop.s")), 0x040000);
        for mask in [0, 4] {
            let mut h = Harness::new(&image, &caller, mask);
            h.bus.map(0xab789a, &[0x34], true);
            h.run();
            h.guards(mask);
            assert_eq!(
                &h.bus.ram[0x7100..0x710d],
                &[
                    0x12, 0, 0x56, 0x34, 0x9a, 0x78, 0xab, 0, 0x12, 0xf0, 0xde, 0xbc, 0
                ]
            );
            assert_eq!(
                h.bus.value(0x7000, 4),
                0xbcdef012u32.wrapping_add(0x12 + 0x3456 + 0x34)
            );
            assert_eq!(h.bus.value(0x7004, 2), 0x00b7);
            assert_eq!(h.bus.value(0x7006, 2), 0xfedc);
            assert_eq!(h.bus.value(0x7008, 4), 0x00834567);
            assert_eq!(h.bus.value(0x700c, 4), 0x00efabcd);
            assert_eq!(h.bus.value(0x7010, 4), 0x00ab789a);
            assert_eq!(h.bus.value(0x7014, 4), 0x89abcdef);
            assert_eq!(h.bus.value(0x7200, 3), 1);
            let result = image
                .data
                .iter()
                .find(|d| d.name.to_ascii_uppercase().contains("IMPORTEDRESULT"))
                .unwrap();
            assert_eq!(h.bus.value(result.address, 4), 0x89abcdef);
        }
        let mut wrong = options.clone();
        wrong.imports[0].signature ^= 1;
        assert!(
            prepared
                .compile(&wrong)
                .unwrap_err()
                .to_string()
                .contains("signature mismatch")
        );
    }
}
