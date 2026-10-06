use super::*;

fn routine(source: &str) -> Mir65816Routine {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&nir).unwrap();
    let nir = crate::nir::optimize_program(&nir).unwrap();
    crate::mir65816::lower_program(&nir)
        .unwrap()
        .routines
        .into_iter()
        .find(|r| r.name == "Work")
        .unwrap()
}

#[test]
fn adjacent_long_parameter_reads_keep_frames_and_cover_both_operand_roles() {
    for ty in ["LONGCARD", "LONGINT"] {
        for op in ["+", "-", "&", "%", "XOR", "=", "#", "<", ">=", ">", "<="] {
            for swap in [false, true] {
                let constant = format!("{ty}($8123FFFF)");
                let (left, right) = if swap {
                    (constant.as_str(), "x")
                } else {
                    ("x", constant.as_str())
                };
                let result = if ["+", "-", "&", "%", "XOR"].contains(&op) {
                    ty
                } else {
                    "CARD"
                };
                let r = routine(&format!(
                    "{result} FUNC Work({ty} x) RETURN({left} {op} {right})"
                ));
                let frame = AllocatedFrame::new(&r).unwrap();
                let plan = Plan::new(&r, &frame).unwrap();
                assert_eq!(plan.bindings.len(), 1, "{ty}/{op}/{swap}");
                let binding = &plan.bindings[0];
                let machine = super::super::routine(&r, true).unwrap();
                assert!(machine.code.mir_spans[&binding.definition].is_empty());
                assert_eq!(format!("{frame:?}"), format!("{:?}", machine.frame));
                assert!(machine.frame.temps.contains_key(&binding.temp));
                assert_eq!(binding.source.home.width, 4);
            }
        }
    }
}

#[test]
fn scalar_bindings_reject_extra_uses_mutability_escape_and_noncanonical_homes() {
    let base = routine("LONGINT FUNC Work(LONGINT x) RETURN(x+LONGINT(1))");
    let frame = AllocatedFrame::new(&base).unwrap();
    let original = Plan::new(&base, &frame).unwrap();
    let binding = &original.bindings[0];
    let index = binding.definition.1;
    let consumer = binding.consumer;
    for problem in 0..9 {
        let mut r = base.clone();
        match problem {
            0 => {
                let op = r.blocks[0].ops[consumer].clone();
                r.blocks[0].ops.push(op);
            }
            1 => {
                let op = r.blocks[0].ops[index].clone();
                r.blocks[0].ops.push(op);
            }
            2 => {
                if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[index] {
                    *volatile = true;
                }
            }
            3 => {
                if let Mir65816Op::Load { address, .. } = &mut r.blocks[0].ops[index] {
                    address.displacement = ByteOffset::new(1);
                }
            }
            4 => r.frame.parameters[0].frame_object = Some(Mir65816FrameObjectId(99)),
            5 => {
                let Mir65816Op::Load { address, .. } = &r.blocks[0].ops[index] else {
                    unreachable!()
                };
                let address = address.clone();
                r.blocks[0].ops.push(Mir65816Op::Store {
                    address,
                    value: Mir65816Value::U32(0),
                    width: ByteSize::new(4),
                    volatile: false,
                });
            }
            6 => {
                let Mir65816Op::Load { address, .. } = &r.blocks[0].ops[index] else {
                    unreachable!()
                };
                let address = address.clone();
                r.blocks[0].ops.push(Mir65816Op::AddressOf {
                    dest: TempId(99),
                    address,
                    width: ByteSize::new(3),
                });
            }
            7 => {
                if let Mir65816Op::Binary { right, .. } = &mut r.blocks[0].ops[consumer] {
                    *right = Mir65816Value::Temp(binding.temp, ByteSize::new(4));
                }
            }
            _ => {
                if let Mir65816Terminator::Return { value, .. } = &mut r.blocks[0].terminator {
                    *value = Some(Mir65816Value::Temp(binding.temp, ByteSize::new(4)));
                }
            }
        }
        assert!(
            Plan::new(&r, &frame).unwrap().bindings.is_empty(),
            "{problem}"
        );
    }
    let mut forged = frame.clone();
    forged
        .temps
        .insert(binding.temp, Location::Stack(binding.source.home));
    assert!(Plan::new(&base, &forged).is_err());
    let mut forged = frame.clone();
    forged.extent = 252;
    assert!(Plan::new(&base, &forged).is_err());
}

