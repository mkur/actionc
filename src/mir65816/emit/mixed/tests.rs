use super::*;

pub(in crate::mir65816::emit) fn routine() -> Mir65816Routine {
    let source = "TYPE Cell=[BYTE tag CARD value Cell POINTER next] PROC Work(Cell POINTER p) p=p.next RETURN PROC Main() RETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let mut r = crate::mir65816::lower_program(&nir)
        .unwrap()
        .routines
        .remove(0);
    let pointer = r.temps.iter().find(|(_, ty)| ty.pointer).unwrap().1.clone();
    let word = super::super::select::word_tests::program().routines[0].temps[0]
        .1
        .clone();
    r.frame.objects.clear();
    r.frame.extent = ByteSize::ZERO;
    r.frame.parameters[0].frame_object = None;
    r.temps = vec![(TempId(0), pointer), (TempId(1), word)];
    let mut input = match &r.blocks[0].ops[0] {
        Mir65816Op::Load { address, .. } => address.clone(),
        _ => panic!(),
    };
    input.base = Mir65816AddressBase::Indirect(Mir65816Value::Param(r.frame.parameters[0].param));
    input.displacement = ByteOffset::new(4);
    let mut field = input.clone();
    field.base = Mir65816AddressBase::Indirect(Mir65816Value::Temp(TempId(0), ByteSize::new(3)));
    field.displacement = ByteOffset::new(2);
    let mut tag = field.clone();
    tag.displacement = ByteOffset::ZERO;
    r.blocks[0].ops = vec![
        Mir65816Op::Load {
            dest: TempId(0),
            width: ByteSize::new(3),
            address: input,
            volatile: false,
        },
        Mir65816Op::Load {
            dest: TempId(1),
            width: ByteSize::new(2),
            address: field.clone(),
            volatile: false,
        },
        Mir65816Op::Store {
            address: field,
            value: Mir65816Value::Temp(TempId(1), ByteSize::new(2)),
            width: ByteSize::new(2),
            volatile: false,
        },
        Mir65816Op::Store {
            address: tag,
            value: Mir65816Value::U8(7),
            width: ByteSize::ONE,
            volatile: false,
        },
    ];
    r
}

pub(in crate::mir65816::emit) fn diamond() -> Mir65816Routine {
    let mut r = routine();
    let pointer = r.temps[0].1.clone();
    let word = r.temps[1].1.clone();
    r.temps.extend([
        (TempId(2), word.clone()),
        (TempId(3), pointer),
        (TempId(4), word),
    ]);
    let old = r.blocks[0].clone();
    let args = |word| Mir65816Edge {
        target: BlockId(3),
        args: vec![
            Mir65816Value::Temp(TempId(0), ByteSize::new(3)),
            Mir65816Value::Temp(TempId(word), ByteSize::new(2)),
        ],
    };
    let mut load = old.ops[1].clone();
    if let Mir65816Op::Load { dest, .. } = &mut load {
        *dest = TempId(2);
    }
    let mut store = old.ops[2].clone();
    if let Mir65816Op::Store { address, value, .. } = &mut store {
        address.base =
            Mir65816AddressBase::Indirect(Mir65816Value::Temp(TempId(3), ByteSize::new(3)));
        *value = Mir65816Value::Temp(TempId(4), ByteSize::new(2));
    }
    r.blocks = vec![
        Mir65816Block {
            id: BlockId(0),
            params: vec![],
            ops: old.ops[..2].to_vec(),
            terminator: Mir65816Terminator::Branch {
                condition: Mir65816Value::U8(1),
                then_edge: Mir65816Edge {
                    target: BlockId(1),
                    args: vec![],
                },
                else_edge: Mir65816Edge {
                    target: BlockId(2),
                    args: vec![],
                },
            },
        },
        Mir65816Block {
            id: BlockId(1),
            params: vec![],
            ops: vec![],
            terminator: Mir65816Terminator::Goto(args(1)),
        },
        Mir65816Block {
            id: BlockId(2),
            params: vec![],
            ops: vec![load],
            terminator: Mir65816Terminator::Goto(args(2)),
        },
        Mir65816Block {
            id: BlockId(3),
            params: vec![(TempId(3), ByteSize::new(3)), (TempId(4), ByteSize::new(2))],
            ops: vec![store],
            terminator: old.terminator,
        },
    ];
    r
}

