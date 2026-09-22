use super::super::{
    analysis::{homes::HomeContract, sites::Node},
    selected::*,
    tracked::TrackedEmitter65816,
    *,
};
use super::{context::*, driver::*, plan::*, rules};

fn fixture(nops: usize, consume: bool) -> Code {
    let p = crate::compiler::native65816::prepare_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tools/native65816-runtime-tests/tests/fixtures/code_quality/add.act"),
        false,
        &Default::default(),
    )
    .unwrap()
    .mir;
    let (routine, frame, temp, home) = p
        .routines
        .iter()
        .find_map(|r| {
            let f = AllocatedFrame::new(r).unwrap();
            let (&t, &h) = f.temps.iter().find(|(_, h)| h.slot().width == 2)?;
            Some((r, f, t, h))
        })
        .unwrap();
    let mut e = TrackedEmitter65816::default();
    e.op(Implied::Tsc);
    e.op(Implied::Sec);
    e.word(WordOp::SbcImm, frame.extent);
    e.op(Implied::Tcs);
    e.establish_body();
    e.register_home(home);
    e.a16();
    e.word(WordOp::LdaImm, 7);
    let word = match home {
        Location::Stack(s) => copies::WordHome::Stack(s.offset as u8),
        Location::DirectPage(s) => copies::WordHome::DirectPage(s.offset as u8),
    };
    e.store_word(word);
    e.remember_word(temp, home);
    for _ in 0..nops {
        e.op(Implied::Nop);
    }
    if consume {
        assert_eq!(
            e.consume_word(Some(temp), Some(home), Some(word)),
            nops == 0
        );
    }
    match word {
        copies::WordHome::Stack(offset) => e.byte(ByteOp::LdaStack, offset),
        copies::WordHome::DirectPage(offset) => e.byte(ByteOp::LdaDp, offset),
    }
    e.op(Implied::Tsc);
    e.op(Implied::Clc);
    e.word(WordOp::AdcImm, frame.extent);
    e.op(Implied::Tcs);
    e.native_return(None).unwrap();
    layout::finalize(
        e.finish_selected(
            routine.id,
            &frame,
            Some(HomeContract::from_verified(routine, &frame).unwrap()),
        )
        .unwrap(),
        true,
    )
    .unwrap()
}
fn plan(code: &Code) -> Plan {
    let s = code.selected.as_ref().unwrap();
    let i = s
        .records()
        .iter()
        .position(|r| {
            matches!(
                r.action,
                Action::Instruction {
                    form: Instruction::Implied(Implied::Nop),
                    ..
                }
            )
        })
        .unwrap();
    let site = s.site(Node(i)).unwrap();
    rules::identity(&Context::new(s).unwrap(), site, site)
        .into_result()
        .unwrap()
}
fn unchanged(a: &Code, b: &Code) {
    replay::equivalent(a, b).unwrap();
    assert_eq!(
        a.selected.as_ref().unwrap().records(),
        b.selected.as_ref().unwrap().records()
    );
    let site = a.selected.as_ref().unwrap().site(Node(0)).unwrap();
    assert!(b.selected.as_ref().unwrap().validate(site).is_ok());
}
fn reject(code: &Code, plan: &Plan) -> String {
    let mut copy = code.clone();
    let mut driver = Driver::new(4);
    let error = driver
        .apply(&mut copy, plan, false)
        .into_result()
        .unwrap_err();
    unchanged(code, &copy);
    assert_eq!(driver.statistics.applied, 0);
    error.reason
}
#[test]
fn identity_replays_atomically_and_invalidates_all_old_sites() {
    let mut code = fixture(1, false);
    let original = code.clone();
    let p = plan(&code);
    let mut driver = Driver::new(4);
    assert_eq!(driver.apply(&mut code, &p, false), Proof::Proven(()));
    replay::equivalent(&original, &code).unwrap();
    assert!(
        Context::new(code.selected.as_ref().unwrap())
            .unwrap()
            .facts
            .validate(p.first)
            .is_err()
    );
    assert!(reject(&code, &p).contains("stale"));
    let fresh = plan(&code);
    assert!(
        driver
            .apply(&mut code, &fresh, false)
            .into_result()
            .unwrap_err()
            .reason
            .contains("one-shot")
    );
}
#[test]
fn foreign_owner_reversed_range_and_changed_original_are_rejected() {
    let code = fixture(2, false);
    let p = plan(&code);
    assert!(reject(&fixture(2, false), &p).contains("stale"));
    let mut changed = p.clone();
    changed.original[0].decision = Some(true);
    assert!(reject(&code, &changed).contains("content"));
    let s = code.selected.as_ref().unwrap();
    let start = s.validate(p.first).unwrap().0;
    changed = p.clone();
    changed.first = s.site(Node(start + 1)).unwrap();
    assert!(reject(&code, &changed).contains("window"));
}
#[test]
fn two_plans_must_revalidate_even_when_windows_do_not_overlap() {
    let mut code = fixture(2, false);
    let mut a = plan(&code);
    a.rule = Rule::RemoveNop;
    a.replacement.clear();
    let s = code.selected.as_ref().unwrap();
    let i = s.validate(a.first).unwrap().0 + 1;
    let site = s.site(Node(i)).unwrap();
    let mut b = rules::identity(&Context::new(s).unwrap(), site, site)
        .into_result()
        .unwrap();
    b.rule = Rule::RemoveNop;
    b.replacement.clear();
    let mut d = Driver::new(2);
    assert_eq!(d.apply(&mut code, &a, false), Proof::Proven(()));
    assert!(
        d.apply(&mut code, &b, false)
            .into_result()
            .unwrap_err()
            .reason
            .contains("stale")
    );
    let mut fresh = plan(&code);
    fresh.rule = Rule::RemoveNop;
    fresh.replacement.clear();
    assert_eq!(d.apply(&mut code, &fresh, false), Proof::Proven(()));
    assert_eq!(d.statistics.applied, 2);
}
#[test]
fn effect_declarations_cannot_authorize_arithmetic_or_new_reads() {
    let code = fixture(1, false);
    let p = plan(&code);
    for form in [
        Instruction::Implied(Implied::Clc),
        Instruction::Word(WordOp::LdaImm, 9),
        Instruction::Byte(ByteOp::LdaStack, 1),
        Instruction::Byte(ByteOp::LdaIndirect, 0),
    ] {
        let mut bad = p.clone();
        bad.replacement = vec![form.clone()];
        assert!(reject(&code, &bad).contains("effects"));
        let effects = form.effects(p.original[0].before.env);
        bad.delta.registers = effects.writes;
        bad.delta.flags = effects.flag_writes;
        assert!(
            reject(&code, &bad).contains("equivalence") || reject(&code, &bad).contains("effects")
        );
    }
}
#[test]
fn stores_protected_events_modes_and_calls_cannot_be_removed() {
    let code = fixture(1, false);
    let s = code.selected.as_ref().unwrap();
    for (i, r) in s.records().iter().enumerate() {
        if matches!(
            r.action,
            Action::Request(_)
                | Action::Instruction {
                    form: Instruction::Byte(ByteOp::StaStack | ByteOp::Rep, _),
                    ..
                }
                | Action::Instruction {
                    form: Instruction::Implied(Implied::Tcs),
                    ..
                }
        ) {
            let mut bad = plan(&code);
            let site = s.site(Node(i)).unwrap();
            bad.first = site;
            bad.last = site;
            bad.original = vec![r.clone()];
            bad.replacement.clear();
            reject(&code, &bad);
        }
    }
    let mut bad = plan(&code);
    bad.replacement = vec![Instruction::Reference(
        ReferenceOp::Jsl,
        Target::Routine(RoutineId(0)),
        0,
        None,
    )];
    assert!(reject(&code, &bad).contains("protected"));
}
#[test]
fn replay_failure_rolls_back_without_panic_catching() {
    // NOP makes the capture stale. Removing it would change the later failed
    // consumption into success: a protected event dependency rejects the edit.
    let code = fixture(1, true);
    let mut p = plan(&code);
    p.rule = Rule::RemoveNop;
    p.replacement.clear();
    assert!(reject(&code, &p).contains("consume decisions differ"));
}
#[test]
fn bounded_driver_rejects_non_decreasing_or_exhausted_transactions() {
    let mut code = fixture(1, false);
    let p = plan(&code);
    let before = code.clone();
    assert!(
        Driver::new(0)
            .apply(&mut code, &p, false)
            .into_result()
            .unwrap_err()
            .reason
            .contains("limit")
    );
    unchanged(&code, &before);
    let mut p = p;
    p.rule = Rule::NonDecreasingControl;
    assert!(reject(&code, &p).contains("metric"));
}

