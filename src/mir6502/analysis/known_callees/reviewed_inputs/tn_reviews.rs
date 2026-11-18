use super::*;

#[test]
fn tn_destination_contracts_match_both_backends_and_bound_native_writers() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/tn/modern/TN.ACT");
    let ast = crate::includes::load_program_with_includes(path).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let sem = crate::semantic::ir::lower_program(&ast, &model);
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