#[test]
fn diamond_keeps_complete_homes_and_establishes_every_entry() {
    let r = diamond();
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert_eq!(p.demand.mixed.regions.len(), 5);
    assert_eq!(p.frame.extent, 0);
    assert!(p.frame.edge_copies.is_empty());
    assert_eq!(p.frame.temps[&TempId(0)], p.frame.temps[&TempId(3)]);
    assert_eq!(p.frame.temps[&TempId(1)], p.frame.temps[&TempId(4)]);
    assert_eq!(p.demand.mixed.entries[&BlockId(3)].len(), 2);
    select::routine(&r, false).unwrap();
}

#[test]
fn parallel_same_target_edges_preserve_different_arguments() {
    let mut r = diamond();
    r.temps.retain(|(id, _)| *id != TempId(2));
    r.blocks.remove(2);
    r.blocks.remove(1);
    r.blocks[0].terminator = Mir65816Terminator::Branch {
        condition: Mir65816Value::U8(1),
        then_edge: Mir65816Edge {
            target: BlockId(3),
            args: vec![
                Mir65816Value::Temp(TempId(0), ByteSize::new(3)),
                Mir65816Value::Temp(TempId(1), ByteSize::new(2)),
            ],
        },
        else_edge: Mir65816Edge {
            target: BlockId(3),
            args: vec![
                Mir65816Value::Param(r.frame.parameters[0].param),
                Mir65816Value::U16(99),
            ],
        },
    };
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert_eq!(p.demand.mixed.regions.len(), 4);
    assert!(p.frame.edge_copies.is_empty());
    assert!(select::routine(&r, false).unwrap().code.bytes.len() > 0);
}

#[test]
fn a_conflicting_predecessor_refuses_but_a_call_free_loop_establishes_residence() {
    let mut r = diamond();
    let mut barrier = routine().blocks[0].ops[3].clone();
    if let Mir65816Op::Store { volatile, .. } = &mut barrier {
        *volatile = true;
    }
    r.blocks[2].ops.push(barrier);
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert!(!p.demand.mixed.regions.contains_key(&TempId(0)));
    assert!(matches!(p.frame.temps[&TempId(0)], Location::Stack(_)));
    select::routine(&r, false).unwrap();

    let mut r = routine();
    r.temps.truncate(1);
    let old = r.blocks[0].clone();
    r.blocks[0].ops.truncate(1);
    r.blocks[0].terminator = Mir65816Terminator::Goto(Mir65816Edge {
        target: BlockId(1),
        args: vec![],
    });
    r.blocks.push(Mir65816Block {
        id: BlockId(1),
        params: vec![],
        ops: vec![old.ops[3].clone()],
        terminator: Mir65816Terminator::Branch {
            condition: Mir65816Value::U8(1),
            then_edge: Mir65816Edge {
                target: BlockId(1),
                args: vec![],
            },
            else_edge: Mir65816Edge {
                target: BlockId(2),
                args: vec![],
            },
        },
    });
    r.blocks.push(Mir65816Block {
        id: BlockId(2),
        params: vec![],
        ops: vec![old.ops[3].clone()],
        terminator: old.terminator,
    });
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert!(p.demand.mixed.loops.contains(&TempId(0)));
    assert!(matches!(p.frame.temps[&TempId(0)], Location::DirectPage(_)));
    select::routine(&r, false).unwrap();
}

#[test]
fn forged_branch_region_or_entry_is_rejected() {
    let r = diamond();
    for bad in 0..3 {
        let mut p = placement::Plan::new(&r, &[]).unwrap();
        match bad {
            0 => {
                p.demand.mixed.regions.remove(&TempId(0));
            }
            1 => {
                p.demand.mixed.entries.remove(&BlockId(3));
            }
            _ => {
                p.demand
                    .mixed
                    .entries
                    .get_mut(&BlockId(3))
                    .unwrap()
                    .remove(&TempId(3));
            }
        }
        assert!(p.verify().is_err());
    }
}

