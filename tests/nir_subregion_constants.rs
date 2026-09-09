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
fn fresh_constructor_baseline_has_explicit_stores_and_unfolded_dispatch() {
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
        2
    );
    assert!(ops.iter().any(|op| matches!(
        op,
        NirOp::Load {
            place: NirPlace {
                kind: NirPlaceKind::Field { .. },
                ..
            },
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
