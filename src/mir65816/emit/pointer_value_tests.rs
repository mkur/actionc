use super::*;

fn program() -> Mir65816Program {
    let source = "ADDRESS FUNC Echo(ADDRESS p) RETURN(p) ADDRESS FUNC Mutate(ADDRESS p) p=ADDRESS(0) RETURN(p) PROC Main() RETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&nir).unwrap();
    crate::mir65816::lower_program(&nir).unwrap()
}
fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let frame = AllocatedFrame::stack(routine).unwrap();
    Builder {
        routine,
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
    }
}
fn input(b: &mut Builder<'_>, src: Location, dst: Location) -> Mir65816Value {
    b.frame.temps.insert(TempId(998), src);
    b.frame.temps.insert(TempId(999), dst);
    Mir65816Value::Temp(TempId(998), ByteSize::new(3))
}
fn stack(offset: u16) -> Location {
    Location::Stack(Slot { offset, width: 3 })
}
fn dp(offset: u16) -> Location {
    Location::DirectPage(Slot { offset, width: 3 })
}
fn encoding(memory: Memory, load: bool, byte: u8) -> [u8; 2] {
    match memory {
        Memory::Stack(at) => [if load { 0xa3 } else { 0x83 }, at as u8 + byte],
        Memory::DirectPage(at) => [if load { 0xa5 } else { 0x85 }, at as u8 + byte],
        _ => panic!(),
    }
}
#[test]
fn native_pointer_casts_copy_exact_private_extents_in_both_entry_widths() {
    let p = program();
    for r in &p.routines[..2] {
        for (src, dst) in [
            (stack(250), stack(253)),
            (stack(253), stack(253)),
            (dp(61), dp(0)),
            (dp(0), dp(0)),
            (stack(253), dp(61)),
            (dp(61), stack(253)),
        ] {
            for byte in [false, true] {
                let mut b = builder(r);
                let value = input(&mut b, src, dst);
                if byte {
                    b.code.a8();
                } else {
                    b.code.a16();
                }
                let at = b.code.position();
                let frame = b.frame.clone();
                assert!(b.pointer_cast(TempId(999), &value).unwrap());
                let mut expected = if byte { vec![0xc2, 0x20] } else { vec![] };
                for offset in [0, 1] {
                    expected.extend(encoding(src.into(), true, offset));
                    expected.extend(encoding(dst.into(), false, offset));
                }
                assert_eq!(&b.code.code().bytes[at..], expected);
                assert_eq!(b.frame.temps, frame.temps);
                assert_eq!(b.frame.extent, frame.extent);
            }
        }
        let mut b = builder(r);
        b.frame.temps.insert(TempId(999), dp(0));
        let value = Mir65816Value::Param(r.frame.parameters[0].param);
        let memory = b.value_memory(&value).unwrap().unwrap();
        b.code.a16();
        let at = b.code.position();
        assert!(b.pointer_cast(TempId(999), &value).unwrap());
        let mut expected = Vec::new();
        for offset in [0, 1] {
            expected.extend(encoding(memory, true, offset));
            expected.extend(encoding(dp(0).into(), false, offset));
        }
        assert_eq!(&b.code.code().bytes[at..], expected);
    }
}
#[test]
fn pointer_copy_rejects_incomplete_homes_atomically_and_keeps_overlap_fallback() {
    let p = program();
    for (src, dst, delta, error) in [
        (stack(0), stack(10), 0, true),
        (stack(254), stack(10), 0, true),
        (stack(253), stack(10), 1, true),
        (dp(62), stack(10), 0, true),
        (stack(10), dp(62), 0, true),
        (stack(10), stack(254), 0, true),
        (stack(10), stack(11), 0, false),
        (dp(0), dp(2), 0, false),
    ] {
        let mut b = builder(&p.routines[0]);
        let value = input(&mut b, src, dst);
        b.code.test_delta(delta);
        b.code.a8();
        let before = format!("{:?}", b.code);
        let result = b.pointer_cast(TempId(999), &value);
        if error {
            assert!(result.is_err());
        } else {
            assert_eq!(result, Ok(false));
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
    for value in [
        Mir65816Value::U24(0x123456),
        Mir65816Value::Null(ByteSize::new(3)),
        Mir65816Value::RoutineAddress(0, ByteSize::new(3)),
        Mir65816Value::U16(0xffff),
    ] {
        let mut b = builder(&p.routines[0]);
        b.frame.temps.insert(TempId(999), stack(10));
        assert!(!b.pointer_cast(TempId(999), &value).unwrap());
        assert!(b.code.code().bytes.is_empty());
    }
}

fn address(value: Mir65816Value, offset: u32) -> Mir65816Address {
    Mir65816Address {
        base: Mir65816AddressBase::Indirect(value),
        displacement: ByteOffset::new(offset),
        index: None,
        mode: Mir65816AddressMode::LongIndirect,
    }
}
#[test]
fn zero_offset_address_formation_copies_disjoint_captures_without_scratch() {
    let p = program();
    for (src, dst) in [
        (stack(250), stack(253)),
        (dp(0), dp(3)),
        (stack(253), dp(61)),
        (dp(61), stack(253)),
    ] {
        for byte in [false, true] {
            let mut b = builder(&p.routines[0]);
            let value = input(&mut b, src, dst);
            if byte {
                b.code.a8();
            } else {
                b.code.a16();
            }
            let at = b.code.position();
            assert!(b.pointer_address(TempId(999), &address(value, 0)).unwrap());
            let mut expected = if byte { vec![0xc2, 0x20] } else { vec![] };
            for i in [0, 1] {
                expected.extend(encoding(src.into(), true, i));
                expected.extend(encoding(dst.into(), false, i));
            }
            assert_eq!(&b.code.code().bytes[at..], expected);
        }
    }
}
#[test]
fn zero_offset_address_formation_keeps_unsupported_bases_and_geometry_atomic() {
    let p = program();
    for problem in 0..8 {
        let mut b = builder(&p.routines[0]);
        let value = input(&mut b, stack(10), stack(20));
        let mut addr = address(value, 0);
        match problem {
            0 => addr.displacement = ByteOffset::new(65536),
            1 => {
                addr.index = Some(Mir65816Index {
                    value: Mir65816Value::U16(1),
                    stride: ByteSize::ONE,
                })
            }
            2 => {
                addr.base = Mir65816AddressBase::Parameter(p.routines[0].frame.parameters[0].param)
            }
            3 => addr.base = Mir65816AddressBase::Indirect(Mir65816Value::U24(0x123456)),
            4 => {
                b.frame.temps.insert(TempId(999), stack(10));
            }
            5 => {
                b.frame.temps.insert(TempId(999), stack(11));
            }
            6 => {
                b.frame.temps.insert(TempId(998), stack(254));
            }
            7 => {
                b.frame.temps.insert(TempId(999), dp(62));
            }
            _ => unreachable!(),
        }
        b.code.a8();
        let before = format!("{:?}", b.code);
        let result = b.pointer_address(TempId(999), &addr);
        if problem >= 6 {
            assert!(result.is_err());
        } else {
            assert_eq!(result, Ok(false));
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
}

#[test]
fn constant_captured_addresses_select_a_word_and_one_bank_byte() {
    let p = program();
    for offset in [1, 3, 255, 256, 65535] {
        for (src, dst) in [
            (stack(250), stack(253)),
            (dp(0), dp(3)),
            (stack(253), dp(61)),
        ] {
            for byte in [false, true] {
                let mut b = builder(&p.routines[0]);
                let value = input(&mut b, src, dst);
                if byte {
                    b.code.a8();
                } else {
                    b.code.a16();
                }
                let at = b.code.position();
                assert!(
                    b.pointer_address(TempId(999), &address(value, offset))
                        .unwrap()
                );
                let mut expected = if byte { vec![0xc2, 0x20] } else { vec![] };
                expected.extend(encoding(src.into(), true, 0));
                expected.extend([0x18, 0x69, offset as u8, (offset >> 8) as u8]);
                expected.extend(encoding(dst.into(), false, 0));
                expected.extend([0xe2, 0x20]);
                expected.extend(encoding(src.into(), true, 2));
                expected.extend([0x69, 0]);
                expected.extend(encoding(dst.into(), false, 2));
                assert_eq!(&b.code.code().bytes[at..], expected);
            }
        }
    }
}

fn edge_routine() -> Mir65816Routine {
    let mut r = program().routines.remove(0);
    r.temps.push((TempId(999), r.temps[0].1.clone()));
    r.blocks.push(Mir65816Block {
        id: BlockId(99),
        params: vec![(TempId(999), ByteSize::new(3))],
        ops: vec![],
        terminator: r.blocks[0].terminator.clone(),
    });
    r
}
#[test]
fn single_pointer_edges_preserve_hidden_b_and_repair_identity_flags() {
    let r = edge_routine();
    for (src, dst) in [
        (stack(10), stack(20)),
        (dp(0), dp(3)),
        (stack(10), stack(10)),
    ] {
        let mut b = builder(&r);
        let value = input(&mut b, src, dst);
        b.frame.edge_copies = vec![Slot {
            offset: 80,
            width: 2,
        }];
        b.blocks.insert(BlockId(99), b.code.label());
        let edge = Mir65816Edge {
            target: BlockId(99),
            args: vec![value],
        };
        b.code.a16();
        let at = b.code.position();
        let plan = b.frame.pointer_copies(&r, &edge, 0).unwrap().unwrap();
        assert_eq!(
            b.frame.pointer_staging(&plan, 0).unwrap(),
            if src == dst { None } else { Some(80) }
        );
        assert!(b.pointer_edge(&edge, false).unwrap());
        let mut expected = vec![];
        if src != dst {
            expected.extend([0x83, 80]);
            for i in [0, 1] {
                expected.extend(encoding(src.into(), true, i));
                expected.extend(encoding(dst.into(), false, i));
            }
            expected.extend([0xa3, 80]);
        }
        expected.extend([0xe2, 0x20]);
        expected.extend(encoding(dst.into(), true, 2));
        expected.extend([0xc2, 0x20, 0x5c, 0, 0, 0]);
        assert_eq!(&b.code.code().bytes[at..], expected);
    }
}
#[test]
fn single_pointer_edge_preflight_checks_geometry_and_exact_state_staging() {
    let r = edge_routine();
    for problem in 0..8 {
        let mut b = builder(&r);
        let value = input(&mut b, stack(10), stack(20));
        b.blocks.insert(BlockId(99), b.code.label());
        b.frame.edge_copies = vec![Slot {
            offset: 80,
            width: 2,
        }];
        let mut edge = Mir65816Edge {
            target: BlockId(99),
            args: vec![value],
        };
        match problem {
            0 => {
                b.frame.temps.insert(TempId(999), stack(11));
            }
            1 => edge.args[0] = Mir65816Value::Null(ByteSize::new(3)),
            2 => b.frame.edge_copies.clear(),
            3 => b.frame.edge_copies[0].offset = 12,
            4 => b.frame.edge_copies[0].offset = 19,
            5 => b.frame.edge_copies[0].width = 1,
            6 => b.frame.edge_copies[0].offset = 255,
            7 => {
                b.frame.temps.insert(TempId(998), stack(254));
            }
            _ => unreachable!(),
        }
        let before = format!("{:?}", b.code);
        let result = b.pointer_edge(&edge, false);
        if problem < 2 {
            assert_eq!(result, Ok(false));
        } else {
            assert!(result.is_err(), "{problem}");
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
}

#[test]
fn captured_parameter_edges_share_exact_staging_between_allocation_and_emission() {
    for mut r in program().routines.into_iter().take(2) {
        let ty = r
            .temps
            .iter()
            .find(|(_, t)| t.width == Some(ByteSize::new(3)))
            .unwrap()
            .1
            .clone();
        r.temps.push((TempId(999), ty));
        let parameter = r.frame.parameters[0].param;
        let last = r.blocks.last_mut().unwrap();
        let mut ret = last.terminator.clone();
        let Mir65816Terminator::Return { value, .. } = &mut ret else {
            panic!()
        };
        *value = Some(Mir65816Value::Temp(TempId(999), ByteSize::new(3)));
        last.terminator = Mir65816Terminator::Goto(Mir65816Edge {
            target: BlockId(99),
            args: vec![Mir65816Value::Param(parameter)],
        });
        r.blocks.push(Mir65816Block {
            id: BlockId(99),
            params: vec![(TempId(999), ByteSize::new(3))],
            ops: vec![],
            terminator: ret,
        });
        let frame = AllocatedFrame::stack(&r).unwrap();
        assert_eq!(frame.edge_copies.len(), 1);
        assert_eq!(frame.edge_copies[0].width, 2);
        frame.verify_stack(&r).unwrap();
        for problem in 0..3 {
            let mut bad = frame.clone();
            match problem {
                0 => bad.edge_copies.clear(),
                1 => bad.edge_copies[0].width = 3,
                2 => bad.edge_copies[0].offset = bad.temps[&TempId(999)].slot().offset,
                _ => unreachable!(),
            }
            assert!(bad.verify_stack(&r).is_err());
        }
    }
}

#[test]
fn multiple_pointer_edges_reorder_complete_moves_and_repair_original_final_state() {
    let mut r = edge_routine();
    r.temps.push((TempId(997), r.temps[0].1.clone()));
    r.blocks
        .last_mut()
        .unwrap()
        .params
        .push((TempId(997), ByteSize::new(3)));
    for (sources, destinations, order) in [
        ([10, 20], [20, 30], vec![1, 0]),
        ([10, 10], [20, 30], vec![0, 1]),
        ([20, 30], [20, 30], vec![]),
        ([10, 30], [20, 30], vec![0]),
    ] {
        for byte in [false, true] {
            let mut b = builder(&r);
            let first = input(&mut b, stack(sources[0]), stack(destinations[0]));
            b.frame.temps.insert(TempId(996), stack(sources[1]));
            b.frame.temps.insert(TempId(997), stack(destinations[1]));
            b.frame.edge_copies = if order.is_empty() {
                vec![]
            } else {
                vec![Slot {
                    offset: 80,
                    width: 2,
                }]
            };
            b.blocks.insert(BlockId(99), b.code.label());
            let edge = Mir65816Edge {
                target: BlockId(99),
                args: vec![first, Mir65816Value::Temp(TempId(996), ByteSize::new(3))],
            };
            if byte {
                b.code.a8();
            } else {
                b.code.a16();
            }
            let at = b.code.position();
            assert!(b.pointer_edge(&edge, false).unwrap());
            let mut expected = if byte { vec![0xc2, 0x20] } else { vec![] };
            if !order.is_empty() {
                expected.extend([0x83, 80]);
                for &i in &order {
                    for offset in [0, 1] {
                        expected.extend(encoding(stack(sources[i]).into(), true, offset));
                        expected.extend(encoding(stack(destinations[i]).into(), false, offset));
                    }
                }
                expected.extend([0xa3, 80]);
            }
            expected.extend([
                0xe2,
                0x20,
                0xa3,
                destinations[1] as u8 + 2,
                0xc2,
                0x20,
                0x5c,
                0,
                0,
                0,
            ]);
            assert_eq!(&b.code.code().bytes[at..], expected);
        }
    }
}

#[test]
fn multiple_pointer_edges_preflight_every_home_before_mutation() {
    let mut r = edge_routine();
    r.temps.push((TempId(997), r.temps[0].1.clone()));
    r.blocks
        .last_mut()
        .unwrap()
        .params
        .push((TempId(997), ByteSize::new(3)));
    for (source, destination, stage, fallback) in [
        (20, 10, 80, true),   // cycle
        (21, 30, 80, true),   // partial source overlap
        (40, 22, 80, true),   // partial destination overlap
        (40, 30, 42, false),  // A-save overlaps a later source
        (40, 30, 29, false),  // A-save overlaps a later destination
        (254, 30, 80, false), // incomplete later source
        (40, 254, 80, false), // incomplete later destination
    ] {
        let mut b = builder(&r);
        let first = input(&mut b, stack(10), stack(20));
        b.frame.temps.insert(TempId(996), stack(source));
        b.frame.temps.insert(TempId(997), stack(destination));
        b.frame.edge_copies = vec![Slot {
            offset: stage,
            width: 2,
        }];
        b.blocks.insert(BlockId(99), b.code.label());
        let edge = Mir65816Edge {
            target: BlockId(99),
            args: vec![first, Mir65816Value::Temp(TempId(996), ByteSize::new(3))],
        };
        let before = format!("{:?}", b.code);
        let result = b.pointer_edge(&edge, false);
        if fallback {
            assert_eq!(result, Ok(false));
        } else {
            assert!(result.is_err());
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
}

#[test]
fn pointer_steps_use_native_carry_and_borrow_between_complete_private_homes() {
    let r = program().routines.remove(0);
    for (src, dst) in [
        (stack(250), stack(253)),
        (stack(10), stack(10)),
        (dp(0), dp(61)),
        (dp(3), stack(20)),
        (stack(10), dp(3)),
    ] {
        for (operation, commuted) in [
            (NirBinaryOp::Add, false),
            (NirBinaryOp::Add, true),
            (NirBinaryOp::Sub, false),
        ] {
            for byte in [false, true] {
                let mut b = builder(&r);
                let value = input(&mut b, src, dst);
                if byte {
                    b.code.a8();
                } else {
                    b.code.a16();
                }
                let at = b.code.position();
                let one = Mir65816Value::U24(1);
                let (left, right) = if commuted {
                    (&one, &value)
                } else {
                    (&value, &one)
                };
                assert!(
                    b.captured_pointer_step(TempId(999), 3, operation, left, right)
                        .unwrap()
                );
                let sub = operation == NirBinaryOp::Sub;
                let mut expected = if byte { vec![0xc2, 0x20] } else { vec![] };
                expected.extend(encoding(src.into(), true, 0));
                expected.extend([
                    if sub { 0x38 } else { 0x18 },
                    if sub { 0xe9 } else { 0x69 },
                    1,
                    0,
                ]);
                expected.extend(encoding(dst.into(), false, 0));
                expected.extend([0xe2, 0x20]);
                expected.extend(encoding(src.into(), true, 2));
                expected.extend([if sub { 0xe9 } else { 0x69 }, 0]);
                expected.extend(encoding(dst.into(), false, 2));
                assert_eq!(&b.code.code().bytes[at..], expected);
            }
        }
    }
}

#[test]
fn pointer_steps_keep_unsupported_operands_and_partial_overlap_on_the_fallback() {
    let r = program().routines.remove(0);
    for problem in 0..8 {
        let mut b = builder(&r);
        let value = input(&mut b, stack(10), stack(20));
        let (mut left, mut right, mut op, mut bytes) =
            (value.clone(), Mir65816Value::U24(1), NirBinaryOp::Add, 3);
        match problem {
            0 => right = Mir65816Value::U24(2),
            1 => right = value.clone(),
            2 => {
                left = Mir65816Value::U24(1);
                right = value;
                op = NirBinaryOp::Sub;
            }
            3 => {
                b.frame.temps.insert(TempId(999), stack(11));
            }
            4 => {
                b.frame.temps.insert(TempId(998), stack(254));
            }
            5 => {
                b.frame.temps.insert(TempId(999), dp(62));
            }
            6 => bytes = 4,
            7 => op = NirBinaryOp::Xor,
            _ => unreachable!(),
        }
        let before = format!("{:?}", b.code);
        let result = b.captured_pointer_step(TempId(999), bytes, op, &left, &right);
        if [4, 5].contains(&problem) {
            assert!(result.is_err());
        } else {
            assert_eq!(result, Ok(false));
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
}