#[test]
fn local_equivalence_replays_the_actual_lda_and_requires_full_identity() {
    let code = fixture(1, false);
    let selected = code.selected.as_ref().unwrap();
    let context = Context::new(selected).unwrap();
    let p = plan(&code);
    let (temp, home) = selected
        .records()
        .iter()
        .find_map(|r| match r.action {
            Action::Request(Request::RememberWord(t, h)) => Some((t, h)),
            _ => None,
        })
        .unwrap();
    let load = match home {
        Location::Stack(s) => Instruction::Byte(ByteOp::LdaStack, s.offset as u8),
        Location::DirectPage(s) => Instruction::Byte(ByteOp::LdaDp, s.offset as u8),
    };
    assert_eq!(
        context.adjacent_load(p.first, Some(temp), Some(home), &load),
        Proof::Proven(())
    );
    assert!(
        context
            .adjacent_load(p.first, Some(TempId(u32::MAX)), Some(home), &load)
            .into_result()
            .is_err()
    );
    assert!(
        context
            .adjacent_load(
                p.first,
                Some(temp),
                Some(home),
                &Instruction::Word(WordOp::LdaImm, 7)
            )
            .into_result()
            .is_err()
    );
    let next = selected
        .site(Node(selected.validate(p.first).unwrap().0 + 1))
        .unwrap();
    assert!(
        context
            .adjacent_load(next, Some(temp), Some(home), &load)
            .into_result()
            .is_err()
    );
}

