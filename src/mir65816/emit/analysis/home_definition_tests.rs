use super::super::home_liveness::HomeLiveness;
use super::super::home_tests::{DP, Graph, HIGH, LOW, access, homes};
use super::super::homes::{HomeInfo, HomeOwner};
use super::*;

fn facts(transfers: Vec<Vec<HomeAccess>>) -> Homes {
    let mut h = homes(transfers);
    h.info = [LOW, HIGH, DP]
        .into_iter()
        .map(|home| {
            (
                home,
                HomeInfo {
                    owners: [HomeOwner::DomainScratch].into(),
                    private: true,
                    entry_defined: false,
                },
            )
        })
        .collect();
    h
}
fn definition(home: HomeByte, store: usize) -> Definition {
    Definition {
        home,
        store: Node(store),
    }
}
fn nodes(defs: &HomeDefinitions, h: &Homes, home: HomeByte, store: usize) -> Vec<usize> {
    defs.uses_of_definition(h, definition(home, store))
        .unwrap()
        .iter()
        .map(|u| u.node.0)
        .collect()
}
#[test]
fn definitions_distinguish_stores_when_whole_home_is_live_at_window_end() {
    let graph = Graph::new(3, &[(0, 1), (1, 2)]);
    let h = facts(vec![
        vec![access(Access::Write, &[LOW])],
        vec![access(Access::Write, &[LOW])],
        vec![access(Access::Read, &[LOW])],
    ]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    assert_eq!(nodes(&defs, &h, LOW, 0), Vec::<usize>::new());
    assert_eq!(nodes(&defs, &h, LOW, 1), [2]);
    assert!(
        defs.definition_dead_outside_window(&graph, &h, definition(LOW, 0), Node(1))
            .unwrap()
    );
    assert!(
        !defs
            .definition_dead_outside_window(&graph, &h, definition(LOW, 1), Node(1))
            .unwrap()
    );
    assert!(
        HomeLiveness::analyze(&graph, &h)
            .after(Node(1))
            .unwrap()
            .contains(&LOW)
    );
}
#[test]
fn intermediate_read_is_attributed_before_later_overwrite() {
    let graph = Graph::new(4, &[(0, 1), (1, 2), (2, 3)]);
    let h = facts(vec![
        vec![access(Access::Write, &[LOW])],
        vec![access(Access::Read, &[LOW])],
        vec![access(Access::Write, &[LOW])],
        vec![access(Access::Read, &[LOW])],
    ]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    assert_eq!(nodes(&defs, &h, LOW, 0), [1]);
    assert_eq!(nodes(&defs, &h, LOW, 2), [3]);
    assert!(
        !defs
            .definition_dead_outside_window(&graph, &h, definition(LOW, 0), Node(0))
            .unwrap()
    );
    // True only about outside uses: the replacement still owes read 1 its value.
    assert!(
        defs.definition_dead_outside_window(&graph, &h, definition(LOW, 0), Node(2))
            .unwrap()
    );
}
#[test]
fn conditional_initialization_retains_undefined_path_and_joined_candidates() {
    let graph = Graph::new(4, &[(0, 1), (0, 2), (1, 3), (2, 3)]);
    for both in [false, true] {
        let h = facts(vec![
            vec![],
            vec![access(Access::Write, &[LOW])],
            if both {
                vec![access(Access::Write, &[LOW])]
            } else {
                vec![]
            },
            vec![access(Access::Read, &[LOW])],
        ]);
        let defs = HomeDefinitions::analyze(&graph, &h);
        assert_eq!(nodes(&defs, &h, LOW, 1), [3]);
        if both {
            assert_eq!(nodes(&defs, &h, LOW, 2), [3]);
        }
        assert_eq!(
            defs.undefined_private_reads()
                .iter()
                .any(|r| r.home == LOW && r.usage.node == Node(3)),
            !both
        );
    }
}
#[test]
fn partial_alias_overwrite_replaces_only_the_written_byte() {
    let graph = Graph::new(3, &[(0, 1), (1, 2)]);
    let h = facts(vec![
        vec![access(Access::Write, &[LOW, HIGH])],
        vec![access(Access::Write, &[LOW])],
        vec![access(Access::Read, &[LOW, HIGH])],
    ]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    assert!(nodes(&defs, &h, LOW, 0).is_empty());
    assert_eq!(nodes(&defs, &h, HIGH, 0), [2]);
    assert_eq!(nodes(&defs, &h, LOW, 1), [2]);
    assert!(defs.undefined_private_reads().is_empty());
}
#[test]
fn may_writes_preserve_candidates_and_cannot_initialize_private_bytes() {
    let graph = Graph::new(3, &[(0, 1), (1, 2)]);
    let h = facts(vec![
        vec![access(Access::Write, &[LOW])],
        vec![access(Access::MayWrite, &[LOW, HIGH])],
        vec![access(Access::Read, &[LOW, HIGH])],
    ]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    let uses = defs.uses_of_definition(&h, definition(LOW, 0)).unwrap();
    assert_eq!(uses.len(), 1);
    assert!(uses.first().unwrap().uncertain);
    assert!(
        defs.definition_dead_outside_window(&graph, &h, definition(LOW, 0), Node(0))
            .is_err()
    );
    assert_eq!(
        defs.undefined_private_reads()
            .iter()
            .map(|r| r.home)
            .collect::<Vec<_>>(),
        [HIGH]
    );
    assert!(defs.uses_of_definition(&h, definition(HIGH, 1)).is_err());
}
#[test]
fn unknown_reads_attribute_all_candidates_and_block_proof() {
    let graph = Graph::new(2, &[(0, 1)]);
    let mut read = access(Access::Read, &[LOW, HIGH, DP]);
    read.uncertain = true;
    let h = facts(vec![vec![access(Access::Write, &[LOW, DP])], vec![read]]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    assert_eq!(nodes(&defs, &h, LOW, 0), [1]);
    assert_eq!(nodes(&defs, &h, DP, 0), [1]);
    assert!(
        defs.definition_dead_outside_window(&graph, &h, definition(LOW, 0), Node(1))
            .is_err()
    );
    assert_eq!(defs.undefined_private_reads().len(), 1);
    assert!(defs.undefined_private_reads()[0].usage.uncertain);
}
#[test]
fn ordered_summary_reads_distinguish_old_and_internal_definitions() {
    let graph = Graph::new(2, &[(0, 1)]);
    let h = facts(vec![
        vec![access(Access::Write, &[HIGH])],
        vec![
            access(Access::Write, &[LOW]),
            access(Access::Read, &[HIGH]),
            access(Access::MayWrite, &[HIGH, DP]),
            access(Access::Read, &[LOW]),
        ],
    ]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    let argument = defs.uses_of_definition(&h, definition(HIGH, 0)).unwrap();
    assert_eq!(argument.first().unwrap().access_index, 1);
    assert!(!argument.first().unwrap().uncertain); // Later clobber cannot taint an earlier read.
    let transfer = defs.uses_of_definition(&h, definition(LOW, 1)).unwrap();
    assert_eq!(transfer.first().unwrap().access_index, 3);
    assert!(defs.undefined_private_reads().is_empty());
    assert!(
        defs.definition_dead_outside_window(&graph, &h, definition(LOW, 1), Node(1))
            .unwrap()
    );
}
#[test]
fn same_static_write_site_cannot_hide_previous_iteration_read() {
    let graph = Graph::new(3, &[(0, 1), (1, 2), (2, 1)]);
    let h = facts(vec![
        vec![access(Access::Write, &[LOW])],
        vec![access(Access::Read, &[LOW]), access(Access::Write, &[LOW])],
        vec![],
    ]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    assert_eq!(nodes(&defs, &h, LOW, 0), [1]);
    assert_eq!(nodes(&defs, &h, LOW, 1), [1]);
    assert!(
        defs.definition_dead_outside_window(&graph, &h, definition(LOW, 1), Node(1))
            .unwrap_err()
            .contains("loop-carried")
    );
    assert!(defs.undefined_private_reads().is_empty());
}
#[test]
fn abi_entry_values_seed_definedness_and_do_not_invent_stores() {
    let graph = Graph::new(1, &[]);
    let mut h = facts(vec![vec![access(Access::Read, &[LOW, HIGH, DP])]]);
    h.info.get_mut(&HIGH).unwrap().entry_defined = true;
    h.info.get_mut(&HIGH).unwrap().private = false;
    h.info.get_mut(&DP).unwrap().entry_defined = true;
    let defs = HomeDefinitions::analyze(&graph, &h);
    assert_eq!(
        defs.undefined_private_reads()
            .iter()
            .map(|r| r.home)
            .collect::<Vec<_>>(),
        [LOW]
    );
    assert!(defs.uses_of_definition(&h, definition(HIGH, 0)).is_err());
    assert!(defs.uses_of_definition(&h, definition(DP, 0)).is_err());
}
#[test]
fn invalid_unreachable_protected_and_nonlinear_windows_block() {
    let graph = Graph::new(5, &[(0, 1), (1, 2), (1, 3), (2, 3)]);
    let mut h = facts(vec![
        vec![access(Access::Write, &[LOW])],
        vec![],
        vec![],
        vec![],
        vec![access(Access::Write, &[LOW])],
    ]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    for node in [1, 4, 100] {
        assert!(defs.uses_of_definition(&h, definition(LOW, node)).is_err());
    }
    assert!(
        defs.uses_of_definition(&h, definition(HomeByte::Stack(-999), 0))
            .is_err()
    );
    for end in [2, 3, 4, 100] {
        assert!(
            defs.definition_dead_outside_window(&graph, &h, definition(LOW, 0), Node(end))
                .is_err()
        );
    }
    h.info.get_mut(&LOW).unwrap().private = false;
    assert!(
        defs.definition_dead_outside_window(&graph, &h, definition(LOW, 0), Node(0))
            .is_err()
    );
}
#[test]
fn repeated_internal_writes_block_coarse_definition_identity() {
    let graph = Graph::new(1, &[]);
    let h = facts(vec![vec![
        access(Access::Write, &[LOW]),
        access(Access::Read, &[LOW]),
        access(Access::Write, &[LOW]),
    ]]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    assert!(
        defs.uses_of_definition(&h, definition(LOW, 0))
            .unwrap_err()
            .contains("multiple writes")
    );
}
#[test]
fn new_replacement_reads_need_fresh_definedness_validation() {
    let graph = Graph::new(2, &[(0, 1)]);
    let original = facts(vec![vec![access(Access::Write, &[LOW])], vec![]]);
    let defs = HomeDefinitions::analyze(&graph, &original);
    assert!(
        defs.definition_dead_outside_window(&graph, &original, definition(LOW, 0), Node(1))
            .unwrap()
    );
    // An outside-use proof for the original is insufficient for a replacement
    // that introduces an uninitialized scratch read or retains a local read
    // after deleting its store. The future driver must analyze that candidate.
    let replacement = facts(vec![vec![], vec![access(Access::Read, &[LOW, DP])]]);
    let next = HomeDefinitions::analyze(&graph, &replacement);
    assert_eq!(
        next.undefined_private_reads()
            .iter()
            .map(|r| r.home)
            .collect::<BTreeSet<_>>(),
        [LOW, DP].into()
    );
}

#[test]
fn rmw_reads_previous_definition_and_then_defines_its_result() {
    let graph = Graph::new(3, &[(0, 1), (1, 2)]);
    let h = facts(vec![
        vec![access(Access::Write, &[DP])],
        vec![access(Access::Read, &[DP]), access(Access::Write, &[DP])],
        vec![access(Access::Read, &[DP])],
    ]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    assert_eq!(nodes(&defs, &h, DP, 0), [1]);
    assert_eq!(nodes(&defs, &h, DP, 1), [2]);
    assert!(defs.undefined_private_reads().is_empty());
}
#[test]
fn definite_write_clears_uncertainty_and_initializes_only_written_bytes() {
    let graph = Graph::new(4, &[(0, 1), (1, 2), (2, 3)]);
    let mut unknown = access(Access::Write, &[LOW, HIGH]);
    unknown.uncertain = true;
    let h = facts(vec![
        vec![access(Access::Write, &[LOW])],
        vec![unknown],
        vec![access(Access::Write, &[LOW])],
        vec![access(Access::Read, &[LOW, HIGH])],
    ]);
    let defs = HomeDefinitions::analyze(&graph, &h);
    assert!(
        defs.uses_of_definition(&h, definition(LOW, 0))
            .unwrap()
            .is_empty()
    );
    assert!(
        !defs
            .uses_of_definition(&h, definition(LOW, 2))
            .unwrap()
            .first()
            .unwrap()
            .uncertain
    );
    assert_eq!(
        defs.undefined_private_reads()
            .iter()
            .map(|r| r.home)
            .collect::<Vec<_>>(),
        [HIGH]
    );
    assert!(
        defs.definition_dead_outside_window(&graph, &h, definition(LOW, 0), Node(0))
            .unwrap()
    );
}
