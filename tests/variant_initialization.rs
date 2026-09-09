use std::ops::Range;

use actionc::{
    nir,
    semantic::{self, SemanticOptions, ir::*},
    target::TargetId,
};

const TARGETS: [TargetId; 4] = [
    TargetId::Atari6502,
    TargetId::Motorola68000,
    TargetId::Wdc65816Native,
    TargetId::Wdc65816Small,
];

#[derive(Debug)]
struct Write {
    bytes: Range<u32>,
    zero: bool,
    tag: bool,
}

fn constant(expr: &SemExpr) -> u32 {
    let SemExprKind::Literal(SemLiteral::Constant(value)) = &expr.kind else {
        panic!("expected generated constant: {expr:?}")
    };
    u32::try_from(value.bits).unwrap()
}

// Record the constructor's actual SemIR writes, not just its layout plan.
// The sources below have one constructor and no user loops/field assignments.
fn writes(body: &[SemStmt], out: &mut Vec<Write>) {
    for stmt in body {
        match stmt {
            SemStmt::LexicalBlock { body, .. } => writes(body, out),
            SemStmt::Assign { target, value, .. } => match &target.kind {
                SemLValueKind::Field { field, .. } => out.push(Write {
                    bytes: field.offset.unwrap()..field.offset.unwrap() + field.size,
                    zero: false,
                    tag: field.name == "__variant_tag",
                }),
                SemLValueKind::Index { index, .. } => {
                    assert_eq!(constant(value), 0);
                    let offset = constant(index);
                    out.push(Write {
                        bytes: offset..offset + 1,
                        zero: true,
                        tag: false,
                    });
                }
                _ => {}
            },
            SemStmt::For {
                start, end, body, ..
            } => {
                assert_eq!(body.len(), 1);
                let SemStmt::Assign { target, value, .. } = &body[0] else {
                    panic!("unexpected gap loop: {body:?}")
                };
                assert!(matches!(target.kind, SemLValueKind::Index { .. }));
                assert_eq!(constant(value), 0);
                out.push(Write {
                    bytes: constant(start)..constant(end) + 1,
                    zero: true,
                    tag: false,
                });
            }
            SemStmt::RecordCopy {
                destination, size, ..
            } => {
                if let SemLValueKind::Field { field, .. } = &destination.kind {
                    assert_eq!(*size, field.size);
                    out.push(Write {
                        bytes: field.offset.unwrap()..field.offset.unwrap() + size,
                        zero: false,
                        tag: false,
                    });
                }
            }
            _ => {}
        }
    }
}

fn check(source: &str, target: TargetId) -> Vec<Range<u32>> {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap();
    let variant = model
        .variants
        .types
        .values()
        .find(|v| v.identity.name == "Value")
        .unwrap();
    let semir = semantic::ir::lower_program(&ast, &model);
    let main = semir
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .find_map(|item| match item {
            SemItem::Routine(r) if r.symbol.name == "Main" => Some(r),
            _ => None,
        })
        .unwrap();
    let mut actual = Vec::new();
    writes(&main.body, &mut actual);
    let mut coverage = vec![0u8; variant.size as usize];
    for write in &actual {
        for offset in write.bytes.clone() {
            coverage[offset as usize] += 1;
        }
    }
    assert!(
        coverage.iter().all(|n| *n == 1),
        "{target:?}: {actual:?}: {coverage:?}"
    );
    assert!(
        actual.last().unwrap().tag,
        "tag must be the last constructor write: {actual:?}"
    );
    assert_eq!(actual.iter().filter(|w| w.tag).count(), 1);
    let payload_end = actual.iter().rposition(|w| !w.zero && !w.tag);
    let first_zero = actual.iter().position(|w| w.zero);
    if let (Some(payload_end), Some(first_zero)) = (payload_end, first_zero) {
        assert!(
            payload_end < first_zero,
            "payload evaluation precedes padding: {actual:?}"
        );
    }
    let raw = nir::lower_program(&semir);
    nir::verify_program(&raw).unwrap();
    let optimized = nir::optimize_program(&raw).unwrap();
    for program in [&raw, &optimized] {
        nir::verify_program(program).unwrap();
        match target {
            TargetId::Atari6502 => {
                actionc::mir6502::lower_program(program).unwrap();
            }
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(program).unwrap();
            }
            _ if variant.size < 256 => {
                actionc::mir65816::lower_program(program).unwrap();
            }
            // The 65816 backend currently limits automatic objects to its
            // initial 8-bit stack displacement range. Large gaps still verify
            // as target-width NIR; Atari executes the page-crossing case.
            _ => {}
        }
    }
    actual
        .into_iter()
        .filter(|w| w.zero)
        .map(|w| w.bytes)
        .collect()
}

#[test]
fn fully_written_and_tag_only_constructors_need_no_clear() {
    for target in TARGETS {
        for source in [
            "TYPE Value=VARIANT [NONE SOME [BYTE value]] Value current PROC Main() current=Value.SOME(42) RETURN",
            "TYPE Value=VARIANT [ONLY] Value current PROC Main() current=Value.ONLY RETURN",
        ] {
            assert!(check(source, target).is_empty());
        }
        assert_eq!(
            check(
                "TYPE Value=VARIANT [NONE SOME [BYTE value]] Value current PROC Main() current=Value.NONE RETURN",
                target,
            ),
            vec![1..2]
        );
    }
}

#[test]
fn construction_zeros_only_alignment_gaps_and_unused_alternative_bytes() {
    for target in TARGETS {
        assert_eq!(
            check(
                "TYPE Value=VARIANT [NONE DATA [BYTE a CARD b BYTE c]] Value current PROC Main() current=Value.DATA(11,$2345,67) RETURN",
                target,
            ),
            if target == TargetId::Atari6502 {
                vec![]
            } else {
                vec![1..2, 3..4, 7..8]
            }
        );
        assert_eq!(
            check(
                "TYPE Value=VARIANT [NONE WIDE [CARD first,second] SMALL [BYTE value]] Value current PROC Main() current=Value.SMALL(42) RETURN",
                target,
            ),
            if target == TargetId::Atari6502 {
                vec![2..5]
            } else {
                vec![1..2, 3..6]
            }
        );
        assert_eq!(
            check(
                "TYPE Payload=[BYTE ARRAY bytes(257)] TYPE Value=VARIANT [EMPTY DATA [Payload payload]] Value current PROC Main() current=Value.EMPTY RETURN",
                target,
            ),
            vec![1..258]
        );
    }
}

#[test]
fn complete_aggregate_and_target_sized_scalar_payloads_are_not_cleared() {
    for target in TARGETS {
        check(
            "TYPE Payload=[BYTE head CARD word BYTE tail] TYPE Bits=UNION [CARD word BYTE ARRAY bytes(3)] TYPE Value=VARIANT [NONE DATA [Payload payload Bits bits CARD POINTER link]] Payload sourceRecord Bits sourceBits Value current CARD number PROC Main() current=Value.DATA(sourceRecord,sourceBits,@number) RETURN",
            target,
        );
        check(
            "TYPE Value=VARIANT [NONE DATA [LONGCARD number]] Value current PROC Main() current=Value.DATA(LONGCARD(42)) RETURN",
            target,
        );
    }
}