#[test]
fn edge_only_preheader_captures_keep_unsupported_loop_parameter_stack_affinity() {
    let mut r = routine();
    r.temps.push((TempId(2), r.temps[0].1.clone()));
    let mut load = r.blocks[0].ops[1].clone();
    if let Mir65816Op::Load { address, .. } = &mut load {
        address.base =
            Mir65816AddressBase::Indirect(Mir65816Value::Param(r.frame.parameters[0].param));
    }
    r.blocks[0].ops = vec![r.blocks[0].ops[0].clone(), load];
    r.blocks[0].terminator = Mir65816Terminator::Goto(Mir65816Edge {
        target: BlockId(1),
        args: vec![Mir65816Value::Temp(TempId(0), ByteSize::new(3))],
    });
    let mut store = routine().blocks[0].ops[3].clone();
    if let Mir65816Op::Store {
        address, volatile, ..
    } = &mut store
    {
        *volatile = true;
        address.base =
            Mir65816AddressBase::Indirect(Mir65816Value::Temp(TempId(2), ByteSize::new(3)));
    }
    r.blocks.push(Mir65816Block {
        id: BlockId(1),
        params: vec![(TempId(2), ByteSize::new(3))],
        ops: vec![store],
        terminator: Mir65816Terminator::Goto(Mir65816Edge {
            target: BlockId(1),
            args: vec![Mir65816Value::Temp(TempId(2), ByteSize::new(3))],
        }),
    });
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert!(p.demand.mixed.regions.is_empty());
    assert!(matches!(p.frame.temps[&TempId(0)], Location::Stack(_)));
    select::routine(&r, false).unwrap();
}

#[test]
fn mixed_complete_captures_share_one_checked_block_plan() {
    let r = routine();
    assert!(AllocatedFrame::pointer_leaf(&r).unwrap().is_none());
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert_eq!(p.demand.mixed.values.len(), 1);
    assert!(p.demand.accumulator(TempId(1)).is_some());
    assert!(
        p.frame
            .temps
            .values()
            .all(|h| matches!(h, Location::DirectPage(_)))
    );
    assert_eq!(
        (
            p.frame.extent,
            p.frame.spill_bytes,
            p.frame.peak_below_entry
        ),
        (0, 0, 0)
    );
    p.verify().unwrap();
    let m = select::routine(&r, false).unwrap();
    assert_eq!(m.frame, p.frame);
}

