mod support;
use actionc::mir65816::{emit::proof, image::TemporaryHome};
use actionc_vm::native65816::{Access, Inputs};
use std::collections::BTreeSet;
use support::*;

const SOURCE: &str = "TYPE Cell=[BYTE flag CARD value Cell POINTER next,peer]\n\
BYTE mode=$7000\n\
PROC Work(Cell POINTER root BYTE choice)\n\
 LET left=root.next LET right=root.peer LET a=left.value LET b=right.value\n\
 IF choice THEN left.value=b right.value=a ELSE left.value=a right.value=b FI\n\
 root.flag=7 RETURN\n\
PROC Main() Work(Cell POINTER($8100),mode) RETURN\n";

fn execute(image: &actionc::mir65816::image::Image, choice: u8) {
    execute_case(image, choice, false);
}

fn execute_case(image: &actionc::mir65816::image::Image, choice: u8, conflicting: bool) {
    for (left, right) in [(0x8500u32, 0x8700u32), (0x21ffff, 0x32fffe)] {
        for (a, b) in [(0u16, 65535u16), (0x1234, 0x8000)] {
            for mask in [0, 4] {
                let mut h = Harness::new(image, &caller(image.entry), mask);
                for at in [0x8100, left, right] {
                    h.bus.map(at - 1, &[0xa5; 14], true);
                    h.bus.watched.extend(at..at + 12);
                }
                h.bus.ram[0x7000] = choice;
                h.bus.ram[0x8104..0x8107].copy_from_slice(&left.to_le_bytes()[..3]);
                h.bus.ram[0x8107..0x810a].copy_from_slice(&right.to_le_bytes()[..3]);
                h.bus.ram[left as usize + 2..left as usize + 4].copy_from_slice(&a.to_le_bytes());
                h.bus.ram[right as usize + 2..right as usize + 4].copy_from_slice(&b.to_le_bytes());
                h.run();
                h.guards(mask);
                let (wanted_a, wanted_b) = if choice == 0 { (a, b) } else { (b, a) };
                assert_eq!(h.bus.value(left + 2, 2), u32::from(wanted_a));
                assert_eq!(h.bus.value(right + 2, 2), u32::from(wanted_b));
                assert_eq!(h.bus.value(0x8100, 1), 7);
                for at in [0x8100, left, right] {
                    for offset in [-1i32, 1, 11, 12] {
                        assert_eq!(
                            h.bus.ram[(i64::from(at) + i64::from(offset)) as usize],
                            0xa5
                        );
                    }
                }
                let mut expected = Vec::new();
                for (start, bytes, access) in [
                    (0x8104, 3, Access::Read),
                    (0x8107, 3, Access::Read),
                    (left + 2, 2, Access::Read),
                    (right + 2, 2, Access::Read),
                ] {
                    expected.extend((start..start + bytes).map(|at| (at, access)));
                }
                if conflicting && choice != 0 {
                    expected.push((0x8100, Access::Read));
                }
                for (start, data) in [
                    (left + 2, wanted_a.to_le_bytes().to_vec()),
                    (right + 2, wanted_b.to_le_bytes().to_vec()),
                    (0x8100, vec![7]),
                ] {
                    expected.extend(
                        data.into_iter()
                            .enumerate()
                            .map(|(byte, value)| (start + byte as u32, Access::Write(value))),
                    );
                }
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|(_, at, access)| (*at, *access))
                        .collect::<Vec<_>>(),
                    expected
                );
            }
        }
    }
}

#[test]
fn branching_record_source_fallback_preserves_layout_and_exact_accesses() {
    for optimize in [false, true] {
        let p = prepare(SOURCE, optimize);
        let c = p.compile(&layout()).unwrap();
        assert_eq!(
            c.image.to_json().unwrap(),
            prepare(&SOURCE.replace('\n', "\r\n"), optimize)
                .compile(&layout())
                .unwrap()
                .image
                .to_json()
                .unwrap()
        );
        // Existing private-storage lowering keeps these source locals in
        // invocation homes. The authored graph below independently exercises
        // complete typed captures rather than broadening shared NIR policy.
        for choice in [0, 1, 255] {
            execute(&c.image, choice);
        }
    }
}

