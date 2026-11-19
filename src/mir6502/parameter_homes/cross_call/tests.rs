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
fn live_caller_byte_avoids_the_outgoing_fourth_argument_lane() {
    let source = "BYTE a0=$A0,a1=$A1,a2=$A2
;@actionc reviewed-6502 1111111111111111
;@actionc abi-inputs-only
;@actionc writes-only none
PROC Four=*(CARD ptr BYTE c,d)
ASM
  lda $A3
  rts
ENDASM
BYTE FUNC Hold(BYTE keep) BYTE n n=keep+1 Four(0,0,7) RETURN(n)
PROC Main() Hold(1) RETURN";
    let mir = mir::lower_program(&nir(source)).unwrap();
    let hash = mir::analysis::known_callees::body_fingerprint(
        &mir, &mir.routines[0], crate::callee6502::CONTRACT_VERSION,
    ).unwrap();
    let n = nir(&source.replace("1111111111111111", &format!("{hash:016x}")));
    let session = Mir6502Trace::default();
    let (selected, resume) = mir::prepare_program_for_emission(&n, 0x3000,
        &mir::Mir6502Config::optimized(), crate::runtime::Runtime::ActionCart, &Trace::disabled()).unwrap();
    let resume = resume.unwrap();
    let proposed = propose_with_scope(&resume.core.checkpoint.input.program, &selected, true, true, true, &session.context())
        .unwrap_or_else(|| panic!("{:?}", session.events()));
    let bytes = hold(&proposed).frame.resident_bytes();
    let live = bytes.iter().find(|(mem, _)| matches!(mem, MirMem::Local { .. })).unwrap();
    assert_ne!(live.1, MirFixedZpSlot(0xa3));
}

#[test]
fn certified_assembly_leaf_planning_and_expanded_conflict_keep_artifacts_paired() {
    let source = include_str!("../../../../fixtures/runtime/certified_assembly_leaf_homes.act")
        .replace("\r\n", "\n");
    let mut bindings = None;
    for text in [source.clone(), source.replace('\n', "\r\n")] {
        let (input, selected) = programs(&text);
        let proposed = propose_with_scope(&input, &selected, true, true, true, &Trace::disabled()).unwrap();
        assert!(!hold(&proposed).frame.resident_bytes().is_empty());
        let leaf = proposed.routines.iter().find(|r| r.name == "Touch").unwrap();
        assert!(leaf.frame.resident_bytes().is_empty());
        let MirStorageBase::Param(keep) = hold(&input).frame.params.iter()
            .find(|s| s.name.as_deref() == Some("keep")).unwrap().base else { panic!() };
        assert!(matches!(hold(&proposed).frame.parameter_home(keep), Some(MirParamHome::ZeroPage(_))));
        if let Some(off) = propose_with_scope(&input, &selected, true, true, false, &Trace::disabled()) {
            assert!(hold(&off).frame.parameter_home(keep).is_none_or(|h| *h == MirParamHome::Inline));
        }
        let homes = hold(&proposed).frame.resident_bytes();
        if let Some(expected) = &bindings { assert_eq!(&homes, expected); }
        bindings = Some(homes);
    }

    let n = nir(&source);
    let config = mir::Mir6502Config { enable_small_leaf_inlining: false,
        ..mir::Mir6502Config::optimized() };
    let runtime = crate::runtime::Runtime::ActionCart;
    let session = Mir6502Trace::default();
    let trace = session.context();
    let (_, resume) = mir::prepare_program_for_emission(&n, 0x3000, &config, runtime, &trace).unwrap();
    let resume = resume.unwrap();
    let initial = resume.core.checkpoint.input.program.clone();
    let executable = mir::preparation::resume_output(&n, &resume, initial.clone(), &config, 0x3000, runtime, &trace, None).unwrap();
    let selected = SelectedPlacement { input: Arc::new(initial), executable };
    let mut proposal = propose_with_scope(&selected.input, &selected.executable.program, true, true, true, &trace).unwrap();
    let byte = hold(&proposal).frame.resident_bytes()[0].1.0;
    // The selected payload was disjoint. A changed payload must be checked in
    // the expanded candidate, not against the previously certified leaf.
    proposal.machine_blocks[0].items.splice(0..0, [MirMachineItem::Byte(0x85), MirMachineItem::Byte(byte)]);
    let expected_input = Arc::clone(&selected.input);
    let expected = selected.executable.clone();
    let result = try_proposal(&n, selected, &resume, proposal, "cross-call-homes", &config, 0x3000, runtime, &trace);
    assert!(Arc::ptr_eq(&result.input, &expected_input));
    assert_eq!(result.executable.program, expected.program);
    assert_eq!(result.executable.output, expected.output);
    assert_eq!(result.executable.memory, expected.memory);
    assert_eq!(result.executable.costs, expected.costs);
    assert_eq!(result.executable.trace_snapshot, expected.trace_snapshot);
    assert_eq!(session.manifest()["work"]["cross_call_home_trials"], 1);
    assert!(session.events().iter().any(|e| e.kind == "cross-call-home-result"
        && e.data["reason"] == "invalid-trial"));
}

