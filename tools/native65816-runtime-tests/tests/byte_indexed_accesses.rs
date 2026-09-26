mod support;
use actionc::mir65816::{Mir65816Op, o65 as format};
use actionc::target::ByteOffset;
use actionc_vm::native65816::Access;
use support::*;

fn prepared(
    source: &str,
    optimize: bool,
    displacement: u32,
) -> actionc::compiler::native65816::Prepared {
    let mut p = prepare(source, optimize);
    for op in p
        .mir
        .routines
        .iter_mut()
        .flat_map(|r| &mut r.blocks)
        .flat_map(|b| &mut b.ops)
    {
        match op {
            Mir65816Op::Load { address, .. } | Mir65816Op::Store { address, .. }
                if address.index.is_some() =>
            {
                address.displacement = ByteOffset::new(displacement);
            }
            _ => {}
        }
    }
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
}

#[test]
fn byte_index_reads_exactly_one_byte_and_preserves_banked_load_store_order() {
    let source = "BYTE POINTER base=$7100 BYTE index=$7104,value=$7106,result=$7107 BYTE FUNC Exchange(BYTE POINTER p BYTE i BYTE v) BYTE old old=p(i) p(i)=v RETURN(old) PROC Main() result=Exchange(base,index,value) RETURN";
    for optimize in [false, true] {
        for displacement in [0, 17, 65280, 65281] {
            let p = prepared(source, optimize, displacement);
            let c = p.compile(&layout()).unwrap();
            assert_eq!(
                c.image.to_json().unwrap(),
                prepared(&source.replace('\n', "\r\n"), optimize, displacement)
                    .compile(&layout())
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            let r = c
                .machine
                .prepared
                .routines
                .iter()
                .find(|r| r.name == "Exchange")
                .unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let mut selected = 0;
            for b in &r.blocks {
                for (i, op) in b.ops.iter().enumerate() {
                    if matches!(op,Mir65816Op::Load{address,..}|Mir65816Op::Store{address,..} if address.index.is_some())
                    {
                        let bytes = &m.code.bytes[m.code.mir_spans[&(b.id, i)].clone()];
                        selected += usize::from(bytes.windows(3).any(|w| w == [0x29, 0xff, 0]));
                    }
                }
            }
            assert_eq!(selected, if displacement <= 65280 { 2 } else { 0 });
            let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
            for variant in 0..3 {
                let loaded = (variant > 0).then(|| {
                    format::relocate(
                        &bytes,
                        &o65::placement(&bytes, variant - 1, vec![o65::fault(variant - 1)]),
                    )
                    .unwrap()
                });
                let indexes: Vec<_> = if displacement == 0 && variant == 0 {
                    (0..=255u8).collect()
                } else {
                    vec![0, 1, 127, 128, 255]
                };
                for index in indexes {
                    for base in [0x22fff0u32, 0xfffff0] {
                        let target = (base + u32::from(index) + displacement) & 0xffffff;
                        let mask = if index & 1 == 0 { 0 } else { 4 };
                        let mut h = if let Some(l) = &loaded {
                            Harness::new_o65(l, &caller(l.entry()), mask)
                        } else {
                            Harness::new(&c.image, &caller(c.image.entry), mask)
                        };
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                        h.bus.ram[0x7104..0x7108].copy_from_slice(&[index, 0xee, 231, 0]);
                        for (offset, value) in [(0xffffff, 0xa5), (0, 117), (1, 0xa5)] {
                            let at = (target + offset) & 0xffffff;
                            h.bus.map(at, &[value], true);
                            h.bus.watched.insert(at);
                        }
                        h.run();
                        h.guards(mask);
                        assert_eq!(h.bus.ram[0x7107], 117);
                        assert_eq!(h.bus.ram[target as usize], 231);
                        assert_eq!(
                            h.bus
                                .trace
                                .iter()
                                .map(|&(_, a, k)| (a, k))
                                .collect::<Vec<_>>(),
                            [(target, Access::Read), (target, Access::Write(231))]
                        );
                        assert_eq!(h.bus.ram[((target + 0xffffff) & 0xffffff) as usize], 0xa5);
                        assert_eq!(h.bus.ram[((target + 1) & 0xffffff) as usize], 0xa5);
                    }
                }
            }
        }
    }
}

#[test]
fn byte_indexes_preserve_relocated_symbol_bases() {
    let source = "BYTE ARRAY bytes(300) BYTE index=$7100,result=$7102 PROC Main() bytes(index)=217 result=bytes(index) RETURN";
    for optimize in [false, true] {
        let object = o65::compile(source, optimize, vec![]);
        for variant in 0..2 {
            let l = format::relocate(
                &object,
                &o65::placement(&object, variant, vec![o65::fault(variant)]),
            )
            .unwrap();
            for index in [0, 1, 127, 255] {
                let mut h = Harness::new_o65(&l, &caller(l.entry()), 0);
                h.bus.ram[0x7100] = index;
                h.bus.ram[0x7101] = 0xee;
                let target = o65::object(&l, "bytes") + u32::from(index);
                h.bus.watched.insert(target);
                h.run();
                h.guards(0);
                assert_eq!(h.bus.ram[0x7102], 217);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, a, k)| (a, k))
                        .collect::<Vec<_>>(),
                    [(target, Access::Write(217)), (target, Access::Read)]
                );
            }
        }
    }
}
