use actionc::{
    nir::{self, *},
    semantic::{self, SemanticOptions},
    target::{AddressValue, ByteOffset, ByteSize, TargetId},
};

const TARGETS: [TargetId; 4] = [
    TargetId::Atari6502,
    TargetId::Wdc65816Small,
    TargetId::Wdc65816Native,
    TargetId::Motorola68000,
];

fn lower(source: &str, target: TargetId) -> NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap();
    let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program).unwrap();
    program
}

fn main(program: &NirProgram) -> &NirRoutine {
    program.routines.iter().find(|r| r.name == "Main").unwrap()
}

fn byte_type() -> NirType {
    NirType {
        kind: NirTypeKind::Integer(NirIntegerType::U8),
        summary: "Byte".into(),
        width: Some(ByteSize::ONE),
        pointer: false,
    }
}

fn field(local: &NirLocal, offset: u32) -> NirPlace {
    NirPlace {
        kind: NirPlaceKind::Field {
            base: Box::new(NirPlace {
                kind: NirPlaceKind::Local {
                    id: local.id,
                    name: local.name.clone(),
                },
                ty: Some(local.ty.clone()),
            }),
            offset: ByteOffset::new(offset),
            ty: byte_type(),
        },
        ty: Some(byte_type()),
    }
}

