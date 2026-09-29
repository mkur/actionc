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


#[test]
fn repeated_field_reads_reuse_addresses_but_observe_writes_and_calls() {
    for body in [
        "RETURN(p.a+p.b)",
        "CARD first first=p.a p.a=17 RETURN(first+p.a)",
        "CARD first first=p.a Mutate(p) RETURN(first+p.a)",
    ] {
        let source=format!("TYPE Packet=[CARD a,b] CARD result=$7100 \
            PROC Mutate(Packet POINTER p) p.a=17 RETURN \
            CARD FUNC Work(Packet POINTER p) {body} \
            PROC Main() result=Work(Packet POINTER($12fffe)) RETURN");
        for optimize in [false,true] {
            let image=compile(&source,optimize);
            let mut h=Harness::new(&image,&caller(image.entry),0);
            h.bus.map(0x12fffe,&[5,0,9,0],true);
            h.run(); h.guards(0);
            assert_eq!(h.bus.value(0x7100,2),if body.contains("17")||body.contains("Mutate") {22} else {14});
            let reads:Vec<_>=h.bus.reads.iter().filter(|a| (0x12fffe..0x130002).contains(*a)).copied().collect();
            assert_eq!(reads,if body=="RETURN(p.a+p.b)" {vec![0x12fffe,0x12ffff,0x130000,0x130001]} else {vec![0x12fffe,0x12ffff,0x12fffe,0x12ffff]});
        }
    }
}

#[test]
fn address_store_and_cached_base_lifetimes_survive_irq_nmi_and_reentry() {
    use actionc_vm::native65816::Inputs;
    use std::collections::BTreeSet;
    use support::context::*;
    let source = r#"MODULE TEST PUBLIC EXTERNAL PROC Yield()
        VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current
        TYPE Source=[BYTE ARRAY padding(3) BYTE last]
        TYPE Job=[Source POINTER item BYTE done ADDRESS result BYTE POINTER peer CARD left,right,sum]
        PROC Fill(Job POINTER work Source POINTER item)
          work.result=ADDRESS(@item.last)
          work.sum=work.left+work.right
        RETURN
        CARD FUNC Dispatch(CARD saved BYTE reason)
          irqAck=1 Fill(Job POINTER($7140),Source POINTER($ffffff))
          IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI
          taskB=saved current=0 RETURN(taskA)
        PROC Task(Job POINTER work)
          Fill(work,work.item) work.done=1
          WHILE work.peer^=0 DO Yield() OD
        RETURN
        PROC Main() RETURN ENDMODULE"#;
    let check=|h:&ContextHarness| {
        h.guards(); assert_eq!(h.bus.value(DONE,2),1);
        for (job,result,sum) in [(0x7100,1,14),(0x7120,0x130002,254),(0x7140,2,18)] {
            assert_eq!(h.bus.value(job+4,3),result);
            assert_eq!(h.bus.value(job+14,2),sum);
        }
    };
    for optimize in [false,true] {
        let mut h=ContextHarness::new(source,optimize,"Task",&[0x7100,0x7120]);
        // Check real newline-sensitive source loading in both representations.
        let crlf=ContextHarness::new(&source.replace('\n',"\r\n"),optimize,"Task",&[0x7100,0x7120]);
        assert_eq!(h.image.to_json().unwrap(),crlf.image.to_json().unwrap());
        for (job,input,peer,left,right) in [
            (0x7100usize,0xfffffeu32,0x7123u32,5u16,9u16),
            (0x7120,0x12ffff,0x7103,255,65535),
            (0x7140,0,0,7,11)] {
            h.bus.ram[job..job+3].copy_from_slice(&input.to_le_bytes()[..3]);
            h.bus.ram[job+7..job+10].copy_from_slice(&peer.to_le_bytes()[..3]);
            h.bus.ram[job+10..job+12].copy_from_slice(&left.to_le_bytes());
            h.bus.ram[job+12..job+14].copy_from_slice(&right.to_le_bytes());
        }
        let start=context::routine(&h.image,"Fill");
        let end=start+h.image.routines.iter().find(|r|r.address==start).unwrap().size;
        let mut seen=BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {break;}
            let r=h.cpu.registers();
            if h.cpu.is_instruction_boundary() && r.p&4==0 && [0x2000,0x2100].contains(&r.d)
                && (start..end).contains(&h.cpu.pc()) && seen.insert((r.d,h.cpu.pc())) {
                let saved_cpu=h.cpu.clone(); let saved_bus=h.bus.clone();
                let mut pending=true;
                for tick in 0..2_000_000 {
                    if h.cpu.is_stopped() {break;}
                    let writes=h.bus.writes.len();
                    h.tick(Inputs {irq:pending,nmi:tick==40,..Default::default()});
                    if h.bus.writes[writes..].iter().any(|&(at,_)|at==IRQ_ACK) {pending=false;}
                }
                check(&h); h.cpu=saved_cpu; h.bus=saved_bus;
            }
            h.tick(Inputs::default());
        }
        check(&h);
        assert!(seen.len()>=40,"only {} selected instruction sites",seen.len());
    }
}
