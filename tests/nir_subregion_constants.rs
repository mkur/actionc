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