#[test]
fn fresh_constructor_has_explicit_stores_and_folds_dispatch() {
    let program = lower(
        include_str!("support/fresh_maybe_byte.act"),
        TargetId::Atari6502,
    );
    let optimized = nir::optimize_program(&program).unwrap();
    let ops: Vec<_> = main(&optimized)
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .collect();
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, NirOp::Compare { .. }))
            .count(),
        0
    );
    assert!(!ops.iter().any(|op| matches!(
        op,
        NirOp::Call {
            callee: NirCallee::Fault(_),
            ..
        }
    )));
    let analysis = analyze_aggregate_regions(&program).unwrap();
    let routine = main(&program);
    let facts = analysis.routine(routine.id).unwrap();
    let stores: Vec<_> = routine
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .ops
                .iter()
                .enumerate()
                .filter_map(|(op_index, op)| {
                    if let NirOp::Store {
                        place,
                        src:
                            NirValue::IntegerConst {
                                bits,
                                ty: NirIntegerType::U8,
                            },
                        ..
                    } = op
                    {
                        facts
                            .region(
                                place,
                                ByteSize::ONE,
                                NirAggregatePoint {
                                    block: block.id,
                                    op_index,
                                },
                            )
                            .ok()
                            .map(|region| (region.memory.offset.get(), *bits))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(stores, [(1, 42), (0, 2)]);
}

#[test]
fn byte_region_census_uses_identity_and_offsets_for_all_aggregate_kinds() {
    for target in TARGETS {
        for declaration in [
            "TYPE Value=[BYTE first,second,third]",
            "TYPE Value=UNION [BYTE first CARD wider BYTE ARRAY bytes(3)]",
            "TYPE Value=VARIANT [EMPTY FULL [BYTE first,second]]",
        ] {
            let program = lower(
                &format!("{declaration} Value source PROC Main() LET saved=source RETURN"),
                target,
            );
            let routine = main(&program);
            let local = routine
                .locals
                .iter()
                .find(|l| l.purpose == NirLocalPurpose::AggregateCapture)
                .unwrap();
            let analysis = analyze_aggregate_regions(&program).unwrap();
            let facts = analysis.routine(routine.id).unwrap();
            let at = NirAggregatePoint {
                block: routine.blocks[0].id,
                op_index: 0,
            };
            assert_eq!(
                facts.address_use(NirStorageId::Local(local.id)),
                NirAggregateAddressUse::NotFormed
            );
            let a = facts.region(&field(local, 0), ByteSize::ONE, at).unwrap();
            let b = facts.region(&field(local, 1), ByteSize::ONE, at).unwrap();
            let wide = facts
                .region(&field(local, 0), ByteSize::new(2), at)
                .unwrap();
            assert!(!a.memory.overlaps(&b.memory));
            assert!(wide.memory.overlaps(&a.memory) && wide.memory.overlaps(&b.memory));
            assert_eq!(
                a,
                facts.region(&field(local, 0), ByteSize::ONE, at).unwrap()
            );
            assert!(
                facts
                    .region(&field(local, local.layout.size.get()), ByteSize::ONE, at)
                    .is_err()
            );
        }
    }
}

#[test]
fn absolute_capture_backing_blocks_region_and_privacy_proofs() {
    let mut program = lower(
        "TYPE Value=[BYTE x,y] Value source PROC Main() LET saved=source RETURN",
        TargetId::Atari6502,
    );
    let routine = program
        .routines
        .iter_mut()
        .find(|r| r.name == "Main")
        .unwrap();
    let local = routine
        .locals
        .iter_mut()
        .find(|l| l.purpose == NirLocalPurpose::AggregateCapture)
        .unwrap();
    local.backing = NirLocalBacking::Absolute(AddressValue::data(0x900));
    // The verifier rejects this before any optimizer can trust the capture marker.
    assert!(analyze_aggregate_regions(&program).is_err());
}

fn probe(target: TargetId) -> NirProgram {
    let mut p = lower(
        "TYPE Value=UNION [BYTE ARRAY bytes(8) CARD word] Value source BYTE out,flag BYTE POINTER ptr PROC Touch() RETURN PROC Main() LET saved=source Touch() RETURN",
        target,
    );
    let r = p.routines.iter_mut().find(|r| r.name == "Main").unwrap();
    r.blocks.truncate(1);
    r.blocks[0].ops.clear();
    r.blocks[0].terminator = NirTerminator::Return(None);
    r.temps.clear();
    p
}

fn capture(p: &NirProgram) -> NirLocal {
    main(p)
        .locals
        .iter()
        .find(|l| l.purpose == NirLocalPurpose::AggregateCapture)
        .unwrap()
        .clone()
}

fn store(local: &NirLocal, offset: u32, value: u8) -> NirOp {
    NirOp::Store {
        place: field(local, offset),
        src: NirValue::ConstU8(value),
        ty: byte_type(),
    }
}

fn observe(p: &NirProgram, local: &NirLocal, offset: u32, temp: u32) -> Vec<NirOp> {
    let out = p.globals.iter().find(|g| g.name == "out").unwrap();
    vec![
        NirOp::Load {
            dest: TempId(temp),
            ty: byte_type(),
            place: field(local, offset),
        },
        NirOp::Store {
            place: NirPlace {
                kind: NirPlaceKind::Global {
                    id: out.id,
                    name: out.name.clone(),
                },
                ty: Some(byte_type()),
            },
            src: NirValue::Temp {
                id: TempId(temp),
                ty: byte_type(),
            },
            ty: byte_type(),
        },
    ]
}

fn set_ops(p: &mut NirProgram, ops: Vec<NirOp>) {
    let r = p.routines.iter_mut().find(|r| r.name == "Main").unwrap();
    r.blocks[0].ops = ops;
    rebuild_temps(p);
}

fn rebuild_temps(p: &mut NirProgram) {
    let r = p.routines.iter_mut().find(|r| r.name == "Main").unwrap();
    r.temps = r
        .blocks
        .iter()
        .flat_map(|b| {
            b.params
                .iter()
                .map(|p| NirTemp {
                    id: p.dest,
                    ty: p.ty.clone(),
                    def: NirTempDef {
                        block: b.id,
                        op_index: None,
                    },
                })
                .chain(b.ops.iter().enumerate().filter_map(|(i, op)| {
                    let (id, ty) = match op {
                        NirOp::Load { dest, ty, .. }
                        | NirOp::VolatileLoad { dest, ty, .. }
                        | NirOp::AddrOf { dest, ty, .. }
                        | NirOp::Binary { dest, ty, .. }
                        | NirOp::Compare { dest, ty, .. } => (*dest, ty.clone()),
                        _ => return None,
                    };
                    Some(NirTemp {
                        id,
                        ty,
                        def: NirTempDef {
                            block: b.id,
                            op_index: Some(i),
                        },
                    })
                }))
                .collect::<Vec<_>>()
        })
        .collect();
    nir::verify_program(p).unwrap();
}

fn field_loads(p: &NirProgram) -> usize {
    main(p)
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .filter(|op| {
            matches!(
                op,
                NirOp::Load {
                    place: NirPlace {
                        kind: NirPlaceKind::Field { .. },
                        ..
                    },
                    ..
                }
            )
        })
        .count()
}

#[test]
fn exact_stores_fold_generic_bytes_and_preserve_disjoint_cells() {
    for target in TARGETS {
        for value in [0, 1, 255] {
            let mut p = probe(target);
            let local = capture(&p);
            let mut ops = vec![
                store(&local, 3, value),
                store(&local, 4, 99),
                store(&local, 3, value),
            ];
            ops.extend(observe(&p, &local, 3, 100));
            set_ops(&mut p, ops);
            let opt = nir::optimize_program(&p).unwrap();
            assert_eq!(field_loads(&opt), 0);
            assert_eq!(opt, nir::optimize_program(&opt).unwrap());
            assert!(main(&opt).blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(op, NirOp::Store { place: NirPlace { kind: NirPlaceKind::Global { .. }, .. }, src: NirValue::IntegerConst { bits, .. }, .. } if *bits == u64::from(value))));
        }
    }
}

#[test]
fn overlapping_wide_stores_invalidate_the_full_range_without_unpacking() {
    for target in TARGETS {
        for (offset, remaining) in [(0, 0), (1, 1), (2, 1), (3, 0)] {
            let mut p = probe(target);
            let local = capture(&p);
            let wide = NirType {
                kind: NirTypeKind::Integer(NirIntegerType::U16),
                summary: "Card".into(),
                width: Some(ByteSize::new(2)),
                pointer: false,
            };
            let mut place = field(&local, 1);
            place.ty = Some(wide.clone());
            if let NirPlaceKind::Field { ty, .. } = &mut place.kind {
                *ty = wide.clone();
            }
            let mut ops = vec![
                store(&local, offset, 7),
                NirOp::Store {
                    place,
                    src: NirValue::ConstU16(0x1234),
                    ty: wide,
                },
            ];
            ops.extend(observe(&p, &local, offset, 100));
            set_ops(&mut p, ops);
            assert_eq!(
                field_loads(&nir::optimize_program(&p).unwrap()),
                remaining,
                "{target:?} offset {offset}"
            );
        }
    }
}

#[test]
fn calls_including_pure_calls_and_observable_memory_are_barriers() {
    for target in TARGETS {
        for kind in 0..7 {
            let mut p = probe(target);
            let local = capture(&p);
            let touch = p.routines.iter().find(|r| r.name == "Touch").unwrap();
            let absolute = NirPlace {
                kind: NirPlaceKind::Absolute(AddressValue::data(0xD000)),
                ty: Some(byte_type()),
            };
            let boundary = match kind {
                0 => NirOp::Call {
                    callee: NirCallee::User {
                        id: touch.id,
                        name: touch.name.clone(),
                    },
                    args: vec![],
                    result: None,
                    aggregate_result: None,
                    signature: Some(touch.signature.clone()),
                    effects: NirCallEffects {
                        memory: NirMemoryEffects {
                            reads: NirMemoryAccess::None,
                            writes: NirMemoryAccess::None,
                        },
                        may_call_external: false,
                        opaque: false,
                    },
                },
                1 => NirOp::Load {
                    dest: TempId(99),
                    ty: byte_type(),
                    place: absolute,
                },
                2 => NirOp::Store {
                    place: absolute,
                    src: NirValue::ConstU8(1),
                    ty: byte_type(),
                },
                3 => NirOp::VolatileLoad {
                    dest: TempId(99),
                    ty: byte_type(),
                    place: field(&local, 4),
                },
                4 => NirOp::VolatileStore {
                    place: field(&local, 4),
                    src: NirValue::ConstU8(1),
                    ty: byte_type(),
                },
                5 => NirOp::Binary {
                    dest: TempId(99),
                    ty: byte_type(),
                    op: NirBinaryOp::Div,
                    left: NirValue::ConstU8(7),
                    right: NirValue::ConstU8(0),
                },
                _ => NirOp::CopyBytes {
                    destination: field(&local, 4),
                    source: field(&local, 5),
                    size: ByteSize::ONE,
                    source_volatile: true,
                    destination_volatile: false,
                },
            };
            let mut ops = vec![store(&local, 3, 7), boundary];
            ops.extend(observe(&p, &local, 3, 100));
            set_ops(&mut p, ops);
            assert_eq!(
                field_loads(&nir::optimize_program(&p).unwrap()),
                1,
                "{target:?} barrier {kind}"
            );
        }
    }
}

#[test]
fn loads_without_executable_initialization_remain_unknown() {
    for target in TARGETS {
        let mut p = probe(target);
        let local = capture(&p);
        let ops = observe(&p, &local, 3, 100);
        set_ops(&mut p, ops);
        assert_eq!(field_loads(&nir::optimize_program(&p).unwrap()), 1);
    }
}

fn edge(target: u32) -> NirEdge {
    NirEdge {
        target: BlockId(target),
        args: vec![],
    }
}
fn block(id: u32, ops: Vec<NirOp>, terminator: NirTerminator) -> NirBlock {
    NirBlock {
        id: BlockId(id),
        label: format!("probe{id}"),
        params: vec![],
        ops,
        terminator,
    }
}
fn condition_type() -> NirType {
    NirType {
        kind: NirTypeKind::Bool,
        summary: "condition".into(),
        width: Some(ByteSize::ONE),
        pointer: false,
    }
}
fn flag_load(p: &NirProgram) -> Vec<NirOp> {
    let flag = p.globals.iter().find(|g| g.name == "flag").unwrap();
    vec![
        NirOp::Load {
            dest: TempId(89),
            ty: byte_type(),
            place: NirPlace {
                kind: NirPlaceKind::Global {
                    id: flag.id,
                    name: flag.name.clone(),
                },
                ty: Some(byte_type()),
            },
        },
        NirOp::Compare {
            dest: TempId(90),
            ty: condition_type(),
            operand_ty: byte_type(),
            op: NirCompareOp::Ne,
            left: NirValue::Temp {
                id: TempId(89),
                ty: byte_type(),
            },
            right: NirValue::ConstU8(0),
        },
    ]
}
fn branch(left: u32, right: u32) -> NirTerminator {
    NirTerminator::Branch {
        condition: NirValue::Temp {
            id: TempId(90),
            ty: condition_type(),
        },
        then_edge: edge(left),
        else_edge: edge(right),
    }
}
fn set_blocks(p: &mut NirProgram, blocks: Vec<NirBlock>) {
    p.routines
        .iter_mut()
        .find(|r| r.name == "Main")
        .unwrap()
        .blocks = blocks;
    rebuild_temps(p);
}

#[test]
fn cfg_joins_require_the_same_initialized_byte_on_every_path() {
    for target in TARGETS {
        for (right, remaining) in [(Some(7), 0), (Some(8), 1), (None, 1)] {
            let mut p = probe(target);
            let local = capture(&p);
            let blocks = vec![
                block(1000, flag_load(&p), branch(1001, 1002)),
                block(
                    1001,
                    vec![store(&local, 3, 7)],
                    NirTerminator::Goto(edge(1003)),
                ),
                block(
                    1002,
                    right.map(|v| vec![store(&local, 3, v)]).unwrap_or_default(),
                    NirTerminator::Goto(edge(1003)),
                ),
                block(
                    1003,
                    observe(&p, &local, 3, 100),
                    NirTerminator::Return(None),
                ),
            ];
            set_blocks(&mut p, blocks);
            let opt = nir::optimize_program(&p).unwrap();
            assert_eq!(field_loads(&opt), remaining, "{target:?} {right:?}");
            assert_eq!(opt, nir::optimize_program(&opt).unwrap());
        }
    }
}

#[test]
fn loop_backedges_never_bootstrap_entry_or_zero_trip_initialization() {
    for target in TARGETS {
        for (initial, carried, remaining) in [(None, 7, 1), (Some(7), 7, 0), (Some(7), 8, 1)] {
            let mut p = probe(target);
            let local = capture(&p);
            let blocks = vec![
                block(
                    1000,
                    initial
                        .map(|v| vec![store(&local, 3, v)])
                        .unwrap_or_default(),
                    NirTerminator::Goto(edge(1001)),
                ),
                block(1001, flag_load(&p), branch(1002, 1003)),
                block(
                    1002,
                    vec![store(&local, 3, carried)],
                    NirTerminator::Goto(edge(1001)),
                ),
                block(
                    1003,
                    observe(&p, &local, 3, 100),
                    NirTerminator::Return(None),
                ),
            ];
            set_blocks(&mut p, blocks);
            let opt = nir::optimize_program(&p).unwrap();
            assert_eq!(
                field_loads(&opt),
                remaining,
                "{target:?} {initial:?}/{carried}"
            );
            assert_eq!(opt, nir::optimize_program(&opt).unwrap());
        }
    }
}

#[test]
fn captured_ssa_byte_survives_a_call_but_a_later_load_needs_a_new_proof() {
    for target in TARGETS {
        let mut p = probe(target);
        let local = capture(&p);
        let touch = p.routines.iter().find(|r| r.name == "Touch").unwrap();
        let call = NirOp::Call {
            callee: NirCallee::User {
                id: touch.id,
                name: touch.name.clone(),
            },
            args: vec![],
            result: None,
            aggregate_result: None,
            signature: Some(touch.signature.clone()),
            effects: NirCallEffects {
                memory: NirMemoryEffects {
                    reads: NirMemoryAccess::Unknown,
                    writes: NirMemoryAccess::Unknown,
                },
                opaque: true,
                may_call_external: true,
            },
        };
        let first = observe(&p, &local, 3, 100);
        let mut second = vec![call, first[1].clone()];
        second.extend(observe(&p, &local, 3, 101));
        let blocks = vec![
            block(
                1000,
                vec![store(&local, 3, 7), first[0].clone()],
                NirTerminator::Goto(edge(1001)),
            ),
            block(1001, second, NirTerminator::Return(None)),
        ];
        set_blocks(&mut p, blocks);
        let opt = nir::optimize_program(&p).unwrap();
        assert_eq!(field_loads(&opt), 1);
        assert!(
            main(&opt)
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(
                    op,
                    NirOp::Store {
                        place: NirPlace {
                            kind: NirPlaceKind::Global { .. },
                            ..
                        },
                        src: NirValue::IntegerConst { bits: 7, .. },
                        ..
                    }
                ))
        );
    }
}