#[test]
fn source_bindings_expire_at_the_exact_consumer_and_never_redirect_writes() {
    let r = routine("LONGINT FUNC Work(LONGINT x) RETURN(x+LONGINT(1))");
    let frame = AllocatedFrame::new(&r).unwrap();
    let plan = Plan::new(&r, &frame).unwrap();
    let binding = &plan.bindings[0];
    let mut b = Builder {
        stack_checks: false,
        routine: &r,
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
        scalar_borrowed: BTreeMap::new(),
        resident: BTreeMap::new(),
    };
    let allocated = b.temp(binding.temp).unwrap();
    let value = Mir65816Value::Temp(binding.temp, ByteSize::new(4));
    assert!(plan.enter(&mut b, binding.definition.0, binding.definition.1));
    assert!(!plan.enter(&mut b, binding.definition.0, binding.consumer));
    assert_eq!(b.temp(binding.temp).unwrap(), allocated);
    assert!(
        matches!(b.value_memory(&value).unwrap(), Some(Memory::Stack(s)) if s == u32::from(binding.source.home.offset))
    );
    assert!(!plan.enter(&mut b, binding.definition.0, binding.consumer + 1));
    assert!(
        matches!(b.value_memory(&value).unwrap(), Some(Memory::Stack(s)) if s == u32::from(allocated.slot().offset))
    );
}

#[test]
fn local_long_sources_require_complete_nonescaping_disjoint_ownership() {
    let base = routine(
        "PROC Touch() RETURN LONGINT FUNC Work(LONGINT x) LONGINT saved saved=x Touch() RETURN(LONGINT(1)+saved)",
    );
    let frame = AllocatedFrame::new(&base).unwrap();
    let plan = Plan::new(&base, &frame).unwrap();
    let binding = plan
        .bindings
        .iter()
        .find(|b| matches!(b.source.kind, SourceKind::FrameObject(_)))
        .unwrap();
    let SourceKind::FrameObject(id) = binding.source.kind else {
        unreachable!()
    };
    let machine = super::super::routine(&base, true).unwrap();
    assert!(machine.code.mir_spans[&binding.definition].is_empty());
    assert_eq!(format!("{frame:?}"), format!("{:?}", machine.frame));
    for problem in 0..6 {
        let mut r = base.clone();
        let at = r.frame.objects.iter().position(|o| o.id == id).unwrap();
        match problem {
            0 => r.frame.objects[at].addressable = true,
            1 => r.frame.objects[at].size = ByteSize::new(5),
            2 => {
                let op = r.blocks[0].ops.iter().find(|op| matches!(op, Mir65816Op::Store { address, .. } if address.base == Mir65816AddressBase::AutomaticFrame(id))).unwrap().clone();
                r.blocks[0].ops.insert(binding.consumer, op);
            }
            3 => {
                let op = r.blocks[0].ops[binding.definition.1].clone();
                let Mir65816Op::Load { address, .. } = op else {
                    unreachable!()
                };
                r.blocks[0].ops.push(Mir65816Op::AddressOf {
                    dest: TempId(99),
                    width: ByteSize::new(3),
                    address,
                });
            }
            4 => {
                for op in &mut r.blocks[0].ops {
                    if let Mir65816Op::Store {
                        address, volatile, ..
                    } = op
                    {
                        if address.base == Mir65816AddressBase::AutomaticFrame(id) {
                            *volatile = true;
                        }
                    }
                }
            }
            _ => {
                r.frame.objects[at].owner =
                    Mir65816FrameObjectOwner::Param(r.frame.parameters[0].param)
            }
        }
        assert!(
            !Plan::new(&r, &frame)
                .unwrap()
                .bindings
                .iter()
                .any(|b| b.temp == binding.temp),
            "{problem}"
        );
    }
    let mut r = base.clone();
    let mut overlap = r.frame.objects.iter().find(|o| o.id == id).unwrap().clone();
    overlap.id = Mir65816FrameObjectId(99);
    r.frame.objects.push(overlap);
    assert!(Plan::new(&r, &frame).is_err());
}

