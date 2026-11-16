use super::*;
use crate::mir6502::{
    self as mir,
    trace::{Mir6502Trace, Trace},
};

const SOURCE: &str = "BYTE FUNC First(BYTE x) RETURN(x+1)
    BYTE FUNC Second(BYTE x) RETURN(x+2)
    CARD FUNC Hold(CARD keep) BYTE n n=7 First(3) Second(5) RETURN(keep+n)";

fn nir(source: &str) -> crate::nir::NirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model))
}

fn programs(source: &str) -> (MirProgram, MirProgram) {
    let input = mir::lower_program(&nir(source)).unwrap();
    let expanded = mir::materialize_program(input.clone(), &Default::default()).unwrap();
    (input, expanded)
}

#[test]
fn mixed_objects_keep_exposed_and_persistent_homes_inline() {
    let source = include_str!("../../../../fixtures/runtime/mixed_zp_homes.act").replace("\r\n", "\n");
    let (input, selected) = programs(&source);
    assert!(propose(&input, &selected, true, false, &Trace::disabled()).is_none());
    let proposed = propose(&input, &selected, true, true, &Trace::disabled()).unwrap();
    let r = proposed.routines.iter().find(|r| r.name == "Mixed").unwrap();
    let homes = r.frame.parameter_homes.as_ref().unwrap();
    assert_eq!(homes[0].home, MirParamHome::Inline);
    assert!(matches!(homes[1].home, MirParamHome::ZeroPage(_)));
    assert!(matches!(homes[2].home, MirParamHome::ZeroPage(_)));
    assert!(r.frame.local_homes.as_ref().is_none_or(|b| b.iter().all(|b| b.home == MirLocalHome::Inline)));
    let expanded = mir::materialize_program(proposed, &Default::default()).unwrap();
    let r = expanded.routines.iter().find(|r| r.name == "Mixed").unwrap();
    assert!(verify_storage(&expanded, r, MirCalleeEntryStage::ActualEntry).is_empty());
    assert!(verify_lifetimes(r, Some(&mut ResidenceCalls::new(&expanded, MirCalleeEntryStage::ActualEntry))).is_empty());

    // An address-only parameter cannot become Absent or enter register binding.
    let (input, _) = programs("CARD exposed CARD FUNC Leaf(BYTE c CARD keep BYTE tail) exposed=@c keep==+1 keep==+2 RETURN(keep)");
    let proposed = super::super::propose(&input, true).unwrap();
    assert_eq!(proposed.routines[0].frame.parameter_homes.as_ref().unwrap()[0].home, MirParamHome::Inline);
}

fn hold(p: &MirProgram) -> &MirRoutine {
    p.routines.iter().find(|r| r.name == "Hold").unwrap()
}

fn second_call(p: &mut MirProgram) -> &mut MirOp {
    p.routines
        .iter_mut()
        .find(|r| r.name == "Hold")
        .unwrap()
        .blocks
        .iter_mut()
        .flat_map(|b| &mut b.ops)
        .filter(|op| matches!(op, MirOp::Call { .. }))
        .nth(1)
        .unwrap()
}

fn clobber_second(p: &mut MirProgram, byte: MirFixedZpSlot) {
    p.routines
        .iter_mut()
        .find(|r| r.name == "Second")
        .unwrap()
        .blocks[0]
        .ops
        .insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(byte)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        );
}

#[test]
fn multiple_leaf_calls_admit_distinct_and_repeated_targets_only_when_enabled() {
    for source in [SOURCE.to_string(), SOURCE.replace("Second(5)", "First(5)")] {
        let (input, selected) = programs(&source);
        assert!(propose(&input, &selected, false, false, &Trace::disabled()).is_none());
        let session = Mir6502Trace::default();
        let proposed = propose(&input, &selected, true, false, &session.context()).unwrap();
        assert!(hold(&proposed).frame.parameter_homes.is_some());
        assert!(hold(&proposed).frame.local_homes.is_some());
        let expanded = mir::materialize_program(proposed, &Default::default()).unwrap();
        assert!(
            verify_lifetimes(
                hold(&expanded),
                Some(&mut ResidenceCalls::new(
                    &expanded,
                    MirCalleeEntryStage::ActualEntry,
                ))
            )
            .is_empty()
        );
        assert!(
            session
                .events()
                .iter()
                .any(|e| e.kind == "cross-call-candidate"
                    && e.data["routine"] == "Hold"
                    && e.data["call_count"] == 2)
        );
    }
}