#[test]
fn byte_substitution_reaches_edge_arguments_and_block_parameters() {
    let mut p = probe(TargetId::Atari6502);
    let local = capture(&p);
    let observation = observe(&p, &local, 3, 100);
    let mut output = observe(&p, &local, 3, 101);
    output.remove(0);
    let mut dest = block(1001, output, NirTerminator::Return(None));
    dest.params.push(NirBlockParam {
        dest: TempId(101),
        ty: byte_type(),
    });
    let blocks = vec![
        block(
            1000,
            vec![store(&local, 3, 255), observation[0].clone()],
            NirTerminator::Goto(NirEdge {
                target: BlockId(1001),
                args: vec![NirValue::Temp {
                    id: TempId(100),
                    ty: byte_type(),
                }],
            }),
        ),
        dest,
    ];
    set_blocks(&mut p, blocks);
    let opt = nir::optimize_program(&p).unwrap();
    assert_eq!(field_loads(&opt), 0);
    assert!(main(&opt).blocks.iter().all(|b| b.params.is_empty()));
    assert_eq!(opt, nir::optimize_program(&opt).unwrap());
}

fn copy(destination: NirPlace, source: NirPlace, size: u32) -> NirOp {
    NirOp::CopyBytes {
        destination,
        source,
        size: ByteSize::new(size),
        source_volatile: false,
        destination_volatile: false,
    }
}
fn add_capture(p: &mut NirProgram) -> NirLocal {
    let mut local = capture(p);
    let r = p.routines.iter_mut().find(|r| r.name == "Main").unwrap();
    local.id = LocalId(r.locals.iter().map(|l| l.id.0).max().unwrap_or(0) + 1);
    local.name = format!("snapshot{}", local.id.0);
    r.locals.push(local.clone());
    local
}