#[test]
fn acyclic_planning_coordinates_subtrees_independently_of_source_order() {
    let source = include_str!("../../../../fixtures/runtime/acyclic_call_homes.act").replace("\r\n", "\n");
    let (input, selected) = programs(&source);
    let proposed = propose_with_scope(&input, &selected, true, true, true, &Trace::disabled()).unwrap();
    let bytes = |name: &str| proposed.routines.iter().find(|r| r.name == name).unwrap()
        .frame.resident_bytes().iter().map(|(_, s)| s.0).collect::<BTreeSet<_>>();
    assert!(!bytes("Hold").is_empty());
    assert_eq!(bytes("Middle"), bytes("Sibling"));
    assert!(bytes("Hold").is_disjoint(&bytes("Middle")));
    let mut reordered = input.clone();
    reordered.routines.reverse();
    let again = propose_with_scope(&reordered, &selected, true, true, true, &Trace::disabled()).unwrap();
    for r in &proposed.routines {
        assert_eq!(r.frame.resident_bytes(), again.routines.iter().find(|a| a.id == r.id).unwrap().frame.resident_bytes());
    }
    let expanded = mir::materialize_program(proposed, &Default::default()).unwrap();
    mir::verify_program(&expanded, MirPhase::PostMaterialization).unwrap();

    // A source-address-exposed parameter stays inline; its independent local
    // and the outer caller still participate in the same chain proposal.
    let exposed = source.replace("BYTE c\n", "BYTE c CARD address=$684\n  address=@x\n");
    let (input, selected) = programs(&exposed);
    let proposed = propose_with_scope(&input, &selected, true, true, true, &Trace::disabled()).unwrap();
    let middle = proposed.routines.iter().find(|r| r.name == "Middle").unwrap();
    assert!(middle.frame.parameter_homes.as_ref().is_none_or(|h| h.iter().all(|b| b.home == MirParamHome::Inline)));
    assert!(middle.frame.resident_bytes().iter().any(|(m, _)| matches!(m, MirMem::Local { .. })));
    assert!(!hold(&proposed).frame.resident_bytes().is_empty());
}

#[test]
fn acyclic_planning_shares_dead_ancestor_bytes_and_keeps_per_site_lifetimes() {
    let (input, selected) = programs("BYTE reserved=$A1 BYTE FUNC Leaf(BYTE a,b,c) RETURN(a+b+c) CARD FUNC Middle(CARD x) Leaf(1,2,3) RETURN(x) CARD FUNC Gone(CARD x) Leaf(1,2,3) Middle(x) RETURN(7) CARD FUNC Hold(CARD x) CARD n Middle(1) n=x Middle(2) RETURN(n)");
    let session = Mir6502Trace::default();
    let proposed = propose_with_scope(&input, &selected, true, true, true, &session.context()).unwrap();
    let home = |name: &str| proposed.routines.iter().find(|r| r.name == name).unwrap().frame.resident_bytes();
    for name in ["Middle", "Gone"] {
        assert!(!home(name).is_empty(), "{name}: {:?}\n{}", session.events(), mir::format_program(&selected));
    }
    assert_eq!(home("Middle")[0].1, home("Gone")[0].1);
    assert!(home("Hold").iter().all(|(_, s)| *s != home("Middle")[0].1));
    let h = hold(&input);
    let n = h.frame.locals.iter().find(|s| s.name.as_deref() == Some("n")).unwrap();
    let MirStorageBase::Local(id) = n.base else { panic!() };
    let sites: Vec<_> = h.blocks.iter().flat_map(|b| b.ops.iter().enumerate().filter_map(|(op_index, op)|
        matches!(op, MirOp::Call { .. }).then_some(MirSite::Op { block: b.id, op_index }))).collect();
    let logical = logical::LogicalHomes::new(h).unwrap();
    assert_eq!(logical.live_after(&MirMem::Local { id, offset: 0 }, sites[0]), Some(false));
    assert_eq!(logical.live_after(&MirMem::Local { id, offset: 0 }, sites[1]), Some(true));
    mir::materialize_program(proposed, &Default::default()).unwrap();

    // A retained cycle cannot borrow the selected acyclic graph's certificate.
    let mut cyclic = input.clone();
    let middle = cyclic.routines.iter().find(|r| r.name == "Middle").unwrap().id;
    let mut op = cyclic.routines.iter().find(|r| r.name == "Gone").unwrap().blocks.iter()
        .flat_map(|b| &b.ops).find(|o| matches!(o, MirOp::Call { target: MirCallTarget::Routine(id), .. } if *id == middle)).unwrap().clone();
    if let MirOp::Call { args, result, .. } = &mut op {
        for arg in args { arg.value = MirValue::ConstU8(1); }
        *result = None;
    }
    cyclic.routines[0].blocks[0].ops.push(op);
    let graph = chains::CallChains::new(&cyclic, &selected);
    assert!(!graph.supports(middle));
    let p = propose_with_scope(&cyclic, &selected, true, true, true, &Trace::disabled()).unwrap();
    assert!(hold(&p).frame.resident_bytes().is_empty());
    // Gone dies at Middle, but its earlier live call to Leaf now reaches the
    // cycle too. Dead at one site must not hide the other site's obligation.
    assert!(p.routines.iter().find(|r| r.name == "Gone").unwrap().frame.resident_bytes().is_empty());
}

