//! Read-only aggregate proof foundation. No test enables copy forwarding.
use actionc::{
    nir::*,
    semantic::{self, SemanticOptions},
    target::TargetId,
};

const TARGETS: [TargetId; 4] = [
    TargetId::Atari6502,
    TargetId::Motorola68000,
    TargetId::Wdc65816Small,
    TargetId::Wdc65816Native,
];

fn lower(source: &str, target: TargetId) -> NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap();
    let program = lower_program(&semantic::ir::lower_program(&ast, &model));
    verify_program(&program).unwrap();
    program
}

fn local(routine: &NirRoutine, suffix: &str) -> NirPlace {
    let local = routine
        .locals
        .iter()
        .find(|l| l.name == suffix || l.name.ends_with(&format!("::{suffix}")))
        .unwrap();
    NirPlace {
        kind: NirPlaceKind::Local {
            id: local.id,
            name: local.name.clone(),
        },
        ty: Some(local.ty.clone()),
    }
}

fn global(program: &NirProgram, name: &str) -> NirPlace {
    let global = program.globals.iter().find(|g| g.name == name).unwrap();
    NirPlace {
        kind: NirPlaceKind::Global {
            id: global.id,
            name: global.name.clone(),
        },
        ty: global.ty.clone(),
    }
}

fn point(block: &NirBlock, op_index: usize) -> NirAggregatePoint {
    NirAggregatePoint {
        block: block.id,
        op_index,
    }
}

fn byte_field(base: &NirPlace, offset: u32) -> NirPlace {
    let ty = NirType {
        summary: "Byte".into(),
        kind: NirTypeKind::U8,
        width: Some(ByteSize::new(1)),
        pointer: false,
    };
    NirPlace {
        kind: NirPlaceKind::Field {
            base: Box::new(base.clone()),
            offset: ByteOffset::new(offset),
            ty: ty.clone(),
        },
        ty: Some(ty),
    }
}

#[test]
fn exact_aggregate_regions_preserve_nominal_layout_and_activation_on_all_targets() {
    for target in TARGETS {
        for declaration in [
            "TYPE Value=[BYTE first CARD word BYTE ARRAY bytes(3)]",
            "TYPE Value=UNION [BYTE first CARD word BYTE ARRAY bytes(3)]",
            "TYPE Value=VARIANT [NONE SOME [BYTE first] WIDE [CARD word]]",
        ] {
            let program = lower(
                &format!("{declaration} Value original PROC Main() LET saved=original RETURN"),
                target,
            );
            let before = program.clone();
            let main = program.routines.last().unwrap();
            let analysis = analyze_aggregate_regions(&program).unwrap();
            let facts = analysis.routine(main.id).unwrap();
            let saved = local(main, "saved");
            let extent = saved.ty.as_ref().unwrap().width.unwrap();
            let at = point(&main.blocks[0], 0);
            let region = facts.region(&saved, extent, at).unwrap();
            assert_eq!(region.memory.size, extent);
            assert_eq!(region.memory.offset, ByteOffset::new(0));
            assert_eq!(
                region.identity_domain,
                if target == TargetId::Atari6502 {
                    NirStorageIdentityDomain::Routine(main.id)
                } else {
                    NirStorageIdentityDomain::Invocation(main.id)
                }
            );
            let id = direct_storage_id(&saved).unwrap();
            assert_eq!(facts.storage().homes[&id].ty, saved.ty);
            assert!(
                !facts.storage().homes[&id].is_promotable(),
                "does not relax scalar eligibility"
            );
            assert_eq!(
                facts.relation(&saved, extent, &global(&program, "original"), extent, at),
                Ok(NirRegionRelation::Disjoint)
            );
            assert_eq!(
                facts.region(&saved, ByteSize::new(extent.get() + 1), at),
                Err(NirAggregateProofFailure::OutOfBounds)
            );
            assert_eq!(program, before, "analysis does not rewrite NIR");
        }
    }
}