#[test]
fn retained_copy_chains_keep_independent_byte_facts_after_source_mutation() {
    for target in TARGETS {
        let mut p = probe(target);
        let source = capture(&p);
        let middle = add_capture(&mut p);
        let saved = add_capture(&mut p);
        let mut ops = vec![
            store(&source, 3, 7),
            copy(field(&middle, 0), field(&source, 0), 8),
            store(&source, 3, 8),
            copy(field(&saved, 0), field(&middle, 0), 8),
            store(&middle, 3, 9),
        ];
        ops.extend(observe(&p, &saved, 3, 100));
        // Unknown neighboring payload keeps both snapshots materially live.
        ops.extend(observe(&p, &saved, 4, 101));
        set_ops(&mut p, ops);
        let opt = nir::optimize_program(&p).unwrap();
        assert_eq!(field_loads(&opt), 1);
        assert_eq!(
            main(&opt)
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|op| matches!(op, NirOp::CopyBytes { .. }))
                .count(),
            2
        );
        assert!(
            main(&opt)
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(
                    op,
                    NirOp::Store {
                        place: NirPlace {
                            kind: NirPlaceKind::Global { .. },
                            ..
                        },
                        src: NirValue::IntegerConst { bits: 7, .. },
                        ..
                    }
                ))
        );
        assert_eq!(opt, nir::optimize_program(&opt).unwrap());
    }
}