#[test]
fn acyclic_expanded_capture_conflict_rechecks_unchanged_callers_and_rolls_back() {
    let source = include_str!("../../../../fixtures/runtime/acyclic_call_homes.act").replace("\r\n", "\n");
    let n = nir(&source);
    let config = mir::Mir6502Config { enable_small_leaf_inlining: false,
        enable_acyclic_call_homes: true, ..mir::Mir6502Config::optimized() };
    let runtime = crate::runtime::Runtime::ActionCart;
    let session = Mir6502Trace::default();
    let trace = session.context();
    let (_, resume) = mir::prepare_program_for_emission(&n, 0x3000, &config, runtime, &trace).unwrap();
    let resume = resume.unwrap();
    let mut initial = resume.core.checkpoint.input.program.clone();
    let h = initial.routines.iter_mut().find(|r| r.name == "Hold").unwrap();
    let bindings = h.frame.params.iter().enumerate().map(|(i, slot)| {
        let MirStorageBase::Param(param) = slot.base else { panic!() };
        MirParamBinding { param, home: if i == 0 { MirParamHome::ZeroPage(MirFixedZpSlot(0xe8)) } else { MirParamHome::Inline } }
    }).collect();
    install_bindings(h, bindings);
    let executable = mir::preparation::resume_output(&n, &resume, initial.clone(), &config, 0x3000, runtime, &trace, None).unwrap();
    let selected = SelectedPlacement { input: Arc::new(initial), executable };
    let mut proposal = propose_with_scope(&selected.input, &selected.executable.program, true, true, true, &trace).unwrap();
    assert_eq!(hold(&proposal), hold(&selected.input));
    let middle = proposal.routines.iter_mut().find(|r| r.name == "Middle").unwrap();
    let MirStorageBase::Param(param) = middle.frame.params[0].base else { panic!() };
    // The conflicting store is introduced by capture expansion, not present as
    // a physical store in this retained input. Hold's own bindings never change.
    install_bindings(middle, vec![MirParamBinding { param, home: MirParamHome::ZeroPage(MirFixedZpSlot(0xe8)) }]);
    let expected_input = Arc::clone(&selected.input);
    let expected = selected.executable.clone();
    let result = try_proposal(&n, selected, &resume, proposal, "cross-call-homes", &config, 0x3000, runtime, &trace);
    assert!(Arc::ptr_eq(&result.input, &expected_input));
    assert_eq!(result.executable.program, expected.program);
    assert_eq!(result.executable.output, expected.output);
    assert_eq!(result.executable.memory, expected.memory);
    assert_eq!(result.executable.costs, expected.costs);
    assert_eq!(result.executable.trace_snapshot, expected.trace_snapshot);
    assert_eq!(session.manifest()["work"]["cross_call_home_trials"], 1);
    assert!(session.events().iter().any(|e| e.kind == "cross-call-home-result"
        && e.data["reason"] == "invalid-trial"));
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
    let (input, selected) = programs("CARD exposed CARD FUNC Leaf(BYTE c CARD keep BYTE tail) exposed=@c keep==+1 keep==+2 RETURN(keep)");
    let proposed = super::super::propose(&input, &selected, true, 0x3000).unwrap();
    assert_eq!(proposed.routines[0].frame.parameter_homes.as_ref().unwrap()[0].home, MirParamHome::Inline);
}