#[test]
fn union_views_and_nested_ranges_compare_bytes_not_member_names() {
    for target in TARGETS {
        let program = lower(
            "TYPE Pair=[BYTE low,high] TYPE View=UNION [CARD word Pair pair BYTE ARRAY bytes(3)] View value CARD out PROC Main() out=value.word out=value.pair.low RETURN",
            target,
        );
        let main = program.routines.last().unwrap();
        let analysis = analyze_aggregate_regions(&program).unwrap();
        let facts = analysis.routine(main.id).unwrap();
        let base = global(&program, "value");
        let at = point(&main.blocks[0], 0);
        let nested = byte_field(&byte_field(&base, 0), 0);
        assert_eq!(
            facts.relation(
                &byte_field(&base, 0),
                ByteSize::new(1),
                &nested,
                ByteSize::new(1),
                at
            ),
            Ok(NirRegionRelation::Identical)
        );
        assert_eq!(
            facts.relation(
                &base,
                ByteSize::new(2),
                &byte_field(&base, 1),
                ByteSize::new(1),
                at
            ),
            Ok(NirRegionRelation::PartialOverlap)
        );
        assert_eq!(
            facts.relation(
                &byte_field(&base, 0),
                ByteSize::new(1),
                &byte_field(&base, 1),
                ByteSize::new(1),
                at
            ),
            Ok(NirRegionRelation::Disjoint)
        );
        assert_eq!(
            facts.region(
                &byte_field(&byte_field(&base, u32::MAX), 1),
                ByteSize::new(1),
                at
            ),
            Err(NirAggregateProofFailure::OutOfBounds)
        );
    }
}

#[test]
fn snapshot_interval_allows_disjoint_writes_but_rejects_source_mutation() {
    for kind in ["", "UNION "] {
        for (intervening, expected) in [
            ("other.bytes(0)=9", Ok(())),
            (
                "original.bytes(0)=9",
                Err(NirAggregateProofFailure::OverlappingWrite),
            ),
            (
                "original=other",
                Err(NirAggregateProofFailure::OverlappingWrite),
            ),
        ] {
            let program = lower(
                &format!(
                    "TYPE Value={kind}[BYTE ARRAY bytes(3)] Value original,other,output PROC Main() LET saved=original {intervening} output=saved RETURN"
                ),
                TargetId::Atari6502,
            );
            let main = program.routines.last().unwrap();
            let block = &main.blocks[0];
            let analysis = analyze_aggregate_regions(&program).unwrap();
            let facts = analysis.routine(main.id).unwrap();
            let copies: Vec<_> = block
                .ops
                .iter()
                .enumerate()
                .filter_map(|(i, op)| matches!(op, NirOp::CopyBytes { .. }).then_some(i))
                .collect();
            assert_eq!(
                facts.unchanged_between(
                    &global(&program, "original"),
                    ByteSize::new(3),
                    point(block, copies[0] + 1),
                    point(block, *copies.last().unwrap())
                ),
                expected,
                "{kind}/{intervening}\n{}",
                format_program(&program)
            );
        }
    }
}

#[test]
fn unchanged_proof_keeps_effect_and_unknown_pointer_boundaries() {
    for (declarations, middle, expected) in [
        (
            "PROC Touch() RETURN",
            "Touch()",
            NirAggregateProofFailure::CallBoundary,
        ),
        (
            "VOLATILE BYTE status=$D20A BYTE result",
            "result=status",
            NirAggregateProofFailure::VolatileBoundary,
        ),
        (
            "BYTE POINTER raw",
            "raw(0)=9",
            NirAggregateProofFailure::UnknownWrite,
        ),
        (
            "CARD divisor,result",
            "result=9/divisor",
            NirAggregateProofFailure::FaultBoundary,
        ),
        ("", "[ $EA ]", NirAggregateProofFailure::MachineBoundary),
    ] {
        let source = format!(
            "TYPE Value=[BYTE first,second] Value original,output {declarations} PROC Main() LET saved=original {middle} output=saved RETURN"
        );
        let program = lower(&source, TargetId::Atari6502);
        let main = program.routines.last().unwrap();
        let block = &main.blocks[0];
        let copies: Vec<_> = block
            .ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| matches!(op, NirOp::CopyBytes { .. }).then_some(i))
            .collect();
        let analysis = analyze_aggregate_regions(&program).unwrap();
        let result = analysis.routine(main.id).unwrap().unchanged_between(
            &global(&program, "original"),
            ByteSize::new(2),
            point(block, copies[0] + 1),
            point(block, *copies.last().unwrap()),
        );
        assert_eq!(
            result,
            Err(expected),
            "{middle}\n{}",
            format_program(&program)
        );
    }
}