#[test]
fn copies_use_pre_copy_state_and_decline_partial_overlap() {
    for target in TARGETS {
        for (from, to, size, remaining) in [(1, 1, 2, 0), (0, 1, 2, 1), (1, 0, 2, 1), (5, 1, 1, 0)]
        {
            let mut p = probe(target);
            let local = capture(&p);
            let mut ops = vec![
                store(&local, 1, 7),
                store(&local, 5, 9),
                copy(field(&local, to), field(&local, from), size),
            ];
            ops.extend(observe(&p, &local, 1, 100));
            set_ops(&mut p, ops);
            let opt = nir::optimize_program(&p).unwrap();
            assert_eq!(
                field_loads(&opt),
                remaining,
                "{target:?} {from}->{to} size {size}"
            );
            assert!(
                main(&opt)
                    .blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .any(|op| matches!(op, NirOp::CopyBytes { .. }))
            );
        }
    }
}

#[test]
fn unknown_copy_sources_and_volatile_copies_kill_known_destination_bytes() {
    for kind in 0..3 {
        let mut p = probe(TargetId::Atari6502);
        let local = capture(&p);
        let mut transfer = copy(field(&local, 3), field(&local, 4), 1);
        if let NirOp::CopyBytes {
            source,
            source_volatile,
            destination_volatile,
            ..
        } = &mut transfer
        {
            match kind {
                0 => source.kind = NirPlaceKind::Absolute(AddressValue::data(0x900)),
                1 => *source_volatile = true,
                _ => *destination_volatile = true,
            }
        }
        let mut ops = vec![store(&local, 3, 7), transfer];
        ops.extend(observe(&p, &local, 3, 100));
        set_ops(&mut p, ops);
        assert_eq!(field_loads(&nir::optimize_program(&p).unwrap()), 1);
    }
}

