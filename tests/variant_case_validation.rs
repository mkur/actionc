use actionc::{
    nir::{self, *},
    semantic::{self, SemanticOptions, ir::*},
    target::TargetId,
};

const TARGETS: [TargetId; 4] = [
    TargetId::Atari6502,
    TargetId::Motorola68000,
    TargetId::Wdc65816Small,
    TargetId::Wdc65816Native,
];

fn lower(source: &str, target: TargetId) -> (SemProgram, NirProgram) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap();
    let semir = semantic::ir::lower_program(&ast, &model);
    let raw = nir::lower_program(&semir);
    nir::verify_program(&raw).unwrap();
    let opt = nir::optimize_program(&raw).unwrap();
    assert_eq!(opt, nir::optimize_program(&opt).unwrap());
    (semir, raw)
}

fn cases(program: &SemProgram) -> Vec<&[SemCaseArm]> {
    fn visit<'a>(body: &'a [SemStmt], output: &mut Vec<&'a [SemCaseArm]>) {
        for stmt in body {
            match stmt {
                SemStmt::LexicalBlock { body, .. } => visit(body, output),
                SemStmt::Case { arms, .. } => output.push(arms),
                _ => {}
            }
        }
    }
    let main = program
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .find_map(|i| match i {
            SemItem::Routine(r) if r.symbol.name == "Main" => Some(r),
            _ => None,
        })
        .unwrap();
    let mut result = Vec::new();
    visit(&main.body, &mut result);
    result
}

#[test]
fn scalar_record_and_union_payloads_use_dispatch_without_preliminary_validation() {
    for target in TARGETS {
        for payload in ["BYTE", "RecordPayload", "UnionPayload"] {
            let (semir, raw) = lower(
                &format!(
                    "TYPE RecordPayload=[BYTE x] \
                TYPE UnionPayload=UNION [BYTE x CARD word] TYPE Value=VARIANT [NONE SOME [{payload} value]] \
                Value current BYTE out PROC Main()\nCASE current OF\nWHEN Value.NONE THEN\nout=1\n\
                WHEN Value.SOME(_) THEN\nout=2\nESAC\nRETURN"
                ),
                target,
            );
            let cases = cases(&semir);
            assert_eq!(cases.len(), 1, "no preliminary validation CASE");
            let fallback = cases[0].last().unwrap();
            assert!(fallback.labels.is_none() && fallback.guard.is_none());
            assert!(matches!(&fallback.body[..], [SemStmt::Fault { .. }]));
            let main = raw.routines.last().unwrap();
            let comparisons: Vec<_> = main
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter_map(|op| match op {
                    NirOp::Compare { op, .. } => Some(*op),
                    _ => None,
                })
                .collect();
            assert_eq!(comparisons, [NirCompareOp::Eq, NirCompareOp::Eq]);
            assert_eq!(
                main.blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .filter(|op| matches!(
                        op,
                        NirOp::Call {
                            callee: NirCallee::Fault(_),
                            ..
                        }
                    ))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn else_and_wildcard_guards_are_restricted_to_valid_tags_before_the_final_fault() {
    for target in TARGETS {
        let (semir, _) = lower(
            "TYPE Value=VARIANT [NONE SOME [BYTE value]] Value current BYTE out \
            BYTE FUNC Test() out=99 RETURN(0) PROC Main()\nCASE current OF\n\
            WHEN Value.SOME(n) IF Test() THEN\nout=n\n\
            WHEN _ IF Test() THEN\nout=1\nELSE\nout=2\nESAC\nRETURN",
            target,
        );
        let cases = cases(&semir);
        assert_eq!(cases.len(), 1);
        let arms = cases[0];
        assert_eq!(arms.len(), 4);
        for arm in &arms[1..3] {
            let labels = arm.labels.as_ref().unwrap();
            assert_eq!(labels.len(), 1);
            assert_eq!((labels[0].low, labels[0].high), (1, 2));
        }
        assert!(arms[1].guard.is_some() && arms[2].guard.is_none());
        assert!(matches!(&arms[3].body[..], [SemStmt::Fault { .. }]));
    }
}

#[test]
fn active_inline_variants_keep_early_validation_even_when_payload_is_ignored() {
    for target in TARGETS {
        for payload in ["Inner", "Container"] {
            let (semir, _) = lower(
                &format!(
                    "TYPE Inner=VARIANT [OFF ON] \
                TYPE Container=[Inner ARRAY entries(2)] TYPE Value=VARIANT [NONE SOME [{payload} value]] \
                Value current BYTE out BYTE FUNC Test() out=99 RETURN(1) \
                PROC Main()\nCASE current OF\nWHEN Value.SOME(_) IF Test() THEN\nout=1\n\
                ELSE\nout=2\nESAC\nRETURN"
                ),
                target,
            );
            assert_eq!(
                cases(&semir).len(),
                2,
                "nested validity precedes source dispatch"
            );
        }
    }
}

#[test]
fn aggregate_call_selector_is_evaluated_once_and_only_its_final_check_is_deferred() {
    for target in TARGETS {
        let (semir, raw) = lower(
            "TYPE Value=VARIANT [NONE SOME [BYTE value]] BYTE out \
            Value FUNC Make() RETURN(Value.SOME(42)) PROC Main()\n\
            CASE Make() OF\nWHEN Value.NONE THEN\nout=0\nWHEN Value.SOME(n) THEN\nout=n\nESAC\nRETURN",
            target,
        );
        assert_eq!(cases(&semir).len(), 1);
        assert_eq!(
            raw.routines
                .last()
                .unwrap()
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|op| matches!(
                    op,
                    NirOp::Call {
                        callee: NirCallee::User { .. },
                        ..
                    }
                ))
                .count(),
            1
        );
    }
}