#[test]
fn call_lifetime_proofs_recheck_lanes_sites_and_unknown_boundaries() {
    let source = include_str!("../../../../fixtures/runtime/live_call_homes.act").replace("\r\n", "\n");
    let (input, selected) = programs(&source);
    let proposed = propose(&input, &selected, true, true, &Trace::disabled()).unwrap();
    let expanded = mir::materialize_program(proposed, &Default::default()).unwrap();
    let short = expanded.routines.iter().find(|r| r.name == "Short").unwrap();
    assert!(verify_lifetimes(short, Some(&mut ResidenceCalls::new(&expanded, MirCalleeEntryStage::ActualEntry))).is_empty());
    let live: Vec<_> = short.frame.resident_bytes().into_iter()
        .filter_map(|(m, b)| matches!(m, MirMem::Local { .. }).then_some(b)).collect();
    assert_eq!(live.len(), 2);
    for byte in live {
        let mut damaged = expanded.clone();
        damaged.routines.iter_mut().find(|r| r.name == "First").unwrap().blocks[0].ops.insert(0,
            MirOp::Store { dst: MirAddr::Direct(MirMem::FixedZeroPage(byte)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte });
        let r = damaged.routines.iter().find(|r| r.name == "Short").unwrap();
        assert!(!verify_lifetimes(r, Some(&mut ResidenceCalls::new(&damaged, MirCalleeEntryStage::ActualEntry))).is_empty());
    }
    let gone = expanded.routines.iter().find(|r| r.name == "Gone").unwrap();
    assert!(!gone.frame.resident_bytes().is_empty());
    assert!(verify_lifetimes(gone, Some(&mut ResidenceCalls::new(&expanded, MirCalleeEntryStage::ActualEntry))).is_empty());
    let mut bad_args = expanded.clone();
    let gone = bad_args.routines.iter_mut().find(|r| r.name == "Gone").unwrap();
    let MirOp::Call { args, .. } = gone.blocks.iter_mut().flat_map(|b| &mut b.ops)
        .find(|op| matches!(op, MirOp::Call { .. })).unwrap() else { panic!() };
    args.clear();
    let gone = bad_args.routines.iter().find(|r| r.name == "Gone").unwrap();
    assert!(!verify_lifetimes(gone, Some(&mut ResidenceCalls::new(&bad_args, MirCalleeEntryStage::ActualEntry))).is_empty());

    // The same unknown target is harmless before n's definition, but the later
    // site cannot preserve n. Cached target facts cannot erase that distinction.
    let (input, selected) = programs("CARD FUNC First(CARD x) RETURN(x+1) CARD FUNC Nested(CARD x) RETURN(First(x)) CARD FUNC Hold(CARD keep) CARD n Nested(1) n=keep Nested(2) RETURN(n)");
    if let Some(proposed) = propose(&input, &selected, true, true, &Trace::disabled()) {
        assert!(hold(&proposed).frame.resident_bytes().is_empty());
    }
}

#[test]
fn call_lifetimes_exclude_known_numeric_observations_before_the_shared_trial() {
    let (input, selected) = programs("PROC Observer=*()\nASM\n lda ($A2),y\n rts\nENDASM\nCARD FUNC First(CARD x) RETURN(x+1) CARD FUNC Writer(CARD ARRAY a) CARD h First(1) h=a(0) a(0)=h h==+1 RETURN(h)");
    let proposed = propose(&input, &selected, true, true, &Trace::disabled()).unwrap();
    let writer = proposed.routines.iter().find(|r| r.name == "Writer").unwrap();
    assert!(!writer.frame.resident_bytes().is_empty());
    assert!(writer.frame.resident_bytes().iter().all(|(_, b)| ![0xa2, 0xa3].contains(&b.0)));
    let expanded = mir::materialize_program(proposed, &Default::default()).unwrap();
    let writer = expanded.routines.iter().find(|r| r.name == "Writer").unwrap();
    assert!(verify_residence(writer,
        Some(&mut ResidenceCalls::new(&expanded, MirCalleeEntryStage::ActualEntry)),
        Some(&ObjectAddressControl::new(&expanded))).is_empty());
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
                "mismatched residence arguments"
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