#[test]
fn declared_store_removal_is_still_blocked_by_its_outside_read() {
    let code = fixture(1, false);
    let mut checked = 0;
    {
        let code = &code;
        let s = code.selected.as_ref().unwrap();
        let c = Context::new(s).unwrap();
        for (i, r) in s.records().iter().enumerate() {
            if !matches!(
                r.action,
                Action::Instruction {
                    form: Instruction::Byte(ByteOp::StaStack | ByteOp::StaDp, _),
                    ..
                }
            ) || r.parent.is_some()
            {
                continue;
            }
            let site = s.site(Node(i)).unwrap();
            let homes = c.facts.homes.accesses[&Node(i)]
                .iter()
                .flat_map(|a| a.homes.iter().copied())
                .collect::<Vec<_>>();
            if !homes.iter().any(|&h| {
                c.facts
                    .uses_of_definition(h, site)
                    .is_ok_and(|u| !u.is_empty())
            }) {
                continue;
            }
            let mut p = rules::identity(&c, site, site).into_result().unwrap();
            p.replacement.clear();
            p.removed_definitions = homes.into_iter().map(|h| (h, site)).collect();
            assert!(reject(code, &p).contains("live outside use"));
            checked += 1;
        }
    }
    assert!(checked > 0);
}

fn adjacent_plan(code: &Code) -> Plan {
    let s = code.selected.as_ref().unwrap();
    let i = s
        .records()
        .iter()
        .position(|r| matches!(r.action, Action::Request(Request::ConsumeWord(..))))
        .unwrap()
        + 2;
    rules::adjacent(&Context::new(s).unwrap(), s.site(Node(i)).unwrap())
        .into_result()
        .unwrap()
}
#[test]
fn actual_load_candidate_is_removed_only_after_checked_replay() {
    let mut code = fixture(0, true);
    let old = code.clone();
    let p = adjacent_plan(&code);
    assert_eq!(
        Driver::new(1).apply(&mut code, &p, false),
        Proof::Proven(())
    );
    assert_eq!(code.bytes.len() + 2, old.bytes.len());
    assert!(code.selected.as_ref().unwrap().validate(p.first).is_err());
    assert_eq!(
        Context::new(code.selected.as_ref().unwrap())
            .unwrap()
            .facts
            .undefined_private_reads()
            .len(),
        0
    );
}
#[test]
fn pilot_rejects_partial_home_stale_capture_and_undeclared_changes() {
    let code = fixture(0, true);
    let p = adjacent_plan(&code);
    let Rule::Adjacent {
        request,
        temp,
        home,
    } = p.rule
    else {
        unreachable!()
    };
    let mut bad = p.clone();
    bad.delta.flags = 0;
    assert!(reject(&code, &bad).contains("undeclared"));
    bad = p.clone();
    bad.rule = Rule::Adjacent {
        request,
        temp: TempId(u32::MAX),
        home,
    };
    assert!(reject(&code, &bad).contains("attribution"));
    let partial = match home {
        Location::Stack(mut s) => {
            s.offset += 1;
            Location::Stack(s)
        }
        Location::DirectPage(mut s) => {
            s.offset += 1;
            Location::DirectPage(s)
        }
    };
    bad = p.clone();
    bad.rule = Rule::Adjacent {
        request,
        temp,
        home: partial,
    };
    assert!(reject(&code, &bad).contains("attribution"));
    bad = p.clone();
    bad.rule = Rule::Adjacent {
        request: adjacent_plan(&fixture(0, true)).first,
        temp,
        home,
    };
    assert!(reject(&code, &bad).contains("stale"));
    bad = p;
    bad.replacement = vec![Instruction::Implied(Implied::Clc)];
    reject(&code, &bad);
}