#[test]
fn same_target_mixed_edges_execute_distinct_simultaneous_bindings() {
    use actionc::mir65816::{
        Mir65816Address as Address, Mir65816AddressBase as Base, Mir65816Block as Block,
        Mir65816Edge as Edge, Mir65816Op as Op, Mir65816Terminator as Term, Mir65816Value as V,
    };
    use actionc::nir::{BlockId, ByteOffset, ByteSize, TempId};
    let mut p = prepare(SOURCE, false);
    let byte = p
        .mir
        .routines
        .iter()
        .flat_map(|r| &r.temps)
        .find(|(_, ty)| ty.width == Some(ByteSize::ONE))
        .unwrap()
        .1
        .clone();
    let r = p
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
    let param = r.frame.parameters[0].param;
    let condition = V::Param(r.frame.parameters[1].param);
    let mut prototype = r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| match op {
            Op::Load { address, .. } => Some(address.clone()),
            _ => None,
        })
        .unwrap();
    prototype.index = None;
    let place = |base: V, displacement| -> Address {
        let mut a = prototype.clone();
        a.base = Base::Indirect(base);
        a.mode = actionc::mir65816::Mir65816AddressMode::LongIndirect;
        a.displacement = ByteOffset::new(displacement);
        a
    };
    let w = ByteSize::new(2);
    let ptr = ByteSize::new(3);
    let temp = |id, bytes| V::Temp(TempId(id), bytes);
    r.frame.objects.clear();
    r.frame.extent = ByteSize::ZERO;
    r.frame.automatic_bytes = ByteSize::ZERO;
    r.frame.minimum_stack_peak = ByteSize::ZERO;
    r.prologue.reserve_bytes = ByteSize::ZERO;
    r.prologue.parameter_copies.clear();
    r.epilogue.release_bytes = ByteSize::ZERO;
    for param in &mut r.frame.parameters {
        param.frame_object = None;
        let actionc::mir65816::Mir65816AbiHome::StackArgument { offset, size, .. } = param.incoming
        else {
            panic!()
        };
        param.body_stack_offset = Some(
            actionc::mir65816::abi::stack::incoming_displacement(ByteSize::ZERO, offset, size)
                .unwrap(),
        );
    }
    r.temps = (0..8)
        .map(|i| {
            (
                TempId(i),
                if matches!(i, 0 | 1 | 4 | 6) {
                    pointer.clone()
                } else {
                    word.clone()
                },
            )
        })
        .collect();
    let ret = Term::Return {
        value: None,
        release_frame_bytes: r.frame.extent,
        form: r.epilogue.return_form,
        restored_mode: r.epilogue.restored_mode,
    };
    r.blocks = vec![
        Block {
            id: BlockId(0),
            params: vec![],
            ops: vec![
                Op::Load {
                    dest: TempId(0),
                    width: ptr,
                    address: place(V::Param(param), 4),
                    volatile: false,
                },
                Op::Load {
                    dest: TempId(1),
                    width: ptr,
                    address: place(V::Param(param), 7),
                    volatile: false,
                },
                Op::Load {
                    dest: TempId(2),
                    width: w,
                    address: place(temp(0, ptr), 2),
                    volatile: false,
                },
                Op::Load {
                    dest: TempId(3),
                    width: w,
                    address: place(temp(1, ptr), 2),
                    volatile: false,
                },
            ],
            terminator: Term::Branch {
                condition,
                then_edge: Edge {
                    target: BlockId(1),
                    args: vec![temp(0, ptr), temp(3, w), temp(1, ptr), temp(2, w)],
                },
                else_edge: Edge {
                    target: BlockId(1),
                    args: vec![temp(0, ptr), temp(2, w), temp(1, ptr), temp(3, w)],
                },
            },
        },
        Block {
            id: BlockId(1),
            params: vec![
                (TempId(4), ptr),
                (TempId(5), w),
                (TempId(6), ptr),
                (TempId(7), w),
            ],
            ops: vec![
                Op::Store {
                    address: place(temp(4, ptr), 2),
                    value: temp(5, w),
                    width: w,
                    volatile: false,
                },
                Op::Store {
                    address: place(temp(6, ptr), 2),
                    value: temp(7, w),
                    width: w,
                    volatile: false,
                },
                Op::Store {
                    address: place(V::Param(param), 0),
                    value: V::U8(7),
                    width: ByteSize::ONE,
                    volatile: false,
                },
            ],
            terminator: ret,
        },
    ];
    actionc::mir65816::verify_program(&p.mir).unwrap();
    let direct = p.clone();
    // Parallel edges, a diamond, separate returns and a conflicting predecessor.
    for variant in 0..4 {
        p = direct.clone();
        if variant != 0 {
            let r = p
                .mir
                .routines
                .iter_mut()
                .find(|r| r.name == "Work")
                .unwrap();
            let Term::Branch {
                condition,
                then_edge,
                else_edge,
            } = r.blocks[0].terminator.clone()
            else {
                panic!()
            };
            r.blocks[0].terminator = Term::Branch {
                condition,
                then_edge: Edge {
                    target: BlockId(2),
                    args: vec![],
                },
                else_edge: Edge {
                    target: BlockId(3),
                    args: vec![],
                },
            };
            if variant == 2 {
                let joined = r.blocks.remove(1);
                r.temps.retain(|(id, _)| id.0 < 4);
                for (id, edge) in [(2, then_edge), (3, else_edge)] {
                    let mut ops = joined.ops.clone();
                    let bind = |value: &mut V| {
                        if let V::Temp(temp, _) = value {
                            *value = edge.args[(temp.0 - 4) as usize].clone();
                        }
                    };
                    for op in &mut ops {
                        let Op::Store { address, value, .. } = op else {
                            panic!()
                        };
                        if let Base::Indirect(base) = &mut address.base {
                            bind(base);
                        }
                        bind(value);
                    }
                    r.blocks.push(Block {
                        id: BlockId(id),
                        params: vec![],
                        ops,
                        terminator: joined.terminator.clone(),
                    });
                }
            } else {
                r.blocks.insert(
                    1,
                    Block {
                        id: BlockId(2),
                        params: vec![],
                        ops: vec![],
                        terminator: Term::Goto(then_edge),
                    },
                );
                r.blocks.insert(
                    2,
                    Block {
                        id: BlockId(3),
                        params: vec![],
                        ops: vec![],
                        terminator: Term::Goto(else_edge),
                    },
                );
                if variant == 3 {
                    r.temps.push((TempId(8), byte.clone()));
                    r.blocks[1].ops.push(Op::Load {
                        dest: TempId(8),
                        width: ByteSize::ONE,
                        address: place(V::Param(param), 0),
                        volatile: true,
                    });
                }
            }
        }
        let c = p.compile(&layout()).unwrap();
        let work = c
            .machine
            .routines
            .iter()
            .find(|r| r.id == p.mir.routines.iter().find(|r| r.name == "Work").unwrap().id)
            .unwrap();
        let s = proof::placement_summary(&work.code).unwrap().unwrap();
        assert_eq!(s.branch_homes, if variant >= 2 { 4 } else { 8 });
        assert_eq!(s.mixed_edges, if variant == 2 { 0 } else { 2 });
        if variant < 2 {
            assert!(s.staging_bytes > 0);
        }
        if variant == 3 {
            for id in 0..4 {
                assert!(work.frame.temps[&TempId(id)].stack().is_ok());
            }
        }
        let (reference, _) = proof::materialize_reference(&p.mir, true).unwrap();
        assert_eq!(
            reference
                .routines
                .iter()
                .map(|r| &r.code.bytes)
                .collect::<Vec<_>>(),
            c.machine
                .routines
                .iter()
                .map(|r| &r.code.bytes)
                .collect::<Vec<_>>()
        );
        for choice in [0, 1, 255] {
            execute_case(&c.image, choice, variant == 3);
        }
    }
}