#[test]
fn large_copy_extents_track_queried_cells_without_enumerating_payload_bytes() {
    let mut p = lower(
        "TYPE Value=[BYTE ARRAY bytes(30000)] Value source BYTE out PROC Main() LET saved=source RETURN",
        TargetId::Atari6502,
    );
    let first = capture(&p);
    let saved = add_capture(&mut p);
    let mut ops = vec![
        store(&first, 29999, 255),
        copy(field(&saved, 0), field(&first, 0), 30000),
        store(&first, 29999, 1),
    ];
    ops.extend(observe(&p, &saved, 29999, 100));
    set_ops(&mut p, ops);
    let opt = nir::optimize_program(&p).unwrap();
    assert_eq!(field_loads(&opt), 0);
}

#[test]
fn fresh_nested_constructor_checks_need_each_nested_byte_proof() {
    for target in TARGETS {
        let source = "TYPE Inner=VARIANT [OFF ON [BYTE n]] TYPE Outer=VARIANT [NONE SOME [Inner child]] BYTE out PROC Main() LET saved=Outer.SOME(Inner.ON(42))\nCASE saved OF\nWHEN Outer.SOME(Inner.ON(n)) THEN\nout=n\nELSE\nout=0\nESAC\nRETURN";
        let p = lower(source, target);
        let opt = nir::optimize_program(&p).unwrap();
        assert_eq!(field_loads(&opt), 0, "{target:?}");
        assert!(
            !main(&opt)
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(
                    op,
                    NirOp::Call {
                        callee: NirCallee::Fault(_),
                        ..
                    }
                ))
        );
        assert_eq!(opt, nir::optimize_program(&opt).unwrap());
    }
}

