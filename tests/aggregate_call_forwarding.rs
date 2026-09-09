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
    let raw = lower_program(&semantic::ir::lower_program(&ast, &model));
    verify_program(&raw).unwrap();
    raw
}
fn optimize(raw: &NirProgram) -> NirProgram {
    let opt = optimize_program(raw).unwrap();
    assert_eq!(opt, optimize_program(&opt).unwrap());
    opt
}
fn copies(program: &NirProgram) -> usize {
    program
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .filter(|op| matches!(op, NirOp::CopyBytes { .. }))
        .count()
}
fn physical_copies(program: &NirProgram) -> usize {
    actionc::mir6502::lower_program(program)
        .unwrap()
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .filter(|op| matches!(op, actionc::mir6502::MirOp::CopyBytes { .. }))
        .count()
}

#[test]
fn stable_whole_arguments_remove_logical_and_physical_staging_for_every_shape() {
    for target in TARGETS {
        for declaration in [
            "TYPE Value=[BYTE first CARD word BYTE tail]",
            "TYPE Value=UNION [BYTE first CARD word BYTE ARRAY bytes(3)]",
            "TYPE Value=VARIANT [NONE SOME [BYTE first] WIDE [CARD word]]",
        ] {
            let raw = lower(
                &format!(
                    "{declaration} Value original PROC Take(Value a, b) RETURN \
                PROC Main() LET saved=original Take(saved,saved) RETURN"
                ),
                target,
            );
            let opt = optimize(&raw);
            assert_eq!(
                copies(&raw) - copies(&opt),
                2,
                "{target:?}\n{}",
                format_program(&opt)
            );
            if target == TargetId::Atari6502 {
                assert_eq!(physical_copies(&raw) - physical_copies(&opt), 2);
                // Entry copies into separate mutable parameter homes remain.
                assert!(physical_copies(&opt) >= 3);
            }
        }
    }
}

#[test]
fn whole_return_captures_reuse_the_image_but_keep_physical_result_transfer() {
    for target in TARGETS {
        let raw = lower(
            "TYPE Value=VARIANT [NONE SOME [BYTE first]] \
            Value FUNC Make() LET saved=Value.SOME(42) RETURN(saved) PROC Main() RETURN",
            target,
        );
        let opt = optimize(&raw);
        assert_eq!(copies(&raw), 1);
        assert_eq!(copies(&opt), 0);
        assert!(opt.routines[0].blocks.iter().any(|b| matches!(
            b.terminator,
            NirTerminator::Return(Some(NirValue::Aggregate { .. }))
        )));
        if target == TargetId::Atari6502 {
            assert_eq!(physical_copies(&opt), 1);
        }
    }
}

