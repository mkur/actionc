mod support;
use actionc::mir65816::{Mir65816AddressBase, Mir65816Op, Mir65816Value, emit, o65};
use actionc_vm::native65816::{Access, Inputs};
use support::*;

fn reach(h: &mut Harness, pc: u32) {
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                200_000,
                |_| Inputs::default(),
                |c| c.is_instruction_boundary() && c.pc() == pc
            )
            .unwrap()
    );
}

#[test]
fn final_pointer_values_preserve_private_copies_and_exact_external_traffic() {
    for local in [false, true] {
        for private in [false, true] {
            let payload = if local { "local" } else { "v" };
            let source = format!(
                "TYPE Node=[BYTE tag BYTE POINTER next]\nNode POINTER base=$7100\nBYTE POINTER input=$7104\nPROC Touch() RETURN\nPROC Write(Node POINTER p BYTE POINTER v) BYTE POINTER local,result {} {} p.next={} Touch() RETURN\nPROC Main() Write(base,input) RETURN\n",
                if local {
                    "local=v IF v=0 THEN local=v FI"
                } else {
                    ""
                },
                if private {
                    format!("result={payload} IF v=0 THEN result=0 FI")
                } else {
                    String::new()
                },
                if private { "result" } else { payload },
            );
            for optimize in [false, true] {
                let prepared = prepare(&source, optimize);
                let compiled = prepared.compile(&layout()).unwrap();
                assert_eq!(
                    compiled.image.to_json().unwrap(),
                    prepare(&source.replace('\n', "\r\n"), optimize)
                        .compile(&layout())
                        .unwrap()
                        .image
                        .to_json()
                        .unwrap()
                );
                let (direct, a) = emit::proof::materialize_reference(&prepared.mir, true).unwrap();
                let (replayed, b) = emit::proof::materialize_replayed(&prepared.mir, true).unwrap();
                for ((x, y), (a, b)) in direct
                    .routines
                    .iter()
                    .zip(&replayed.routines)
                    .zip(a.iter().zip(&b))
                {
                    emit::proof::compare_replay_output(&x.code, &y.code).unwrap();
                    assert_eq!(a.snapshots, b.snapshots);
                }
                let r = compiled
                    .machine
                    .prepared
                    .routines
                    .iter()
                    .find(|r| r.name == "Write")
                    .unwrap();
                let m = compiled
                    .machine
                    .routines
                    .iter()
                    .find(|m| m.id == r.id)
                    .unwrap();
                let (block, index, base, value) = r
                    .blocks
                    .iter()
                    .find_map(|b| {
                        b.ops.iter().enumerate().find_map(|(i, op)| {
                            if let Mir65816Op::Store {
                                address,
                                value: Mir65816Value::Temp(value, _),
                                width,
                                volatile: false,
                            } = op
                                && width.get() == 3
                                && let Mir65816AddressBase::Indirect(Mir65816Value::Temp(base, _)) =
                                    address.base
                            {
                                Some((b.id, i, base, *value))
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap();
                assert_ne!(base, value);
                let captures = [base, value].map(|id| {
                    let (b, i, address) = r
                        .blocks
                        .iter()
                        .find_map(|b| {
                            b.ops.iter().enumerate().find_map(|(i, op)| {
                                if let Mir65816Op::Load {
                                    dest,
                                    address,
                                    width,
                                    ..
                                } = op
                                    && *dest == id
                                    && width.get() == 3
                                {
                                    Some((b.id, i, address))
                                } else {
                                    None
                                }
                            })
                        })
                        .unwrap();
                    assert!(
                        m.code.mir_spans[&(b, i)].is_empty(),
                        "{local}/{private}/{optimize}/{id:?}"
                    );
                    if id == value {
                        assert_eq!(
                            matches!(address.base, Mir65816AddressBase::AutomaticFrame(_)),
                            local || private
                        );
                    }
                    m.frame.temps[&id].stack().unwrap().offset
                });
                let span = &m.code.mir_spans[&(block, index)];
                let bytes = prepared.compile_o65(&Default::default()).unwrap().bytes;
                for variant in 0..3 {
                    let loaded = (variant > 0).then(|| {
                        o65::relocate(
                            &bytes,
                            &support::o65::placement(
                                &bytes,
                                variant - 1,
                                vec![support::o65::fault(variant - 1)],
                            ),
                        )
                        .unwrap()
                    });
                    let address = if let Some(l) = &loaded {
                        l.routine_address(
                            l.profile()
                                .routines
                                .iter()
                                .find(|r| r.name == "Write")
                                .unwrap(),
                        )
                    } else {
                        compiled
                            .image
                            .routines
                            .iter()
                            .find(|r| r.name == "Write")
                            .unwrap()
                            .address
                    };
                    for value in [0u32, 0x12fffe, 0xffffff] {
                        for mask in [0, 4] {
                            let mut h = if let Some(l) = &loaded {
                                Harness::new_o65(l, &caller(l.entry()), mask)
                            } else {
                                Harness::new(&compiled.image, &caller(compiled.image.entry), mask)
                            };
                            let target = 0x12fffeu32;
                            h.bus.ram[0x7100..0x7103]
                                .copy_from_slice(&(target - 1).to_le_bytes()[..3]);
                            h.bus.ram[0x7104..0x7107].copy_from_slice(&value.to_le_bytes()[..3]);
                            h.bus.map(target - 1, &[0xa5; 5], true);
                            h.bus.watched.extend(target - 1..target + 4);
                            reach(&mut h, address + span.start as u32);
                            let body = usize::from(h.cpu.registers().s);
                            for slot in captures {
                                h.bus.ram[body + slot as usize..body + slot as usize + 3]
                                    .fill(0xa7);
                            }
                            let before = h.bus.reads.len();
                            reach(&mut h, address + span.end as u32);
                            for slot in captures {
                                assert!(!h.bus.reads[before..].iter().any(|&a| {
                                    (body + slot as usize..body + slot as usize + 3)
                                        .contains(&(a as usize))
                                }));
                            }
                            h.run();
                            h.guards(mask);
                            assert_eq!(h.bus.value(target, 3), value);
                            assert_eq!(
                                h.bus
                                    .trace
                                    .iter()
                                    .map(|&(_, a, k)| (a, k))
                                    .collect::<Vec<_>>(),
                                (0..3)
                                    .map(|i| (target + i, Access::Write((value >> (8 * i)) as u8)))
                                    .collect::<Vec<_>>()
                            );
                            assert_eq!(
                                (
                                    h.bus.ram[(target - 1) as usize],
                                    h.bus.ram[(target + 3) as usize]
                                ),
                                (0xa5, 0xa5)
                            );
                        }
                    }
                }
            }
        }
    }
}
