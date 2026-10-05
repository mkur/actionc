use super::*;
use crate::nir::{NirCastKind, NirCompareOp, NirIntegerRole, NirIntegerType, NirType};

// Independently authored logical graphs. Only boilerplate ABI plans come from
// a checked helper descriptor; definitions, edges and memory operations do not
// come from lowering or from the analyses being tested.
fn routine(blocks: Vec<Mir65816Block>, types: &[(u32, u32)]) -> Mir65816Routine {
    let mut r = arithmetic::Helper {
        operation: arithmetic::Operation::Multiply,
        bytes: 2,
        signed: false,
    }
    .routine(RoutineId(17), SignatureId(1))
    .unwrap();
    r.helper = None;
    r.frame.parameters.clear();
    r.blocks = blocks;
    r.temps = types
        .iter()
        .map(|&(id, bytes)| (TempId(id), ty(bytes)))
        .collect();
    r
}
fn ty(bytes: u32) -> NirType {
    NirType {
        kind: NirTypeKind::Integer(NirIntegerType {
            bits: (bytes * 8) as u8,
            signed: false,
            role: NirIntegerRole::Ordinary,
        }),
        summary: "debug only".into(),
        width: Some(ByteSize::new(bytes)),
        pointer: false,
    }
}
fn v(id: u32, bytes: u32) -> Mir65816Value {
    Mir65816Value::Temp(TempId(id), ByteSize::new(bytes))
}
fn edge(target: u32, args: Vec<Mir65816Value>) -> Mir65816Edge {
    Mir65816Edge {
        target: BlockId(target),
        args,
    }
}
fn block(id: u32, ops: Vec<Mir65816Op>, term: Mir65816Terminator) -> Mir65816Block {
    Mir65816Block {
        id: BlockId(id),
        params: vec![],
        ops,
        terminator: term,
    }
}
fn cast(dest: u32, bytes: u32, value: Mir65816Value) -> Mir65816Op {
    Mir65816Op::Cast {
        dest: TempId(dest),
        from: ByteSize::new(bytes),
        from_signed: false,
        to: ByteSize::new(bytes),
        kind: NirCastKind::Integer,
        value,
    }
}
fn point(a: &RoutineAnalysis<'_>, block: u32, index: usize) -> Point {
    a.point(a.block(BlockId(block)).unwrap(), index).unwrap()
}
fn temp(a: &RoutineAnalysis<'_>, id: u32) -> Temp {
    a.temp(TempId(id)).unwrap()
}
fn object(r: &mut Mir65816Routine, id: u32, bytes: u32, addressable: bool) {
    r.frame.objects.push(Mir65816FrameObject {
        id: Mir65816FrameObjectId(id),
        owner: Mir65816FrameObjectOwner::Local(crate::nir::LocalId(id)),
        size: ByteSize::new(bytes),
        alignment: ByteSize::ONE,
        stack_offset: ByteOffset::new(1 + id * 4),
        mutable: true,
        addressable,
    });
}
fn addr(id: u32, offset: u32) -> Mir65816Address {
    Mir65816Address {
        base: Mir65816AddressBase::AutomaticFrame(Mir65816FrameObjectId(id)),
        displacement: ByteOffset::new(offset),
        index: None,
        mode: Mir65816AddressMode::AutomaticFrame,
    }
}
fn store(id: u32, offset: u32, value: Mir65816Value, bytes: u32) -> Mir65816Op {
    Mir65816Op::Store {
        address: addr(id, offset),
        value,
        width: ByteSize::new(bytes),
        volatile: false,
    }
}
fn call() -> Mir65816Op {
    let h = arithmetic::Helper {
        operation: arithmetic::Operation::Multiply,
        bytes: 2,
        signed: false,
    };
    Mir65816Op::Call {
        target: Mir65816CallTarget::Helper(RoutineId(77)),
        signature: Some(SignatureId(1)),
        args: vec![Mir65816Value::U16(1), Mir65816Value::U16(2)],
        result: None,
        convention: Mir65816CallConvention::Native,
        plan: h.plan(SignatureId(1)).unwrap(),
    }
}
fn contents(
    a: &RoutineAnalysis<'_>,
    b: u32,
    i: usize,
    obj: u32,
    offset: u32,
    bytes: u32,
) -> StorageContents {
    a.storage_at(
        point(a, b, i),
        a.storage(StorageId::Frame(Mir65816FrameObjectId(obj)))
            .unwrap(),
        offset,
        bytes,
    )
    .unwrap()
}