#[test]
fn multiple_leaf_calls_union_includes_second_callee_word_lane_and_local() {
    let (input, selected) = programs(SOURCE);
    let proposed = propose(&input, &selected, true, false, &Trace::disabled()).unwrap();
    let bytes = hold(&proposed).frame.resident_bytes();
    assert_eq!(
        bytes.len(),
        3,
        "a complete word parameter and initialized byte local"
    );
    for (_, byte) in bytes {
        let mut changed = selected.clone();
        clobber_second(&mut changed, byte);
        let safe = propose(&input, &changed, true, false, &Trace::disabled()).unwrap();
        assert!(
            hold(&safe)
                .frame
                .resident_bytes()
                .iter()
                .all(|(_, b)| *b != byte)
        );
        assert!(hold(&safe).frame.local_homes.is_some());

        // The old placement becomes unsafe only at the later call. Expanded
        // liveness must reject it even if discovery facts were stale.
        let mut unsafe_expanded =
            mir::materialize_program(proposed.clone(), &Default::default()).unwrap();
        clobber_second(&mut unsafe_expanded, byte);
        assert!(
            !verify_lifetimes(
                hold(&unsafe_expanded),
                Some(&mut ResidenceCalls::new(
                    &unsafe_expanded,
                    MirCalleeEntryStage::ActualEntry,
                ))
            )
            .is_empty()
        );
    }
}

#[test]
fn multiple_leaf_calls_validate_later_sites_even_with_cached_entry_facts() {
    for repeated in [false, true] {
        let source = if repeated {
            SOURCE.replace("Second(5)", "First(5)")
        } else {
            SOURCE.into()
        };
        let (mut input, mut selected) = programs(&source);
        if repeated {
            let MirOp::Call { args, .. } = second_call(&mut input) else {
                unreachable!()
            };
            args.clear();
        } else {
            selected
                .routines
                .iter_mut()
                .find(|r| r.name == "Second")
                .unwrap()
                .blocks[0]
                .ops
                .push(MirOp::Barrier {
                    effects: Default::default(),
                });
        }
        let session = Mir6502Trace::default();
        assert!(propose(&input, &selected, true, false, &session.context()).is_none());
        let events = session.events();
        let rejected = events
            .iter()
            .find(|e| e.kind == "cross-call-candidate")
            .unwrap();
        assert_eq!(rejected.data["call_index"], 1);
        assert_eq!(
            rejected.data["reason"],
            if repeated {
                "extra-entry-register"
            } else {
                "unsupported residence callee"
            }
        );
    }
}

#[test]
fn multiple_leaf_calls_invalid_second_site_restores_exact_selected_artifact() {
    let n = nir(SOURCE);
    let config = mir::Mir6502Config {
        enable_parameter_home_selection: true,
        enable_multiple_leaf_call_homes: true,
        ..Default::default()
    };
    let runtime = crate::runtime::Runtime::ActionCart;
    let session = Mir6502Trace::default();
    let trace = session.context();
    let (p, resume) =
        mir::prepare_program_for_emission(&n, 0x3000, &config, runtime, &trace).unwrap();
    let baseline = mir::finalize_output(&n, p, 0x3000, runtime, &trace).unwrap();
    let resume = resume.unwrap();
    let selected = SelectedPlacement {
        input: Arc::new(resume.core.checkpoint.input.program.clone()),
        executable: baseline,
    };
    let mut proposal =
        propose(&selected.input, &selected.executable.program, true, false, &trace).unwrap();
    let MirOp::Call { args, .. } = second_call(&mut proposal) else {
        unreachable!()
    };
    args.clear();
    let expected_input = Arc::clone(&selected.input);
    let expected = selected.executable.clone();
    let before = session.manifest()["work"]["materializations"]
        .as_u64()
        .unwrap();
    let result = try_proposal(
        &n,
        selected,
        &resume,
        proposal,
        "cross-call-homes",
        &config,
        0x3000,
        runtime,
        &trace,
    );
    assert!(Arc::ptr_eq(&result.input, &expected_input));
    assert_eq!(result.executable.program, expected.program);
    assert_eq!(result.executable.output, expected.output);
    assert_eq!(result.executable.costs, expected.costs);
    assert_eq!(result.executable.trace_snapshot, expected.trace_snapshot);
    assert_eq!(session.manifest()["work"]["cross_call_home_trials"], 1);
    assert!(
        session.manifest()["work"]["materializations"]
            .as_u64()
            .unwrap()
            <= before + 1
    );
    assert!(
        session
            .events()
            .iter()
            .any(|e| e.kind == "cross-call-home-result"
                && e.data["accepted"] == false
                && e.data["reason"] == "invalid-trial")
    );
}
