use super::*;

fn tn_semir() -> crate::semantic::ir::SemProgram {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/tn/modern/TN.ACT");
    let ast = crate::includes::load_program_with_includes(path).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    crate::semantic::ir::lower_program(&ast, &model)
}

#[test]
fn tn_destination_contracts_match_both_backends_and_bound_native_writers() {
    let sem = tn_semir();
    let mir = crate::mir6502::lower_program(&crate::nir::lower_program(&sem)).unwrap();

    for name in [
        "MoveToArena",
        "_ImageAdvance",
        "Block",
        "GetImage",
        "PutImage",
    ] {
        let r = mir.routines.iter().find(|r| r.name == name).unwrap();
        let review = r.frame.callee_review.as_ref().unwrap();
        let computed = body_fingerprint(&mir, r, review.version());
        assert_eq!(
            computed,
            Some(review.fingerprint),
            "{name} fingerprint {computed:016x?}"
        );
        let f = reviewed_contract(&mir, r).unwrap();
        let classic = crate::codegen::projected_review_for_test(&sem, name)
            .unwrap_or_else(|| panic!("stale classic review for {name}"));
        let common = crate::callee6502::Review {
            facts: classic,
            fingerprint: review.fingerprint,
        }
        .map_ids(|_| None)
        .unwrap();
        assert_eq!(common.facts, *f, "{name}");
        if let Some(range) = f.indirect_writes {
            let mut altered = mir.clone();
            let body = &mut altered
                .routines
                .iter_mut()
                .find(|r| r.name == name)
                .unwrap()
                .frame;
            body.callee_review
                .as_mut()
                .unwrap()
                .facts
                .indirect_writes
                .as_mut()
                .unwrap()
                .end = range.end - 1;
            let changed = altered.routines.iter().find(|r| r.name == name).unwrap();
            assert!(
                reviewed_contract(&altered, changed).is_none(),
                "{name}: stale destination range"
            );
        }
        if name != "_ImageAdvance" {
            assert!(f.abi_inputs_only && f.physical_abi_inputs);
        }
    }
    let mir = crate::mir6502::materialize_program(mir, &Default::default()).unwrap();
    let summaries = MirKnownCalleeSummaries::analyze_for_residence(&mir);
    for name in ["Block", "GetImage", "PutImage", "Push", "SavePos"] {
        let r = mir.routines.iter().find(|r| r.name == name).unwrap();
        let writes = summaries.get(r.id).unwrap().writes();
        assert!(
            writes.preserves_fixed_byte(MirFixedZpSlot(0xe2), summaries.fixed_storage()),
            "{name}: {writes:?}"
        );
        assert!(
            !writes.preserves_fixed_byte(MirFixedZpSlot(0xa0), summaries.fixed_storage()),
            "{name}"
        );
        let state = if matches!(name, "Push" | "SavePos") {
            0xe8
        } else {
            0xe6
        };
        assert!(
            !writes.preserves_fixed_byte(MirFixedZpSlot(state), summaries.fixed_storage()),
            "{name}"
        );
    }
}

#[test]
fn internal_review_composes_through_native_print_callers() {
    use crate::mir6502::analysis::known_callees::{MirAcyclicEntryAnalysis, MirCalleeEntryFact, MirCalleeEntryStage};
    let input = crate::mir6502::lower_program(&crate::nir::lower_program(&tn_semir())).unwrap();
    let p = crate::mir6502::materialize_program(input, &Default::default()).unwrap();
    let id = |name| p.routines.iter().find(|r| r.name == name).unwrap().id;
    let stage = MirCalleeEntryStage::ActualEntry;
    let internal = id("Internal");
    let review = reviewed_contract(&p, p.routines.iter().find(|r| r.id == internal).unwrap()).unwrap();
    assert!(review.abi_inputs_only && !review.physical_abi_inputs);
    assert!(MirCalleeEntryFact::analyze(&p, internal, stage).is_none());
    let mut home = MirAcyclicEntryAnalysis::new(&p);
    let leaf = home.analyze(internal).unwrap();
    assert!(leaf.inputs.registers.a && !leaf.inputs.registers.x && !leaf.inputs.registers.y);
    assert!(!leaf.inputs.flags.any() && leaf.inputs.fixed.is_empty());
    // With no placement evidence the table might overlap $A0. At TN's
    // origin the checked inline array lies entirely above the zero page.
    assert!(home.analyze(id("Putchar")).unwrap().inputs.fixed.contains(&0xa0));
    let mut placed = MirAcyclicEntryAnalysis::at_origin(&p, 0x2c00);
    for name in ["Putchar", "Print"] {
        let inputs = &placed.analyze(id(name)).unwrap().inputs;
        assert!(inputs.fixed.is_empty(), "{name}: {inputs:?}");
    }
    // Removing only the guaranteed writes leaves the original may-write contract.
    let mut weaker = p.clone();
    weaker.routines.iter_mut().find(|r| r.name == "CalcAdr").unwrap()
        .frame.callee_review.as_mut().unwrap().facts.defines_on_return.clear();
    for name in ["Block", "GetImage", "PutImage"] {
        let caller = weaker.routines.iter().find(|r| r.name == name).unwrap();
        assert!(reviewed_contract(&weaker, caller).is_none(), "{name}");
    }
    let mut old = MirAcyclicEntryAnalysis::at_origin(&weaker, 0x2c00);
    assert_eq!(old.analyze(id("Putchar")).unwrap().inputs.fixed, BTreeSet::from([0xe6, 0xe7]));
    for name in ["Putchar", "Print"] {
        let wrapper = home.analyze(id(name)).unwrap();
        assert!(!wrapper.inputs.flags.c);
        assert!(wrapper.is_current(&p, stage));
        let mut stale = p.clone();
        stale.globals.iter_mut().find(|g| g.name == "internalMap").unwrap().storage_size = 3;
        assert!(!wrapper.is_current(&stale, stage));
        assert!(MirAcyclicEntryAnalysis::new(&stale).analyze(id(name)).is_none());
    }
}
