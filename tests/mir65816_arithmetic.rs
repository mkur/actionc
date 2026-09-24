use actionc::{
    lexer,
    mir65816::{self, *},
    nir, parser, semantic,
    target::TargetId,
};
fn mir(source: &str, optimize: bool) -> Mir65816Program {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
    )
    .unwrap();
    let n = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let n = if optimize {
        nir::optimize_program_with_promotion(&n, nir::NirPromotionPolicy::Native65816).unwrap()
    } else {
        n
    };
    mir65816::lower_program(&n).unwrap()
}
fn layout() -> image::LinkOptions {
    serde_json::from_str(r#"{"code_origin":"$01fff0","data_origin":"$12fffc","stack_overflow":"$048000","arithmetic_fault":"$049000","nmi_extra_stack":0,"imports":[]}"#).unwrap()
}
#[test]
fn every_legal_family_has_one_verified_leaf_and_an_ordinary_call() {
    for optimize in [false, true] {
        for (ty, width, signed) in [
            ("BYTE", 1, false),
            ("CARD", 2, false),
            ("INT", 2, true),
            ("SIZE", 3, false),
            ("LONGCARD", 4, false),
            ("LONGINT", 4, true),
        ] {
            let p = mir(
                &format!("{ty} a,b,q,r PROC Main() q=a/b r=a MOD b q=a/b RETURN"),
                optimize,
            );
            let prepared = arithmetic::prepare(&p).unwrap();
            assert_eq!(arithmetic::prepare(&prepared).unwrap(), prepared);
            let helpers: Vec<_> = prepared
                .routines
                .iter()
                .filter(|r| r.helper.is_some())
                .collect();
            assert_eq!(helpers.len(), 2);
            for r in helpers {
                let h = r.helper.unwrap();
                assert_eq!((h.bytes, h.signed), (width, signed));
                assert_eq!(r.frame.extent.get(), 0);
                assert!(r.entry.source_span.is_none());
            }
            let machine = emit::materialize(&p).unwrap();
            assert_eq!(machine.prepared, prepared);
            let i = image::link(&p, &machine, &layout()).unwrap();
            assert_eq!(i.version, 4);
            image::Image::from_json(&i.to_json().unwrap()).unwrap();
            for r in &i.routines {
                assert_eq!(r.address >> 16, (r.address + r.size - 1) >> 16);
            }
            let mut missing = layout();
            missing.arithmetic_fault = None;
            assert!(
                image::link(&p, &machine, &missing)
                    .unwrap_err()
                    .contains("arithmetic_fault")
            );
            let mut malformed = prepared.clone();
            malformed
                .routines
                .iter_mut()
                .find(|r| r.helper.is_some())
                .unwrap()
                .helper
                .as_mut()
                .unwrap()
                .bytes = 5;
            assert!(verify_program(&malformed).is_err());
            let mut malformed = prepared.clone();
            for op in malformed
                .routines
                .iter_mut()
                .flat_map(|r| &mut r.blocks)
                .flat_map(|b| &mut b.ops)
            {
                if let Mir65816Op::Call {
                    target: Mir65816CallTarget::Helper(_),
                    args,
                    ..
                } = op
                {
                    args[0] = Mir65816Value::U32(9);
                    break;
                }
            }
            if width != 4 {
                assert!(verify_program(&malformed).is_err());
            }
        }
    }
}
#[test]
fn constant_reductions_precede_helper_collection_in_both_modes() {
    for optimize in [false, true] {
        for (ty, bits) in [("BYTE", 8), ("CARD", 16), ("SIZE", 24), ("LONGCARD", 32)] {
            for n in 0..bits {
                let constant = if n < 16 {
                    format!("{ty}(CARD({}))", 1u64 << n)
                } else {
                    format!("{ty}({})", 1u64 << n)
                };
                let p = mir(
                    &format!("{ty} a,q,r PROC Main() q=a/{constant} r=a MOD {constant} RETURN"),
                    optimize,
                );
                let m = emit::materialize(&p).unwrap();
                assert!(
                    !m.prepared.routines.iter().any(|r| r.helper.is_some()),
                    "{ty} {n} optimize={optimize}: {p:#?}"
                );
                assert_eq!(image::link(&p, &m, &layout()).unwrap().version, 3);
            }
        }
        for factor in [0, 1, 2, 128, 32768] {
            let p = mir(
                &format!("CARD a INT q PROC Main() q=a*{factor} RETURN"),
                optimize,
            );
            assert!(
                !emit::materialize(&p)
                    .unwrap()
                    .prepared
                    .routines
                    .iter()
                    .any(|r| r.helper.is_some())
            );
        }
        let p = mir("INT a,q,r PROC Main() q=a/8 r=a MOD 8 RETURN", optimize);
        assert_eq!(
            emit::materialize(&p)
                .unwrap()
                .prepared
                .routines
                .iter()
                .filter(|r| r.helper.is_some())
                .count(),
            2
        );
    }
}
#[test]
fn mul_and_widening_follow_computation_width() {
    for optimize in [false, true] {
        for (ty, expected) in [
            ("BYTE", 2),
            ("CARD", 2),
            ("INT", 2),
            ("LONGCARD", 4),
            ("LONGINT", 4),
        ] {
            let p = mir(
                &format!("{ty} a,b LONGCARD q PROC Main() q=LONGCARD(a*b) RETURN"),
                optimize,
            );
            let m = emit::materialize(&p).unwrap_or_else(|e| panic!("{ty}: {e} {p:#?}"));
            let h = m.prepared.routines.iter().find_map(|r| r.helper).unwrap();
            assert_eq!((h.bytes, h.signed), (expected, false));
            assert_eq!(image::link(&p, &m, &layout()).unwrap().version, 3);
        }
    }
}
#[test]
fn raw_fault_requires_versioned_image_and_o65_contracts() {
    let p = mir("CARD a,b,q PROC Main() q=a/b RETURN", true);
    let artifact = o65::prepare(&p, &Default::default()).unwrap();
    assert_eq!(artifact.profile().version, 2);
    let bytes = o65::write(&artifact).unwrap();
    let profile = o65::inspect(&bytes).unwrap();
    assert_eq!(profile.imports[1].name, o65::profile::ARITHMETIC_FAULT);
    assert_eq!(
        profile.imports[1].contract,
        o65::profile::Contract::arithmetic_fault()
    );
    assert!(
        o65::prepare(
            &p,
            &o65::Options {
                profile: o65::profile::ID.into(),
                ..Default::default()
            }
        )
        .is_err()
    );
    let machine = emit::materialize(&p).unwrap();
    let mut i = image::link(&p, &machine, &layout()).unwrap();
    i.version = 3;
    assert!(i.verify().is_err());
    i.version = 4;
    i.arithmetic_fault = Some(i.entry);
    assert!(i.verify().is_err());
    i.arithmetic_fault = None;
    assert!(i.verify().is_err());
}
#[test]
fn explicit_optimized_zero_fault_has_the_same_terminal_dependency() {
    let ast = parser::parse(&lexer::tokenize("BYTE before PROC Main() before=1 RETURN").unwrap())
        .unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
    )
    .unwrap();
    let mut n = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let block = &mut n.routines[0].blocks[0];
    block.ops.push(nir::NirOp::Call {
        callee: nir::NirCallee::Fault(actionc::runtime_fault::RuntimeFault::DivisionByZero),
        args: vec![],
        result: None,
        aggregate_result: None,
        signature: None,
        effects: nir::NirCallEffects {
            memory: nir::NirMemoryEffects {
                reads: nir::NirMemoryAccess::Unknown,
                writes: nir::NirMemoryAccess::Unknown,
            },
            may_call_external: true,
            opaque: true,
        },
    });
    block.terminator = nir::NirTerminator::Exit;
    nir::verify_program(&n).unwrap();
    let n = nir::optimize_program_with_promotion(&n, nir::NirPromotionPolicy::Native65816).unwrap();
    let p = mir65816::lower_program(&n).unwrap();
    assert!(
        p.routines
            .iter()
            .flat_map(|r| &r.blocks)
            .any(|b| matches!(b.terminator, Mir65816Terminator::ArithmeticFault))
    );
    let m = emit::materialize(&p).unwrap();
    assert!(!m.prepared.routines.iter().any(|r| r.helper.is_some()));
    let i = image::link(&p, &m, &layout()).unwrap();
    assert_eq!(i.arithmetic_fault, Some(0x049000));
}

