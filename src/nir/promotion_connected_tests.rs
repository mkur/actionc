use super::*;

fn program(locals: &str, body: &str) -> NirProgram {
    let source = format!(
        "BYTE input,side,result\nCARD address,wide\nVOLATILE BYTE device=$D20A\nBYTE ARRAY table(256)\nBYTE POINTER target=[$680]\nTYPE R=[BYTE member]\nR left,right\nPROC Touch()\nside==+1\nRETURN\nPROC Main()\n{locals}\n{body}\nRETURN"
    );
    let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let program = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    verify_program(&program).unwrap();
    program
}

fn main(program: &NirProgram) -> &NirRoutine {
    program.routines.iter().find(|r| r.name == "Main").unwrap()
}

fn main_mut(program: &mut NirProgram) -> &mut NirRoutine {
    program
        .routines
        .iter_mut()
        .find(|r| r.name == "Main")
        .unwrap()
}

fn no_local_accesses(program: &NirProgram) -> bool {
    main(program).blocks.iter().flat_map(|b| &b.ops).all(|op| {
        !matches!(op, NirOp::Load { place, .. } | NirOp::Store { place, .. }
            if matches!(direct_storage_id(place), Some(NirStorageId::Local(_))))
    })
}

fn pair_program() -> NirProgram {
    program(
        "BYTE first,second",
        "first=input\nsecond=first XOR $5A\nresult=second",
    )
}

