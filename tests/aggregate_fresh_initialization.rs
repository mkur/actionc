use actionc::{
    nir::{self, *},
    semantic::{self, SemanticOptions, ir::*},
    target::{RoutineActivationModel, TargetId},
};

const TARGETS: [TargetId; 4] = [
    TargetId::Atari6502,
    TargetId::Motorola68000,
    TargetId::Wdc65816Small,
    TargetId::Wdc65816Native,
];

fn lower(source: &str, target: TargetId) -> (SemProgram, NirProgram) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap())
        .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    let semir = semantic::ir::lower_program(&ast, &model);
    let raw = nir::lower_program(&semir);
    nir::verify_program(&raw).unwrap();
    let optimized = nir::optimize_program(&raw).unwrap();
    assert_eq!(optimized, nir::optimize_program(&optimized).unwrap());
    (semir, raw)
}

fn main_semir(program: &SemProgram) -> &SemRoutine {
    program
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .find_map(|i| match i {
            SemItem::Routine(r) if r.symbol.name == "Main" => Some(r),
            _ => None,
        })
        .unwrap()
}

fn main_nir(program: &NirProgram) -> &NirRoutine {
    program.routines.iter().find(|r| r.name == "Main").unwrap()
}

fn copies(routine: &NirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .filter(|op| matches!(op, NirOp::CopyBytes { .. }))
        .count()
}

const MAYBE: &str = "TYPE MaybeByte=VARIANT [NONE SOME [BYTE value]]";

