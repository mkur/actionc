use super::*;
use crate::mir65816::emit::{self, work};

fn fixture(name: &str) -> emit::MachineProgram {
    let p = crate::compiler::native65816::prepare_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tools/native65816-runtime-tests/tests/fixtures/code_quality/{name}.act"
        )),
        false,
        &Default::default(),
    )
    .unwrap();
    emit::materialize(&p.mir).unwrap()
}
fn eager(selected: &SelectedRoutine) -> AnalysisSnapshot<'_> {
    let s = AnalysisSnapshot::new(selected).unwrap();
    assert!(
        s.live
            .set(HomeLiveness::analyze(selected.cfg(), &s.homes))
            .is_ok()
    );
    assert!(
        s.definitions
            .set(HomeDefinitions::analyze(selected.cfg(), &s.homes))
            .is_ok()
    );
    assert!(s.machine.set(MachineLiveness::analyze(selected)).is_ok());
    s
}

#[test]
fn lazy_queries_equal_eager_results_across_calls_loops_and_pointer_aliases() {
    for case in ["add", "loop_rotation", "direct_calls", "forward_copy"] {
        let machine = fixture(case);
        for routine in &machine.routines {
            let selected = routine.code.selected.as_ref().unwrap();
            let reference = eager(selected);
            let lazy = AnalysisSnapshot::new(selected).unwrap();
            for i in 0..selected.records().len() {
                let site = selected.site(Node(i)).unwrap();
                assert_eq!(
                    lazy.home_live_before(site),
                    reference.home_live_before(site)
                );
                assert_eq!(lazy.home_live_after(site), reference.home_live_after(site));
                assert_eq!(
                    lazy.machine_live_before(site),
                    reference.machine_live_before(site)
                );
                assert_eq!(
                    lazy.machine_live_after(site),
                    reference.machine_live_after(site)
                );
                for access in &reference.homes.accesses[&Node(i)] {
                    for &home in &access.homes {
                        assert_eq!(
                            lazy.uses_of_definition(home, site),
                            reference.uses_of_definition(home, site)
                        );
                        assert_eq!(
                            lazy.definition_dead_outside_window(home, site, site),
                            reference.definition_dead_outside_window(home, site, site)
                        );
                    }
                }
            }
            assert_eq!(
                lazy.undefined_private_reads(),
                reference.undefined_private_reads()
            );
        }
    }
}

#[test]
fn invalid_sites_do_not_trigger_solvers_and_each_demand_runs_once() {
    let machine = fixture("add");
    let selected = machine.routines[0].code.selected.as_ref().unwrap();
    let site = selected.site(Node(0)).unwrap();
    let edited = selected.edited(selected.records().to_vec()).unwrap();
    let wrong = edited.site(Node(0)).unwrap();
    let (_, work) = work::measure(|| {
        let s = AnalysisSnapshot::new(selected).unwrap();
        let home = *s.homes.info.keys().next().unwrap();
        assert!(s.home_live_before(wrong).is_err());
        assert!(s.machine_live_after(wrong).is_err());
        assert!(s.uses_of_definition(home, wrong).is_err());
        assert!(s.definition_dead_outside_window(home, site, wrong).is_err());
        assert!(
            s.live.get().is_none() && s.definitions.get().is_none() && s.machine.get().is_none()
        );
        for _ in 0..3 {
            s.home_live_before(site).unwrap();
            s.home_live_after(site).unwrap();
            s.machine_live_before(site).unwrap();
            s.machine_live_after(site).unwrap();
            let _ = s.undefined_private_reads();
        }
        let next = AnalysisSnapshot::new(&edited).unwrap();
        assert!(next.machine.get().is_none());
        assert!(next.machine_live_after(site).is_err());
        assert!(next.machine.get().is_none());
    });
    assert_eq!(work.get("homes"), Some(&2));
    for kind in ["home_liveness", "home_definitions", "machine_liveness"] {
        assert_eq!(work.get(kind), Some(&1), "{kind}");
    }
}

#[test]
fn adjacent_transactions_do_not_request_unused_liveness() {
    let (_, work) = work::measure(|| fixture("add"));
    assert_eq!(work.get("home_liveness"), None);
    assert_eq!(work.get("machine_liveness"), None);
    assert!(work["home_definitions"] > 0);
}