#[test]
fn connected_relays_promote_two_or_more_single_definition_homes() {
    for count in [2, 3, 5] {
        let locals = (0..count)
            .map(|i| format!("v{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut body = "v0=input\n".to_string();
        for i in 1..count {
            body.push_str(&format!("v{i}=v{} XOR $5A\n", i - 1));
        }
        body.push_str(&format!("result=v{}", count - 1));
        let input = program(&format!("BYTE {locals}"), &body);
        assert!(!no_local_accesses(&input));
        let promoted = promote_program(&input).unwrap();
        assert!(no_local_accesses(&promoted), "{promoted:#?}");
        assert_eq!(promote_program(&promoted).unwrap(), promoted);
    }
}

#[test]
fn connected_relays_compose_table_indexing_for_fresh_locals_and_let() {
    for (locals, body) in [
        (
            "BYTE first,second",
            "first=target(0)\nsecond=table(first)\ntarget(0)=second",
        ),
        (
            "",
            "LET first=target(0)\nLET second=table(first)\ntarget(0)=second",
        ),
    ] {
        let input = program(locals, body);
        assert!(no_local_accesses(&promote_program(&input).unwrap()));
        let optimized = crate::nir::optimize_program(&input).unwrap();
        assert!(main(&optimized).locals.is_empty());
        assert!(
            main(&optimized)
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| {
                    matches!(
                        op,
                        NirOp::Load {
                            place: NirPlace {
                                kind: NirPlaceKind::Index { .. },
                                ..
                            },
                            ..
                        }
                    )
                })
        );
    }
}

#[test]
fn connected_relays_leave_isolated_and_unrelated_cold_homes_alone() {
    for body in [
        "first=input\nresult=first",
        "first=input\nside=first\nsecond=input\nresult=second",
    ] {
        let input = program("BYTE first,second", body);
        assert_eq!(promote_program(&input).unwrap(), input);
    }
}

#[test]
fn connected_relays_reject_multiple_home_reads_and_temp_fanout() {
    let input = program(
        "BYTE first,second",
        "first=input\nside=first\nsecond=first XOR $5A\nresult=second",
    );
    assert_eq!(promote_program(&input).unwrap(), input);

    // The home is loaded once, but either its loaded SSA value or an
    // intermediate computation in the connecting path has two consumers.
    for intermediate in [false, true] {
        let mut input = pair_program();
        let routine = main_mut(&mut input);
        let ops = &mut routine.blocks[0].ops;
        let (producer, dest, ty) = ops
            .iter()
            .enumerate()
            .find_map(|(i, op)| match op {
                NirOp::Load { dest, ty, place }
                    if !intermediate
                        && matches!(direct_storage_id(place), Some(NirStorageId::Local(_))) =>
                {
                    Some((i, *dest, ty.clone()))
                }
                NirOp::Binary { dest, ty, .. } if intermediate => Some((i, *dest, ty.clone())),
                _ => None,
            })
            .unwrap();
        let mut extra_use = ops.last().unwrap().clone();
        let NirOp::Store { src, .. } = &mut extra_use else {
            panic!("expected result store")
        };
        *src = NirValue::Temp { id: dest, ty };
        ops.insert(producer + 1, extra_use);
        routine.temps = collect_temps(&routine.blocks);
        assert_eq!(promote_program(&input).unwrap(), input);
    }
}

#[test]
fn connected_relays_reject_cross_block_lifetimes() {
    for body in [
        "first=input\nIF input THEN\nsecond=first XOR $5A\nresult=second\nFI",
        "first=input\nsecond=first XOR $5A\nIF input THEN\nresult=second\nFI",
    ] {
        let input = program("BYTE first,second", body);
        assert_eq!(promote_program(&input).unwrap(), input);
    }
}

#[test]
fn connected_relays_reject_escaped_initialized_and_absolute_homes() {
    for (locals, body) in [
        (
            "BYTE first,second",
            "first=input\naddress=@first\nsecond=first XOR $5A\nresult=second",
        ),
        (
            "BYTE first=[1],second",
            "first=input\nsecond=first XOR $5A\nresult=second",
        ),
        (
            "BYTE first=$700,second",
            "first=input\nsecond=first XOR $5A\nresult=second",
        ),
    ] {
        let input = program(locals, body);
        assert_eq!(promote_program(&input).unwrap(), input);
    }
}

#[test]
fn connected_relays_leave_wider_storage_on_the_existing_tiers() {
    for ty in ["CARD", "INT", "LONGINT", "LONGCARD"] {
        let input = program(
            &format!("{ty} first,second"),
            "first=input\nsecond=first XOR $5A\nwide=second",
        );
        assert_eq!(promote_program(&input).unwrap(), input);
    }
}

#[test]
fn connected_relays_reject_ordering_and_fault_barriers() {
    for barrier in [
        "Touch()",
        "side=device",
        "device=side",
        "[ $EA ]",
        "side=input/side",
        "side=input MOD side",
        "left=right",
    ] {
        for body in [
            format!("first=input\n{barrier}\nsecond=first XOR $5A\nresult=second"),
            format!("first=input\nsecond=first XOR $5A\n{barrier}\nresult=second"),
        ] {
            let input = program("BYTE first,second", &body);
            assert_eq!(
                promote_program(&input).unwrap(),
                input,
                "barrier: {barrier}"
            );
        }
    }
}

#[test]
fn connected_relays_bound_both_storage_and_def_use_gaps() {
    for after_load in [false, true] {
        for gap in [MAX_BOUNDED_RELAY_GAP_OPS, MAX_BOUNDED_RELAY_GAP_OPS + 1] {
            let mut input = pair_program();
            let routine = main_mut(&mut input);
            let ops = &mut routine.blocks[0].ops;
            let load_index = ops
                .iter()
                .position(|op| {
                    matches!(op,
                        NirOp::Load { place, .. }
                            if matches!(direct_storage_id(place), Some(NirStorageId::Local(_)))
                    )
                })
                .unwrap();
            let mut padding = ops.last().unwrap().clone();
            let NirOp::Store { src, .. } = &mut padding else {
                panic!("expected result store")
            };
            *src = NirValue::ConstU8(7);
            // The chain's XOR already occupies one load-to-store gap slot.
            let count = gap - usize::from(after_load);
            for _ in 0..count {
                ops.insert(load_index + usize::from(after_load), padding.clone());
            }
            routine.temps = collect_temps(&routine.blocks);
            let promoted = promote_program(&input).unwrap();
            if gap == MAX_BOUNDED_RELAY_GAP_OPS {
                assert!(no_local_accesses(&promoted));
            } else {
                assert_eq!(promoted, input);
            }
        }
    }
}

#[test]
fn connected_relays_do_not_connect_through_an_unsupported_operation() {
    let mut input = pair_program();
    let routine = main_mut(&mut input);
    let index = routine.blocks[0]
        .ops
        .iter()
        .position(|op| matches!(op, NirOp::Binary { .. }))
        .unwrap();
    routine.blocks[0].ops.insert(
        index,
        NirOp::Unsupported {
            note: "barrier".into(),
        },
    );
    routine.temps = collect_temps(&routine.blocks);
    assert_eq!(promote_program(&input).unwrap(), input);
}