#[test]
fn parallel_edges_keep_distinct_simultaneous_assignments_and_census() {
    let mut join = block(9, vec![], Mir65816Terminator::Exit);
    join.params = vec![(TempId(3), ByteSize::new(2)), (TempId(4), ByteSize::new(2))];
    let r = routine(
        vec![
            block(
                5,
                vec![
                    cast(1, 2, Mir65816Value::U16(10)),
                    cast(2, 2, Mir65816Value::U16(20)),
                ],
                Mir65816Terminator::Branch {
                    condition: Mir65816Value::U8(1),
                    then_edge: edge(9, vec![v(1, 2), v(2, 2)]),
                    else_edge: edge(9, vec![v(2, 2), v(1, 2)]),
                },
            ),
            join,
        ],
        &[(1, 2), (2, 2), (3, 2), (4, 2)],
    );
    let a = RoutineAnalysis::new(&r).unwrap();
    let left = a
        .edge_mapping(a.edge(a.block(BlockId(5)).unwrap(), 0).unwrap())
        .unwrap();
    let right = a
        .edge_mapping(a.edge(a.block(BlockId(5)).unwrap(), 1).unwrap())
        .unwrap();
    assert_eq!(left.target, right.target);
    assert_eq!(left.bindings[0], (temp(&a, 3), v(1, 2)));
    assert_eq!(right.bindings[0], (temp(&a, 3), v(2, 2)));
    assert_eq!(a.value(temp(&a, 1)).unwrap().uses.len(), 2);
    assert!(
        a.dominates(a.block(BlockId(5)).unwrap(), a.block(BlockId(9)).unwrap())
            .unwrap()
    );
    assert!(a.available(point(&a, 9, 0), temp(&a, 3)).unwrap());
}

#[test]
fn loop_parameters_backedges_and_cross_loop_values_reach_a_fixed_point() {
    let mut header = block(
        1,
        vec![],
        Mir65816Terminator::Branch {
            condition: v(2, 1),
            then_edge: edge(2, vec![]),
            else_edge: edge(3, vec![]),
        },
    );
    header.params = vec![(TempId(2), ByteSize::ONE)];
    let r = routine(
        vec![
            block(
                0,
                vec![cast(1, 1, Mir65816Value::U8(7))],
                Mir65816Terminator::Goto(edge(1, vec![Mir65816Value::U8(1)])),
            ),
            header,
            block(
                2,
                vec![cast(3, 1, v(1, 1))],
                Mir65816Terminator::Goto(edge(1, vec![v(3, 1)])),
            ),
            block(3, vec![cast(4, 1, v(1, 1))], Mir65816Terminator::Exit),
        ],
        &[(1, 1), (2, 1), (3, 1), (4, 1)],
    );
    let a = RoutineAnalysis::new(&r).unwrap();
    assert_eq!(
        a.live_at(point(&a, 1, 0)).unwrap(),
        vec![temp(&a, 1), temp(&a, 2)]
    );
    assert_eq!(
        a.live_at(point(&a, 2, 1)).unwrap(),
        vec![temp(&a, 1), temp(&a, 3)]
    );
    assert!(a.available(point(&a, 3, 1), temp(&a, 1)).unwrap());
    assert!(!a.available(point(&a, 3, 1), temp(&a, 3)).unwrap());
    assert!(a.solver_evaluations() >= 8);
}

