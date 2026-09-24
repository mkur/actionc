use super::super::tracked::TrackedEmitter65816;
use super::*;
use crate::analysis::graph::DataflowGraph;

fn frame() -> AllocatedFrame {
    AllocatedFrame {
        extent: 0,
        spill_bytes: 0,
        peak_below_entry: 0,
        temps: BTreeMap::new(),
        edge_copies: Vec::new(),
    }
}
fn finish(e: TrackedEmitter65816) -> Code {
    e.finish_selected(RoutineId(7), &frame(), None).unwrap()
}
fn simple() -> Code {
    let mut e = TrackedEmitter65816::default();
    e.a16();
    e.a16();
    e.native_return(None).unwrap();
    finish(e)
}
fn graph_code() -> Code {
    let mut e = TrackedEmitter65816::default();
    let safe = e.label();
    let head = e.label();
    let right = e.label();
    let done = e.label();
    e.a16();
    e.branch(Branch::CarrySet, safe);
    e.reference(ReferenceOp::Jml, Target::StackOverflow, 0, None);
    e.mark(safe);
    e.mark(head);
    e.dispatch(Branch::Equal, right);
    e.op(Implied::Nop);
    e.jump(done);
    e.mark(right);
    e.reference(ReferenceOp::Jsl, Target::Routine(RoutineId(3)), 0, None);
    e.jump(head);
    e.mark(done);
    e.native_return(None).unwrap();
    finish(e)
}
#[test]
fn sites_reject_other_compilations_allocations_generations_and_bounds() {
    let a = simple();
    let b = simple();
    let s = a.selected.as_ref().unwrap();
    let point = s.site(Node(1)).unwrap();
    assert!(s.validate(point).is_ok());
    assert!(b.selected.as_ref().unwrap().validate(point).is_err());
    assert!(
        s.identity
            .next_selection()
            .validate(point, s.records.len())
            .is_err()
    );
    assert!(
        s.identity
            .next_allocation()
            .validate(point, s.records.len())
            .is_err()
    );
    assert!(s.site(Node(s.records.len())).is_err());
    // Clones share the same immutable snapshot, not a newly selected routine.
    assert!(a.clone().selected.as_ref().unwrap().validate(point).is_ok());
}
#[test]
fn graph_has_exact_diamond_loop_fault_and_direct_call_successors() {
    let code = graph_code();
    let s = code.selected.as_ref().unwrap();
    let mut branches = 0;
    let mut calls = 0;
    let mut returns = 0;
    let mut faults = 0;
    for (i, r) in s.records.iter().enumerate() {
        let n = Node(i);
        if let Action::Instruction { effects, .. } = &r.action {
            let next = s.cfg.successors(n);
            match effects.control {
                super::super::effects::Control::Branch { target, .. } => {
                    branches += 1;
                    assert_eq!(next.len(), 2);
                    assert!(next.contains(&Node(i + 1)));
                    assert!(
                        next.iter()
                            .any(|n| matches!(s.records[n.0].action,Action::Bind(l) if l==target))
                    );
                }
                super::super::effects::Control::Call { .. } => {
                    calls += 1;
                    assert_eq!(next, &[Node(i + 1)].into());
                }
                super::super::effects::Control::Return => {
                    returns += 1;
                    assert!(matches!(
                        s.records[next.first().unwrap().0].action,
                        Action::ReturnExit
                    ));
                }
                super::super::effects::Control::Jump(Target::StackOverflow) => {
                    faults += 1;
                    assert!(matches!(
                        s.records[next.first().unwrap().0].action,
                        Action::FaultExit
                    ));
                }
                _ => {}
            }
        }
        for &next in s.cfg.successors(n) {
            assert!(s.cfg.predecessors(next).contains(&n));
        }
    }
    assert_eq!((branches, calls, returns, faults), (2, 1, 1, 1));
    assert_eq!(s.cfg.postorder().len(), s.cfg.reachable().len());
    assert_eq!(s.cfg.reverse_postorder().first(), Some(&Node(0)));
}
#[test]
fn omitted_mode_requests_remain_inputs_and_instructions_have_one_owner() {
    let code = simple();
    let s = code.selected.as_ref().unwrap();
    let requests: Vec<_> = s
        .records
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.action, Action::Request(Request::Mode(Width::Word))))
        .collect();
    assert_eq!(requests.len(), 2);
    let first = requests[0].0;
    let second = requests[1].0;
    assert!(matches!(
        s.records[first + 1].action,
        Action::Instruction { .. }
    ));
    assert_eq!(s.records[first + 1].parent, Some(Node(first)));
    assert!(matches!(s.records[second+1].action,Action::EndRequest(n) if n==Node(second)));
    assert_eq!(code.bytes, [0xc2, 0x20, 0x6b]);
}
#[test]
fn layout_changes_only_offsets_and_keeps_sites_and_graph() {
    let before = graph_code();
    let s = before.selected.as_ref().unwrap();
    let sites: Vec<_> = (0..s.records.len())
        .map(|i| s.site(Node(i)).unwrap())
        .collect();
    let after = super::super::layout::finalize(before.clone(), true).unwrap();
    assert!(after.bytes.len() < before.bytes.len());
    let new = after.selected.as_ref().unwrap();
    for (i, site) in sites.into_iter().enumerate() {
        assert_eq!(new.validate(site), Ok(Node(i)));
        assert_eq!(s.cfg.successors(Node(i)), new.cfg.successors(Node(i)));
        assert_eq!(s.records[i].parent, new.records[i].parent);
        assert_eq!(s.records[i].action, new.records[i].action);
        assert_eq!(s.records[i].before, new.records[i].before);
        assert_eq!(s.records[i].after, new.records[i].after);
    }
    new.reconcile(&after).unwrap();
}
#[test]
fn duplicate_mir_edges_are_counted_and_empty_fallthrough_is_a_real_edge() {
    let mut e = TrackedEmitter65816::default();
    e.op(Implied::Tsc);
    e.op(Implied::Tcs);
    e.establish_body();
    let a = e.label();
    let b = e.label();
    let c = e.label();
    e.declare_blocks([a, b, c].into_iter());
    e.prove_entries(
        [
            (a, [(None, 1)].into()),
            (b, [(Some(a), 2)].into()),
            (c, [(Some(b), 1)].into()),
        ]
        .into(),
        [a, b, c].into(),
    );
    e.mark(a);
    e.branch(Branch::Equal, b);
    e.jump(b);
    e.mark(b);
    e.fallthrough(c);
    e.mark(c);
    e.native_return(None).unwrap();
    let code = finish(e);
    let s = code.selected.as_ref().unwrap();
    let (i, _) = s
        .records
        .iter()
        .enumerate()
        .find(|(_, r)| matches!(r.action, Action::Request(Request::Fallthrough(_))))
        .unwrap();
    let next = s.cfg.successors(Node(i));
    assert_eq!(next.len(), 1);
    assert!(matches!(s.records[next.first().unwrap().0].action,Action::Bind(l) if l==c));
    let mut bad = s.records.clone();
    let r = bad
        .iter_mut()
        .find_map(|r| {
            if let Action::Request(Request::ProveEntries { predecessors, .. }) = &mut r.action {
                Some(predecessors)
            } else {
                None
            }
        })
        .unwrap();
    r.get_mut(&b).unwrap().insert(Some(a), 1);
    assert!(
        SelectedCfg::build(&bad)
            .unwrap_err()
            .contains("multiplicities")
    );
}
#[test]
fn malformed_labels_requests_modes_and_stack_equations_block_the_graph() {
    let c = graph_code();
    let s = c.selected.as_ref().unwrap();
    for mutation in 0..5 {
        let mut records = s.records.clone();
        match mutation {
            0 => {
                let r = records
                    .iter_mut()
                    .find(|r| matches!(r.action, Action::Bind(_)))
                    .unwrap();
                r.action = Action::Bind(Label(999));
            }
            1 => {
                let r = records
                    .iter_mut()
                    .find(|r| matches!(r.action, Action::EndRequest(_)))
                    .unwrap();
                r.action = Action::EndRequest(Node(999));
            }
            2 => {
                let r = records
                    .iter_mut()
                    .find(|r| matches!(r.action, Action::Instruction { .. }))
                    .unwrap();
                r.after.env.depth += 1;
            }
            3 => {
                let r = records
                    .iter_mut()
                    .find(|r| matches!(r.action, Action::Instruction { .. }))
                    .unwrap();
                r.parent = None;
            }
            4 => {
                let r = records
                    .iter_mut()
                    .find(|r| matches!(r.action, Action::Request(Request::Mode(_))))
                    .unwrap();
                r.action = Action::Request(Request::Mode(Width::Byte));
            }
            _ => unreachable!(),
        }
        assert!(SelectedCfg::build(&records).is_err(), "{mutation}");
        assert!(s.edited(records.clone()).is_err(), "edited/{mutation}");
        // Both entry constructors and replay publication reject the same bad
        // graph; callers cannot manufacture a prevalidated selected routine.
        let recording = Recording {
            records: records[..records.len() - 2].to_vec(),
            parents: vec![],
            source: None,
        };
        assert!(
            SelectedRoutine::new(RoutineId(7), &frame(), None, recording.clone(), &c).is_err(),
            "new/{mutation}"
        );
        assert!(s.replayed(recording, &c).is_err(), "replayed/{mutation}");
    }
}
#[test]
fn reconciliation_rejects_bytes_fixups_sources_and_encoding_boundaries() {
    let program = super::super::select::word_tests::program();
    let p = super::super::materialize(&program).unwrap();
    let code = &p.routines[0].code;
    let s = code.selected.as_ref().unwrap();
    for mutation in 0..5 {
        let mut code = code.clone();
        match mutation {
            0 => code.bytes[0] ^= 1,
            1 => code.fixups[0].target = Target::Routine(RoutineId(999)),
            2 => code.mir_spans.clear(),
            3 => *code.labels.values_mut().next().unwrap() += 1,
            4 => {
                code.boundaries.remove(&code.bytes.len());
            }
            _ => unreachable!(),
        }
        assert!(s.reconcile(&code).is_err(), "{mutation}");
    }
}

