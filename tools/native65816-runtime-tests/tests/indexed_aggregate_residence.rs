mod support;
use actionc::mir65816::{
    Mir65816Address as Address, Mir65816AddressBase as Base, Mir65816AddressMode as Mode,
    Mir65816ExternalAddress as External, Mir65816Op as Op, Mir65816Value as V,
    emit::{Location, proof},
};
use actionc::nir::{ByteOffset, ByteSize, NirIntegerType, NirType, NirTypeKind, TempId};
use actionc_vm::native65816::{Access, Inputs};
use support::*;

fn pointer(value: V, offset: u32) -> Address {
    Address {
        base: Base::Indirect(value),
        index: None,
        displacement: ByteOffset::new(offset),
        mode: Mode::LongIndirect,
    }
}

#[test]
fn embedded_byte_arrays_keep_captures_at_large_offsets_and_relocated_bank_crossings() {
    for offset in [3u32, 65532, 65533, 65536] {
        let mut padding = String::new();
        let mut remaining = offset - 3;
        let mut n = 0;
        while remaining != 0 {
            let bytes = remaining.min(16384);
            padding.push_str(&format!("BYTE ARRAY padding{n}({bytes}) "));
            remaining -= bytes;
            n += 1;
        }
        let source = format!(
            "TYPE Box=[Box POINTER next {padding}BYTE ARRAY data(8)]\n\
Box POINTER root=$7100 BYTE index=$7103,result=$7104\n\
BYTE FUNC Work(Box POINTER root BYTE i) LET p=root.next LET old=p.data(i) p.data(i)=old+1 RETURN(p.data(i))\n\
PROC Main() result=Work(root,index) RETURN\n"
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            let c = p.compile(&layout()).unwrap();
            assert_eq!(
                c.image.to_json().unwrap(),
                prepare(&source.replace('\n', "\r\n"), optimize)
                    .compile(&layout())
                    .unwrap()
                    .image
                    .to_json()
                    .unwrap()
            );
            let work = c
                .machine
                .routines
                .iter()
                .find(|m| {
                    c.machine
                        .prepared
                        .routines
                        .iter()
                        .any(|r| r.id == m.id && r.name == "Work")
                })
                .unwrap();
            assert!(
                proof::placement_summary(&work.code)
                    .unwrap()
                    .unwrap()
                    .resident_indexed_windows
                    > 0,
                "{optimize}/{offset}"
            );
            let object = p.compile_o65(&Default::default()).unwrap().bytes;
            let loaded = actionc::mir65816::o65::relocate(
                &object,
                &o65::placement(&object, 1, vec![o65::fault(1)]),
            )
            .unwrap();
            for relocated in [false, true] {
                for (base, index, old) in [
                    (0x21fffeu32, 0u8, 0u8),
                    (0x32fffe, 255, 127),
                    (0xfffff0, 1, 255),
                ] {
                    for mask in [0, 4] {
                        let target = (base + offset + u32::from(index)) & 0xffffff;
                        let mut h = if relocated {
                            Harness::new_o65(&loaded, &caller(loaded.entry()), mask)
                        } else {
                            Harness::new(&c.image, &caller(c.image.entry), mask)
                        };
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&0x8100u32.to_le_bytes()[..3]);
                        h.bus.ram[0x7103] = index;
                        h.bus.map(0x8100, &base.to_le_bytes()[..3], true);
                        h.bus.watched.extend(0x8100..0x8103);
                        for (delta, value) in [(0xffffff, 0xa5), (0, old), (1, 0x5a)] {
                            let at = (target + delta) & 0xffffff;
                            h.bus.map(at, &[value], true);
                            h.bus.watched.insert(at);
                        }
                        h.run();
                        h.guards(mask);
                        assert_eq!(h.bus.ram[0x7104], old.wrapping_add(1));
                        let mut expected: Vec<_> =
                            (0x8100..0x8103).map(|at| (at, Access::Read)).collect();
                        expected.extend([
                            (target, Access::Read),
                            (target, Access::Write(old.wrapping_add(1))),
                            (target, Access::Read),
                        ]);
                        assert_eq!(
                            h.bus
                                .trace
                                .iter()
                                .map(|(_, at, access)| (*at, *access))
                                .collect::<Vec<_>>(),
                            expected
                        );
                        assert_eq!(h.bus.ram[((target + 0xffffff) & 0xffffff) as usize], 0xa5);
                        assert_eq!(h.bus.ram[((target + 1) & 0xffffff) as usize], 0x5a);
                    }
                }
            }
        }
    }
}

