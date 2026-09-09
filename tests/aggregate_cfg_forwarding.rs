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
    lower_program(&semantic::ir::lower_program(&ast, &model))
}
fn optimize(raw: &NirProgram) -> NirProgram {
    let optimized = optimize_program(raw).unwrap();
    assert_eq!(
        optimized,
        optimize_program(&optimized).unwrap(),
        "fixed point"
    );
    optimized
}
fn saved(program: &NirProgram) -> bool {
    program
        .routines
        .iter()
        .flat_map(|r| &r.locals)
        .any(|l| l.name.ends_with("::saved"))
}

#[test]
fn diamonds_and_loops_require_availability_on_every_incoming_path() {
    for target in TARGETS {
        for (body, retained) in [
            (
                "LET saved=original IF flag THEN other.first=9 ELSE other.second=8 FI out=saved.first",
                false,
            ),
            (
                "LET saved=original IF flag THEN original.first=9 FI out=saved.first",
                true,
            ),
            (
                "LET saved=original FOR i=0 TO 3 DO out=saved.first OD",
                false,
            ),
            (
                "LET saved=original FOR i=0 TO 3 DO out=saved.first original.first=9 OD",
                true,
            ),
            (
                "LET saved=original IF flag THEN Touch() ELSE out=saved.first FI",
                false,
            ),
            (
                "LET saved=original IF flag THEN Touch() FI out=saved.first",
                true,
            ),
            (
                "FOR i=0 TO 3 DO\nBEGIN\noriginal.first=i LET saved=original IF flag THEN other.first=9 FI out=saved.first Touch()\nEND\nOD",
                false,
            ),
        ] {
            let raw = lower(
                &format!(
                    "TYPE Value=[BYTE first,second] Value original,other BYTE flag,out,i PROC Touch() original.first=9 RETURN PROC Main() {body} RETURN"
                ),
                target,
            );
            let optimized = optimize(&raw);
            assert_eq!(
                saved(&optimized),
                retained,
                "{target:?}/{body}\n{}",
                format_program(&optimized)
            );
        }
    }
}

#[test]
fn nested_record_and_union_ranges_use_byte_overlap_not_field_identity() {
    for target in TARGETS {
        for (declaration, write, retained) in [
            ("TYPE Box=[Value child BYTE tail]", "original.tail=9", false),
            (
                "TYPE Box=[Value child BYTE tail]",
                "original.child.second=9",
                true,
            ),
            (
                "TYPE Box=UNION [Value child CARD word]",
                "original.word=9",
                true,
            ),
            (
                "TYPE Box=UNION [Value child CARD word]",
                "other.child.first=9",
                false,
            ),
        ] {
            let raw = lower(
                &format!(
                    "TYPE Value=[BYTE first,second] {declaration} Box original,other BYTE out,flag PROC Main() LET saved=original.child IF flag THEN {write} FI out=saved.first+saved.second RETURN"
                ),
                target,
            );
            let optimized = optimize(&raw);
            assert_eq!(
                saved(&optimized),
                retained,
                "{target:?}/{write}\n{}",
                format_program(&optimized)
            );
        }
    }
}

#[test]
fn minimal_case_and_aggregate_binders_reuse_stable_subobjects() {
    for target in TARGETS {
        let raw = lower(
            "TYPE Payload=[BYTE value] TYPE Event=VARIANT [NONE DATA [Payload item]] Event original BYTE out PROC Main()\nCASE original OF\nWHEN Event.DATA(saved) THEN\nout=saved.value\nELSE\nout=0\nESAC\nRETURN",
            target,
        );
        let optimized = optimize(&raw);
        assert!(!saved(&optimized));
        assert!(
            !optimized.routines[0]
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(op, NirOp::CopyBytes { .. })),
            "{}",
            format_program(&optimized)
        );
        let faults = |p: &NirProgram| {
            p.routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .filter(|op| {
                    matches!(
                        op,
                        NirOp::Call {
                            callee: NirCallee::Fault(_),
                            ..
                        }
                    )
                })
                .count()
        };
        assert_eq!(faults(&raw), faults(&optimized));
    }
}

#[test]
fn mutating_guard_keeps_the_original_image_for_later_arms() {
    for target in TARGETS {
        let raw = lower(
            "TYPE Event=VARIANT [NONE DATA [BYTE value]] Event original BYTE out BYTE FUNC Reject() original=Event.NONE RETURN(0) PROC Main()\nCASE original OF\nWHEN Event.DATA(x) IF Reject() THEN\nout=255\nWHEN Event.DATA(x) THEN\nout=x\nELSE\nout=0\nESAC\nRETURN",
            target,
        );
        let optimized = optimize(&raw);
        assert!(
            optimized
                .routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .any(|op| matches!(op, NirOp::CopyBytes { .. }))
        );
    }
}