#[test]
fn branch_residence_survives_reentrant_irq_nmi_and_task_domains() {
    use support::context::*;
    let source = "MODULE TEST PUBLIC EXTERNAL PROC Yield()\n\
VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current\n\
TYPE Cell=[BYTE flag CARD value Cell POINTER next]\n\
TYPE Job=[Cell POINTER item BYTE done CARD result BYTE POINTER peer]\n\
CARD irqResult\n\
CARD FUNC Form(Cell POINTER root) LET p=root.next IF p.flag THEN RETURN(p.value+CARD(p.flag)) FI RETURN(p.value)\n\
CARD FUNC Dispatch(CARD saved BYTE reason) irqAck=1 irqResult=Form(Cell POINTER($9300))\n\
 IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI\n\
 taskB=saved current=0 RETURN(taskA)\n\
PROC Task(Job POINTER work) work.result=Form(work.item) work.done=1\n\
 WHILE work.peer^=0 DO Yield() OD RETURN\n\
PROC Main() RETURN ENDMODULE\n";
    for optimize in [false, true] {
        use actionc::mir65816::{
            Mir65816AbiHome as Home, Mir65816AddressBase as Base, Mir65816Block as Block,
            Mir65816Edge as Edge, Mir65816Op as Op, Mir65816Terminator as Term, Mir65816Value as V,
        };
        use actionc::nir::{BlockId, ByteOffset, ByteSize, NirBinaryOp, TempId};
        let mut prepared = prepare(source, optimize);
        let r = prepared
            .mir
            .routines
            .iter_mut()
            .find(|r| r.name == "Form" || r.name.starts_with("M_TEST_FORM_"))
            .unwrap();
        let pointer = r.temps.iter().find(|(_, ty)| ty.pointer).unwrap().1.clone();
        let word = r
            .temps
            .iter()
            .find(|(_, ty)| ty.width == Some(ByteSize::new(2)))
            .unwrap()
            .1
            .clone();
        let byte = r
            .temps
            .iter()
            .find(|(_, ty)| {
                ty.width == Some(ByteSize::ONE) && ty.kind != actionc::nir::NirTypeKind::Bool
            })
            .unwrap()
            .1
            .clone();
        let root = r.frame.parameters[0].param;
        let prototype = r
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .find_map(|op| match op {
                Op::Load { address, .. } => Some(address.clone()),
                _ => None,
            })
            .unwrap();
        let place = |base, offset| {
            let mut a = prototype.clone();
            a.base = Base::Indirect(base);
            a.displacement = ByteOffset::new(offset);
            a.index = None;
            a.mode = actionc::mir65816::Mir65816AddressMode::LongIndirect;
            a
        };
        let w = ByteSize::new(2);
        let ptr = ByteSize::new(3);
        let temp = |id, bytes| V::Temp(TempId(id), bytes);
        r.frame.objects.clear();
        r.frame.extent = ByteSize::ZERO;
        r.frame.automatic_bytes = ByteSize::ZERO;
        r.frame.minimum_stack_peak = ByteSize::ZERO;
        r.prologue.reserve_bytes = ByteSize::ZERO;
        r.prologue.parameter_copies.clear();
        r.epilogue.release_bytes = ByteSize::ZERO;
        for param in &mut r.frame.parameters {
            param.frame_object = None;
            let Home::StackArgument { offset, size, .. } = param.incoming else {
                panic!()
            };
            param.body_stack_offset = Some(
                actionc::mir65816::abi::stack::incoming_displacement(ByteSize::ZERO, offset, size)
                    .unwrap(),
            );
        }
        r.temps = (0..8)
            .map(|i| {
                (
                    TempId(i),
                    match i {
                        0 | 5 => pointer.clone(),
                        2 => byte.clone(),
                        _ => word.clone(),
                    },
                )
            })
            .collect();
        r.blocks = vec![
            Block {
                id: BlockId(0),
                params: vec![],
                ops: vec![
                    Op::Load {
                        dest: TempId(0),
                        width: ptr,
                        address: place(V::Param(root), 4),
                        volatile: false,
                    },
                    Op::Load {
                        dest: TempId(1),
                        width: w,
                        address: place(temp(0, ptr), 2),
                        volatile: false,
                    },
                    Op::Load {
                        dest: TempId(2),
                        width: ByteSize::ONE,
                        address: place(temp(0, ptr), 0),
                        volatile: false,
                    },
                    Op::Cast {
                        dest: TempId(3),
                        from: ByteSize::ONE,
                        to: w,
                        from_signed: false,
                        value: temp(2, ByteSize::ONE),
                        kind: actionc::nir::NirCastKind::Integer,
                    },
                    Op::Binary {
                        dest: TempId(4),
                        operation: NirBinaryOp::Add,
                        left: temp(1, w),
                        right: temp(3, w),
                        width: w,
                        signed: false,
                    },
                ],
                terminator: Term::Branch {
                    condition: temp(2, ByteSize::ONE),
                    then_edge: Edge {
                        target: BlockId(1),
                        args: vec![temp(0, ptr), temp(4, w)],
                    },
                    else_edge: Edge {
                        target: BlockId(1),
                        args: vec![temp(0, ptr), temp(1, w)],
                    },
                },
            },
            Block {
                id: BlockId(1),
                params: vec![(TempId(5), ptr), (TempId(6), w)],
                ops: vec![Op::Load {
                    dest: TempId(7),
                    width: w,
                    address: place(temp(5, ptr), 2),
                    volatile: false,
                }],
                terminator: Term::Return {
                    value: Some(temp(6, w)),
                    release_frame_bytes: ByteSize::ZERO,
                    form: r.epilogue.return_form,
                    restored_mode: r.epilogue.restored_mode,
                },
            },
        ];
        actionc::mir65816::verify_program(&prepared.mir).unwrap();
        let ordinary = actionc::mir65816::emit::materialize(&prepared.mir).unwrap();
        let form = ordinary
            .routines
            .iter()
            .find(|m| {
                m.id == prepared
                    .mir
                    .routines
                    .iter()
                    .find(|r| r.name == "Form" || r.name.starts_with("M_TEST_FORM_"))
                    .unwrap()
                    .id
            })
            .unwrap();
        let summary = proof::placement_summary(&form.code).unwrap().unwrap();
        assert!(summary.branch_homes >= 4);
        assert_eq!(summary.mixed_edges, 2);
        let mut h =
            ContextHarness::from_prepared(source, optimize, "Task", &[0x7100, 0x7120], prepared);
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
