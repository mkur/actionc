mod support;
use actionc::mir65816::{emit::proof, image::TemporaryHome};
use actionc_vm::native65816::{Access, Inputs};
use std::collections::BTreeSet;
use support::*;

const WORK: &str = "TYPE Cell=[BYTE flag CARD value LONGCARD wide Cell POINTER next]\n\
PROC Touch(Cell POINTER p) p.value==+3 RETURN\n\
PROC Work(Cell POINTER root,other)\n\
 BYTE POINTER lane\n\
 LET p=root.next\n\
 p.flag==+1 other.flag=2 p.value==+1 other.value=5\n\
 p.wide=LONGCARD(p.value)+LONGCARD($010203) other.wide=LONGCARD($FFFFFFFF)\n\
 lane=BYTE POINTER(@p.wide) lane^=BYTE(p.value)\n\
 Touch(other) p.value==+other.value\n\
RETURN\n\
PROC Main() Work(Cell POINTER($8100),Cell POINTER($8300)) RETURN\n";

#[test]
fn mixed_fields_and_call_barriers_preserve_exact_accesses_and_all_widths() {
    for optimize in [false, true] {
        let p = prepare(WORK, optimize);
        let compiled = p.compile(&layout()).unwrap();
        assert_eq!(
            compiled.image.to_json().unwrap(),
            prepare(&WORK.replace('\n', "\r\n"), optimize)
                .compile(&layout())
                .unwrap()
                .image
                .to_json()
                .unwrap()
        );
        if optimize {
            assert!(compiled.machine.routines.iter().any(|r| {
                proof::placement_summary(&r.code)
                    .unwrap()
                    .is_some_and(|s| s.mixed_homes > 0)
            }));
        }
        let image = compiled.image;
        for at in [0x8500u32, 0x21fffe, 0x32ffff] {
            for (flag, word) in [(0u8, 0u16), (255, 65535), (128, 32767)] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), mask);
                    for start in [0x8100, 0x8300, at] {
                        h.bus.map(start - 1, &[0xa5; 14], true);
                        h.bus.watched.extend(start..start + 12);
                    }
                    // Alignment: flag 0, CARD 2, LONGCARD 4, next pointer 8.
                    h.bus.ram[0x8108..0x810b].copy_from_slice(&at.to_le_bytes()[..3]);
                    h.bus.ram[at as usize] = flag;
                    h.bus.ram[at as usize + 2..at as usize + 4]
                        .copy_from_slice(&word.to_le_bytes());
                    let next = word.wrapping_add(1);
                    let wide = u32::from(next).wrapping_add(0x010203);
                    let mut expected = vec![];
                    let mut read = |address: u32, bytes: u32| {
                        expected.extend((address..address + bytes).map(|a| (a, Access::Read)))
                    };
                    read(0x8108, 3);
                    read(at, 1);
                    let put = |trace: &mut Vec<_>, address: u32, value: u32, bytes: u32| {
                        trace.extend(
                            (0..bytes)
                                .map(|b| (address + b, Access::Write((value >> (b * 8)) as u8))),
                        )
                    };
                    put(&mut expected, at, u32::from(flag.wrapping_add(1)), 1);
                    put(&mut expected, 0x8300, 2, 1);
                    expected.extend((at + 2..at + 4).map(|a| (a, Access::Read)));
                    put(&mut expected, at + 2, u32::from(next), 2);
                    put(&mut expected, 0x8302, 5, 2);
                    expected.extend((at + 2..at + 4).map(|a| (a, Access::Read)));
                    put(&mut expected, at + 4, wide, 4);
                    put(&mut expected, 0x8304, u32::MAX, 4);
                    expected.extend((at + 2..at + 4).map(|a| (a, Access::Read)));
                    put(&mut expected, at + 4, u32::from(next & 255), 1);
                    expected.extend((0x8302..0x8304).map(|a| (a, Access::Read)));
                    put(&mut expected, 0x8302, 8, 2);
                    expected.extend(
                        (0x8302..0x8304)
                            .chain(at + 2..at + 4)
                            .map(|a| (a, Access::Read)),
                    );
                    put(&mut expected, at + 2, u32::from(next.wrapping_add(8)), 2);
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.bus.value(at + 2, 2), u32::from(next.wrapping_add(8)));
                    assert_eq!(
                        h.bus.value(at + 4, 4),
                        (wide & !255) | u32::from(next & 255)
                    );
                    for start in [0x8100, 0x8300, at] {
                        for offset in [-1i32, 1, 11, 12] {
                            assert_eq!(
                                h.bus.ram[(i64::from(start) + i64::from(offset)) as usize],
                                0xa5
                            );
                        }
                    }
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|(_, at, access)| (*at, *access))
                            .collect::<Vec<_>>(),
                        expected,
                        "{optimize}/{at:x}/{flag}/{word}"
                    );
                }
            }
        }
    }
}