#[test]
fn selected_cfg_supports_the_shared_fixed_point_solver() {
    use crate::analysis::dataflow::{DataflowDirection, DataflowProblem, solve_dataflow};
    struct Ancestors;
    impl DataflowProblem<SelectedCfg> for Ancestors {
        type State = BTreeSet<Node>;
        fn direction(&self) -> DataflowDirection {
            DataflowDirection::Forward
        }
        fn bottom(&self) -> Self::State {
            BTreeSet::new()
        }
        fn boundary(&self, n: Node) -> Option<Self::State> {
            (n == Node(0)).then(|| [n].into())
        }
        fn join(&self, into: &mut Self::State, other: &Self::State) {
            into.extend(other);
        }
        fn transfer(&self, n: Node, state: &Self::State) -> Self::State {
            let mut s = state.clone();
            s.insert(n);
            s
        }
    }
    let code = graph_code();
    let s = code.selected.as_ref().unwrap();
    let result = solve_dataflow(s.cfg(), &Ancestors);
    for &n in s.cfg.nodes() {
        if s.cfg.reachable().contains(&n) {
            let ancestors = result.out_state(n).unwrap();
            assert!(ancestors.contains(&Node(0)) && ancestors.contains(&n));
            for p in s.cfg.predecessors(n) {
                if s.cfg.reachable().contains(p) {
                    assert!(ancestors.contains(p));
                }
            }
        } else {
            assert!(result.out_state(n).is_none());
        }
    }
}

#[test]
fn indirect_continuation_must_match_per_even_when_labels_share_a_pc() {
    let mut e = TrackedEmitter65816::default();
    let resume = e.label();
    let other = e.label();
    e.a16();
    e.op(Implied::Phk);
    e.push_return(resume);
    e.a8();
    e.byte(ByteOp::LdaImm, 1);
    e.op(Implied::Pha);
    e.a16();
    e.word(WordOp::LdaImm, 0x8000);
    e.op(Implied::Pha);
    e.indirect_transfer();
    e.mark(resume);
    e.mark(other);
    e.native_return(None).unwrap();
    let code = finish(e);
    let s = code.selected.as_ref().unwrap();
    assert_eq!(code.labels[&resume], code.labels[&other]);
    let mut bad = s.records.clone();
    for r in &mut bad {
        if let Action::Instruction {
            form: Instruction::IndirectTransfer(_),
            continuation,
            ..
        } = &mut r.action
        {
            *continuation = Some(other);
        }
    }
    assert!(
        SelectedCfg::build(&bad)
            .unwrap_err()
            .contains("differs from PER")
    );
    let mut bad = code.clone();
    bad.return_fixups[0].1 = other;
    assert!(s.reconcile(&bad).is_err());
}
