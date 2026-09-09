#[path = "support/aggregate_forwarding_cases.rs"]
mod cases;

use actionc::{
    nir::*,
    semantic::{self, SemanticOptions},
    target::TargetId,
};

#[test]
fn aggregate_forwarding_corpus_is_verified_and_analysis_only_on_every_target() {
    eprintln!(
        "target,shape,case,stage,copy_sites,copied_bytes,capture_bytes,exact_copy_endpoints,unknown_copy_endpoints"
    );
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        for shape in cases::SHAPES {
            for case in cases::CASES.into_iter().chain(cases::ABI_CASES) {
                let source = cases::source(shape, case);
                let ast =
                    actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
                let model = semantic::analyze_with_options(
                    &ast,
                    SemanticOptions::modern().with_target(target),
                )
                .unwrap();
                let raw = lower_program(&semantic::ir::lower_program(&ast, &model));
                verify_program(&raw).unwrap();
                let optimized = optimize_program(&raw).unwrap();
                assert_eq!(
                    optimized,
                    optimize_program(&optimized).unwrap(),
                    "existing optimizer idempotence"
                );
                for (stage, program) in [("raw", &raw), ("optimized", &optimized)] {
                    let before = program.clone();
                    let analysis = analyze_aggregate_regions(program).unwrap();
                    let (mut copies, mut bytes, mut exact, mut unknown) = (0, 0u64, 0, 0);
                    for routine in &program.routines {
                        let facts = analysis.routine(routine.id).unwrap();
                        for block in &routine.blocks {
                            for (op_index, op) in block.ops.iter().enumerate() {
                                if let NirOp::CopyBytes {
                                    source,
                                    destination,
                                    size,
                                    ..
                                } = op
                                {
                                    copies += 1;
                                    bytes += u64::from(size.get());
                                    for place in [source, destination] {
                                        if facts
                                            .region(
                                                place,
                                                *size,
                                                NirAggregatePoint {
                                                    block: block.id,
                                                    op_index,
                                                },
                                            )
                                            .is_ok()
                                        {
                                            exact += 1;
                                        } else {
                                            unknown += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let captures: u64 = program
                        .routines
                        .iter()
                        .flat_map(|r| &r.locals)
                        .filter(|l| l.purpose == NirLocalPurpose::AggregateCapture)
                        .map(|l| u64::from(l.layout.size.get()))
                        .sum();
                    assert_eq!(program, &before);
                    assert!(captures > 0, "whole ABI/producer homes remain allocated");
                    if stage == "raw" {
                        assert!(copies > 0);
                    }
                    eprintln!(
                        "{target:?},{shape},{case},{stage},{copies},{bytes},{captures},{exact},{unknown}"
                    );
                    let result = match target {
                        TargetId::Atari6502 => {
                            actionc::mir6502::lower_program(program).unwrap();
                            Ok(())
                        }
                        TargetId::Motorola68000 => actionc::mir68k::lower_program(program)
                            .map(|_| ())
                            .map_err(|error| match error {
                                actionc::backend::BackendLoweringError::Backend(errors) => {
                                    errors.into_iter().map(|e| e.message).collect::<Vec<_>>()
                                }
                                error => panic!("{error:?}"),
                            }),
                        _ => actionc::mir65816::lower_program(program)
                            .map(|_| ())
                            .map_err(|error| match error {
                                actionc::backend::BackendLoweringError::Backend(errors) => {
                                    errors.into_iter().map(|e| e.message).collect::<Vec<_>>()
                                }
                                error => panic!("{error:?}"),
                            }),
                    };
                    if let Err(errors) = result {
                        // Existing native runtime capability gate, not native
                        // execution coverage or an error suppressed by analysis.
                        assert_eq!(shape, "variant");
                        assert!(!errors.is_empty());
                        assert!(errors.iter().all(|e| e == "invalid-variant runtime fault requires a native target Error adapter"), "{errors:?}");
                    }
                }
                assert_eq!(
                    cases::expected(case, 255)[1],
                    0,
                    "host byte arithmetic wraps"
                );
            }
        }
    }
}