#[test]
fn scalar_cleanup_exposes_byte_stores_in_the_same_public_optimizer_invocation() {
    for target in TARGETS {
        let mut p = probe(target);
        let capture = capture(&p);
        let mut scalar = capture.clone();
        scalar.id = LocalId(100);
        scalar.name = "scalar_seed".into();
        scalar.purpose = NirLocalPurpose::Storage;
        scalar.storage = NirStorageClass::Scalar;
        scalar.ty = byte_type();
        scalar.layout.size = ByteSize::ONE;
        scalar.layout.alignment = ByteSize::ONE;
        p.routines
            .iter_mut()
            .find(|r| r.name == "Main")
            .unwrap()
            .locals
            .push(scalar.clone());
        let scalar_place = NirPlace {
            kind: NirPlaceKind::Local {
                id: scalar.id,
                name: scalar.name,
            },
            ty: Some(byte_type()),
        };
        let mut ops = vec![
            NirOp::Store {
                place: scalar_place.clone(),
                src: NirValue::ConstU8(7),
                ty: byte_type(),
            },
            NirOp::Load {
                dest: TempId(99),
                place: scalar_place,
                ty: byte_type(),
            },
            NirOp::Store {
                place: field(&capture, 3),
                src: NirValue::Temp {
                    id: TempId(99),
                    ty: byte_type(),
                },
                ty: byte_type(),
            },
        ];
        ops.extend(observe(&p, &capture, 3, 100));
        set_ops(&mut p, ops);
        let opt = nir::optimize_program(&p).unwrap();
        assert_eq!(opt, nir::optimize_program(&opt).unwrap(), "{target:?}");
        assert_eq!(field_loads(&opt), 0);
    }
}

#[test]
fn exact_internal_addresses_work_but_escaped_addresses_block_fresh_facts() {
    for target in TARGETS {
        for escaped in [false, true] {
            let mut p = probe(target);
            let local = capture(&p);
            let pointer = p.globals.iter().find(|g| g.name == "ptr").unwrap();
            let pointer_ty = pointer.ty.clone().unwrap();
            let address = NirValue::Temp {
                id: TempId(99),
                ty: pointer_ty.clone(),
            };
            let mut ops = vec![NirOp::AddrOf {
                dest: TempId(99),
                ty: pointer_ty.clone(),
                place: field(&local, 3),
            }];
            if escaped {
                ops.push(NirOp::Store {
                    place: NirPlace {
                        kind: NirPlaceKind::Global {
                            id: pointer.id,
                            name: pointer.name.clone(),
                        },
                        ty: Some(pointer_ty.clone()),
                    },
                    src: address.clone(),
                    ty: pointer_ty,
                });
            }
            ops.push(NirOp::Store {
                place: NirPlace {
                    kind: NirPlaceKind::Deref { addr: address },
                    ty: Some(byte_type()),
                },
                src: NirValue::ConstU8(7),
                ty: byte_type(),
            });
            ops.extend(observe(&p, &local, 3, 100));
            set_ops(&mut p, ops);
            let analysis = analyze_aggregate_regions(&p).unwrap();
            assert_eq!(
                analysis
                    .routine(main(&p).id)
                    .unwrap()
                    .address_use(NirStorageId::Local(local.id)),
                if escaped {
                    NirAggregateAddressUse::ExposedOrUnknown
                } else {
                    NirAggregateAddressUse::InternalOnly
                }
            );
            assert_eq!(
                field_loads(&nir::optimize_program(&p).unwrap()),
                usize::from(escaped)
            );
        }
    }
}

#[test]
fn live_aliases_and_data_relocations_disqualify_capture_roots() {
    for data in [false, true] {
        let mut p = probe(TargetId::Atari6502);
        let local = capture(&p);
        let mut holder = add_capture(&mut p);
        holder.purpose = NirLocalPurpose::Storage;
        let mut ops = vec![store(&local, 3, 7)];
        if data {
            let ptr_ty = p
                .globals
                .iter()
                .find(|g| g.name == "ptr")
                .unwrap()
                .ty
                .clone()
                .unwrap();
            holder.ty = ptr_ty;
            holder.storage = NirStorageClass::Scalar;
            holder.layout.size = ByteSize::new(2);
            holder.init = Some(NirStorageInit::Bytes {
                image: NirDataImage {
                    bytes: vec![0, 0],
                    fragments: vec![NirDataFragment::Address {
                        offset: ByteOffset::ZERO,
                        encoding: NirDataAddressEncoding::Pointer {
                            address_space: actionc::target::TargetLayout::DATA_ADDRESS_SPACE,
                            width: ByteSize::new(2),
                        },
                        target: NirDataAddressTarget::Storage(NirStorageId::Local(local.id)),
                        addend: 0,
                        span: actionc::source::Span { start: 0, end: 0 },
                    }],
                },
                zero_fill: ByteSize::ZERO,
                mutable: false,
                section: ".data".into(),
            });
        } else {
            holder.backing = NirLocalBacking::Alias {
                target: local.id,
                target_name: local.name.clone(),
                offset: ByteOffset::ZERO,
            };
            ops.extend(observe(&p, &holder, 4, 101));
        }
        let r = p.routines.iter_mut().find(|r| r.name == "Main").unwrap();
        let holder_id = holder.id;
        *r.locals.iter_mut().find(|l| l.id == holder_id).unwrap() = holder;
        ops.extend(observe(&p, &local, 3, 100));
        set_ops(&mut p, ops);
        let analysis = analyze_aggregate_regions(&p).unwrap();
        assert_eq!(
            analysis
                .routine(main(&p).id)
                .unwrap()
                .address_use(NirStorageId::Local(local.id)),
            NirAggregateAddressUse::ExposedOrUnknown
        );
        assert!(field_loads(&nir::optimize_program(&p).unwrap()) >= 1);
    }
}