#[test]
fn reads_remain_distinct_captures_and_identity_requires_exact_representation() {
    let pointer = Mir65816Address {
        base: Mir65816AddressBase::Indirect(v(1, 3)),
        displacement: ByteOffset::new(2),
        index: None,
        mode: Mir65816AddressMode::LongIndirect,
    };
    let mut r = routine(
        vec![block(
            0,
            vec![
                cast(1, 3, Mir65816Value::U24(0x123456)),
                Mir65816Op::Load {
                    dest: TempId(2),
                    width: ByteSize::new(2),
                    address: pointer.clone(),
                    volatile: false,
                },
                Mir65816Op::Load {
                    dest: TempId(3),
                    width: ByteSize::new(2),
                    address: pointer,
                    volatile: false,
                },
                cast(4, 2, v(2, 2)),
                cast(5, 2, v(2, 2)),
            ],
            Mir65816Terminator::Exit,
        )],
        &[(1, 3), (2, 2), (3, 2), (4, 2), (5, 2)],
    );
    r.temps[4].1.kind = NirTypeKind::Integer(NirIntegerType {
        bits: 16,
        signed: true,
        role: NirIntegerRole::Ordinary,
    });
    // Display metadata cannot defeat exact structured representation equality.
    r.temps[3].1.summary = "unrelated print name".into();
    let a = RoutineAnalysis::new(&r).unwrap();
    assert!(
        !a.same_value(point(&a, 0, 5), temp(&a, 2), temp(&a, 3))
            .unwrap()
    );
    assert!(
        a.same_value(point(&a, 0, 5), temp(&a, 2), temp(&a, 4))
            .unwrap()
    );
    assert!(
        !a.same_value(point(&a, 0, 5), temp(&a, 2), temp(&a, 5))
            .unwrap()
    );
    assert!(
        !a.same_value(point(&a, 0, 0), temp(&a, 2), temp(&a, 4))
            .unwrap()
    );
}

#[test]
fn partial_overlapping_writes_and_calls_preserve_only_proved_versions() {
    let mut r = routine(
        vec![block(
            0,
            vec![
                store(0, 0, Mir65816Value::U16(0xabcd), 2),
                store(0, 1, Mir65816Value::U8(0x12), 1),
                store(0, 2, Mir65816Value::U8(0x34), 1),
                call(),
            ],
            Mir65816Terminator::Exit,
        )],
        &[],
    );
    object(&mut r, 0, 3, false);
    let a = RoutineAnalysis::new(&r).unwrap();
    assert!(!contents(&a, 0, 0, 0, 0, 3).definitely_initialized());
    let partial = contents(&a, 0, 2, 0, 0, 3);
    assert_eq!(
        partial
            .bytes
            .iter()
            .map(|b| b.definitely_initialized)
            .collect::<Vec<_>>(),
        vec![true, true, false]
    );
    assert_eq!(
        partial.bytes[0].version,
        Some(StorageVersion::Write {
            site: point(&a, 0, 0),
            storage: a
                .storage(StorageId::Frame(Mir65816FrameObjectId(0)))
                .unwrap(),
            byte: 0
        })
    );
    assert_eq!(
        partial.bytes[1].version,
        Some(StorageVersion::Write {
            site: point(&a, 0, 1),
            storage: a
                .storage(StorageId::Frame(Mir65816FrameObjectId(0)))
                .unwrap(),
            byte: 1
        })
    );
    let full = contents(&a, 0, 3, 0, 0, 3);
    assert!(full.definitely_initialized());
    assert_eq!(contents(&a, 0, 4, 0, 0, 3), full);
}