#[test]
fn later_argument_calls_and_indirect_entries_keep_required_capture_boundaries() {
    for target in TARGETS {
        for (prefix, invocation) in [
            ("", "Take(saved,Mutate())"),
            ("callback=Take", "callback(saved,0)"),
        ] {
            let raw = lower(
                &format!(
                    "TYPE Value=[BYTE first,second] Value original BYTE out \
                PROC POINTER callback(Value input BYTE n) \
                BYTE FUNC Mutate() original.first=99 RETURN(0) \
                PROC Take(Value input BYTE n) out=input.first RETURN \
                PROC Main() {prefix} LET saved=original {invocation} RETURN"
                ),
                target,
            );
            let opt = optimize(&raw);
            // LET can disappear into the still-independent argument buffer;
            // that final ABI snapshot must not disappear into mutable storage.
            assert!(copies(&opt) >= 1);
            let main = opt.routines.last().unwrap();
            for op in main.blocks.iter().flat_map(|b| &b.ops) {
                if let NirOp::Call { args, .. } = op {
                    for arg in args {
                        if let NirValue::Aggregate { place } = arg {
                            assert!(matches!(
                                direct_storage_id(place),
                                Some(NirStorageId::Local(_))
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn single_use_native_results_are_routed_to_the_final_fresh_home() {
    for target in TARGETS {
        for declaration in [
            "TYPE Value=[BYTE first CARD word]",
            "TYPE Value=UNION [BYTE first CARD word]",
        ] {
            let raw = lower(
                &format!(
                    "{declaration} Value original BYTE out \
                Value FUNC Make() RETURN(original) \
                PROC Main() LET first=Make() LET saved=first out=saved.first RETURN"
                ),
                target,
            );
            let opt = optimize(&raw);
            let main = opt.routines.last().unwrap();
            if target != TargetId::Atari6502 {
                assert!(!main.locals.iter().any(|l| l.name.ends_with("::first")));
                let saved = main
                    .locals
                    .iter()
                    .find(|l| l.name.ends_with("::saved"))
                    .unwrap();
                assert!(
                    main.blocks
                        .iter()
                        .flat_map(|b| &b.ops)
                        .any(|op| matches!(op,
                    NirOp::Call { aggregate_result: Some(place), .. }
                    if direct_storage_id(place) == Some(NirStorageId::Local(saved.id))))
                );
                if target == TargetId::Motorola68000 {
                    actionc::mir68k::lower_program(&opt).unwrap();
                } else {
                    actionc::mir65816::lower_program(&opt).unwrap();
                }
            } else {
                // Static calls retain their separately captured result image.
                assert!(main.blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(op,
                    NirOp::CopyBytes { source, .. } if matches!(direct_storage_id(source), Some(NirStorageId::Local(_))))));
            }
        }
    }
}

#[test]
fn verifier_still_rejects_subobject_abi_arguments_after_forwarding() {
    let mut raw = lower(
        "TYPE Value=[BYTE first,second] TYPE Box=[Value child] Box original \
        PROC Take(Value input) RETURN PROC Main() LET saved=original.child Take(saved) RETURN",
        TargetId::Atari6502,
    );
    let main = raw.routines.last_mut().unwrap();
    let source = main
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| match op {
            NirOp::CopyBytes { source, .. }
                if matches!(source.kind, NirPlaceKind::Field { .. }) =>
            {
                Some(source.clone())
            }
            _ => None,
        })
        .unwrap();
    for op in main.blocks.iter_mut().flat_map(|b| &mut b.ops) {
        if let NirOp::Call { args, .. } = op {
            for arg in args {
                if let NirValue::Aggregate { place } = arg {
                    **place = source.clone();
                }
            }
        }
    }
    assert!(verify_program(&raw).is_err());
    assert!(optimize_program(&raw).is_err());
}

#[test]
fn intervening_calls_and_result_aliases_do_not_reuse_an_input_image() {
    for target in TARGETS {
        for aliases_result in [false, true] {
            let later = if aliases_result { "0" } else { "Touch()" };
            let mut raw = lower(
                &format!(
                    "TYPE Value=[BYTE first] Value original BYTE out \
                Value FUNC Make() RETURN(original) BYTE FUNC Touch() RETURN(0) \
                Value FUNC Update(Value input BYTE later) input.first=99 RETURN(input) \
                PROC Main() LET saved=Make() LET result=Update(saved,{later}) out=result.first RETURN"
                ),
                target,
            );
            let main = raw.routines.last_mut().unwrap();
            let capture = main
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .find_map(|op| match op {
                    NirOp::Call {
                        callee: NirCallee::User { name, .. },
                        args,
                        ..
                    } if name == "Update" => {
                        let NirValue::Aggregate { place } = &args[0] else {
                            unreachable!()
                        };
                        direct_storage_id(place)
                    }
                    _ => None,
                })
                .unwrap();
            if aliases_result {
                // Legal whole-buffer NIR, but the proposed argument source
                // is also the call's output. Keep independent input staging.
                let source = main
                    .blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .find_map(|op| match op {
                        NirOp::CopyBytes {
                            destination,
                            source,
                            ..
                        } if direct_storage_id(destination) == Some(capture) => {
                            Some(source.clone())
                        }
                        _ => None,
                    })
                    .unwrap();
                for op in main.blocks.iter_mut().flat_map(|b| &mut b.ops) {
                    if let NirOp::Call {
                        callee: NirCallee::User { name, .. },
                        aggregate_result,
                        ..
                    } = op
                        && name == "Update"
                    {
                        *aggregate_result = Some(source.clone());
                    }
                }
            }
            verify_program(&raw).unwrap();
            let opt = optimize(&raw);
            assert!(opt.routines.last().unwrap().blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(op,
                NirOp::Call { callee: NirCallee::User { name, .. }, args, .. }
                if name == "Update" && matches!(&args[0], NirValue::Aggregate { place } if direct_storage_id(place) == Some(capture)))),
                "{target:?}/aliases_result={aliases_result}\n{}", format_program(&opt));
        }
    }
}
