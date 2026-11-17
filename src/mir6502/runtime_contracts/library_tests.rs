use super::*;
use crate::mir6502::analysis::known_callees::{MirAcyclicEntryAnalysis, MirCalleeEntryStage, MirKnownCalleeSummaries, MirKnownMemoryReads, MirKnownMemoryWrites};
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn program(source: &str, runtime: crate::runtime::Runtime) -> MirProgram {
    let path = std::env::temp_dir().join(format!("actionc-contract-{}-{}.act", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, source).unwrap();
    let loaded = crate::includes::load_compilation(&path, &crate::includes::ModuleLoadOptions::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    let model = crate::semantic::analyze_compilation_with_options(&loaded, crate::semantic::SemanticOptions::modern()).unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(&crate::semantic::ir::lower_compilation(&loaded, &model))).unwrap();
    crate::mir6502::materialize_program_with_origin_and_runtime(crate::mir6502::lower_program(&nir).unwrap(),
        &crate::mir6502::Mir6502Config { enable_small_leaf_inlining: false, ..crate::mir6502::Mir6502Config::optimized() }, 0x3000, runtime).unwrap()
}

#[test]
fn library_reader_contract_keeps_interface_identity_and_selected_body_evidence() {
    let source = "MODULE CLIENT USE SYS
BYTE ARRAY left=\"A\",right=\"B\" INT comparison BYTE byteValue CARD cardValue
INT FUNC Compare() RETURN(SYS.SCompare(left,right))
PROC Main() comparison=Compare() byteValue=SYS.Peek($600) cardValue=SYS.PeekC($600) RETURN ENDMODULE";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let r = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_COMPARE_")).unwrap();
        let call = r.blocks.iter().flat_map(|b| &b.ops).find(|op| matches!(op, MirOp::Call { abi, .. } if abi.library.is_some())).unwrap();
        let contract = library_invocation(&p, call).unwrap_or_else(|| panic!("missing library contract: {call:?}"));
        assert_eq!(contract.inputs.len(), 4);
        assert_eq!(contract.definitions, vec![MirMem::FixedZeroPage(MirFixedZpSlot(0xa0)), MirMem::FixedZeroPage(MirFixedZpSlot(0xa1))]);
        assert_eq!(contract.writes.len(), 6);
        let entry = MirAcyclicEntryAnalysis::new(&p).analyze(r.id).unwrap();
        assert!(entry.inputs.fixed.is_empty());
        assert!(!entry.inputs.flags.any());
        assert_eq!(entry.defined_results, BTreeSet::from([0xa0,0xa1]));
        let full = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
        assert_eq!(full.get(r.id).unwrap().reads(), &MirKnownMemoryReads::Unknown);
        assert!(matches!(full.get(r.id).unwrap().writes(), MirKnownMemoryWrites::Exact(_)));
        let peeks: Vec<_> = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops).filter_map(|op| match op {
            MirOp::Call { abi, .. } => abi.library.as_ref().filter(|b| b.interface.name.to_ascii_uppercase().ends_with(".PEEK") || b.interface.name.to_ascii_uppercase().ends_with(".PEEKC")), _ => None,
        }).collect();
        assert_eq!(peeks.len(), 2);
        assert_ne!(peeks[0].interface.id, peeks[1].interface.id);
        assert_ne!(peeks[0].interface.direct_result_abi, peeks[1].interface.direct_result_abi);
        if runtime == crate::runtime::Runtime::ActionCart {
            assert_eq!(peeks[0].implementation.as_ref().unwrap().target, peeks[1].implementation.as_ref().unwrap().target);
        }
        let mut invalid = call.clone();
        let MirOp::Call { args, .. } = &mut invalid else { unreachable!() };
        args.pop();
        assert!(library_invocation(&p, &invalid).is_none());
        let mut invalid = call.clone();
        let MirOp::Call { abi, .. } = &mut invalid else { unreachable!() };
        abi.library = None;
        assert!(library_invocation(&p, &invalid).is_none());
        let MirOp::Call { abi, .. } = call else { unreachable!() };
        if let MirRuntimeHelperTarget::Routine(id) = abi.library.as_ref().unwrap().implementation.as_ref().unwrap().target {
            let mut changed = p.clone();
            changed.routines.iter_mut().find(|r| r.id == id).unwrap().blocks[0].ops.clear();
            assert!(library_invocation(&changed, call).is_none());
            assert!(!entry.is_current(&changed, MirCalleeEntryStage::ActualEntry));
        }
    }
    let shadow = program("INT FUNC SCompare(INT a,b) RETURN(a+b) PROC Main() SCompare(1,2) RETURN", crate::runtime::Runtime::ActionCart);
    assert!(shadow.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops).all(|op| !matches!(op, MirOp::Call { abi, .. } if abi.library.is_some())));
}