#[test]
fn address_exposure_including_unreachable_code_blocks_call_preservation() {
    let mut r = routine(
        vec![
            block(
                0,
                vec![
                    store(0, 0, Mir65816Value::U16(1), 2),
                    store(1, 0, Mir65816Value::U16(2), 2),
                    call(),
                ],
                Mir65816Terminator::Exit,
            ),
            block(
                1,
                vec![Mir65816Op::AddressOf {
                    dest: TempId(1),
                    address: addr(0, 0),
                    width: ByteSize::new(3),
                }],
                Mir65816Terminator::Exit,
            ),
        ],
        &[(1, 3)],
    );
    object(&mut r, 0, 2, false);
    object(&mut r, 1, 2, false);
    let a = RoutineAnalysis::new(&r).unwrap();
    assert_eq!(
        a.storage_facts(
            a.storage(StorageId::Frame(Mir65816FrameObjectId(0)))
                .unwrap()
        )
        .unwrap()
        .ownership,
        Ownership::Addressable
    );
    assert!(
        contents(&a, 0, 3, 0, 0, 2)
            .bytes
            .iter()
            .all(|b| b.version.is_none())
    );
    assert!(
        contents(&a, 0, 3, 1, 0, 2)
            .bytes
            .iter()
            .all(|b| b.version.is_some())
    );
    assert_eq!(
        a.available(point(&a, 1, 1), temp(&a, 1)),
        Err(QueryError::Unreachable)
    );
    assert_eq!(a.live_at(point(&a, 1, 1)), Err(QueryError::Unreachable));
    assert_eq!(
        a.storage_at(
            point(&a, 1, 0),
            a.storage(StorageId::Frame(Mir65816FrameObjectId(0)))
                .unwrap(),
            0,
            2
        ),
        Err(QueryError::Unreachable)
    );
}

#[test]
fn loop_storage_initialization_intersects_paths_and_versions_do_not_name_iterations() {
    let mut r = routine(
        vec![
            block(
                0,
                vec![store(0, 0, Mir65816Value::U8(1), 1)],
                Mir65816Terminator::Goto(edge(1, vec![])),
            ),
            block(
                1,
                vec![],
                Mir65816Terminator::Branch {
                    condition: Mir65816Value::U8(1),
                    then_edge: edge(2, vec![]),
                    else_edge: edge(3, vec![]),
                },
            ),
            block(
                2,
                vec![
                    store(0, 0, Mir65816Value::U8(2), 1),
                    store(1, 0, Mir65816Value::U8(3), 1),
                ],
                Mir65816Terminator::Goto(edge(1, vec![])),
            ),
            block(3, vec![], Mir65816Terminator::Exit),
        ],
        &[],
    );
    object(&mut r, 0, 1, false);
    object(&mut r, 1, 1, false);
    let a = RoutineAnalysis::new(&r).unwrap();
    assert!(contents(&a, 1, 0, 0, 0, 1).definitely_initialized());
    assert!(!contents(&a, 1, 0, 1, 0, 1).definitely_initialized());
    assert!(contents(&a, 2, 2, 1, 0, 1).definitely_initialized());
    assert!(contents(&a, 1, 0, 0, 0, 1).bytes[0].version.is_none());
    assert!(contents(&a, 2, 1, 0, 0, 1).bytes[0].version.is_none());
}

#[test]
fn unknown_extent_absolute_and_volatile_accesses_never_supply_alias_proofs() {
    let mut dynamic = addr(0, 0);
    dynamic.index = Some(Mir65816Index {
        value: v(1, 1),
        stride: ByteSize::ONE,
    });
    let mut volatile = store(0, 0, Mir65816Value::U8(9), 1);
    if let Mir65816Op::Store { volatile, .. } = &mut volatile {
        *volatile = true;
    }
    let mut external = addr(0, 0);
    external.base = Mir65816AddressBase::External(Mir65816ExternalAddress::Absolute(
        crate::target::AddressValue::data(0x123456),
    ));
    external.mode = Mir65816AddressMode::External;
    for unknown in [
        Mir65816Op::Store {
            address: dynamic,
            value: Mir65816Value::U8(0),
            width: ByteSize::ONE,
            volatile: false,
        },
        Mir65816Op::Store {
            address: external,
            value: Mir65816Value::U8(0),
            width: ByteSize::ONE,
            volatile: false,
        },
        volatile,
    ] {
        let mut r = routine(
            vec![block(
                0,
                vec![
                    cast(1, 1, Mir65816Value::U8(0)),
                    store(0, 0, Mir65816Value::U8(1), 1),
                    store(1, 0, Mir65816Value::U8(2), 1),
                    unknown,
                ],
                Mir65816Terminator::Exit,
            )],
            &[(1, 1)],
        );
        object(&mut r, 0, 1, false);
        object(&mut r, 1, 1, false);
        let a = RoutineAnalysis::new(&r).unwrap();
        assert!(contents(&a, 0, 4, 0, 0, 1).bytes[0].version.is_none());
        assert!(contents(&a, 0, 4, 1, 0, 1).bytes[0].version.is_none());
        assert!(contents(&a, 0, 4, 1, 0, 1).definitely_initialized());
    }
}

