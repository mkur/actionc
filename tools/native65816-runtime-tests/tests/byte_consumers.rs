mod support;
use actionc::mir65816::{Mir65816Op, o65 as format};
use actionc_vm::native65816::Access;
use support::*;

#[test]
fn adjacent_byte_consumers_keep_one_exact_read_and_omit_private_capture() {
    for access in ["p^", "p(3)", "p(i)"] {
        for volatile in [false, true] {
            let source = format!(
                "BYTE POINTER base=$7100\nBYTE result=$7104\nBYTE FUNC Work(BYTE POINTER p BYTE i) IF {access}=0 THEN RETURN(17) FI RETURN(128#{access})\nPROC Main() result=Work(base,3) RETURN\n"
            );
            for optimize in [false, true] {
                let make = |s: &str| {
                    let mut p = prepare(s, optimize);
                    if volatile {
                        for op in p
                            .mir
                            .routines
                            .iter_mut()
                            .flat_map(|r| &mut r.blocks)
                            .flat_map(|b| &mut b.ops)
                        {
                            if let Mir65816Op::Load {
                                address, volatile, ..
                            } = op
                                && matches!(
                                    address.base,
                                    actionc::mir65816::Mir65816AddressBase::Indirect(_)
                                )
                            {
                                *volatile = true;
                            }
                        }
                    }
                    p
                };
                let p = make(&source);
                let c = p.compile(&layout()).unwrap();
                assert_eq!(
                    c.image.to_json().unwrap(),
                    make(&source.replace('\n', "\r\n"))
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
                    .find(|r| r.name == "Work")
                    .unwrap();
                let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                let mut loads = 0;
                for b in &r.blocks {
                    for (i, op) in b.ops.iter().enumerate() {
                        if matches!(op,Mir65816Op::Load{address,..} if matches!(address.base,actionc::mir65816::Mir65816AddressBase::Indirect(_)))
                        {
                            loads += 1;
                            let bytes = &m.code.bytes[m.code.mir_spans[&(b.id, i)].clone()];
                            assert_eq!(
                                bytes[bytes.len() - 2] == 0x83,
                                volatile,
                                "{access}/{optimize}: {bytes:02x?}"
                            );
                        }
                    }
                }
                assert_eq!(loads, 2);
                let object = p.compile_o65(&Default::default()).unwrap().bytes;
                for variant in 0..3 {
                    let loaded = (variant > 0).then(|| {
                        format::relocate(
                            &object,
                            &o65::placement(&object, variant - 1, vec![o65::fault(variant - 1)]),
                        )
                        .unwrap()
                    });
                    for value in [0u8, 1, 127, 128, 255] {
                        let mut h = if let Some(l) = &loaded {
                            Harness::new_o65(l, &caller(l.entry()), 0)
                        } else {
                            Harness::new(&c.image, &caller(c.image.entry), 0)
                        };
                        let base = if access == "p^" {
                            0x230000u32
                        } else {
                            0x22fffd
                        };
                        let target = 0x230000u32;
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&base.to_le_bytes()[..3]);
                        h.bus.map(target - 1, &[0xa5, value, 0x5a], true);
                        h.bus.watched.extend(target - 1..target + 2);
                        h.run();
                        h.guards(0);
                        assert_eq!(
                            h.bus.ram[0x7104],
                            if value == 0 {
                                17
                            } else {
                                u8::from(value != 128)
                            }
                        );
                        assert_eq!(
                            h.bus
                                .trace
                                .iter()
                                .map(|&(_, a, k)| (a, k))
                                .collect::<Vec<_>>(),
                            vec![(target, Access::Read); if value == 0 { 1 } else { 2 }]
                        );
                        assert_eq!(h.bus.ram[(target - 1) as usize], 0xa5);
                        assert_eq!(h.bus.ram[(target + 1) as usize], 0x5a);
                    }
                }
            }
        }
    }
}
