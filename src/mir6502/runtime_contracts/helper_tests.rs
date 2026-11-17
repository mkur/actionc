use super::*;
use super::invocation::helper_invocation;
use crate::mir6502::analysis::known_callees::{MirCalleeEntryFact, MirCalleeEntryStage};
use crate::mir6502::parameter_homes::ResidenceCalls;

#[test]
fn multiply_and_shift_contracts_keep_inputs_results_clobbers_and_binding_identity_separate() {
    let helpers = [MirRuntimeHelper::MulByte, MirRuntimeHelper::Mul, MirRuntimeHelper::Lsh,
        MirRuntimeHelper::Rsh, MirRuntimeHelper::Mul32, MirRuntimeHelper::Lsh32, MirRuntimeHelper::Rsh32];
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let mut p = super::tests::program("PROC Main() RETURN", runtime);
        p.runtime_helpers = helpers.iter().copied().map(crate::mir6502::materialize::deferred_helper_decl).collect();
        crate::mir6502::runtime::resolve_helpers(&mut p, runtime).unwrap();
        for helper in helpers {
            let op = MirOp::RuntimeHelper { helper, args: signatures::helper_args(&helper),
                result: None, additional_results: signatures::helper_additional_results(helper),
                effects: signatures::helper_effects(&helper) };
            let summary = helper_invocation(&p, &op).unwrap();
            p.routines[0].blocks[0].ops = vec![op.clone()];
            let entry = MirCalleeEntryFact::analyze(&p, p.routines[0].id, MirCalleeEntryStage::ActualEntry).unwrap();
            assert!(!entry.inputs.flags.any());
            assert!(!entry.inputs.registers.sp);
            let fixed = summary.inputs.iter().filter_map(|h| match h { MirArgHome::FixedZeroPage(s) => Some(s.0), _ => None }).collect();
            assert_eq!(entry.inputs.fixed, fixed, "initialized scratch must not become input: {helper:?}");
            assert_eq!(summary.definitions.len(), if helper.is_wide() { 4 } else { 0 });
            let mut calls = ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry);
            let fixed = calls.fixed_storage();
            let writes = calls.writes(&op).unwrap();
            assert!(writes.preserves_fixed_byte(MirFixedZpSlot(0xe4), &fixed));
            assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(0x85), &fixed));
            let mut bad = p.clone();
            bad.runtime_helpers.iter_mut().find(|d| d.helper == helper).unwrap().binding = None;
            assert!(helper_invocation(&bad, &op).is_none());
            assert!(!entry.is_current(&bad, MirCalleeEntryStage::ActualEntry));
        }
        let mut bad = p.clone();
        let d = bad.runtime_helpers.iter_mut().find(|d| d.helper == MirRuntimeHelper::MulByte).unwrap();
        d.helper = MirRuntimeHelper::DivU8;
        d.abi = signatures::helper_abi_for(d.helper);
        d.effects = signatures::helper_effects(&d.helper);
        assert!(selected_helper(&bad, MirRuntimeHelper::DivU8).is_none());
        if runtime == crate::runtime::Runtime::Standalone {
            let d = selected_helper(&p, MirRuntimeHelper::Mul).unwrap();
            let binding = d.binding.as_ref().unwrap();
            assert!(!binding.fallthroughs.is_empty(), "Mul's SetSign/SS1 dependency must be retained");
            let (from, to) = binding.fallthroughs[0];
            let mut bad = p.clone();
            let i = bad.routines.iter().position(|r| r.id == from).unwrap();
            let j = bad.routines.iter().position(|r| r.id == to).unwrap();
            bad.routines.swap(i,j);
            assert!(selected_helper(&bad, MirRuntimeHelper::Mul).is_none());
        }
    }
}

#[test]
fn arithmetic_action_wrappers_publish_results_and_preserve_caller_homes() {
    use crate::mir6502::analysis::known_callees::MirAcyclicEntryAnalysis;
    let p = super::tests::program("CARD FUNC Product(CARD a,b) RETURN(a*b)
CARD FUNC Middle(CARD a,b) RETURN(Product(a,b))
CARD FUNC Hold(CARD keep) Middle(5,7) RETURN(keep)
PROC Main() RETURN", crate::runtime::Runtime::Standalone);
    let mut entries = MirAcyclicEntryAnalysis::new(&p);
    for name in ["Product", "Middle"] {
        let r = p.routines.iter().find(|r| r.name == name).unwrap();
        let entry = entries.analyze(r.id).unwrap();
        assert_eq!(entry.defined_results, BTreeSet::from([0xa0,0xa1]));
        assert_eq!(entry.inputs.fixed, BTreeSet::from([0xa3]));
        assert!(!entry.inputs.flags.any());
    }
    let hold = p.routines.iter().find(|r| r.name == "Hold").unwrap();
    let call = hold.blocks.iter().flat_map(|b| &b.ops).find(|o| matches!(o, MirOp::Call { .. })).unwrap();
    let mut calls = ResidenceCalls::with_acyclic(&p, MirCalleeEntryStage::ActualEntry);
    let fixed = calls.fixed_storage();
    assert!(calls.writes(call).unwrap().preserves_fixed_byte(MirFixedZpSlot(0xe4), &fixed));
}