#[test]
fn immutable_inputs_have_stable_incoming_versions_without_pointee_facts() {
    let mut r = routine(
        vec![block(
            0,
            vec![cast(1, 2, Mir65816Value::Param(ParamId(4))), call()],
            Mir65816Terminator::Exit,
        )],
        &[(1, 2)],
    );
    r.frame.parameters.push(Mir65816ParameterPlan {
        param: ParamId(4),
        incoming: Mir65816AbiHome::StackArgument {
            offset: ByteOffset::ZERO,
            size: ByteSize::new(2),
            alignment: ByteSize::ONE,
        },
        frame_object: None,
        body_stack_offset: None,
    });
    let a = RoutineAnalysis::new(&r).unwrap();
    let s = a.parameter_storage(ParamId(4)).unwrap();
    assert_eq!(
        a.storage_facts(s).unwrap().ownership,
        Ownership::ImmutableInput
    );
    let c = a.storage_at(point(&a, 0, 2), s, 0, 2).unwrap();
    assert!(c.definitely_initialized());
    assert_eq!(
        c.bytes[0].version,
        Some(StorageVersion::Incoming {
            storage: s,
            byte: 0
        })
    );
    assert_eq!(
        a.parameter_storage(ParamId(99)),
        Err(QueryError::UnknownStorage)
    );
}

#[test]
fn all_queries_reject_foreign_and_stale_handles_and_invalid_ranges() {
    let mut r = routine(
        vec![
            block(
                0,
                vec![cast(1, 1, Mir65816Value::U8(1))],
                Mir65816Terminator::Goto(edge(1, vec![])),
            ),
            block(1, vec![], Mir65816Terminator::Exit),
        ],
        &[(1, 1)],
    );
    object(&mut r, 0, 1, false);
    let a = RoutineAnalysis::new(&r).unwrap();
    let (b, t, p, e, s) = (
        a.block(BlockId(0)).unwrap(),
        temp(&a, 1),
        point(&a, 0, 1),
        a.edge(a.block(BlockId(0)).unwrap(), 0).unwrap(),
        a.storage(StorageId::Frame(Mir65816FrameObjectId(0)))
            .unwrap(),
    );
    let same = RoutineAnalysis::new(&r).unwrap();
    assert_eq!(same.point(b, 0), Err(QueryError::StaleGeneration));
    assert_eq!(same.value(t), Err(QueryError::StaleGeneration));
    assert_eq!(same.edge_mapping(e), Err(QueryError::StaleGeneration));
    assert_eq!(same.storage_facts(s), Err(QueryError::StaleGeneration));
    assert_eq!(same.live_at(p), Err(QueryError::StaleGeneration));
    assert_eq!(
        same.dominates(b, same.block(BlockId(1)).unwrap()),
        Err(QueryError::StaleGeneration)
    );
    assert_eq!(
        same.available(same.point(same.block(BlockId(0)).unwrap(), 1).unwrap(), t),
        Err(QueryError::StaleGeneration)
    );
    assert_eq!(a.storage_at(p, s, 0, 0), Err(QueryError::InvalidRange));
    assert_eq!(a.storage_at(p, s, 0, 2), Err(QueryError::InvalidRange));
    assert_eq!(a.point(b, 2), Err(QueryError::InvalidPoint));
    assert_eq!(a.edge(b, 2), Err(QueryError::UnknownEdge));
    let mut foreign = r.clone();
    foreign.id = RoutineId(18);
    let f = RoutineAnalysis::new(&foreign).unwrap();
    assert_eq!(f.value(t), Err(QueryError::ForeignRoutine));
    assert_eq!(f.live_at(p), Err(QueryError::ForeignRoutine));
    drop(a);
    drop(same);
    r.blocks[0].ops[0] = cast(1, 1, Mir65816Value::U8(2));
    let edited = RoutineAnalysis::new(&r).unwrap();
    assert_eq!(edited.live_at(p), Err(QueryError::StaleGeneration));
}

