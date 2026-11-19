use super::*;
use crate::mir6502::ir::MirValue;

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
    check_restore_pos_writes(&mir);
}

fn check_restore_pos_writes(mir: &MirProgram) {
    let source_homes = MirKnownCalleeSummaries::analyze_for_source_homes(mir, 0x3000);
    let writes = |name| {
        let id = mir.routines.iter().find(|r| r.name == name).unwrap().id;
        source_homes.get(id).unwrap().writes()
    };
    let restore = writes("RestorePos");
    assert!(matches!(restore, MirKnownMemoryWrites::Pointee { numeric, .. }
        if numeric == &[(0x5a, 4)]), "{restore:?}");
    for slot in [0x5a, 0x5b, 0x5c, 0x5d, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xe8, 0xe9] {
        assert!(!restore.preserves_fixed_byte(MirFixedZpSlot(slot), source_homes.fixed_storage()), "${slot:02x}");
    }
    for slot in [0x59, 0x5e, 0xea] {
        assert!(restore.preserves_fixed_byte(MirFixedZpSlot(slot), source_homes.fixed_storage()), "${slot:02x}: {restore:?}");
    }
    assert_eq!(writes("MovePage"), &MirKnownMemoryWrites::Unknown);
    let wrapper = mir.routines.iter().find(|r| r.name == "RestorePos").unwrap();
    let copy = mir.routines.iter().find(|r| r.name == "MovePage").unwrap().id;
    let (site, call) = wrapper.blocks.iter().find_map(|block| {
        block.ops.iter().enumerate().find_map(|(op_index, op)| {
            matches!(op, MirOp::Call { target: MirCallTarget::Routine(id), .. } if *id == copy)
                .then_some((MirSite::Op { block: block.id, op_index }, op))
        })
    }).unwrap();
    let mut call = call.clone();
    let MirOp::Call { args, .. } = &mut call else { unreachable!() };
    // The diagnostic operand can be stale. The expanded call reads $A4.
    args.last_mut().unwrap().value = MirValue::ConstU8(200);
    let values = crate::mir6502::runtime_contracts::buffers::argument_values(mir, wrapper).unwrap();
    let invocation = source_homes.writes_at_site(mir, wrapper, &call, &values, site);
    assert!(matches!(invocation, MirKnownMemoryWrites::Pointee { ref numeric, .. }
        if numeric == &[(0x5a, 4)]), "{invocation:?}");
}

#[test]
fn opaque_getchar_uses_the_same_boundary_in_range_and_at_a_direct_call() {
    let sem = tn_semir();
    let input = crate::mir6502::lower_program(&crate::nir::lower_program(&sem)).unwrap();
    let p = crate::mir6502::materialize_program(input, &Default::default()).unwrap();
    let leaf = p.routines.iter().find(|r| r.name == "Getchar").unwrap();
    let wrapper = p.routines.iter().find(|r| r.name == "Range").unwrap();
    let hash = body_fingerprint(&p, leaf, crate::callee6502::CONTRACT_VERSION);
    assert_eq!(hash, Some(leaf.frame.callee_review.as_ref().unwrap().fingerprint), "{hash:016x?}");
    let facts = reviewed_contract(&p, leaf).unwrap();
    let classic = crate::codegen::projected_review_for_test(&sem, "Getchar").unwrap();
    assert_eq!(classic.preserves_zp, facts.preserves_zp);
    assert!(facts.abi_inputs_only && facts.returns_a_equals_a0);
    assert!(facts.reads.is_none() && facts.writes.is_none());
    let mut entries = MirAcyclicEntryAnalysis::at_origin(&p, 0x3000);
    let entry = entries.analyze(leaf.id).unwrap();
    assert_eq!(entry.inputs, Default::default());
    assert!(entries.analyze(wrapper.id).is_some());
    let homes = MirKnownCalleeSummaries::analyze_for_source_homes(&p, 0x3000);
    let writes = homes.get(leaf.id).unwrap().writes();
    for slot in 0..=u8::MAX {
        assert_eq!(writes.preserves_fixed_byte(MirFixedZpSlot(slot), homes.fixed_storage()),
            facts.preserves_zp.iter().any(|r| (r.start..=r.end).contains(&slot)));
    }
    // Isolate the other write in Range. The partial OS footprint composes
    // through the native loop and its scratch writes when c^ is omitted.
    let mut control = p.clone();
    let range = control.routines.iter_mut().find(|r| r.id == wrapper.id).unwrap();
    let mut count = 0;
    for b in &mut range.blocks {
        b.ops.retain(|op| {
            let indirect = crate::mir6502::analysis::effects::writes_ordinary_pointee(op);
            if indirect { count += 1; }
            !indirect
        });
    }
    assert!(count > 0);
    let control = MirKnownCalleeSummaries::analyze_for_source_homes(&control, 0x3000);
    let returned = control.get(wrapper.id).unwrap().writes();
    assert!(returned.preserves_private_fixed_byte(MirFixedZpSlot(0xef), control.fixed_storage()), "{returned:?}");
    assert!(!returned.preserves_private_fixed_byte(MirFixedZpSlot(0xa1), control.fixed_storage()));
    let general = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    assert_eq!(general.get(leaf.id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
    assert_eq!(general.get(leaf.id).unwrap().reads(), &MirKnownMemoryReads::Unknown);
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