#[test]
fn mixed_residence_survives_reentrant_irq_nmi_and_task_domains() {
    use support::context::*;
    let source = "MODULE TEST PUBLIC EXTERNAL PROC Yield()\n\
VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current\n\
TYPE Cell=[BYTE flag CARD value Cell POINTER next]\n\
TYPE Job=[Cell POINTER item BYTE done CARD result BYTE POINTER peer]\n\
CARD irqResult\n\
CARD FUNC Form(Cell POINTER root) LET p=root.next RETURN(p.value+CARD(p.flag))\n\
CARD FUNC Dispatch(CARD saved BYTE reason) irqAck=1 irqResult=Form(Cell POINTER($9300))\n\
 IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI\n\
 taskB=saved current=0 RETURN(taskA)\n\
PROC Task(Job POINTER work) work.result=Form(work.item) work.done=1\n\
 WHILE work.peer^=0 DO Yield() OD RETURN\n\
PROC Main() RETURN ENDMODULE\n";
    for optimize in [false, true] {
        let mut h = ContextHarness::new(source, optimize, "Task", &[0x7100, 0x7120]);
        for (job, root, peer) in [
            (0x7100usize, 0x9100u32, 0x7123u32),
            (0x7120, 0x9200, 0x7103),
        ] {
            h.bus.ram[job..job + 3].copy_from_slice(&root.to_le_bytes()[..3]);
            h.bus.ram[job + 6..job + 9].copy_from_slice(&peer.to_le_bytes()[..3]);
        }
        for (root, next, value, flag) in [
            (0x9100usize, 0x21fffeu32, 65535u16, 1u8),
            (0x9200, 0x32ffff, 32767, 128),
            (0x9300, 0x430001, 17, 254),
        ] {
            h.bus.map(root as u32, &[0xa5; 8], true);
            h.bus.map(next, &[0xa5; 8], true);
            h.bus.ram[root + 4..root + 7].copy_from_slice(&next.to_le_bytes()[..3]);
            h.bus.ram[next as usize] = flag;
            h.bus.ram[next as usize + 2..next as usize + 4].copy_from_slice(&value.to_le_bytes());
        }
        let form = h
            .image
            .routines
            .iter()
            .find(|r| r.address == context::routine(&h.image, "Form"))
            .unwrap();
        assert!(
            form.temporaries
                .iter()
                .any(|t| matches!(t.home, TemporaryHome::DirectPage { .. }))
        );
        let range = form.address..form.address + form.size;
        let check = |h: &ContextHarness| {
            h.guards();
            assert_eq!(h.bus.value(DONE, 2), 1);
            assert_eq!(h.bus.value(0x7103, 1), 1);
            assert_eq!(h.bus.value(0x7123, 1), 1);
            assert_eq!(h.bus.value(0x7104, 2), 0);
            assert_eq!(h.bus.value(0x7124, 2), 32895);
            assert_eq!(h.bus.value(context::symbol(&h.image, "irqResult"), 2), 271);
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
        // Without an IRQ, the dispatcher need not have computed irqResult.
        h.guards();
        assert_eq!(h.bus.value(0x7104, 2), 0);
        assert_eq!(h.bus.value(0x7124, 2), 32895);
        assert!(seen.len() >= 20);
    }
}

#[test]
fn backed_capture_survives_a_call_that_overwrites_its_entire_cached_low_word() {
    use actionc::mir65816::{Mir65816AddressBase as Base, Mir65816Op as Op, Mir65816Value as V};
    use actionc::nir::{ByteOffset, ByteSize, TempId};
    let mut prepared = prepare(WORK, false);
    let r = prepared
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Work")
        .unwrap();
    let pointer = r.temps.iter().find(|(_, ty)| ty.pointer).unwrap().1.clone();
    let word = r
        .temps
        .iter()
        .find(|(_, ty)| ty.width == Some(ByteSize::new(2)))
        .unwrap()
        .1
        .clone();
    let root = r.frame.parameters[0].param;
    let other = r.frame.parameters[1].param;
    let mut call = r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find(|op| matches!(op, Op::Call { .. }))
        .unwrap()
        .clone();
    if let Op::Call { args, .. } = &mut call {
        args[0] = V::Param(other);
    }
    let mut address = r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| {
            if let Op::Load { address, .. } = op {
                Some(address.clone())
            } else {
                None
            }
        })
        .unwrap();
    address.index = None;
    address.mode = actionc::mir65816::Mir65816AddressMode::LongIndirect;
    let mut place = |base, offset| {
        address.base = Base::Indirect(base);
        address.displacement = ByteOffset::new(offset);
        address.clone()
    };
    let p = V::Temp(TempId(0), ByteSize::new(3));
    let n = V::Temp(TempId(1), ByteSize::new(2));
    let field = place(p.clone(), 2);
    let flag = place(p.clone(), 0);
    let second = place(V::Param(other), 2);
    let second_flag = place(V::Param(other), 0);
    r.blocks.truncate(1);
    r.blocks[0].ops = vec![
        Op::Load {
            dest: TempId(0),
            width: ByteSize::new(3),
            address: place(V::Param(root), 8),
            volatile: false,
        },
        Op::Load {
            dest: TempId(1),
            width: ByteSize::new(2),
            address: field.clone(),
            volatile: false,
        },
        Op::Store {
            address: second,
            value: n.clone(),
            width: ByteSize::new(2),
            volatile: false,
        },
        Op::Store {
            address: flag.clone(),
            value: V::U8(7),
            width: ByteSize::ONE,
            volatile: false,
        },
        Op::Store {
            address: second_flag,
            value: V::U8(9),
            width: ByteSize::ONE,
            volatile: false,
        },
        Op::Store {
            address: field,
            value: n,
            width: ByteSize::new(2),
            volatile: false,
        },
        call,
        Op::Store {
            address: flag,
            value: V::U8(11),
            width: ByteSize::ONE,
            volatile: false,
        },
    ];
    r.temps = vec![(TempId(0), pointer), (TempId(1), word.clone())];
    r.blocks[0].terminator = actionc::mir65816::Mir65816Terminator::Return {
        value: None,
        release_frame_bytes: r.frame.extent,
        form: r.epilogue.return_form,
        restored_mode: r.epilogue.restored_mode,
    };
    let r = prepared
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Touch")
        .unwrap();
    let param = r.frame.parameters[0].param;
    let field = place(V::Param(param), 2);
    let n = V::Temp(TempId(0), ByteSize::new(2));
    r.blocks.truncate(1);
    r.blocks[0].ops = vec![
        Op::Load {
            dest: TempId(0),
            width: ByteSize::new(2),
            address: field.clone(),
            volatile: false,
        },
        Op::Store {
            address: field.clone(),
            value: n.clone(),
            width: ByteSize::new(2),
            volatile: false,
        },
        Op::Store {
            address: field,
            value: n,
            width: ByteSize::new(2),
            volatile: false,
        },
    ];
    r.temps = vec![(TempId(0), word)];
    r.blocks[0].terminator = actionc::mir65816::Mir65816Terminator::Return {
        value: None,
        release_frame_bytes: r.frame.extent,
        form: r.epilogue.return_form,
        restored_mode: r.epilogue.restored_mode,
    };
    actionc::mir65816::verify_program(&prepared.mir).unwrap();
    let compiled = prepared.compile(&layout()).unwrap();
    let summary = |name| {
        let r = prepared
            .mir
            .routines
            .iter()
            .find(|r| r.name == name)
            .unwrap();
        proof::placement_summary(
            &compiled
                .machine
                .routines
                .iter()
                .find(|m| m.id == r.id)
                .unwrap()
                .code,
        )
        .unwrap()
        .unwrap()
    };
    assert_eq!(summary("Work").backed_residences, 1);
    assert_eq!(summary("Touch").mixed_homes, 1);
    for mask in [0, 4] {
        let image = &compiled.image;
        let mut h = Harness::new(image, &caller(image.entry), mask);
        for start in [0x8100, 0x8300, 0x21fffe] {
            h.bus.map(start, &[0xa5; 12], true);
        }
        h.bus.ram[0x8108..0x810b].copy_from_slice(&0x21fffeu32.to_le_bytes()[..3]);
        h.bus.ram[0x220000..0x220002].copy_from_slice(&0x1234u16.to_le_bytes());
        h.run();
        h.guards(mask);
        assert_eq!(h.bus.value(0x21fffe, 1), 11);
        assert_eq!(h.bus.value(0x220000, 2), 0x1234);
        assert_eq!(h.bus.value(0x8302, 2), 0x1234);
    }
}

