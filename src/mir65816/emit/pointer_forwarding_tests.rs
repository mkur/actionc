use super::*;

#[test]
fn local_call_sources_fit_the_full_outgoing_delta_or_keep_their_captures() {
    let mut r = source_routine(
        "PROC Sink(BYTE POINTER p BYTE POINTER q) RETURN PROC Read(BYTE POINTER p) BYTE POINTER local local=p Sink(local,local) RETURN",
    );
    let mut frame = AllocatedFrame::new(&r).unwrap();
    let original = Plan::new(&r, &frame).unwrap();
    assert_eq!(original.bindings.len(), 2);
    let SourceKind::FrameObject(id) = original.bindings[0].source.kind else {
        unreachable!()
    };
    let Mir65816Op::Call { plan, .. } = r.blocks[0]
        .ops
        .iter()
        .find(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    let last = 255 - plan.outgoing_bytes.get() - 2;
    for extra in [0, 1] {
        r.frame
            .objects
            .iter_mut()
            .find(|o| o.id == id)
            .unwrap()
            .stack_offset = ByteOffset::new(last + extra);
        r.frame.extent = ByteSize::new(last + extra + 2);
        frame.extent = (last + extra + 2) as u16;
        assert_eq!(
            Plan::new(&r, &frame).unwrap().bindings.len(),
            if extra == 0 { 2 } else { 0 }
        );
    }
}

#[test]
fn local_terminal_stores_require_a_stable_window_and_fresh_capture_after_writes() {
    let base = source_routine(
        "PROC Touch() RETURN PROC Read(BYTE POINTER p BYTE v) BYTE POINTER local local=p local^=v local=p local^=0 Touch() RETURN",
    );
    let plan = Plan::new(&base, &AllocatedFrame::new(&base).unwrap()).unwrap();
    assert_eq!(plan.bindings.len(), 2);
    assert!(
        plan.bindings
            .iter()
            .all(|b| matches!(b.source.kind, SourceKind::FrameObject(_)))
    );
    let binding = &plan.bindings[0];
    let SourceKind::FrameObject(object) = binding.source.kind else {
        unreachable!()
    };
    for problem in 0..3 {
        let mut r = base.clone();
        if problem == 0 {
            r.frame
                .objects
                .iter_mut()
                .find(|o| o.id == object)
                .unwrap()
                .addressable = true;
        } else {
            let store=r.blocks[0].ops.iter().find(|op|matches!(op,Mir65816Op::Store{address,..} if address.base==Mir65816AddressBase::AutomaticFrame(object))).unwrap().clone();
            if problem == 1 {
                r.blocks[0].ops.insert(*binding.uses.last().unwrap(), store);
            } else {
                let at = *binding.uses.last().unwrap();
                let repeated = r.blocks[0].ops[at].clone();
                r.blocks[0].ops.insert(at + 1, repeated);
            }
        }
        let p = Plan::new(&r, &AllocatedFrame::new(&r).unwrap()).unwrap();
        assert!(
            !p.bindings
                .iter()
                .any(|b| b.definition == binding.definition),
            "{problem}"
        );
    }
}

fn terminal_call_routine() -> Mir65816Routine {
    source_routine(
        "PROC Sink(BYTE POINTER p BYTE POINTER q) RETURN PROC Read(BYTE POINTER p) Sink(p,p) RETURN",
    )
}

#[test]
fn final_direct_call_counts_repeated_arguments_and_expires_before_transfer() {
    let mut r = terminal_call_routine();
    let at = r.blocks[0]
        .ops
        .iter()
        .position(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap();
    let Mir65816Op::Call { args, .. } = &mut r.blocks[0].ops[at] else {
        unreachable!()
    };
    args[1] = args[0].clone();
    let frame = AllocatedFrame::new(&r).unwrap();
    let plan = Plan::new(&r, &frame).unwrap();
    assert_eq!(plan.bindings.len(), 1);
    assert_eq!(plan.bindings[0].uses, [at].into());
    let mut b = Builder {
        routine: &r,
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
    };
    assert!(!plan.enter(&mut b, r.blocks[0].id, at));
    assert_eq!(b.borrowed.len(), 1);
    b.operation(&r.blocks[0].ops[at]).unwrap();
    assert!(b.borrowed.is_empty());
    assert_eq!(b.code.delta(), 0);
}

#[test]
fn final_call_falls_back_when_the_authoritative_source_exceeds_outgoing_reach() {
    let r = terminal_call_routine();
    let mut frame = AllocatedFrame::new(&r).unwrap();
    let Mir65816Op::Call { plan, .. } = r.blocks[0]
        .ops
        .iter()
        .find(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    let relative = frame
        .incoming_home(&r, r.frame.parameters[0].param)
        .unwrap()
        - u32::from(frame.extent);
    frame.extent = (255 - relative - plan.outgoing_bytes.get() - 2) as u16;
    assert_eq!(Plan::new(&r, &frame).unwrap().bindings.len(), 2);
    frame.extent += 1;
    // The source still fits without outgoing bytes: only the borrowed read is
    // unsupported. Its existing, lower captured slots remain valid call inputs.
    assert!(Plan::new(&r, &frame).unwrap().bindings.is_empty());
}

#[test]
fn final_call_rejects_later_uses_indirect_targets_and_width_changes() {
    let base = terminal_call_routine();
    let frame = AllocatedFrame::new(&base).unwrap();
    let at = base.blocks[0]
        .ops
        .iter()
        .position(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap();
    for problem in 0..4 {
        let mut r = base.clone();
        if problem == 0 {
            let call = r.blocks[0].ops[at].clone();
            r.blocks[0].ops.push(call);
        } else {
            let Mir65816Op::Call {
                target, args, plan, ..
            } = &mut r.blocks[0].ops[at]
            else {
                unreachable!()
            };
            match problem {
                1 => *target = Mir65816CallTarget::Indirect(args[0].clone(), ByteSize::new(3)),
                2 => {
                    for home in &mut plan.arguments {
                        let Mir65816AbiHome::StackArgument { size, .. } = home else {
                            unreachable!()
                        };
                        *size = ByteSize::new(2);
                    }
                }
                _ => plan.native = None,
            }
        }
        assert!(
            Plan::new(&r, &frame).unwrap().bindings.is_empty(),
            "{problem}"
        );
    }
}

#[test]
fn terminal_store_borrows_only_the_complete_incoming_address_base() {
    for ty in ["BYTE", "CARD", "ADDRESS", "LONGCARD"] {
        for place in ["p^", "p(3)", "p(i)"] {
            let r = source_routine(&format!(
                "PROC Touch() RETURN PROC Read({ty} POINTER p {ty} v BYTE i) {place}=v Touch() RETURN"
            ));
            let frame = AllocatedFrame::new(&r).unwrap();
            let plan = Plan::new(&r, &frame).unwrap();
            assert_eq!(plan.bindings.len(), 1, "{ty}/{place}");
            let binding = &plan.bindings[0];
            let machine = super::super::routine(&r, false).unwrap();
            assert!(machine.code.mir_spans[&binding.definition].is_empty());
            assert_eq!(format!("{:?}", frame), format!("{:?}", machine.frame));
            let store = &r.blocks[0].ops[*binding.uses.last().unwrap()];
            assert!(matches!(store, Mir65816Op::Store { .. }));
            assert!(barrier(store));
        }
    }
}

#[test]
fn terminal_store_keeps_the_capture_if_any_role_or_later_use_is_unsupported() {
    let base =
        source_routine("PROC Touch() RETURN PROC Read(BYTE POINTER p BYTE v) p^=v Touch() RETURN");
    let original = Plan::new(&base, &AllocatedFrame::new(&base).unwrap()).unwrap();
    let binding = &original.bindings[0];
    let at = *binding.uses.last().unwrap();
    for problem in 0..6 {
        let mut r = base.clone();
        let store = r.blocks[0].ops[at].clone();
        match problem {
            0 => r.blocks[0].ops.push(store), // A use after the terminal barrier.
            1 => {
                let call = r.blocks[0].ops[at + 1].clone();
                r.blocks[0].ops.insert(at, call);
            }
            2 => {
                let Mir65816Op::Store { volatile, .. } = &mut r.blocks[0].ops[at] else {
                    unreachable!()
                };
                *volatile = true;
            }
            3 => {
                let Mir65816Op::Store { value, .. } = &mut r.blocks[0].ops[at] else {
                    unreachable!()
                };
                *value = Mir65816Value::Temp(binding.temp, ByteSize::new(3));
            }
            4 => {
                let Mir65816Op::Store { address, .. } = &mut r.blocks[0].ops[at] else {
                    unreachable!()
                };
                address.index = Some(Mir65816Index {
                    value: Mir65816Value::Temp(binding.temp, ByteSize::new(3)),
                    stride: ByteSize::ONE,
                });
            }
            _ => {
                let Mir65816Op::Store { address, .. } = &mut r.blocks[0].ops[at] else {
                    unreachable!()
                };
                address.displacement = ByteOffset::new(65536);
            }
        }
        assert!(
            Plan::new(&r, &AllocatedFrame::new(&r).unwrap())
                .unwrap()
                .bindings
                .is_empty(),
            "{problem}"
        );
    }
}

#[test]
fn earlier_reads_and_final_store_share_one_incoming_binding() {
    let mut r = routine();
    let original = Plan::new(&r, &AllocatedFrame::new(&r).unwrap()).unwrap();
    let binding = &original.bindings[0];
    let read = *binding.uses.last().unwrap();
    let Mir65816Op::Load { address, .. } = &r.blocks[0].ops[read] else {
        unreachable!()
    };
    r.blocks[0].ops[read + 1] = Mir65816Op::Store {
        address: address.clone(),
        value: Mir65816Value::U8(7),
        width: ByteSize::ONE,
        volatile: false,
    };
    let plan = Plan::new(&r, &AllocatedFrame::new(&r).unwrap()).unwrap();
    assert_eq!(plan.bindings[0].uses, [read, read + 1].into());
}

fn routine() -> Mir65816Routine {
    source_routine(
        "PROC Touch() RETURN BYTE FUNC Read(BYTE POINTER p) BYTE v v=p^ Touch() RETURN(v)",
    )
}
fn source_routine(source: &str) -> Mir65816Routine {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&nir).unwrap();
    crate::mir65816::lower_program(&nir)
        .unwrap()
        .routines
        .into_iter()
        .find(|r| r.name == "Read")
        .unwrap()
}

#[test]
fn adjacent_pointer_binding_keeps_authoritative_identity_and_reserved_homes() {
    let r = routine();
    let frame = AllocatedFrame::new(&r).unwrap();
    let plan = Plan::new(&r, &frame).unwrap();
    assert_eq!(plan.bindings.len(), 1);
    let binding = &plan.bindings[0];
    assert_eq!(
        binding.source.kind,
        SourceKind::Parameter(r.frame.parameters[0].param)
    );
    assert_eq!(binding.source.home.width, 3);
    let machine = super::super::routine(&r, false).unwrap();
    assert_eq!(format!("{:?}", frame), format!("{:?}", machine.frame));
    assert!(machine.code.mir_spans[&binding.definition].is_empty());
    assert!(!machine.code.mir_spans[&(binding.definition.0, binding.definition.1 + 1)].is_empty());
}

#[test]
fn unsafe_or_incomplete_adjacent_bindings_retain_the_capture() {
    for problem in 0..9 {
        let mut r = routine();
        let original = Plan::new(&r, &AllocatedFrame::new(&r).unwrap()).unwrap();
        let binding = &original.bindings[0];
        let temp = binding.temp;
        let def = binding.definition.1;
        let consumer = r.blocks[0].ops[def + 1].clone();
        let mut frame = AllocatedFrame::new(&r).unwrap();
        match problem {
            0 => {
                if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[def] {
                    *volatile = true
                }
            }
            1 => {
                if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[def + 1] {
                    *volatile = true
                }
            }
            2 => {
                r.blocks[0].ops.push(consumer);
            }
            3 => {
                let call = r.blocks[0]
                    .ops
                    .iter()
                    .find(|op| matches!(op, Mir65816Op::Call { .. }))
                    .unwrap()
                    .clone();
                r.blocks[0].ops.insert(def + 1, call);
            }
            4 => {
                frame.temps.insert(
                    temp,
                    Location::DirectPage(Slot {
                        offset: 32,
                        width: 3,
                    }),
                );
            }
            5 => {
                if let Mir65816Op::Load { address, .. } = &mut r.blocks[0].ops[def] {
                    address.displacement = ByteOffset::new(1)
                }
            }
            6 => {
                r.frame.parameters[0].frame_object = Some(Mir65816FrameObjectId(999));
            }
            7 => {
                let mut bad = r.blocks[0].ops[def].clone();
                if let Mir65816Op::Load { width, .. } = &mut bad {
                    *width = ByteSize::new(2)
                }
                r.blocks[0].ops.push(bad);
            }
            _ => {
                let Mir65816Op::Load { address, .. } = &r.blocks[0].ops[def] else {
                    unreachable!()
                };
                let address = address.clone();
                r.blocks[0].ops.push(Mir65816Op::AddressOf {
                    dest: temp,
                    address,
                    width: ByteSize::new(3),
                });
            }
        }
        assert!(
            Plan::new(&r, &frame).unwrap().bindings.is_empty(),
            "refusal {problem}"
        );
    }
}

#[test]
fn incoming_pointer_preflight_checks_the_last_byte_at_255() {
    let r = routine();
    let mut f = AllocatedFrame::new(&r).unwrap();
    f.extent = 249;
    let plan = Plan::new(&r, &f).unwrap();
    assert_eq!(plan.bindings[0].source.home.offset, 253);
    f.extent = 250;
    assert!(Plan::new(&r, &f).is_err());
}

#[test]
fn same_block_multi_use_admission_is_atomic_across_every_use_and_barrier() {
    let base = routine();
    let original = Plan::new(&base, &AllocatedFrame::new(&base).unwrap()).unwrap();
    let def = original.bindings[0].definition.1;
    for problem in 0..7 {
        let mut r = base.clone();
        // Fresh definitions keep the positive multi-use fixture SSA-clean.
        let next = r.temps.iter().map(|(id, _)| id.0).max().unwrap() + 1;
        for (at, mut op, id) in [
            (def + 2, r.blocks[0].ops[def].clone(), next),
            (def + 3, r.blocks[0].ops[def + 1].clone(), next + 1),
        ] {
            let Mir65816Op::Load { dest, .. } = &mut op else {
                panic!()
            };
            let ty = r.temps.iter().find(|(id, _)| id == dest).unwrap().1.clone();
            *dest = TempId(id);
            r.temps.push((*dest, ty));
            r.blocks[0].ops.insert(at, op);
        }
        let frame = AllocatedFrame::new(&r).unwrap();
        match problem {
            0 => {}
            1 => {
                let call = r.blocks[0]
                    .ops
                    .iter()
                    .find(|op| matches!(op, Mir65816Op::Call { .. }))
                    .unwrap()
                    .clone();
                r.blocks[0].ops.insert(def + 2, call);
            }
            2 => {
                let store = r.blocks[0]
                    .ops
                    .iter()
                    .find(|op| matches!(op, Mir65816Op::Store { .. }))
                    .unwrap()
                    .clone();
                r.blocks[0].ops.insert(def + 2, store);
            }
            3 => {
                if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[def + 3] {
                    *volatile = true
                }
            }
            4 => {
                let tail = r.blocks[0].ops.split_off(def + 3);
                let mut other = r.blocks[0].clone();
                other.id = BlockId(999);
                other.ops = tail;
                r.blocks.push(other);
            }
            5 => {
                if let Mir65816Op::Load { address, .. } = &mut r.blocks[0].ops[def + 3] {
                    address.index = Some(Mir65816Index {
                        value: Mir65816Value::Temp(original.bindings[0].temp, ByteSize::new(3)),
                        stride: ByteSize::ONE,
                    });
                }
            }
            _ => {
                r.blocks[0].terminator = Mir65816Terminator::Goto(Mir65816Edge {
                    target: BlockId(999),
                    args: vec![Mir65816Value::Temp(
                        original.bindings[0].temp,
                        ByteSize::new(3),
                    )],
                });
            }
        }
        let plan = Plan::new(&r, &frame).unwrap();
        let binding = plan.bindings.iter().find(|b| b.definition.1 == def);
        assert_eq!(binding.is_some(), problem == 0, "case {problem}");
        if let Some(binding) = binding {
            assert_eq!(binding.uses, [def + 1, def + 3].into());
        }
    }
}

#[test]
fn native_pointer_returns_bind_but_identity_casts_keep_their_allocated_home() {
    for (source, expected) in [
        ("ADDRESS FUNC Read(ADDRESS p) RETURN(p)", 1),
        (
            "ADDRESS FUNC Read(ADDRESS p) RETURN(ADDRESS(BYTE POINTER(p)))",
            0,
        ),
    ] {
        let mut r = source_routine(source);
        if expected == 1 {
            // Raw ADDRESS returns may retain an identity Cast; construct the
            // verified direct-return shape without that excluded consumer.
            let Mir65816Op::Load { dest, .. } = r.blocks[0].ops[0] else {
                panic!()
            };
            r.blocks[0].ops.truncate(1);
            r.temps.retain(|(id, _)| *id == dest);
            let Mir65816Terminator::Return { value, .. } = &mut r.blocks[0].terminator else {
                panic!()
            };
            *value = Some(Mir65816Value::Temp(dest, ByteSize::new(3)));
        }
        let f = AllocatedFrame::new(&r).unwrap();
        let plan = Plan::new(&r, &f).unwrap();
        assert_eq!(plan.bindings.len(), expected);
        let m = super::super::routine(&r, false).unwrap();
        for b in &plan.bindings {
            assert!(m.code.mir_spans[&b.definition].is_empty());
            assert!(b.uses.contains(&r.blocks[0].ops.len()));
        }
    }
}

#[test]
fn local_sources_require_complete_unescaped_automatic_ownership() {
    let base = source_routine(
        "PROC Touch() RETURN BYTE FUNC Read(BYTE POINTER p) BYTE POINTER local BYTE v local=p v=local^ Touch() RETURN(v)",
    );
    let frame = AllocatedFrame::new(&base).unwrap();
    let plan = Plan::new(&base, &frame).unwrap();
    let binding = plan
        .bindings
        .iter()
        .find(|b| matches!(b.source.kind, SourceKind::FrameObject(_)))
        .unwrap();
    let SourceKind::FrameObject(object) = binding.source.kind else {
        unreachable!()
    };
    let definition = binding.definition;
    let temp = binding.temp;
    for problem in 0..10 {
        let mut r = base.clone();
        let at = r.frame.objects.iter().position(|o| o.id == object).unwrap();
        match problem {
            0 => {}
            1 => r.frame.objects[at].addressable = true,
            2 => {
                r.frame.objects[at].owner =
                    Mir65816FrameObjectOwner::Param(r.frame.parameters[0].param)
            }
            3 => r.frame.objects[at].size = ByteSize::new(4),
            4 => {
                if let Mir65816Op::Load { address, .. } = &mut r.blocks[0].ops[definition.1] {
                    address.displacement = ByteOffset::new(1)
                }
            }
            5 => {
                if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[definition.1] {
                    *volatile = true
                }
            }
            6 => {
                let Mir65816Op::Load { address, .. } = &r.blocks[0].ops[definition.1] else {
                    panic!()
                };
                let address = address.clone();
                r.blocks[0].ops.push(Mir65816Op::AddressOf {
                    dest: temp,
                    width: ByteSize::new(3),
                    address,
                });
            }
            _ => {
                let store=r.blocks[0].ops.iter().find(|op|matches!(op,Mir65816Op::Store{address,..} if address.base==Mir65816AddressBase::AutomaticFrame(object))).unwrap().clone();
                r.blocks[0].ops.insert(definition.1 + 1, store);
            }
        }
        assert_eq!(
            Plan::new(&r, &frame)
                .unwrap()
                .bindings
                .iter()
                .any(|b| b.definition == definition),
            problem == 0,
            "local case {problem}"
        );
    }
    let mut bad = base.clone();
    bad.frame
        .objects
        .iter_mut()
        .find(|o| o.id == object)
        .unwrap()
        .stack_offset = ByteOffset::new(frame.temps[&temp].slot().offset.into());
    assert!(Plan::new(&bad, &frame).is_err());
    let machine = super::super::routine(&base, false).unwrap();
    assert!(machine.code.mir_spans[&definition].is_empty());
}

#[test]
fn forged_parameter_immutability_does_not_hide_writes_copy_or_indexing() {
    let base = routine();
    let frame = AllocatedFrame::new(&base).unwrap();
    let plan = Plan::new(&base, &frame).unwrap();
    let binding = &plan.bindings[0];
    let Mir65816Op::Load { address, .. } = &base.blocks[0].ops[binding.definition.1] else {
        panic!()
    };
    for problem in 0..3 {
        let mut r = base.clone();
        let mut address = address.clone();
        r.blocks[0].ops.push(match problem {
            0 => Mir65816Op::Store {
                address,
                value: Mir65816Value::U24(0),
                width: ByteSize::new(3),
                volatile: false,
            },
            1 => Mir65816Op::Copy {
                source: address.clone(),
                destination: address,
                bytes: ByteSize::new(3),
                overlap_safe: true,
                source_volatile: false,
                destination_volatile: false,
            },
            _ => {
                address.index = Some(Mir65816Index {
                    value: Mir65816Value::U8(1),
                    stride: ByteSize::new(3),
                });
                Mir65816Op::Load {
                    dest: TempId(999),
                    address,
                    width: ByteSize::new(3),
                    volatile: false,
                }
            }
        });
        assert!(Plan::new(&r, &frame).unwrap().bindings.is_empty());
    }
}
