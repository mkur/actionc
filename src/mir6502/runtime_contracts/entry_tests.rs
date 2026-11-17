use super::*;
use crate::mir6502::analysis::known_callees::{MirAcyclicEntryAnalysis, MirCalleeEntryStage, MirKnownCalleeSummaries, MirKnownMemoryWrites};
use crate::mir6502::parameter_homes::ResidenceCalls;

#[test]
fn sargs_entry_composes_all_lanes_and_normal_writes_without_erasing_full_effects() {
    let source = "BYTE out0=$900 CARD out1=$901 BYTE out2=$903,out3=$904
PROC Capture(BYTE a CARD word BYTE c,d) out0=a out1=word out2=c out3=d RETURN
PROC Middle() Capture(1,$2345,6,7) RETURN
CARD FUNC Hold(CARD keep) Middle() RETURN(keep)
PROC Main() RETURN";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = super::tests::program(source, runtime);
        let capture = p.routines.iter().find(|r| r.name == "Capture").unwrap();
        let middle = p.routines.iter().find(|r| r.name == "Middle").unwrap();
        let hold = p.routines.iter().find(|r| r.name == "Hold").unwrap();
        let call = middle.blocks.iter().flat_map(|b| &b.ops).find(|o| matches!(o, MirOp::Call { .. })).unwrap();
        let mut entries = MirAcyclicEntryAnalysis::new(&p);
        let entry = entries.analyze(capture.id).unwrap();
        assert_eq!(entry.inputs.fixed, BTreeSet::from([0xa3, 0xa4]));
        assert!(entry.inputs.registers.a && entry.inputs.registers.x && entry.inputs.registers.y);
        assert!(!entry.inputs.flags.any());
        assert!(entries.analyze(middle.id).unwrap().inputs.fixed.is_empty());
        assert!(capture.frame.parameter_homes.is_none());
        let full = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
        assert_eq!(full.get(capture.id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
        let mut calls = ResidenceCalls::with_acyclic(&p, MirCalleeEntryStage::ActualEntry);
        let fixed = calls.fixed_storage();
        let writes = calls.writes(call).unwrap();
        assert!(writes.preserves_fixed_byte(MirFixedZpSlot(0xe4), &fixed));
        for scratch in [0x82,0x83,0x84,0x85,0xa0,0xa1,0xa2] {
            assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(scratch), &fixed));
        }
        let nested = hold.blocks.iter().flat_map(|b| &b.ops).find(|o| matches!(o, MirOp::Call { .. })).unwrap();
        assert!(calls.writes(nested).unwrap().preserves_fixed_byte(MirFixedZpSlot(0xe4), &fixed));
        for swapped in [false, true] {
            let mut bad = call.clone();
            let MirOp::Call { args, .. } = &mut bad else { unreachable!() };
            if swapped { args.swap(2,3); } else { args.pop(); }
            assert!(calls.writes(&bad).is_err());
        }
        let mut stale = p.clone();
        stale.runtime_helpers.iter_mut().find(|d| d.helper == MirRuntimeHelper::SArgs).unwrap().binding = None;
        assert!(!entry.is_current(&stale, MirCalleeEntryStage::ActualEntry));
        assert!(ResidenceCalls::with_acyclic(&stale, MirCalleeEntryStage::ActualEntry).writes(call).is_err());
        let mut stale = p.clone();
        stale.routines.iter_mut().find(|r| r.id == capture.id).unwrap().frame.params[1].storage_size = 1;
        assert!(!entry.is_current(&stale, MirCalleeEntryStage::ActualEntry));
        assert!(ResidenceCalls::with_acyclic(&stale, MirCalleeEntryStage::ActualEntry).writes(call).is_err());
    }
}