#[test]
fn pressure_keeps_twelve_distinct_live_pointer_captures_correct() {
    use actionc::mir65816::{Mir65816AddressBase as Base, Mir65816Op as Op, Mir65816Value as V};
    use actionc::nir::{ByteOffset, ByteSize, TempId};
    let mut prepared = prepare(&WORK.replace("Touch(other) ", ""), false);
    let r = prepared
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Work")
        .unwrap();
    let pointer = r.temps.iter().find(|(_, ty)| ty.pointer).unwrap().1.clone();
    let root = r.frame.parameters[0].param;
    let mut address = r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| {
            if let Op::Load { address, .. } = op {
                Some(address.clone())
            } else {
                None
            }
        })
        .unwrap();
    address.index = None;
    address.mode = actionc::mir65816::Mir65816AddressMode::LongIndirect;
    r.blocks.truncate(1);
    r.blocks[0].ops.clear();
    r.temps.clear();
    // Independently specified pointer table, followed by twelve distinct byte
    // destinations. All complete captures coexist before any consumer runs.
    for id in 0..12 {
        r.temps.push((TempId(id), pointer.clone()));
        address.base = Base::Indirect(V::Param(root));
        address.displacement = ByteOffset::new(8 + id * 3);
        r.blocks[0].ops.push(Op::Load {
            dest: TempId(id),
            width: ByteSize::new(3),
            address: address.clone(),
            volatile: false,
        });
    }
    for id in 0..12 {
        address.base = Base::Indirect(V::Temp(TempId(id), ByteSize::new(3)));
        address.displacement = ByteOffset::ZERO;
        r.blocks[0].ops.push(Op::Store {
            address: address.clone(),
            value: V::U8(id as u8 + 1),
            width: ByteSize::ONE,
            volatile: false,
        });
    }
    r.blocks[0].terminator = actionc::mir65816::Mir65816Terminator::Return {
        value: None,
        release_frame_bytes: r.frame.extent,
        form: r.epilogue.return_form,
        restored_mode: r.epilogue.restored_mode,
    };
    actionc::mir65816::verify_program(&prepared.mir).unwrap();
    let compiled = prepared.compile(&layout()).unwrap();
    let r = prepared
        .mir
        .routines
        .iter()
        .find(|r| r.name == "Work")
        .unwrap();
    let machine = compiled
        .machine
        .routines
        .iter()
        .find(|m| m.id == r.id)
        .unwrap();
    assert_eq!(
        proof::placement_summary(&machine.code)
            .unwrap()
            .unwrap()
            .mixed_homes,
        8
    );
    assert_eq!(
        machine
            .frame
            .temps
            .values()
            .filter(|h| matches!(h, actionc::mir65816::emit::Location::Stack(_)))
            .count(),
        4
    );
    for mask in [0, 4] {
        let image = &compiled.image;
        let mut h = Harness::new(image, &caller(image.entry), mask);
        h.bus.map(0x8100, &[0xa5; 48], true);
        let targets: Vec<u32> = (0..12)
            .map(|id| {
                if id == 0 {
                    0x21fffe
                } else {
                    0x320000 + id * 0x010101
                }
            })
            .collect();
        for (id, &at) in targets.iter().enumerate() {
            h.bus.map(at - 1, &[0xa5; 4], true);
            h.bus.ram[0x8108 + id * 3..0x810b + id * 3].copy_from_slice(&at.to_le_bytes()[..3]);
        }
        h.run();
        h.guards(mask);
        for (id, &at) in targets.iter().enumerate() {
            assert_eq!(
                &h.bus.ram[at as usize - 1..at as usize + 3],
                &[0xa5, id as u8 + 1, 0xa5, 0xa5]
            );
        }
    }
}
