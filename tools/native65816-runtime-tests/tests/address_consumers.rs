mod support;
use support::*;

#[test]
fn borrowed_pointer_aliases_preserve_full_values_and_exact_store_extents() {
    let source = "TYPE Packet=[BYTE tag BYTE POINTER link BYTE end] \
        Packet POINTER target=$7100 BYTE POINTER input=$7104 \
        PROC Work(Packet POINTER packet BYTE POINTER ptr) packet.link=BYTE POINTER(ADDRESS(ptr)) RETURN \
        PROC Main() Work(target,input) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let work = image.routines.iter().find(|r| r.name == "Work").unwrap();
        assert_eq!(work.fixed_frame, 0);
        for target in [0x7200u32, 0x12fffe, 0x130000] {
            for ptr in [0u32, 1, 0x123456, 0xffffff] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), mask);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&target.to_le_bytes()[..3]);
                    h.bus.ram[0x7104..0x7107].copy_from_slice(&ptr.to_le_bytes()[..3]);
                    if target >= 0x10000 { h.bus.map(target, &[0xa5;5], true); }
                    else { h.bus.ram[target as usize..target as usize+5].fill(0xa5); }
                    h.run(); h.guards(mask);
                    assert_eq!(h.bus.value(target+1,3),ptr);
                    assert_eq!(h.bus.ram[target as usize],0xa5);
                    assert_eq!(h.bus.ram[target as usize+4],0xa5);
                }
            }
        }
    }
}

#[test]
fn mutable_pointer_reads_remain_snapshots_across_aliasing_writes() {
    let source = "BYTE POINTER shared=$7100 BYTE output=$7104 \
        PROC Work(BYTE POINTER replacement) BYTE POINTER saved \
        saved=shared shared=replacement output=saved^ RETURN \
        PROC Main() Work(BYTE POINTER($7300)) RETURN";
    for optimize in [false,true] {
        let image=compile(source,optimize);
        let mut h=Harness::new(&image,&caller(image.entry),0);
        h.bus.ram[0x7100..0x7103].copy_from_slice(&[0,0x72,0]);
        h.bus.ram[0x7200]=0x35; h.bus.ram[0x7300]=0x89;
        h.run(); h.guards(0);
        assert_eq!(h.bus.ram[0x7104],0x35);
        assert_eq!(h.bus.value(0x7100,3),0x7300);
    }
}
