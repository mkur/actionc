use super::*;

fn routine() -> Mir65816Routine {
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
