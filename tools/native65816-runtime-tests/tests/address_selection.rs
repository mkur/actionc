mod support;
use actionc::mir65816::o65 as format;
use support::*;

#[test]
fn symbolic_addresses_preserve_all_lanes_in_fixed_and_relocated_images() {
    let source = "TYPE Pair=[BYTE first BYTE last] Pair object BYTE initialized=[42] ADDRESS a=$7100,b=$7103,c=$7106 PROC Main() a=ADDRESS(@initialized) b=ADDRESS(@object.last) c=ADDRESS(@object) RETURN";
    for optimize in [false, true] {
        let p = prepare(source, optimize);
        let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
        for variant in 0..2 {
            let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
            let relocated = format::relocate(&bytes, &placement).unwrap();
            let mut fixed_layout = layout();
            fixed_layout.data_origin = placement.bases[1];
            fixed_layout.zero_fill_origin = Some(placement.bases[2]);
            let fixed = p.compile(&fixed_layout).unwrap().image;
            for mask in [0, 4] {
                for use_o65 in [false, true] {
                    let (mut h, initialized, object) = if use_o65 {
                        (
                            Harness::new_o65(&relocated, &caller(relocated.entry()), mask),
                            o65::object(&relocated, "initialized"),
                            o65::object(&relocated, "object"),
                        )
                    } else {
                        (
                            Harness::new(&fixed, &caller(fixed.entry), mask),
                            context::symbol(&fixed, "initialized"),
                            context::symbol(&fixed, "object"),
                        )
                    };
                    h.bus.ram[0x70ff..0x710a].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.bus.value(0x7100, 3), initialized);
                    assert_eq!(h.bus.value(0x7103, 3), object + 1);
                    assert_eq!(h.bus.value(0x7106, 3), object);
                    assert_eq!((h.bus.ram[0x70ff], h.bus.ram[0x7109]), (0xa5, 0xa5));
                    assert_eq!(h.bus.ram[initialized as usize], 42);
                }
            }
        }
    }
}

#[test]
fn constant_address_chains_relocate_with_bank_carry_and_one_past_fallback() {
    for offset in [0, 1, 255, 256, 299, 300, 65535] {
        let source = format!(
            "BYTE ARRAY bytes(300) ADDRESS result=$7100 PROC Main() result=ADDRESS(@bytes({offset})) RETURN"
        );
        for optimize in [false, true] {
            let bytes = o65::compile(&source, optimize, vec![]);
            for variant in 0..3 {
                let mut placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
                if variant == 2 {
                    placement.bases[2] = 0x1000000 - 300;
                }
                let image = format::relocate(&bytes, &placement).unwrap();
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                    h.bus.ram[0x70ff..0x7104].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        h.bus.value(0x7100, 3),
                        (o65::object(&image, "bytes") + offset) & 0xffffff
                    );
                    assert_eq!((h.bus.ram[0x70ff], h.bus.ram[0x7103]), (0xa5, 0xa5));
                }
            }
        }
    }
}