#[test]
fn malformed_definitions_edges_and_memory_extents_are_rejected() {
    let base = routine(
        vec![block(
            0,
            vec![cast(1, 1, Mir65816Value::U8(0)), cast(2, 1, v(1, 1))],
            Mir65816Terminator::Exit,
        )],
        &[(1, 1), (2, 1)],
    );
    let mut cases = vec![];
    let mut r = base.clone();
    r.temps.push(r.temps[0].clone());
    cases.push(r);
    let mut r = base.clone();
    r.blocks[0].ops[1] = cast(1, 1, Mir65816Value::U8(1));
    cases.push(r);
    let mut r = base.clone();
    r.blocks[0].ops.remove(1);
    cases.push(r);
    let mut r = base.clone();
    r.blocks[0].ops.swap(0, 1);
    cases.push(r);
    let mut r = base.clone();
    r.blocks[0].ops[1] = cast(2, 1, v(1, 2));
    cases.push(r);
    let mut r = base.clone();
    r.blocks[0].ops[1] = cast(2, 1, v(99, 1));
    cases.push(r);
    let mut r = base.clone();
    r.blocks[0].terminator = Mir65816Terminator::Goto(edge(99, vec![]));
    cases.push(r);
    let mut r = base.clone();
    r.blocks[0].terminator = Mir65816Terminator::Fallthrough;
    cases.push(r);
    let mut r = base.clone();
    r.blocks[0].ops.push(store(0, 1, Mir65816Value::U16(1), 2));
    object(&mut r, 0, 2, false);
    cases.push(r);
    let mut r = base.clone();
    r.blocks[0].ops[1] = cast(2, 1, Mir65816Value::Param(ParamId(55)));
    cases.push(r);
    for (i, r) in cases.iter().enumerate() {
        assert!(RoutineAnalysis::new(r).is_err(), "malformed graph {i}");
    }
    // A definition on one arm does not initialize the other path to a join.
    let r = routine(
        vec![
            block(
                0,
                vec![],
                Mir65816Terminator::Branch {
                    condition: Mir65816Value::U8(1),
                    then_edge: edge(1, vec![]),
                    else_edge: edge(2, vec![]),
                },
            ),
            block(
                1,
                vec![cast(1, 1, Mir65816Value::U8(7))],
                Mir65816Terminator::Goto(edge(3, vec![])),
            ),
            block(2, vec![], Mir65816Terminator::Goto(edge(3, vec![]))),
            block(3, vec![cast(2, 1, v(1, 1))], Mir65816Terminator::Exit),
        ],
        &[(1, 1), (2, 1)],
    );
    assert!(
        RoutineAnalysis::new(&r)
            .unwrap_err()
            .contains("not definitely defined")
    );
}

#[test]
fn unreachable_predecessors_do_not_destroy_executable_dominance() {
    let r = routine(
        vec![
            block(
                0,
                vec![cast(1, 1, Mir65816Value::U8(1))],
                Mir65816Terminator::Goto(edge(1, vec![])),
            ),
            block(1, vec![cast(2, 1, v(1, 1))], Mir65816Terminator::Exit),
            block(
                9,
                vec![cast(3, 1, v(1, 1))],
                Mir65816Terminator::Goto(edge(1, vec![])),
            ),
        ],
        &[(1, 1), (2, 1), (3, 1)],
    );
    let a = RoutineAnalysis::new(&r).unwrap();
    assert!(
        a.dominates(a.block(BlockId(0)).unwrap(), a.block(BlockId(1)).unwrap())
            .unwrap()
    );
    assert_eq!(a.value(temp(&a, 1)).unwrap().uses.len(), 2);
    assert_eq!(
        a.dominates(a.block(BlockId(9)).unwrap(), a.block(BlockId(1)).unwrap()),
        Err(QueryError::Unreachable)
    );
}