#[test]
fn fresh_literal_uses_its_binding_home_but_case_still_takes_a_snapshot() {
    for target in TARGETS {
        let source = format!("{MAYBE} PROC Main() LET item=MaybeByte.SOME(42) RETURN");
        let (semir, raw) = lower(&source, target);
        let main = main_semir(&semir);
        assert!(
            main.locals.is_empty(),
            "no constructor/destination-pointer home"
        );
        let SemStmt::LexicalBlock {
            declarations, body, ..
        } = &main.body[0]
        else {
            panic!()
        };
        let id = declarations[0].symbol.id;
        // Native layouts can insert gap stores between payload and tag. The
        // actual canonical field stores still go to the fresh binding itself.
        let fields: Vec<_> = body
            .iter()
            .filter_map(|stmt| match stmt {
                SemStmt::Assign {
                    target:
                        SemLValue {
                            kind: SemLValueKind::Field { base, field },
                            ..
                        },
                    ..
                } => {
                    assert!(matches!(&base.kind, SemLValueKind::Symbol(s) if s.id == id));
                    Some(field.name.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[1], "__variant_tag");
        assert_eq!(copies(main_nir(&raw)), 0);
        assert_eq!(
            main_nir(&raw)
                .locals
                .iter()
                .filter(|l| l.purpose == NirLocalPurpose::AggregateCapture)
                .count(),
            1
        );

        let (_, live) = lower(
            &format!(
                "{MAYBE} BYTE output PROC Main()\n\
            LET item=MaybeByte.SOME(42)\nCASE item OF\n\
            WHEN MaybeByte.NONE THEN\noutput=0\n\
            WHEN MaybeByte.SOME(value) THEN\noutput=value\nESAC\nRETURN"
            ),
            target,
        );
        assert_eq!(
            copies(main_nir(&live)),
            1,
            "CASE remains a separate snapshot"
        );
    }
}

#[test]
fn runtime_integer_inputs_and_nested_constructors_initialize_in_place() {
    for target in TARGETS {
        for expression in ["MaybeByte.SOME(seed+1)", "MaybeByte.NONE"] {
            let (semir, raw) = lower(
                &format!(
                    "{MAYBE} BYTE seed PROC Main() \
                LET item={expression} RETURN"
                ),
                target,
            );
            assert_eq!(copies(main_nir(&raw)), 0);
            assert!(
                !main_semir(&semir)
                    .locals
                    .iter()
                    .any(|l| l.ty.value.is_record())
            );
        }
        let (semir, raw) = lower(
            &format!(
                "{MAYBE} \
            TYPE Outer=VARIANT [NONE WRAP [MaybeByte inner]] \
            PROC Main() LET item=Outer.WRAP(MaybeByte.SOME(42)) RETURN"
            ),
            target,
        );
        assert_eq!(copies(main_nir(&raw)), 0);
        assert!(
            !main_semir(&semir)
                .locals
                .iter()
                .any(|l| l.ty.value.is_record())
        );
    }
}

#[test]
fn record_union_and_checked_variant_snapshots_keep_one_complete_copy() {
    for target in TARGETS {
        for definition in [
            "TYPE Value=[BYTE head CARD word BYTE ARRAY tail(17)]",
            "TYPE Value=UNION [CARD word BYTE ARRAY bytes(17)]",
            "TYPE Value=VARIANT [NONE SOME [CARD number]]",
        ] {
            let (semir, raw) = lower(
                &format!(
                    "{definition} Value source \
                PROC Main() LET saved=source RETURN"
                ),
                target,
            );
            assert!(main_semir(&semir).locals.is_empty());
            let main = main_nir(&raw);
            assert_eq!(copies(main), 1);
            let home = main
                .locals
                .iter()
                .find(|l| l.purpose == NirLocalPurpose::AggregateCapture)
                .unwrap();
            let copy = main
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .find_map(|op| match op {
                    NirOp::CopyBytes {
                        destination, size, ..
                    } => Some((destination, size)),
                    _ => None,
                })
                .unwrap();
            assert!(matches!(copy.0.kind, NirPlaceKind::Local { id, .. } if id == home.id));
            assert_eq!(
                *copy.1, home.layout.size,
                "complete image, including padding/tail"
            );
            if definition.contains("VARIANT") {
                assert!(
                    main.blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(
                        op,
                        NirOp::Call {
                            callee: NirCallee::Fault(_),
                            ..
                        }
                    )),
                    "required validation survives"
                );
            }
        }
        let (semir, raw) = lower(
            "TYPE Row=[BYTE head CARD word] \
            TYPE Bits=UNION [CARD word BYTE ARRAY bytes(3)] \
            TYPE Value=VARIANT [NONE DATA [Row row Bits bits]] \
            Row sourceRow Bits sourceBits PROC Main() \
            LET item=Value.DATA(sourceRow,sourceBits) RETURN",
            target,
        );
        assert_eq!(
            copies(main_nir(&raw)),
            2,
            "one snapshot per payload, no relays"
        );
        assert!(
            !main_semir(&semir)
                .locals
                .iter()
                .any(|l| l.ty.value.is_record())
        );
    }
}

#[test]
fn effects_faults_unknown_backing_and_replacement_keep_constructor_staging() {
    for target in TARGETS {
        for (declarations, expression) in [
            ("BYTE FUNC Next() RETURN(42)", "Next()"),
            ("BYTE divisor", "42/divisor"),
            ("BYTE divisor", "42 MOD divisor"),
            ("BYTE alias=$680", "alias"),
            ("VOLATILE BYTE device=$680", "device"),
            ("BYTE original BYTE alias=@original", "alias"),
            ("BYTE POINTER link", "link^"),
            ("BYTE ARRAY bytes(4) BYTE index", "bytes(index)"),
        ] {
            let (_, raw) = lower(
                &format!(
                    "{MAYBE} {declarations} \
                PROC Main() LET item=MaybeByte.SOME({expression}) RETURN"
                ),
                target,
            );
            assert_eq!(copies(main_nir(&raw)), 1, "{target:?}: {expression}");
        }
        let (_, raw) = lower(
            &format!(
                "{MAYBE} MaybeByte existing \
            PROC Main() existing=MaybeByte.SOME(42) RETURN"
            ),
            target,
        );
        assert_eq!(copies(main_nir(&raw)), 1, "replacement remains staged");
    }
}

#[test]
fn whole_native_results_use_the_binding_but_static_and_nested_results_stay_captured() {
    for target in TARGETS {
        for definition in [
            "TYPE Value=[BYTE number]",
            "TYPE Value=UNION [CARD number BYTE ARRAY bytes(3)]",
            "TYPE Value=VARIANT [NONE SOME [BYTE number]]",
        ] {
            let (semir, raw) = lower(
                &format!(
                    "{definition} Value source \
                Value FUNC Make() RETURN(source) \
                PROC Main() LET saved=Make() RETURN"
                ),
                target,
            );
            let native =
                semir.target_layout.routine_activation == RoutineActivationModel::NativeReentrant;
            let main = main_nir(&raw);
            assert_eq!(copies(main), usize::from(!native));
            let result = main
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .find_map(|op| match op {
                    NirOp::Call {
                        aggregate_result: Some(place),
                        ..
                    } => Some(place),
                    _ => None,
                })
                .unwrap();
            let NirPlaceKind::Local { id, .. } = result.kind else {
                panic!("whole ABI home")
            };
            let home = main.locals.iter().find(|l| l.id == id).unwrap();
            assert_eq!(home.purpose, NirLocalPurpose::AggregateCapture);
            assert_eq!(home.name.ends_with("::saved"), native);
        }
        let (_, raw) = lower(
            &format!(
                "{MAYBE} \
            TYPE Outer=VARIANT [NONE WRAP [MaybeByte inner]] \
            MaybeByte FUNC Make() RETURN(MaybeByte.SOME(42)) \
            PROC Main() LET item=Outer.WRAP(Make()) RETURN"
            ),
            target,
        );
        assert_eq!(copies(main_nir(&raw)), 2);
        assert!(
            main_nir(&raw)
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .all(|op| match op {
                    NirOp::Call {
                        aggregate_result: Some(place),
                        ..
                    } => matches!(place.kind, NirPlaceKind::Local { .. }),
                    _ => true,
                })
        );
    }
}

#[test]
fn shadowed_bindings_use_distinct_storage_identities() {
    for target in TARGETS {
        let (_, raw) = lower(
            &format!(
                "{MAYBE} PROC Main() \
            LET item=MaybeByte.SOME(42) LET item=item RETURN"
            ),
            target,
        );
        let main = main_nir(&raw);
        assert_eq!(copies(main), 1);
        let (source, destination) = main
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .find_map(|op| match op {
                NirOp::CopyBytes {
                    source,
                    destination,
                    ..
                } => Some((source, destination)),
                _ => None,
            })
            .unwrap();
        assert_ne!(source.kind, destination.kind);
    }
}