#[test]
fn address_formation_is_distinct_from_escape_but_does_not_imply_privacy() {
    for target in TARGETS {
        for (middle, expected) in [
            ("out=object.first", NirAggregateAddressUse::NotFormed),
            ("out=object.bytes(1)", NirAggregateAddressUse::InternalOnly),
            (
                "raw=BYTE POINTER(@object)",
                NirAggregateAddressUse::ExposedOrUnknown,
            ),
        ] {
            let program = lower(
                &format!(
                    "TYPE Value=[BYTE first BYTE ARRAY bytes(3)] BYTE out BYTE POINTER raw PROC Main() Value object {middle} RETURN"
                ),
                target,
            );
            let main = program.routines.last().unwrap();
            let analysis = analyze_aggregate_regions(&program).unwrap();
            let facts = analysis.routine(main.id).unwrap();
            let place = local(main, "object");
            assert_eq!(
                facts.address_use(direct_storage_id(&place).unwrap()),
                expected,
                "{target:?}/{middle}\n{}",
                format_program(&program)
            );
            for (index, op) in main.blocks[0].ops.iter().enumerate() {
                if let NirOp::AddrOf { dest, ty, place } = op {
                    let deref = NirPlace {
                        kind: NirPlaceKind::Deref {
                            addr: NirValue::Temp {
                                id: *dest,
                                ty: ty.clone(),
                            },
                        },
                        ty: place.ty.clone(),
                    };
                    let at = point(&main.blocks[0], index + 1);
                    assert_eq!(
                        facts.region(&deref, ByteSize::new(1), at),
                        facts.region(place, ByteSize::new(1), at)
                    );
                    assert_eq!(
                        facts.region(&deref, ByteSize::new(1), point(&main.blocks[0], index)),
                        Err(NirAggregateProofFailure::UnavailableAddress)
                    );
                }
            }
        }
    }
}

#[test]
fn absolute_and_alias_storage_are_unknown_not_disjoint() {
    for declarations in [
        "Value original=$700,other=$701",
        "Value original Value other=original",
    ] {
        let program = lower(
            &format!(
                "TYPE Value=[BYTE first,second] {declarations} PROC Main() other=original RETURN"
            ),
            TargetId::Atari6502,
        );
        let main = program.routines.last().unwrap();
        let analysis = analyze_aggregate_regions(&program).unwrap();
        assert!(
            analysis
                .routine(main.id)
                .unwrap()
                .relation(
                    &global(&program, "original"),
                    ByteSize::new(2),
                    &global(&program, "other"),
                    ByteSize::new(2),
                    point(&main.blocks[0], 0)
                )
                .is_err()
        );
    }
}