#[test]
fn complete_index_and_payload_widths_preserve_strides_extents_and_modular_carry() {
    const SOURCE: &str = "TYPE Box=[Box POINTER next BYTE ARRAY data(8)]\nBox POINTER root=$7100 BYTE index=$7103\nPROC Work(Box POINTER root BYTE i) LET p=root.next LET old=p.data(i) p.data(i)=old RETURN\nPROC Main() Work(root,index) RETURN\n";
    for optimize in [false, true] {
        for width in [1u32, 2, 3, 4] {
            for offset in [3u32, 65533, 65536] {
                let mut p = prepare(SOURCE, optimize);
                let r = p
                    .mir
                    .routines
                    .iter_mut()
                    .find(|r| r.name == "Work")
                    .unwrap();
                let pointer_type = r.temps.iter().find(|(_, ty)| ty.pointer).unwrap().1.clone();
                r.temps = vec![
                    (TempId(0), pointer_type),
                    (
                        TempId(1),
                        NirType {
                            kind: NirTypeKind::Integer(NirIntegerType::U8),
                            summary: "BYTE".into(),
                            width: Some(ByteSize::ONE),
                            pointer: false,
                        },
                    ),
                    (
                        TempId(2),
                        NirType {
                            kind: NirTypeKind::Integer(match width {
                                1 => NirIntegerType::U8,
                                2 => NirIntegerType::U16,
                                3 => NirIntegerType::ordinary(24, false),
                                _ => NirIntegerType::U32,
                            }),
                            summary: format!("{}-bit payload", width * 8),
                            width: Some(ByteSize::new(width)),
                            pointer: false,
                        },
                    ),
                ];
                let mut field = pointer(V::Temp(TempId(0), ByteSize::new(3)), offset);
                field.index = Some(actionc::mir65816::Mir65816Index {
                    value: V::Temp(TempId(1), ByteSize::ONE),
                    stride: ByteSize::new(width),
                });
                field.mode = Mode::LongIndexed;
                r.blocks[0].ops = vec![
                    Op::Load {
                        dest: TempId(0),
                        width: ByteSize::new(3),
                        address: pointer(V::Param(r.frame.parameters[0].param), 0),
                        volatile: false,
                    },
                    Op::Load {
                        dest: TempId(1),
                        width: ByteSize::ONE,
                        address: Address {
                            base: Base::Parameter(r.frame.parameters[1].param),
                            index: None,
                            displacement: ByteOffset::ZERO,
                            mode: Mode::Parameter,
                        },
                        volatile: false,
                    },
                    Op::Load {
                        dest: TempId(2),
                        width: ByteSize::new(width),
                        address: field.clone(),
                        volatile: false,
                    },
                    Op::Store {
                        value: V::Temp(TempId(2), ByteSize::new(width)),
                        width: ByteSize::new(width),
                        address: field,
                        volatile: false,
                    },
                ];
                actionc::mir65816::verify_program(&p.mir).unwrap();
                let c = p.compile(&layout()).unwrap();
                for (base, index) in [(0x21ffffu32, 0u8), (0x32fffe, 255), (0xfffff0, 3)] {
                    for mask in [0, 4] {
                        let target = (base + offset + u32::from(index) * width) & 0xffffff;
                        let mut h = Harness::new(&c.image, &caller(c.image.entry), mask);
                        h.bus.ram[0x7100..0x7103].copy_from_slice(&0x8100u32.to_le_bytes()[..3]);
                        h.bus.ram[0x7103] = index;
                        h.bus.map(0x8100, &base.to_le_bytes()[..3], true);
                        h.bus.watched.extend(0x8100..0x8103);
                        let data = [0x98, 0x76, 0x54, 0x32];
                        for byte in 0..width {
                            let at = (target + byte) & 0xffffff;
                            h.bus.map(at, &[data[byte as usize]], true);
                            h.bus.watched.insert(at);
                        }
                        for at in [(target + 0xffffff) & 0xffffff, (target + width) & 0xffffff] {
                            h.bus.map(at, &[0xa5], true);
                            h.bus.watched.insert(at);
                        }
                        h.run();
                        h.guards(mask);
                        let mut expected: Vec<_> =
                            (0x8100..0x8103).map(|at| (at, Access::Read)).collect();
                        expected.extend(
                            (0..width).map(|byte| ((target + byte) & 0xffffff, Access::Read)),
                        );
                        expected.extend((0..width).map(|byte| {
                            (
                                (target + byte) & 0xffffff,
                                Access::Write(data[byte as usize]),
                            )
                        }));
                        assert_eq!(
                            h.bus
                                .trace
                                .iter()
                                .map(|(_, at, access)| (*at, *access))
                                .collect::<Vec<_>>(),
                            expected,
                            "{optimize}/{width}/{offset}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn word_and_24_bit_indexes_keep_full_magnitudes_when_y_cannot_hold_the_offset() {
    const SOURCE: &str = "TYPE Box=[Box POINTER next BYTE ARRAY data(8)]\nBox POINTER root=$7100 BYTE index=$7103\nPROC Work(Box POINTER root BYTE i) LET p=root.next LET old=p.data(i) p.data(i)=old RETURN\nPROC Main() Work(root,index) RETURN\n";
    for optimize in [false, true] {
        for (width, stride, input) in [(2u32, 257u32, 65535u32), (3, 65536, 0xffffff)] {
            let mut p = prepare(SOURCE, optimize);
            let r = p
                .mir
                .routines
                .iter_mut()
                .find(|r| r.name == "Work")
                .unwrap();
            let bases: std::collections::BTreeMap<_, _> = r
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter_map(|op| {
                    if let Op::AddressOf { dest, address, .. } = op {
                        Some((*dest, address.base.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            let ids: std::collections::BTreeSet<_> = r
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter_map(|op| {
                    let a = match op {
                        Op::Load { address, .. } | Op::Store { address, .. } => address,
                        _ => return None,
                    };
                    if let Some(actionc::mir65816::Mir65816Index {
                        value: V::Temp(id, _),
                        ..
                    }) = &a.index
                    {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect();
            for op in r.blocks.iter_mut().flat_map(|b| &mut b.ops) {
                if let Op::Load {
                    dest,
                    width: bytes,
                    address,
                    ..
                } = op
                {
                    if ids.contains(dest) {
                        *bytes = ByteSize::new(width);
                        *address = Address {
                            base: Base::External(External::Absolute(
                                actionc::target::AddressValue::data(0x7103),
                            )),
                            index: None,
                            displacement: ByteOffset::ZERO,
                            mode: Mode::External,
                        };
                    }
                }
                if let Op::Load { address, .. } | Op::Store { address, .. } = op {
                    if let Some(index) = &mut address.index {
                        if let Base::Indirect(V::Temp(id, _)) = address.base {
                            address.base = bases[&id].clone();
                        }
                        if let V::Temp(_, bytes) = &mut index.value {
                            *bytes = ByteSize::new(width);
                        }
                        index.stride = ByteSize::new(stride);
                        address.displacement = ByteOffset::new(65536);
                    }
                }
            }
            for (id, ty) in &mut r.temps {
                if ids.contains(id) {
                    ty.width = Some(ByteSize::new(width));
                    ty.kind =
                        NirTypeKind::Integer(NirIntegerType::ordinary(width as u8 * 8, false));
                }
            }
            actionc::mir65816::verify_program(&p.mir).unwrap();
            let c = p.compile(&layout()).unwrap();
            for mask in [0, 4] {
                let base = 0x32ffffu32;
                let target = (u64::from(base) + u64::from(input) * u64::from(stride) + 65536)
                    as u32
                    & 0xffffff;
                let mut h = Harness::new(&c.image, &caller(c.image.entry), mask);
                h.bus.ram[0x7100..0x7103].copy_from_slice(&0x8100u32.to_le_bytes()[..3]);
                h.bus.ram[0x7103..0x7103 + width as usize]
                    .copy_from_slice(&input.to_le_bytes()[..width as usize]);
                h.bus.map(0x8100, &base.to_le_bytes()[..3], true);
                h.bus.map(target - 1, &[0xa5, 0x37, 0x5a], true);
                h.bus.watched.extend(target - 1..target + 2);
                h.run();
                h.guards(mask);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|(_, at, access)| (*at, *access))
                        .collect::<Vec<_>>(),
                    [(target, Access::Read), (target, Access::Write(0x37))]
                );
            }
        }
    }
}

fn aggregate_prepared(bytes: u32, pressure: usize) -> actionc::compiler::native65816::Prepared {
    let mut p = prepare(
        "TYPE Cell=[BYTE flag CARD value Cell POINTER next]\nCell POINTER root=$7100,other=$7103 CARD before=$7106,after=$7108\nPROC Work(Cell POINTER root,other) LET p=root.next LET n=p.value before=n after=p.value RETURN\nPROC Main() Work(root,other) RETURN\n",
        false,
    );
    let r = p
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Work")
        .unwrap();
    let pointer_ty = r.temps.iter().find(|(_, t)| t.pointer).unwrap().1.clone();
    let word = NirType {
        kind: NirTypeKind::Integer(NirIntegerType::U16),
        summary: "CARD".into(),
        width: Some(ByteSize::new(2)),
        pointer: false,
    };
    let root = V::Param(r.frame.parameters[0].param);
    let other = V::Param(r.frame.parameters[1].param);
    let captured = V::Temp(TempId(0), ByteSize::new(3));
    r.blocks.truncate(1);
    r.temps = vec![(TempId(0), pointer_ty)];
    let mut ops = vec![Op::Load {
        dest: TempId(0),
        width: ByteSize::new(3),
        address: pointer(root, 4),
        volatile: false,
    }];
    for index in 0..pressure {
        let id = TempId(index as u32 + 1);
        r.temps.push((id, word.clone()));
        ops.push(Op::Load {
            dest: id,
            width: ByteSize::new(2),
            address: pointer(captured.clone(), 2),
            volatile: false,
        });
    }
    ops.push(Op::Copy {
        destination: pointer(other, 0),
        source: pointer(captured.clone(), 0),
        bytes: ByteSize::new(bytes),
        overlap_safe: true,
        source_volatile: false,
        destination_volatile: false,
    });
    for index in 0..pressure {
        let address = Address {
            base: Base::External(External::Absolute(actionc::target::AddressValue::data(
                0x7200 + index as u64 * 2,
            ))),
            index: None,
            displacement: ByteOffset::ZERO,
            mode: Mode::External,
        };
        ops.push(Op::Store {
            address,
            value: V::Temp(TempId(index as u32 + 1), ByteSize::new(2)),
            width: ByteSize::new(2),
            volatile: false,
        });
    }
    let id = TempId(pressure as u32 + 1);
    r.temps.push((id, word));
    ops.push(Op::Load {
        dest: id,
        width: ByteSize::new(2),
        address: pointer(captured, 2),
        volatile: false,
    });
    // Reuse a verified external scalar destination, independently of promotion.
    let after = r.blocks[0]
        .ops
        .iter()
        .find_map(|op| {
            if let Op::Store { address, .. } = op {
                matches!(address.base, Base::External(_) | Base::Static(_))
                    .then_some(address.clone())
            } else {
                None
            }
        })
        .unwrap();
    ops.push(Op::Store {
        address: after,
        value: V::Temp(id, ByteSize::new(2)),
        width: ByteSize::new(2),
        volatile: false,
    });
    r.blocks[0].ops = ops;
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
}

#[test]
fn aggregate_protocol_preserves_snapshots_overlap_self_copy_and_pressure_fallback() {
    for (bytes, pressure) in [
        (0u32, 1usize),
        (1, 1),
        (3, 1),
        (8, 1),
        (259, 22),
        (65537, 1),
    ] {
        let p = aggregate_prepared(bytes, pressure);
        for guards in [false, true] {
            let mut options = layout();
            options.stack_checks = guards;
            let c = p.compile(&options).unwrap();
            let work = c
                .machine
                .routines
                .iter()
                .find(|m| {
                    c.machine
                        .prepared
                        .routines
                        .iter()
                        .any(|r| r.id == m.id && r.name == "Work")
                })
                .unwrap();
            let summary = proof::placement_summary(&work.code).unwrap().unwrap();
            assert_eq!(summary.aggregate_windows, 1);
            assert!(summary.resident_aggregate_windows > 0);
            if pressure > 16 {
                assert!(
                    work.frame
                        .temps
                        .values()
                        .any(|h| matches!(h, Location::Stack(_)))
                );
                assert!(
                    work.frame
                        .temps
                        .values()
                        .any(|h| matches!(h, Location::DirectPage(_)))
                );
            }
            let variants: Vec<_> = if bytes > 1000 {
                vec![(0x12fff0u32, 0x12fff1u32, 0u8)]
            } else {
                vec![
                    (0x12fff0, 0x12ffef, 0),
                    (0x12fff0, 0x12fff1, 4),
                    (0x12fff0, 0x12fff0, 0),
                    (0x12fff0, 0x130401, 4),
                ]
            };
            for (source, destination, mask) in variants {
                let start = 0x12ff00u32;
                let size = (destination.max(source) - start + bytes.max(8) + 2) as usize;
                let initial: Vec<_> = (0..size).map(|n| (n * 37 + 11) as u8).collect();
                let mut expected = initial.clone();
                let src = (source - start) as usize;
                let dst = (destination - start) as usize;
                let saved = u16::from_le_bytes([initial[src + 2], initial[src + 3]]);
                expected.copy_within(src..src + bytes as usize, dst);
                let fresh = u16::from_le_bytes([expected[src + 2], expected[src + 3]]);
                let mut h = Harness::new(&c.image, &caller(c.image.entry), mask);
                h.bus.map(start, &initial, true);
                h.bus.map(0x8100, &[0xa5; 8], true);
                h.bus.ram[0x8104..0x8107].copy_from_slice(&source.to_le_bytes()[..3]);
                h.bus.ram[0x7100..0x7103].copy_from_slice(&0x8100u32.to_le_bytes()[..3]);
                h.bus.ram[0x7103..0x7106].copy_from_slice(&destination.to_le_bytes()[..3]);
                // Trace small cases exactly; the large case checks every byte.
                if bytes < 1000 {
                    h.bus.watched.extend(start..start + size as u32);
                }
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            40_000_000,
                            |_| Inputs::default(),
                            |cpu| cpu.is_stopped()
                        )
                        .unwrap()
                );
                h.guards(mask);
                assert_eq!(&h.bus.ram[start as usize..start as usize + size], expected);
                for index in 0..pressure {
                    assert_eq!(h.bus.value(0x7200 + index as u32 * 2, 2), u32::from(saved));
                }
                assert_eq!(h.bus.ram[0x8107], 0xa5);
                // The first external destination in the source is `before`.
                assert_eq!(h.bus.value(0x7106, 2), u32::from(fresh));
                if bytes < 1000 {
                    let mut trace = Vec::new();
                    for _ in 0..pressure {
                        trace.extend((source + 2..source + 4).map(|at| (at, Access::Read)));
                    }
                    if source != destination {
                        let order: Vec<_> = if destination > source {
                            (0..bytes).rev().collect()
                        } else {
                            (0..bytes).collect()
                        };
                        for byte in order {
                            trace.push((source + byte, Access::Read));
                            trace.push((
                                destination + byte,
                                Access::Write(initial[src + byte as usize]),
                            ));
                        }
                    }
                    trace.extend((source + 2..source + 4).map(|at| (at, Access::Read)));
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|(_, at, access)| (*at, *access))
                            .collect::<Vec<_>>(),
                        trace,
                        "{bytes}/{pressure}/{source:x}/{destination:x}/{guards}"
                    );
                }
            }
        }
    }
}

#[test]
fn indexed_and_aggregate_windows_survive_every_enabled_irq_site_nmi_and_task_domains() {
    use std::collections::BTreeSet;
    use support::context::*;
    let source = "MODULE TEST PUBLIC EXTERNAL PROC Yield()\n\
VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current CARD irqResult\n\
TYPE Cell=[BYTE flag CARD value Cell POINTER next BYTE ARRAY data(9)]\n\
TYPE Job=[Cell POINTER item Cell POINTER out BYTE done CARD result BYTE POINTER peer]\n\
PROC Form(Cell POINTER root,other) LET p=root.next LET saved=p.value\n\
 other.value=saved root.value=saved other.flag=p.flag other.value=p.value other.data(3)=p.data(2) RETURN\n\
CARD FUNC Dispatch(CARD saved BYTE reason) Cell POINTER out irqAck=1\n\
 Form(Cell POINTER($9300),Cell POINTER($9400)) out=Cell POINTER($9400) irqResult=out.value\n\
 IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI taskB=saved current=0 RETURN(taskA)\n\
PROC Task(Job POINTER work) Form(work.item,work.out) work.result=work.out.value work.done=1\n\
 WHILE work.peer^=0 DO Yield() OD RETURN PROC Main() RETURN ENDMODULE\n";
    for optimize in [false, true] {
        let mut p = prepare(source, optimize);
        let work = p
            .mir
            .routines
            .iter_mut()
            .find(|r| r.name == "Form" || r.name.starts_with("M_TEST_FORM_"))
            .unwrap();
        let (at, value) = work
            .blocks
            .iter()
            .flat_map(|b| b.ops.iter().enumerate())
            .find_map(|(i, op)| {
                if let Op::Load {
                    width,
                    address:
                        Address {
                            base: Base::Indirect(value),
                            ..
                        },
                    ..
                } = op
                {
                    (width.get() == 2).then_some((i, value.clone()))
                } else {
                    None
                }
            })
            .unwrap();
        let destination = V::Param(work.frame.parameters[1].param);
        work.blocks[0].ops.insert(
            at + 1,
            Op::Copy {
                source: pointer(value, 0),
                destination: pointer(destination, 0),
                bytes: ByteSize::new(16),
                overlap_safe: true,
                source_volatile: false,
                destination_volatile: false,
            },
        );
        actionc::mir65816::verify_program(&p.mir).unwrap();
        let machine = actionc::mir65816::emit::materialize(&p.mir).unwrap();
        assert!(machine.routines.iter().any(|r| {
            proof::placement_summary(&r.code)
                .unwrap()
                .is_some_and(|s| s.resident_aggregate_windows > 0 && s.resident_indexed_windows > 0)
        }));
        let mut h = ContextHarness::from_prepared(source, optimize, "Task", &[0x7100, 0x7120], p);
        for (job, root, out, peer) in [
            (0x7100usize, 0x9100u32, 0x9500u32, 0x7126u32),
            (0x7120, 0x9200, 0x9600, 0x7106),
        ] {
            h.bus.ram[job..job + 3].copy_from_slice(&root.to_le_bytes()[..3]);
            h.bus.ram[job + 3..job + 6].copy_from_slice(&out.to_le_bytes()[..3]);
            h.bus.ram[job + 10..job + 13].copy_from_slice(&peer.to_le_bytes()[..3]);
        }
        for (root, source, destination, value) in [
            (0x9100usize, 0x21fff8u32, 0x9500u32, 65535u16),
            (0x9200, 0x32fff8, 0x9600, 32767),
            (0x9300, 0x43fff8, 0x9400, 271),
        ] {
            for at in [root as u32, source, destination] {
                h.bus.map(at - 1, &[0xa5; 18], true);
            }
            h.bus.ram[root + 4..root + 7].copy_from_slice(&source.to_le_bytes()[..3]);
            h.bus.ram[source as usize + 2..source as usize + 4]
                .copy_from_slice(&value.to_le_bytes());
            h.bus.ram[source as usize + 9] = 0x37;
        }
        let range = h
            .image
            .routines
            .iter()
            .find(|r| r.address == context::routine(&h.image, "Form"))
            .map(|r| r.address..r.address + r.size)
            .unwrap();
        let check = |h: &ContextHarness| {
            h.guards();
            assert_eq!(h.bus.value(DONE, 2), 1);
            assert_eq!(h.bus.value(0x7106, 1), 1);
            assert_eq!(h.bus.value(0x7126, 1), 1);
            assert_eq!(h.bus.value(0x7108, 2), 65535);
            assert_eq!(h.bus.value(0x7128, 2), 32767);
            assert_eq!(h.bus.value(context::symbol(&h.image, "irqResult"), 2), 271);
            for out in [0x9400u32, 0x9500, 0x9600] {
                assert_eq!(h.bus.ram[out as usize + 10], 0x37);
                assert_eq!(h.bus.ram[out as usize - 1], 0xa5);
                assert_eq!(h.bus.ram[out as usize + 16], 0xa5);
            }
        };
        let mut seen = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            if h.cpu.is_instruction_boundary()
                && r.p & 4 == 0
                && [0x2000, 0x2100].contains(&r.d)
                && range.contains(&h.cpu.pc())
                && seen.insert((r.d, h.cpu.pc()))
            {
                let saved_cpu = h.cpu.clone();
                let saved_bus = h.bus.clone();
                let mut pending = true;
                for tick in 0..2_000_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    let writes = h.bus.writes.len();
                    h.tick(Inputs {
                        irq: pending,
                        nmi: tick == 40,
                        ..Default::default()
                    });
                    if h.bus.writes[writes..].iter().any(|&(at, _)| at == IRQ_ACK) {
                        pending = false;
                    }
                }
                check(&h);
                h.cpu = saved_cpu;
                h.bus = saved_bus;
            }
            h.tick(Inputs::default());
        }
        h.guards();
        assert!(h.cpu.is_stopped());
        assert!(seen.len() >= 40);
    }
}