#[test]
fn indirect_callees_address_bases_indices_copy_sources_and_boolean_width_are_censused() {
    let mut indirect_call = call();
    if let Mir65816Op::Call { target, .. } = &mut indirect_call {
        *target = Mir65816CallTarget::Indirect(v(1, 3), ByteSize::new(3));
    }
    let indirect = Mir65816Address {
        base: Mir65816AddressBase::Indirect(v(1, 3)),
        displacement: ByteOffset::ZERO,
        index: Some(Mir65816Index {
            value: v(2, 2),
            stride: ByteSize::ONE,
        }),
        mode: Mir65816AddressMode::LongIndexed,
    };
    let r = routine(
        vec![
            block(
                0,
                vec![
                    cast(1, 3, Mir65816Value::U24(0x123456)),
                    cast(2, 2, Mir65816Value::U16(0)),
                    Mir65816Op::Copy {
                        destination: indirect.clone(),
                        source: indirect,
                        bytes: ByteSize::new(2),
                        overlap_safe: true,
                        destination_volatile: false,
                        source_volatile: false,
                    },
                    indirect_call,
                    Mir65816Op::Compare {
                        dest: TempId(3),
                        width: ByteSize::new(2),
                        signed: false,
                        operation: NirCompareOp::Eq,
                        left: v(2, 2),
                        right: Mir65816Value::U16(0),
                    },
                ],
                Mir65816Terminator::Branch {
                    condition: v(3, 1),
                    then_edge: edge(1, vec![]),
                    else_edge: edge(1, vec![]),
                },
            ),
            block(1, vec![], Mir65816Terminator::Exit),
        ],
        &[(1, 3), (2, 2), (3, 1)],
    );
    let a = RoutineAnalysis::new(&r).unwrap();
    assert_eq!(a.value(temp(&a, 1)).unwrap().uses.len(), 3);
    assert_eq!(a.value(temp(&a, 2)).unwrap().uses.len(), 3);
    assert_eq!(a.value(temp(&a, 3)).unwrap().width, ByteSize::ONE);
    assert_eq!(
        a.live_at(point(&a, 0, 2)).unwrap(),
        vec![temp(&a, 1), temp(&a, 2)]
    );
}

#[test]
fn irreducible_cycles_and_copy_writes_keep_conservative_byte_versions() {
    // Neither cycle entry dominates the other; a backedge-only detector would
    // miss this cycle. Different branch inputs also create a zero-write path.
    let mut r = routine(
        vec![
            block(
                0,
                vec![store(0, 0, Mir65816Value::U16(1), 2)],
                Mir65816Terminator::Branch {
                    condition: Mir65816Value::U8(1),
                    then_edge: edge(1, vec![]),
                    else_edge: edge(2, vec![]),
                },
            ),
            block(
                1,
                vec![Mir65816Op::Copy {
                    destination: addr(0, 0),
                    source: addr(0, 1),
                    bytes: ByteSize::ONE,
                    overlap_safe: true,
                    destination_volatile: false,
                    source_volatile: false,
                }],
                Mir65816Terminator::Goto(edge(2, vec![])),
            ),
            block(
                2,
                vec![],
                Mir65816Terminator::Branch {
                    condition: Mir65816Value::U8(1),
                    then_edge: edge(1, vec![]),
                    else_edge: edge(3, vec![]),
                },
            ),
            block(3, vec![], Mir65816Terminator::Exit),
        ],
        &[],
    );
    object(&mut r, 0, 2, false);
    let a = RoutineAnalysis::new(&r).unwrap();
    assert!(a.is_cyclic(a.block(BlockId(1)).unwrap()).unwrap());
    assert!(a.is_cyclic(a.block(BlockId(2)).unwrap()).unwrap());
    assert!(!a.is_cyclic(a.block(BlockId(3)).unwrap()).unwrap());
    let c = contents(&a, 3, 0, 0, 0, 2);
    assert!(c.definitely_initialized());
    assert!(c.bytes[0].version.is_none());
    assert_eq!(
        c.bytes[1].version,
        Some(StorageVersion::Write {
            site: point(&a, 0, 0),
            storage: a
                .storage(StorageId::Frame(Mir65816FrameObjectId(0)))
                .unwrap(),
            byte: 1
        })
    );
}

