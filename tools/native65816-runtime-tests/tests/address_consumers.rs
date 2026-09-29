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


#[test]
fn address_results_store_with_bank_carry_and_exact_three_byte_writes() {
    for offset in [1u32,3,255,256,65535] {
        let mut padding=String::new();
        let mut remaining=offset;
        let mut n=0;
        while remaining!=0 {
            let size=remaining.min(16384);
            padding.push_str(&format!("BYTE ARRAY pad{n}({size}) "));
            remaining-=size; n+=1;
        }
        let source=format!("TYPE Source=[{padding}BYTE last] TYPE Packet=[BYTE tag BYTE POINTER link BYTE end] \
            Source POINTER input=$7100 \
            PROC Work(Packet POINTER target Source POINTER source) target.link=BYTE POINTER(@source.last) RETURN \
            PROC Main() Work(Packet POINTER($7200),input) RETURN");
        for optimize in [false,true] {
            let image=compile(&source,optimize);
            let work=image.routines.iter().find(|r| r.name=="Work").unwrap();
            assert_eq!(work.fixed_frame,0,"{offset}/{optimize}");
            for value in [0u32,0x12fffe,0xabcdef,0xfffffe,0xffffff] {
                for mask in [0,4] {
                    let mut h=Harness::new(&image,&caller(image.entry),mask);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                    h.bus.ram[0x7200..0x7205].fill(0xa5);
                    h.run(); h.guards(mask);
                    assert_eq!(h.bus.value(0x7201,3),(value+offset)&0xffffff);
                    assert_eq!((h.bus.ram[0x7200],h.bus.ram[0x7204]),(0xa5,0xa5));
                    let writes:Vec<_>=h.bus.writes.iter().filter(|(a,_)| (0x7200..0x7205).contains(a)).map(|(a,_)|*a).collect();
                    assert_eq!(writes,vec![0x7201,0x7202,0x7203]);
                }
            }
        }
    }
}

#[test]
fn scalar_expressions_survive_indirect_destination_preparation() {
    for (ty,bits) in [("BYTE",8),("CARD",16)] {
        let source=format!("TYPE Packet=[BYTE tag {ty} value BYTE end] {ty} input=$7100 \
            PROC Work(Packet POINTER target {ty} n) target.value=n+{ty}(7) RETURN \
            PROC Main() Work(Packet POINTER($7200),input) RETURN");
        for optimize in [false,true] {
            let image=compile(&source,optimize);
            for value in [0u32,1,0x7f,0xff,0x8000,0xffff] {
                let mut h=Harness::new(&image,&caller(image.entry),0);
                h.bus.ram[0x7100..0x7102].copy_from_slice(&(value as u16).to_le_bytes());
                h.bus.ram[0x7200..0x7205].fill(0xa5);
                h.run(); h.guards(0);
                // CARD fields are naturally aligned at offset two.
                let at=if bits==8 {0x7201} else {0x7202};
                assert_eq!(h.bus.value(at,bits/8),(value+7)&((1<<bits)-1),"{ty}/{value}/{optimize}");
                assert_eq!(h.bus.ram[0x7200],0xa5);
            }
        }
    }
}
