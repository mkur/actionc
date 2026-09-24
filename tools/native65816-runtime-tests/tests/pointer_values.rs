mod support;
use support::*;

#[test]
fn captured_pointer_casts_preserve_every_lane_and_mutable_parameter_homes() {
    let source = "ADDRESS input=$7100 ADDRESS ARRAY output=$7200 \
        ADDRESS FUNC Cast(ADDRESS value) BYTE POINTER p \
        p=BYTE POINTER(value) RETURN(ADDRESS(p)) \
        ADDRESS FUNC Mutate(ADDRESS value) BYTE POINTER p \
        value=ADDRESS(BYTE POINTER(value)) p=BYTE POINTER(value) RETURN(ADDRESS(p)) \
        PROC Main() output(0)=Cast(input) output(1)=Mutate(input) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let caller = caller(image.entry);
        for value in [0u32, 0xffffff, 0x12ffff, 0x130000, 0xabcdef]
            .into_iter()
            .chain((0..24).map(|bit| 1 << bit))
        {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                h.bus.ram[0x71ff..0x7207].fill(0xa5);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(0x7200, 3), value);
                assert_eq!(h.bus.value(0x7203, 3), value);
                assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7206]), (0xa5, 0xa5));
            }
        }
    }
}

#[test]
fn zero_offset_field_addresses_preserve_all_bits_without_dereferencing() {
    let source = "TYPE Box=[BYTE first CARD second] \
        Box POINTER input=$7100 ADDRESS output=$7200 \
        ADDRESS FUNC Work(Box POINTER p) RETURN(ADDRESS(@p.first)) \
        PROC Main() output=Work(input) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let caller = caller(image.entry);
        for value in [0u32, 0xffffff, 0x12ffff, 0x130000, 0xabcdef]
            .into_iter()
            .chain((0..24).map(|bit| 1 << bit))
        {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                h.bus.ram[0x71ff..0x7204].fill(0xa5);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(0x7200, 3), value);
                assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7203]), (0xa5, 0xa5));
            }
        }
    }
}