#[test]
fn edge_arity_width_fallthrough_and_entry_parameters_have_closed_boundaries() {
    let mut target = block(1, vec![], Mir65816Terminator::Exit);
    target.params = vec![(TempId(1), ByteSize::new(2))];
    for term in [
        Mir65816Terminator::Fallthrough,
        Mir65816Terminator::Goto(edge(1, vec![])),
        Mir65816Terminator::Goto(edge(1, vec![Mir65816Value::U8(1)])),
    ] {
        let r = routine(vec![block(0, vec![], term), target.clone()], &[(1, 2)]);
        assert!(RoutineAnalysis::new(&r).is_err());
    }
    let r = routine(vec![target], &[(1, 2)]);
    assert!(RoutineAnalysis::new(&r).is_err());
    let r = routine(
        vec![
            block(0, vec![], Mir65816Terminator::Fallthrough),
            block(1, vec![], Mir65816Terminator::Exit),
        ],
        &[],
    );
    let a = RoutineAnalysis::new(&r).unwrap();
    assert_eq!(
        a.edge_mapping(a.edge(a.block(BlockId(0)).unwrap(), 0).unwrap())
            .unwrap()
            .target
            .id(),
        BlockId(1)
    );
}

#[test]
fn pointer_identity_is_not_an_object_extent_alignment_or_alias_fact() {
    let mut r = routine(
        vec![block(
            0,
            vec![
                cast(1, 3, Mir65816Value::U24(0x123457)),
                cast(2, 3, v(1, 3)),
            ],
            Mir65816Terminator::Exit,
        )],
        &[(1, 3), (2, 3)],
    );
    for (_, ty) in &mut r.temps {
        ty.kind = NirTypeKind::Pointer {
            pointee: None,
            address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
        };
        ty.pointer = true;
    }
    let a = RoutineAnalysis::new(&r).unwrap();
    assert!(
        a.same_value(point(&a, 0, 2), temp(&a, 1), temp(&a, 2))
            .unwrap()
    );
    assert_eq!(
        a.value(temp(&a, 1)).unwrap().representation,
        Representation::DataPointer(crate::target::TargetLayout::DATA_ADDRESS_SPACE)
    );
    assert_eq!(
        a.storage(StorageId::Frame(Mir65816FrameObjectId(0))),
        Err(QueryError::UnknownStorage)
    );
}

#[test]
fn unknown_contents_cannot_satisfy_a_version_proof_even_at_the_same_point() {
    let mut r = routine(
        vec![block(
            0,
            vec![store(0, 0, Mir65816Value::U8(1), 1), call()],
            Mir65816Terminator::Exit,
        )],
        &[],
    );
    object(&mut r, 0, 2, false);
    let a = RoutineAnalysis::new(&r).unwrap();
    let s = a
        .storage(StorageId::Frame(Mir65816FrameObjectId(0)))
        .unwrap();
    assert!(
        !a.same_storage_version(point(&a, 0, 0), point(&a, 0, 0), s, 0, 2)
            .unwrap()
    );
    assert!(
        a.same_storage_version(point(&a, 0, 1), point(&a, 0, 2), s, 0, 1)
            .unwrap()
    );
    assert!(
        !a.same_storage_version(point(&a, 0, 1), point(&a, 0, 2), s, 0, 2)
            .unwrap()
    );
}

#[test]
fn unavailable_values_do_not_hide_a_foreign_equality_query() {
    let r = routine(
        vec![block(
            0,
            vec![cast(1, 1, Mir65816Value::U8(1))],
            Mir65816Terminator::Exit,
        )],
        &[(1, 1)],
    );
    let a = RoutineAnalysis::new(&r).unwrap();
    let other = RoutineAnalysis::new(&r).unwrap();
    assert_eq!(
        a.same_value(point(&a, 0, 0), temp(&a, 1), temp(&other, 1)),
        Err(QueryError::StaleGeneration)
    );
}