#[test]
fn invalid_or_cross_block_intervals_and_unverified_input_are_rejected() {
    let program = lower(
        "TYPE Value=[BYTE first,second] Value original BYTE flag PROC Main() IF flag THEN original.first=1 FI RETURN",
        TargetId::Atari6502,
    );
    let main = &program.routines[0];
    let analysis = analyze_aggregate_regions(&program).unwrap();
    let facts = analysis.routine(main.id).unwrap();
    assert_eq!(
        facts.unchanged_between(
            &global(&program, "original"),
            ByteSize::new(2),
            point(&main.blocks[0], 0),
            point(&main.blocks[1], 0)
        ),
        Err(NirAggregateProofFailure::CrossBlockInterval)
    );
    assert_eq!(
        facts.region(
            &global(&program, "original"),
            ByteSize::new(2),
            point(&main.blocks[0], usize::MAX)
        ),
        Err(NirAggregateProofFailure::InvalidPoint)
    );
    let mut invalid = program.clone();
    invalid.routines[0].blocks[0].terminator = NirTerminator::Open;
    assert!(analyze_aggregate_regions(&invalid).is_err());
}

#[test]
fn initializer_address_exposure_cannot_be_mistaken_for_internal_addressing() {
    let program = lower(
        "TYPE Value=[BYTE first,second] CARD result PROC Main() Value object CARD ARRAY addresses=[@object] result=addresses(0) RETURN",
        TargetId::Atari6502,
    );
    let main = program.routines.last().unwrap();
    let analysis = analyze_aggregate_regions(&program).unwrap();
    let facts = analysis.routine(main.id).unwrap();
    let id = direct_storage_id(&local(main, "object")).unwrap();
    assert!(facts.storage().homes[&id].address_in_data);
    assert_eq!(
        facts.address_use(id),
        NirAggregateAddressUse::ExposedOrUnknown
    );
}

#[test]
fn dynamic_indexes_and_loaded_pointers_have_no_exact_range_proof() {
    for body in ["out=object.bytes(index)", "out=raw(0)"] {
        let program = lower(
            &format!(
                "TYPE Value=[BYTE ARRAY bytes(257)] BYTE index,out BYTE POINTER raw PROC Main() Value object {body} RETURN"
            ),
            TargetId::Atari6502,
        );
        let main = program.routines.last().unwrap();
        let block = &main.blocks[0];
        let analysis = analyze_aggregate_regions(&program).unwrap();
        let facts = analysis.routine(main.id).unwrap();
        let (index, place) = block
            .ops
            .iter()
            .enumerate()
            .find_map(|(i, op)| match op {
                NirOp::Load { place, .. }
                    if matches!(
                        place.kind,
                        NirPlaceKind::Index { .. } | NirPlaceKind::Deref { .. }
                    ) =>
                {
                    Some((i, place))
                }
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            facts.region(place, ByteSize::new(1), point(block, index)),
            Err(NirAggregateProofFailure::DynamicIndex | NirAggregateProofFailure::UnknownAddress)
        ));
        let home = local(main, "object");
        assert_eq!(
            facts
                .region(&home, ByteSize::new(257), point(block, 0))
                .unwrap()
                .memory
                .size,
            ByteSize::new(257)
        );
    }
}

#[test]
fn volatile_copies_remain_boundaries_even_between_different_objects() {
    for source_volatile in [false, true] {
        let mut program = lower(
            "TYPE Value=[BYTE first,second] Value original,other,output PROC Main() LET saved=original output=other RETURN",
            TargetId::Atari6502,
        );
        let block = &mut program.routines[0].blocks[0];
        let index = block
            .ops
            .iter()
            .rposition(|op| matches!(op, NirOp::CopyBytes { .. }))
            .unwrap();
        let NirOp::CopyBytes {
            source_volatile: src,
            destination_volatile: dst,
            ..
        } = &mut block.ops[index]
        else {
            unreachable!()
        };
        *src = source_volatile;
        *dst = !source_volatile;
        let main = &program.routines[0];
        let block = &main.blocks[0];
        let analysis = analyze_aggregate_regions(&program).unwrap();
        assert_eq!(
            analysis.routine(main.id).unwrap().unchanged_between(
                &global(&program, "original"),
                ByteSize::new(2),
                point(block, index),
                point(block, index + 1)
            ),
            Err(NirAggregateProofFailure::VolatileBoundary)
        );
    }
}