#[test]
fn real_and_machine_effects_invalidate_preceding_byte_stores() {
    for machine in [false, true] {
        let mut p = lower(
            "TYPE Value=[BYTE x,y] Value source REAL scratch BYTE out PROC Main() LET saved=source scratch=1.0 [$EA] RETURN",
            TargetId::Atari6502,
        );
        let local = capture(&p);
        let effect = main(&p)
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .find(|op| {
                if machine {
                    matches!(op, NirOp::ForeignCode { .. })
                } else {
                    matches!(op, NirOp::Real(_))
                }
            })
            .unwrap()
            .clone();
        let mut ops = vec![store(&local, 0, 7), effect];
        ops.extend(observe(&p, &local, 0, 100));
        set_ops(&mut p, ops);
        assert_eq!(field_loads(&nir::optimize_program(&p).unwrap()), 1);
    }
}

#[test]
fn known_invalid_bytes_choose_faults_and_outer_tags_do_not_prove_nested_validity() {
    let source = "TYPE Inner=VARIANT [OFF ON [BYTE n]] TYPE Outer=VARIANT [NONE SOME [Inner child]] BYTE out PROC Main() LET saved=Outer.SOME(Inner.ON(42))\nCASE saved OF\nWHEN Outer.SOME(Inner.ON(n)) THEN\nout=n\nELSE\nout=0\nESAC\nRETURN";
    for (offset, value) in [(0, Some(0)), (0, Some(255)), (1, Some(0)), (1, None)] {
        let mut p = lower(source, TargetId::Atari6502);
        let analysis = analyze_aggregate_regions(&p).unwrap();
        let r = main(&p);
        let regions = analysis.routine(r.id).unwrap();
        let location = r
            .blocks
            .iter()
            .enumerate()
            .find_map(|(b, block)| {
                block.ops.iter().enumerate().find_map(|(i, op)| {
                    if let NirOp::Store { place, .. } = op {
                        if regions
                            .region(
                                place,
                                ByteSize::ONE,
                                NirAggregatePoint {
                                    block: block.id,
                                    op_index: i,
                                },
                            )
                            .is_ok_and(|r| r.memory.offset.get() == offset)
                        {
                            return Some((b, i));
                        }
                    }
                    None
                })
            })
            .unwrap();
        drop(analysis);
        let r = p.routines.iter_mut().find(|r| r.name == "Main").unwrap();
        if let Some(value) = value {
            if let NirOp::Store { src, .. } = &mut r.blocks[location.0].ops[location.1] {
                *src = NirValue::ConstU8(value);
            }
        } else {
            r.blocks[location.0].ops.remove(location.1);
        }
        rebuild_temps(&mut p);
        let opt = nir::optimize_program(&p).unwrap();
        let ops: Vec<_> = main(&opt).blocks.iter().flat_map(|b| &b.ops).collect();
        assert!(ops.iter().any(|op| matches!(
            op,
            NirOp::Call {
                callee: NirCallee::Fault(_),
                ..
            }
        )));
        if value.is_some() {
            assert!(!ops.iter().any(|op| matches!(
                op,
                NirOp::Store {
                    place: NirPlace {
                        kind: NirPlaceKind::Global { .. },
                        ..
                    },
                    ..
                }
            )));
        } else {
            assert!(field_loads(&opt) > 0);
        }
        assert_eq!(opt, nir::optimize_program(&opt).unwrap());
    }
}