#[test]
fn terminal_calls_cover_scalar_widths_and_refuse_unreachable_or_mismatched_arguments() {
    for (ty, bytes) in [("BYTE", 1), ("CARD", 2), ("LONGINT", 4)] {
        for local in [false, true] {
            let r = routine(&format!(
                "PROC Touch() RETURN PROC Sink(BYTE tag CARD marker {ty} v) RETURN PROC Work({ty} x) {} Sink(7,$CAFE,{}) RETURN",
                if local {
                    format!("{ty} saved saved=x Touch()")
                } else {
                    String::new()
                },
                if local { "saved" } else { "x" }
            ));
            let frame = AllocatedFrame::new(&r).unwrap();
            let plan = Plan::new(&r, &frame).unwrap();
            let binding = plan
                .bindings
                .iter()
                .find(|b| matches!(r.blocks[0].ops[b.consumer], Mir65816Op::Call { .. }))
                .unwrap();
            assert_eq!(binding.source.home.width, bytes);
            assert_eq!(
                matches!(binding.source.kind, SourceKind::FrameObject(_)),
                local
            );
            let m = super::super::routine(&r, true).unwrap();
            assert!(m.code.mir_spans[&binding.definition].is_empty());
            for problem in 0..3 {
                let mut forged = r.clone();
                let Mir65816Op::Call { target, plan, .. } =
                    &mut forged.blocks[0].ops[binding.consumer]
                else {
                    unreachable!()
                };
                match problem {
                    0 => {
                        *target = Mir65816CallTarget::Indirect(
                            Mir65816Value::U24(0x123456),
                            ByteSize::new(3),
                        )
                    }
                    1 => plan.outgoing_bytes = ByteSize::new(255),
                    _ => {
                        let Mir65816AbiHome::StackArgument { size, .. } =
                            plan.arguments.last_mut().unwrap()
                        else {
                            unreachable!()
                        };
                        *size = ByteSize::new(3);
                    }
                }
                assert!(
                    !Plan::new(&forged, &frame)
                        .unwrap()
                        .bindings
                        .iter()
                        .any(|b| b.temp == binding.temp)
                );
            }
        }
    }
    // The existing sole narrow argument in A keeps its omitted home and proof.
    let r = routine("PROC Sink(CARD x) RETURN PROC Work(CARD x) Sink(x) RETURN");
    let frame = AllocatedFrame::new(&r).unwrap();
    assert!(Plan::new(&r, &frame).unwrap().bindings.is_empty());
}

#[test]
fn terminal_long_stores_refuse_volatile_partial_and_overlapping_destinations() {
    let r = routine("PROC Work(LONGCARD POINTER p LONGCARD x) p^=x RETURN");
    let frame = AllocatedFrame::new(&r).unwrap();
    let plan = Plan::new(&r, &frame).unwrap();
    assert_eq!(plan.bindings.len(), 1);
    let binding = &plan.bindings[0];
    assert!(matches!(
        r.blocks[0].ops[binding.consumer],
        Mir65816Op::Store { .. }
    ));
    let machine = super::super::routine(&r, true).unwrap();
    assert!(machine.code.mir_spans[&binding.definition].is_empty());
    for problem in 0..3 {
        let mut r = r.clone();
        let Mir65816Op::Store {
            address,
            width,
            volatile,
            ..
        } = &mut r.blocks[0].ops[binding.consumer]
        else {
            unreachable!()
        };
        match problem {
            0 => *volatile = true,
            1 => *width = ByteSize::new(2),
            _ => address.base = Mir65816AddressBase::Parameter(r.frame.parameters[1].param),
        }
        assert!(Plan::new(&r, &frame).unwrap().bindings.is_empty());
    }
}

#[test]
fn top_bit_branch_selector_keeps_its_original_long_capture() {
    for ty in ["LONGCARD", "LONGINT"] {
        for local in [false, true] {
            for swap in [false, true] {
                let value = if local { "saved" } else { "x" };
                let mask = format!("{ty}($80000000)");
                let expr = if swap {
                    format!("{mask} AND {value}")
                } else {
                    format!("{value} AND {mask}")
                };
                let r = routine(&format!(
                    "PROC Touch() RETURN BYTE FUNC Work({ty} x) {} IF ({expr})#0 THEN RETURN(17) FI RETURN(23)",
                    if local {
                        format!("{ty} saved saved=x Touch()")
                    } else {
                        String::new()
                    }
                ));
                let frame = AllocatedFrame::new(&r).unwrap();
                let plan = Plan::new(&r, &frame).unwrap();
                let counts = liveness::input_counts(&r);
                let (block, mask) =
                    r.blocks
                        .iter()
                        .find_map(|b| {
                            b.ops.iter().enumerate().find_map(|(i, _)| {
                                top_bits::owns_mask(b, i, &counts).then_some((b, i))
                            })
                        })
                        .unwrap();
                let Mir65816Op::Load { dest, .. } = block.ops[mask - 1] else {
                    panic!("adjacent private capture")
                };
                assert!(!plan.bindings.iter().any(|b| b.temp == dest));
                let m = super::super::routine(&r, true).unwrap();
                assert!(!m.code.mir_spans[&(block.id, mask - 1)].is_empty());
                assert!(m.code.mir_spans[&(block.id, mask)].is_empty());
            }
        }
    }
}
