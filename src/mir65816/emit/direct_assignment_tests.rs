use super::*;

fn program(source: &str, optimize: bool) -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let nir = if optimize {
        crate::nir::optimize_program(&nir).unwrap()
    } else {
        nir
    };
    crate::mir65816::lower_program(&nir).unwrap()
}

#[test]
fn direct_assignments_emit_exact_pieces_without_temporary_traffic() {
    for optimize in [false, true] {
        for (ty, bytes) in [
            ("BYTE", 1),
            ("CARD", 2),
            ("BYTE POINTER", 3),
            ("LONGCARD", 4),
        ] {
            let p = program(&format!("{ty} p,q PROC Main() q=p RETURN"), optimize);
            let r = &p.routines[0];
            let block = &r.blocks[0];
            let frame = AllocatedFrame::new(r).unwrap();
            let plan = Plan::new(r, &frame, block, &liveness::input_counts(r), &p.data);
            assert_eq!(plan.0.len(), 1);
            let (&store, _) = plan.0.first_key_value().unwrap();
            let emitted = crate::mir65816::emit::materialize(&p).unwrap();
            let machine = &emitted.routines[0];
            assert!(machine.code.mir_spans[&(block.id, store - 1)].is_empty());
            let span = machine.code.mir_spans[&(block.id, store)].clone();
            let mut expected = vec![];
            if bytes == 1 {
                expected.extend([0xe2, 0x20]);
            }
            expected.extend([0xaf, 0, 0, 0, 0x8f, 0, 0, 0]);
            if bytes == 3 {
                expected.extend([0xe2, 0x20]);
            }
            if bytes >= 3 {
                expected.extend([0xaf, 0, 0, 0, 0x8f, 0, 0, 0]);
            }
            assert_eq!(&machine.code.bytes[span], expected, "{ty}/{optimize}");
            if bytes == 3 {
                assert_eq!(machine.frame.extent, 0);
                assert_eq!(machine.code.bytes.len(), 21); // Direct copy + RTL; no frame.
            }
            #[cfg(feature = "native65816-state-proof")]
            {
                let (reference, _) =
                    crate::mir65816::emit::proof::materialize_reference(&p, false).unwrap();
                crate::mir65816::emit::proof::compare_replay_output(
                    &reference.routines[0].code,
                    &machine.code,
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn direct_parameters_and_locals_use_their_authoritative_stack_homes() {
    let p = program(
        "BYTE POINTER p,q PROC Work(BYTE POINTER incoming) BYTE POINTER first,second first=incoming second=first q=second first=p q=first RETURN PROC Main() Work(p) RETURN",
        false,
    );
    let r = &p.routines[0];
    let frame = AllocatedFrame::new(r).unwrap();
    let counts = liveness::input_counts(r);
    let copies: Vec<_> = r
        .blocks
        .iter()
        .flat_map(|block| {
            Plan::new(r, &frame, block, &counts, &p.data)
                .0
                .into_values()
        })
        .collect();
    assert!(copies.iter().any(|a| matches!(
        (a.source, a.destination),
        (Memory::Stack(_), Memory::Stack(_))
    )));
    assert!(copies.iter().any(|a| matches!(
        (a.source, a.destination),
        (Memory::Symbol(..), Memory::Stack(_))
    )));
    assert!(copies.iter().any(|a| matches!(
        (a.source, a.destination),
        (Memory::Stack(_), Memory::Symbol(..))
    )));
    crate::mir65816::emit::materialize(&p).unwrap();
}

#[test]
fn disjoint_fields_share_an_object_but_overlapping_ranges_keep_the_capture() {
    let mut p = program("BYTE POINTER p,q PROC Main() q=p RETURN", false);
    let source = p.data.iter_mut().find(|d| d.name == "p").unwrap();
    source.size = ByteSize::new(8);
    let id = source.id;
    let Mir65816DataId::Global(symbol) = id else {
        panic!()
    };
    let r = &mut p.routines[0];
    let frame = AllocatedFrame::new(r).unwrap();
    let layout = r.clone();
    let counts = liveness::input_counts(r);
    let block = &mut r.blocks[0];
    for (offset, accepted) in [
        (0, false),
        (1, false),
        (2, false),
        (3, true),
        (5, true),
        (6, false),
    ] {
        let Mir65816Op::Store { address, .. } = &mut block.ops[1] else {
            panic!()
        };
        address.base = Mir65816AddressBase::Static(NirStorageId::Global(symbol));
        address.displacement = ByteOffset::new(offset);
        assert_eq!(
            Plan::new(&layout, &frame, block, &counts, &p.data).0.len(),
            usize::from(accepted)
        );
    }
}

#[test]
fn unsafe_or_nonadjacent_assignments_keep_the_original_load_and_store() {
    let original = program("BYTE POINTER p,q PROC Main() q=p RETURN", false);
    let frame = AllocatedFrame::new(&original.routines[0]).unwrap();
    for variant in 0..11 {
        let mut p = original.clone();
        let r = &mut p.routines[0];
        let ops = &mut r.blocks[0].ops;
        match variant {
            0 => {
                if let Mir65816Op::Load { volatile, .. } = &mut ops[0] {
                    *volatile = true;
                }
            }
            1 => {
                if let Mir65816Op::Store { volatile, .. } = &mut ops[1] {
                    *volatile = true;
                }
            }
            2 => {
                p.data[0].placement =
                    Mir65816DataPlacement::Absolute(AddressValue::new(AddressSpaceId(0), 0x7100))
            }
            3 => {
                p.data[1].placement = Mir65816DataPlacement::Alias {
                    target: p.data[0].id,
                    offset: ByteOffset::new(1),
                }
            }
            4 => p.data[1].mutable = false,
            5 => {
                let extra = ops[1].clone();
                ops.push(extra);
            }
            6 => {
                if let Mir65816Op::Load { address, .. } = &mut ops[0] {
                    address.displacement = ByteOffset::new(1);
                }
            }
            7 => {
                if let Mir65816Op::Load { address, .. } = &mut ops[0] {
                    address.index = Some(Mir65816Index {
                        value: Mir65816Value::U8(0),
                        stride: ByteSize::ONE,
                    });
                }
            }
            8 => {
                if let Mir65816Op::Load { address, .. } = &mut ops[0] {
                    address.base = Mir65816AddressBase::Indirect(Mir65816Value::GlobalAddress(
                        SymbolId(0),
                        ByteSize::new(3),
                    ));
                }
            }
            9 => {
                if let Mir65816Op::Store { width, .. } = &mut ops[1] {
                    *width = ByteSize::new(2);
                }
            }
            10 => ops.insert(
                1,
                Mir65816Op::Unary {
                    dest: TempId(999),
                    width: ByteSize::new(3),
                    operation: NirUnaryOp::Plus,
                    value: Mir65816Value::U24(0),
                },
            ),
            _ => unreachable!(),
        }
        assert!(
            Plan::new(r, &frame, &r.blocks[0], &liveness::input_counts(r), &p.data)
                .0
                .is_empty(),
            "variant {variant}"
        );
    }
}
