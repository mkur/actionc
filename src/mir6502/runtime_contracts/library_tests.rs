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

#[test]
fn buffer_writes_keep_numeric_ranges_private_permissions_and_unknown_full_effects() {
    use crate::mir6502::analysis::{sites::MirSite, storage_ownership::ObjectAddressControl};
    let source = "MODULE CLIENT USE SYS
BYTE ARRAY source(256)=$900,destination(256)=$B00 BYTE POINTER ptr=$700
PROC Copies() SYS.SCopy(destination,source) SYS.SCopy($E0,source) RETURN
PROC Dynamic() SYS.SCopy(ptr,source) RETURN
PROC Forward(STRING dst) SYS.SCopy(dst,source) RETURN
BYTE FUNC Exposed(BYTE keep) SYS.SCopy(@keep,source) RETURN(keep)
PROC Main() RETURN ENDMODULE";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let r = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_COPIES_")).unwrap();
        let values = buffers::argument_values(&p, r).unwrap();
        let ranges: Vec<_> = r.blocks.iter().flat_map(|b| b.ops.iter().enumerate().filter_map(|(op_index, op)| {
            let contract = library_invocation(&p, op)?;
            assert_eq!(contract.buffers.len(), 2);
            assert!(contract.definitions.is_empty());
            Some(contract.buffers.iter().find(|b| b.write).unwrap().project(&p, op,
                Some((&values, MirSite::Op { block: b.id, op_index }))))
        })).collect();
        assert_eq!(ranges, [(Some(0xb00),256),(Some(0xe0),256)]);
        let full = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
        assert_eq!(full.get(r.id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
        assert_eq!(full.get(r.id).unwrap().reads(), &MirKnownMemoryReads::Unknown);
        let normal = MirKnownCalleeSummaries::analyze_for_residence(&p);
        let writes = normal.get(r.id).unwrap().writes();
        assert!(!writes.preserves_private_fixed_byte(MirFixedZpSlot(0xe4), normal.fixed_storage()));
        let forward = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_FORWARD_")).unwrap();
        assert_eq!(normal.get(forward.id).unwrap().writes(), &MirKnownMemoryWrites::Unknown);
        let dynamic = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_DYNAMIC_")).unwrap();
        let writes = normal.get(dynamic.id).unwrap().writes();
        assert!(matches!(writes, MirKnownMemoryWrites::Pointee { numeric, .. } if numeric.is_empty()));
        assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(0xe4), normal.fixed_storage()));
        assert!(writes.preserves_private_fixed_byte(MirFixedZpSlot(0xe4), normal.fixed_storage()));
        assert!(!writes.preserves_private_fixed_byte(MirFixedZpSlot(0xa1), normal.fixed_storage()));
        let exposed = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_EXPOSED_")).unwrap();
        let MirStorageBase::Param(id) = exposed.frame.params[0].base else { panic!() };
        assert!(!ObjectAddressControl::new(&p).compiler_controls_address(exposed.id, &MirMem::Param { id, offset: 0 }));
    }
}
