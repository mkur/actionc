use actionc::{nir, semantic, target::TargetId};

fn lower(source: &str, target: TargetId) -> nir::NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(target),
    )
    .unwrap();
    let raw = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&raw).unwrap();
    nir::verify_program(&nir::optimize_program(&raw).unwrap()).unwrap();
    raw
}

fn copies(program: &nir::NirProgram) -> usize {
    program
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .filter(|op| matches!(op, nir::NirOp::CopyBytes { .. }))
        .count()
}

#[test]
fn checked_assignment_validates_in_place_and_transfers_once_without_a_snapshot() {
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        for (declaration, body) in [
            ("Item first,second", "second=first"),
            ("Item first", "first=first"),
            (
                "TYPE Box=[Item value] Box first,second",
                "second.value=first.value",
            ),
            (
                "TYPE Box=[BYTE marker Item left,right] Box pair",
                "pair.right=pair.left",
            ),
            (
                "TYPE Box=[BYTE marker Item left,right] Box pair",
                "pair.right=pair.right",
            ),
            ("Item POINTER first,second", "second^=first^"),
            ("Item first", "LET saved=first"),
        ] {
            let source = format!(
                "TYPE Item=VARIANT [NONE VALUE [CARD number]] {declaration} PROC Main() {body} RETURN"
            );
            let raw = lower(&source, target);
            assert_eq!(
                copies(&raw),
                usize::from(!matches!(body, "first=first" | "pair.right=pair.right")),
                "{target:?}: {source}"
            );
            let captures: Vec<_> = raw
                .routines
                .iter()
                .flat_map(|r| &r.locals)
                .filter(|l| l.purpose == nir::NirLocalPurpose::AggregateCapture)
                .collect();
            assert_eq!(
                captures.len(),
                usize::from(body.starts_with("LET")),
                "{source}"
            );
            assert!(
                raw.routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .any(|op| matches!(
                        op,
                        nir::NirOp::Call {
                            callee: nir::NirCallee::Fault(_),
                            ..
                        }
                    )),
                "value validation must remain"
            );
        }
    }
}

#[test]
fn ordinary_record_and_union_copies_keep_their_overlap_safe_ir_contract() {
    for kind in ["", "UNION "] {
        let source = format!(
            "TYPE View={kind}[CARD word BYTE low] View POINTER first,second PROC Main() second^=first^ RETURN"
        );
        let raw = lower(&source, TargetId::Atari6502);
        assert_eq!(copies(&raw), 1);
        assert!(
            !raw.routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .any(|op| matches!(
                    op,
                    nir::NirOp::Call {
                        callee: nir::NirCallee::Fault(_),
                        ..
                    }
                ))
        );
    }
}

#[test]
fn overlap_checks_use_target_address_width_and_do_not_add_end_addresses() {
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let raw = lower(
            "TYPE Item=VARIANT [NONE VALUE [CARD number]] Item POINTER first,second PROC Main() second^=first^ RETURN",
            target,
        );
        let printed = nir::format_program(&raw);
        // Differences, not address + extent, keep top-of-address-space ranges
        // from wrapping. NIR verification also checks every typed operand.
        let binary: Vec<_> = raw
            .routines
            .iter()
            .flat_map(|r| &r.blocks)
            .flat_map(|b| &b.ops)
            .filter(|op| matches!(op, nir::NirOp::Binary { .. }))
            .collect();
        assert_eq!(binary.len(), 2, "{target:?}: {printed}");
        assert!(
            binary.iter().all(|op| matches!(
                op,
                nir::NirOp::Binary {
                    op: nir::NirBinaryOp::Sub,
                    ..
                }
            )),
            "{printed}"
        );
        for op in binary {
            let nir::NirOp::Binary { ty, .. } = op else {
                unreachable!()
            };
            assert_eq!(
                ty.width.unwrap().get(),
                u32::from(
                    actionc::target::TargetLayout::for_target(target).address_integer_bits / 8
                )
            );
        }
    }
}