pub(in crate::mir65816::emit) fn calling() -> Mir65816Routine {
    let mut r = routine();
    let ast = crate::parser::parse(
        &crate::lexer::tokenize("PROC Touch() RETURN PROC Other() Touch() RETURN").unwrap(),
    )
    .unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let p = crate::mir65816::lower_program(&nir).unwrap();
    let call = p
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .find(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap()
        .clone();
    let store = r.blocks[0].ops[3].clone();
    let mut other = store.clone();
    if let Mir65816Op::Store { address, .. } = &mut other {
        address.base =
            Mir65816AddressBase::Indirect(Mir65816Value::Param(r.frame.parameters[0].param));
    }
    let mut ops = r.blocks[0].ops[..3].to_vec();
    for _ in 0..3 {
        ops.push(other.clone());
        ops.push(store.clone());
    }
    ops.push(call);
    for _ in 0..3 {
        ops.push(store.clone());
        ops.push(other.clone());
    }
    r.blocks[0].ops = ops;
    r
}

#[test]
fn call_segments_reestablish_complete_captures_from_authoritative_stack_homes() {
    let r = calling();
    let p = placement::Plan::new(&r, &[]).unwrap();
    let id = TempId(0);
    assert!(matches!(p.frame.temps[&id], Location::Stack(_)));
    assert_eq!(p.demand.mixed.segments[&id].len(), 1);
    let call = r.blocks[0]
        .ops
        .iter()
        .position(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap();
    assert!(
        p.demand
            .mixed
            .cache(
                id,
                ProgramPoint {
                    block: r.blocks[0].id,
                    index: call
                }
            )
            .is_none()
    );
    let v = p.demand.mixed.segments[&id][0];
    assert!(v.first > call && v.slot.width == 3);
    select::routine(&r, false).unwrap();
    for bad in 0..3 {
        let mut p = placement::Plan::new(&r, &[]).unwrap();
        let v = &mut p.demand.mixed.segments.get_mut(&id).unwrap()[0];
        match bad {
            0 => v.first = call,
            1 => v.slot.width = 2,
            _ => v.slot.offset += 1,
        }
        assert!(p.verify().is_err());
    }
}

pub(in crate::mir65816::emit) fn looping() -> Mir65816Routine {
    let mut r = diamond();
    r.blocks[3].terminator = Mir65816Terminator::Branch {
        condition: Mir65816Value::U8(0),
        then_edge: Mir65816Edge {
            target: BlockId(3),
            args: vec![
                Mir65816Value::Temp(TempId(3), ByteSize::new(3)),
                Mir65816Value::Temp(TempId(4), ByteSize::new(2)),
            ],
        },
        else_edge: Mir65816Edge {
            target: BlockId(4),
            args: vec![],
        },
    };
    r.blocks.push(Mir65816Block {
        id: BlockId(4),
        params: vec![],
        ops: vec![],
        terminator: routine().blocks[0].terminator.clone(),
    });
    r
}

#[test]
fn fixed_point_loop_entries_include_initial_backedge_and_exit_bindings() {
    let r = looping();
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert!(p.demand.mixed.loops.contains(&TempId(3)) && p.demand.mixed.loops.contains(&TempId(4)));
    assert_eq!(p.demand.mixed.entries[&BlockId(3)].len(), 2);
    select::routine(&r, false).unwrap();
    let mut bad = placement::Plan::new(&r, &[]).unwrap();
    bad.demand.mixed.loops.remove(&TempId(3));
    assert!(bad.verify().is_err());
}

#[test]
fn loop_pressure_keeps_unplaced_captures_in_complete_stack_homes() {
    let mut r = looping();
    let pointer = r.temps[0].1.clone();
    let root = r.frame.parameters[0].param;
    for id in 5..17 {
        r.temps.push((TempId(id), pointer.clone()));
        r.blocks[3].params.push((TempId(id), ByteSize::new(3)));
        let mut store = r.blocks[3].ops[0].clone();
        if let Mir65816Op::Store {
            address,
            value,
            width,
            ..
        } = &mut store
        {
            address.base =
                Mir65816AddressBase::Indirect(Mir65816Value::Temp(TempId(id), ByteSize::new(3)));
            *value = Mir65816Value::U8(7);
            *width = ByteSize::ONE;
        }
        r.blocks[3].ops.push(store);
    }
    for block in &mut r.blocks {
        match &mut block.terminator {
            Mir65816Terminator::Goto(edge) if edge.target == BlockId(3) => edge
                .args
                .extend((5..17).map(|_| Mir65816Value::Param(root))),
            Mir65816Terminator::Branch { then_edge, .. } if then_edge.target == BlockId(3) => {
                then_edge.args.extend((5..17).map(|id| {
                    Mir65816Value::Temp(TempId(if id == 16 { 5 } else { id + 1 }), ByteSize::new(3))
                }))
            }
            _ => (),
        }
    }
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert!(!p.demand.mixed.loops.is_empty());
    let stack = (5..17)
        .filter(|id| {
            matches!(
                p.frame.temps[&TempId(*id)],
                Location::Stack(Slot { width: 3, .. })
            )
        })
        .count();
    assert!(stack >= 4 && stack < 12);
    p.verify().unwrap();
    select::routine(&r, false).unwrap();
}

#[test]
fn barrier_restores_authoritative_stack_reads_and_capture_is_replayed() {
    let mut r = routine();
    let mut other = r.blocks[0].ops[3].clone();
    if let Mir65816Op::Store { address, .. } = &mut other {
        address.base =
            Mir65816AddressBase::Indirect(Mir65816Value::Param(r.frame.parameters[0].param));
    }
    r.blocks[0].ops.insert(2, other.clone());
    r.blocks[0].ops.insert(4, other);
    let mut barrier = r.blocks[0].ops[3].clone();
    if let Mir65816Op::Store { volatile, .. } = &mut barrier {
        *volatile = true;
    }
    r.blocks[0].ops.push(barrier);
    let p = placement::Plan::new(&r, &[]).unwrap();
    let v = p.demand.mixed.values[&TempId(0)];
    assert!(v.backed);
    assert_eq!((v.first, v.last), (0, 5));
    assert!(matches!(p.frame.temps[&TempId(0)], Location::Stack(_)));
    assert!(
        p.demand
            .mixed
            .cache(
                TempId(0),
                ProgramPoint {
                    block: v.block,
                    index: 5
                }
            )
            .is_some()
    );
    assert!(
        p.demand
            .mixed
            .cache(
                TempId(0),
                ProgramPoint {
                    block: v.block,
                    index: 6
                }
            )
            .is_none()
    );
    let m = select::routine(&r, false).unwrap();
    assert_eq!(m.frame, p.frame);
}

#[test]
fn late_uses_and_pressure_keep_complete_stack_homes() {
    let mut r = routine();
    let load = r.blocks[0].ops[0].clone();
    let store = r.blocks[0].ops[3].clone();
    let ty = r.temps[0].1.clone();
    r.temps.truncate(1);
    r.blocks[0].ops.clear();
    for id in 0..12 {
        if id != 0 {
            r.temps.push((TempId(id), ty.clone()));
        }
        let mut op = load.clone();
        if let Mir65816Op::Load { dest, .. } = &mut op {
            *dest = TempId(id);
        }
        r.blocks[0].ops.push(op);
    }
    for id in 0..12 {
        let mut op = store.clone();
        if let Mir65816Op::Store { address, .. } = &mut op {
            address.base =
                Mir65816AddressBase::Indirect(Mir65816Value::Temp(TempId(id), ByteSize::new(3)));
        }
        r.blocks[0].ops.push(op);
    }
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert_eq!(p.demand.mixed.values.len(), 8); // Closed, aligned complete three-byte slots.
    assert_eq!(
        p.frame
            .temps
            .values()
            .filter(|h| matches!(h, Location::Stack(_)))
            .count(),
        4
    );
    p.verify().unwrap();
    select::routine(&r, false).unwrap();
}

#[test]
fn forged_interval_transfer_or_pool_geometry_cannot_publish() {
    let r = routine();
    for bad in 0..4 {
        let mut p = placement::Plan::new(&r, &[]).unwrap();
        let v = p.demand.mixed.values.get_mut(&TempId(0)).unwrap();
        match bad {
            0 => v.last += 1,
            1 => v.slot.width = 2,
            2 => v.slot.offset = resources::PTR.into(),
            _ => v.backed = true,
        }
        assert!(p.verify().is_err());
    }
}

#[test]
fn repeated_uses_of_one_prepared_base_do_not_pay_for_a_backed_cache() {
    let mut r = routine();
    let mut barrier = r.blocks[0].ops[2].clone();
    if let Mir65816Op::Store { volatile, .. } = &mut barrier {
        *volatile = true;
    }
    r.blocks[0].ops.push(barrier);
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert!(!p.demand.mixed.values.contains_key(&TempId(0)));
    assert!(matches!(p.frame.temps[&TempId(0)], Location::Stack(_)));
    p.verify().unwrap();
}

#[test]
fn unreachable_definitions_cannot_establish_residence() {
    let mut r = routine();
    let mut dead = r.blocks[0].clone();
    dead.id = BlockId(1);
    r.blocks[0].ops.clear();
    r.blocks.push(dead);
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert!(p.demand.mixed.values.is_empty());
    assert!(matches!(p.frame.temps[&TempId(0)], Location::Stack(_)));
    p.verify().unwrap();
}

#[test]
fn existing_top_bit_region_keeps_its_capture_ownership() {
    let source =
        "BYTE FUNC Work(CARD x) IF (x AND $8000)#0 THEN RETURN(1) FI RETURN(0) PROC Main() RETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let nir = crate::nir::optimize_program_with_promotion(
        &nir,
        crate::nir::NirPromotionPolicy::Native65816,
    )
    .unwrap();
    let r = crate::mir65816::lower_program(&nir)
        .unwrap()
        .routines
        .remove(0);
    let p = placement::Plan::new(&r, &[]).unwrap();
    assert!(p.demand.mixed.values.is_empty());
    let machine = select::routine(&r, false).unwrap();
    let (block, index) = r
        .blocks
        .iter()
        .find_map(|b| {
            b.ops
                .iter()
                .position(|op| {
                    matches!(
                        op,
                        Mir65816Op::Binary {
                            operation: NirBinaryOp::And,
                            ..
                        }
                    )
                })
                .map(|i| (b.id, i))
        })
        .unwrap();
    assert!(machine.code.mir_spans[&(block, index)].is_empty());
}