#[test]
fn helper_calls_publish_measurable_spans_and_deduplicate_repeated_uses() {
    let mut costs = vec![];
    for optimize in [false, true] {
        for ty in ["BYTE", "CARD", "INT", "SIZE", "LONGCARD", "LONGINT"] {
            let p = mir(
                &format!("{ty} a,b,p,q,r,again PROC Main() p=a*b q=a/b r=a MOD b again=a/b RETURN"),
                optimize,
            );
            let m = emit::materialize(&p).unwrap();
            let mut calls = vec![];
            for r in &m.prepared.routines {
                let machine = m
                    .routines
                    .iter()
                    .find(|machine| machine.id == r.id)
                    .unwrap();
                for block in &r.blocks {
                    for (i, op) in block.ops.iter().enumerate() {
                        if let Mir65816Op::Call {
                            target: Mir65816CallTarget::Helper(id),
                            plan,
                            ..
                        } = op
                        {
                            let helper = m.prepared.routines.iter().find(|r| r.id == *id).unwrap();
                            let body = m.routines.iter().find(|r| r.id == *id).unwrap();
                            let span = &machine.code.mir_spans[&(block.id, i)];
                            assert!(span.len() > 4); // The ordinary checked ABI surrounds JSL.
                            calls.push(*id);
                            costs.push(serde_json::json!({"source_type":ty,"optimize":optimize,"helper":helper.name,"body_bytes":body.code.bytes.len(),"call_bytes":span.len(),"outgoing_bytes":plan.outgoing_bytes.get(),"local_call_peak":plan.outgoing_bytes.get()+3}));
                        }
                    }
                }
            }
            assert_eq!(calls.len(), 4);
            assert_eq!(calls[1], calls[3]);
            assert_eq!(
                m.prepared
                    .routines
                    .iter()
                    .filter(|r| r.helper.is_some())
                    .count(),
                3
            );
        }
    }
    if let Ok(path) = std::env::var("A816_ARITH_COST_PATH") {
        std::fs::write(path, serde_json::to_vec_pretty(&costs).unwrap()).unwrap();
    }
}