fn projected_candidate() -> (Code, super::pilot::Candidate) {
    let mut code = fixture(0, true);
    let plan = adjacent_plan(&code);
    let Rule::Adjacent {
        request,
        temp,
        home,
    } = plan.rule
    else {
        unreachable!()
    };
    let node = code.selected.as_ref().unwrap().validate(request).unwrap();
    let Action::Instruction { form, .. } = &plan.original[0].action else {
        unreachable!()
    };
    let candidate = super::pilot::Candidate {
        request: node,
        temp: Some(temp),
        home: Some(home),
        load: form.clone(),
    };
    assert_eq!(
        Driver::new(1).apply(&mut code, &plan, false),
        Proof::Proven(())
    );
    (code, candidate)
}
#[test]
fn malformed_planned_candidates_cannot_change_the_actual_load() {
    let (code, candidate) = projected_candidate();
    let original = code.clone();
    let mut bad = candidate.clone();
    bad.load = Instruction::Byte(ByteOp::LdaStack, 255);
    assert!(
        super::pilot::apply(&code, &[bad], false)
            .unwrap_err()
            .contains("original consume inputs")
    );
    assert!(
        super::pilot::apply(&code, &[candidate.clone(), candidate.clone()], false)
            .unwrap_err()
            .contains("duplicated")
    );
    let mut changed = code.clone();
    changed.bytes[0] ^= 1;
    assert!(super::pilot::apply(&changed, &[candidate], false).is_err());
    unchanged(&code, &original);
}
#[test]
fn blocked_final_ownership_proof_retains_the_actual_load_and_continuation() {
    let (mut code, candidate) = projected_candidate();
    code.selected.as_mut().unwrap().allocation.temps.clear();
    let result = super::pilot::apply(&code, &[candidate], false).unwrap();
    assert_eq!(result.bytes.len(), code.bytes.len() + 2);
    replay::equivalent(&result, &fixture(0, true)).unwrap();
    #[cfg(feature = "native65816-state-proof")]
    assert!(!result.rewrite_observations[0].accepted);
}
