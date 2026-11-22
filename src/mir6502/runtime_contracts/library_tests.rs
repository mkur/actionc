use super::*;
use super::library::graphics_cio_boundary;
use crate::mir6502::analysis::known_callees::{MirAcyclicEntryAnalysis, MirCalleeEntryStage, MirKnownCalleeSummaries, MirKnownMemoryReads, MirKnownMemoryWrites};
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn checked_nir(source: &str) -> crate::nir::NirProgram {
    let path = std::env::temp_dir().join(format!("actionc-contract-{}-{}.act", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, source).unwrap();
    let loaded = crate::includes::load_compilation(&path, &crate::includes::ModuleLoadOptions::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    let model = crate::semantic::analyze_compilation_with_options(&loaded, crate::semantic::SemanticOptions::modern()).unwrap();
    crate::nir::optimize_program(&crate::nir::lower_program(&crate::semantic::ir::lower_compilation(&loaded, &model))).unwrap()
}

fn program(source: &str, runtime: crate::runtime::Runtime) -> MirProgram {
    crate::mir6502::materialize_program_with_origin_and_runtime(crate::mir6502::lower_program(&checked_nir(source)).unwrap(),
        &crate::mir6502::Mir6502Config { enable_small_leaf_inlining: false, ..crate::mir6502::Mir6502Config::optimized() }, 0x3000, runtime).unwrap()
}

#[test]
fn plot_and_locate_reviews_keep_the_cio_boundary_open() {
    let source = "BYTE colorResult=$0600 PROC Main()
                  Plot(10,20) colorResult=Locate(10,20) RETURN";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let calls = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter(|call| graphics_cio_boundary(&p, call).is_some()).collect::<Vec<_>>();
        assert_eq!(calls.len(), 2, "{runtime:?}");
        assert_eq!(graphics_cio_boundary(&p, calls[0]), Some("PutD on S: (reads COLOR at $02FD)"));
        assert_eq!(graphics_cio_boundary(&p, calls[1]), Some("GetD on S: (successful pixel result at $A0)"));
        assert!(calls.iter().all(|call| library_invocation(&p, call).is_none()));
        if runtime == crate::runtime::Runtime::Standalone {
            let mut changed = p.clone();
            let position = changed.routines.iter_mut().find(|r| r.name.ends_with("::Pos1")).unwrap();
            position.blocks.clear();
            assert!(calls.iter().all(|call| graphics_cio_boundary(&changed, call).is_none()));
        }
    }
}

#[test]
fn draw_and_fill_share_grio_without_sharing_command_identity() {
    let source = "PROC Main() DrawTo(12,22) Fill(14,24) RETURN";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let calls = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter(|call| graphics_cio_boundary(&p, call).is_some()).collect::<Vec<_>>();
        assert_eq!(calls.len(), 2, "{runtime:?}");
        assert_eq!(graphics_cio_boundary(&p, calls[0]), Some("XIO $11 on S: (reads COLOR and writes $02FB)"));
        assert_eq!(graphics_cio_boundary(&p, calls[1]), Some("XIO $12 on S: (reads COLOR and writes $02FB)"));
        assert!(calls.iter().all(|call| library_invocation(&p, call).is_none()));
        if runtime == crate::runtime::Runtime::Standalone {
            let mut changed = p.clone();
            let grio = changed.routines.iter_mut().find(|r| r.name.ends_with("::GrIO")).unwrap();
            grio.blocks.clear();
            assert!(calls.iter().all(|call| graphics_cio_boundary(&changed, call).is_none()));
        }
    }
}

#[test]
fn setcolor_can_preserve_a_caller_home_when_the_register_is_known() {
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        for register in ["2", "register"] {
            let source = format!(
                "BYTE FUNC Across(BYTE keep,register)
                 SetColor({register},10,12)
                 keep==+1
                 RETURN(keep)
                 PROC Main() BYTE out=$600 out=Across(13,2) RETURN"
            );
            let output = crate::mir6502::generate_output_with_config_and_runtime(
                &checked_nir(&source), 0x3000, &crate::mir6502::Mir6502Config::optimized(), runtime,
            ).unwrap();
            let home = output.map.storage_symbols.iter().find(|symbol| symbol.name == "keep"
                && symbol.scope == crate::codegen::CodegenSymbolScope::Routine("Across".into())).unwrap();
            assert_eq!(home.address < 256, register == "2", "{runtime:?} {register} {home:?}");
        }
    }
}

#[test]
fn graphics_leaf_reviews_cover_fallthrough_and_guard_color_writes() {
    let source = "MODULE CLIENT USE SYS
PROC Color(BYTE register) SYS.SetColor(register,10,12) RETURN
PROC Main() SYS.Position($1234,$56) SYS.SetColor(2,10,12) SYS.SetColor(133,10,12) RETURN ENDMODULE";
    use crate::callee6502::sys_graphics::GraphicsRoutine;
    use crate::mir6502::analysis::sites::MirSite;
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let mut writes = Vec::new();
        let mut linked = None;
        for routine in &p.routines {
            for block in &routine.blocks {
                for (op_index, call) in block.ops.iter().enumerate() {
                    let MirOp::Call { abi, .. } = call else { continue; };
                    if abi.library.is_none() { continue; }
                    let summary = library_invocation(&p, call).unwrap_or_else(||
                        panic!("missing {runtime:?} graphics review: {call:?}"));
                    let values = buffers::argument_values(&p, routine).unwrap();
                    let site = MirSite::Op { block: block.id, op_index };
                    writes.push((summary.graphics.unwrap(), summary.normal_writes(&p, call, Some((&values, site)))));
                    if summary.graphics == Some(GraphicsRoutine::Position) {
                        linked = Some(call);
                    }
                }
            }
        }
        assert!(writes.contains(&(GraphicsRoutine::Position, MirKnownMemoryWrites::Exact(
            (0x54..=0x56).chain(0x5a..=0x5c).map(|n| MirMem::FixedZeroPage(MirFixedZpSlot(n))).collect()
        ))), "{writes:?}");
        assert!(writes.contains(&(GraphicsRoutine::SetColor, MirKnownMemoryWrites::Exact(vec![
            MirMem::FixedZeroPage(MirFixedZpSlot(0xa0)), MirMem::FixedZeroPage(MirFixedZpSlot(0xa2)),
            MirMem::Absolute(0x02c6), MirMem::Absolute(0xd018),
        ]))), "{writes:?}");
        assert_eq!(writes.iter().filter(|(_, w)| *w == MirKnownMemoryWrites::Unknown).count(), 2);
        if runtime == crate::runtime::Runtime::Standalone {
            let mut changed = p.clone();
            let pos1 = changed.routines.iter_mut().find(|r| r.name.ends_with("::Pos1")).unwrap();
            pos1.blocks.clear();
            assert!(library_invocation(&changed, linked.unwrap()).is_none());
        }
    }
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

#[test]
fn memory_library_contracts_cover_counts_later_lanes_and_fallthrough() {
    use crate::mir6502::analysis::sites::MirSite;
    let source = "MODULE CLIENT USE SYS
PROC Calls()
SYS.Zero($FFFE,0) SYS.Zero($FFFE,3)
SYS.SetBlock($700,0,7) SYS.SetBlock($700,2,7)
SYS.MoveBlock($800,$900,257)
RETURN PROC Main() RETURN ENDMODULE";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program(source, runtime);
        let r = p.routines.iter().find(|r| r.name.starts_with("M_CLIENT_CALLS_")).unwrap();
        let values = buffers::argument_values(&p, r).unwrap();
        let fixed = crate::mir6502::analysis::known_callees::MirFixedStorage::new(&p);
        let mut writes = Vec::new();
        for b in &r.blocks { for (op_index, call) in b.ops.iter().enumerate() {
            let Some(contract) = library_invocation(&p, call) else { continue; };
            assert!(contract.definitions.is_empty());
            let MirOp::Call { abi, .. } = call else { panic!() };
            let binding = abi.library.as_ref().unwrap();
            if binding.kind == Some(MirLibraryRoutine::MoveBlock) {
                assert_eq!(contract.inputs.len(), 6);
                let mut bad = call.clone();
                let MirOp::Call { args, .. } = &mut bad else { unreachable!() };
                args.pop();
                assert!(library_invocation(&p, &bad).is_none());
            }
            if binding.kind == Some(MirLibraryRoutine::Zero) && runtime == crate::runtime::Runtime::Standalone {
                let implementation = binding.implementation.as_ref().unwrap();
                assert!(!implementation.fallthroughs.is_empty());
                let (_, dependency) = implementation.fallthroughs[0];
                let mut changed = p.clone();
                changed.routines.iter_mut().find(|r| r.id == dependency).unwrap().blocks[0].ops.clear();
                assert!(library_invocation(&changed, call).is_none());
            }
            let footprint = contract.normal_writes(&p, call, Some((&values, MirSite::Op { block: b.id, op_index })));
            assert!(!footprint.preserves_private_fixed_byte(MirFixedZpSlot(0xa1), &fixed));
            writes.push(footprint);
        }}
        assert_eq!(writes.len(), 5);
        assert!(matches!(writes[0], MirKnownMemoryWrites::Exact(_)));
        assert!(matches!(&writes[1], MirKnownMemoryWrites::Pointee { numeric, .. } if numeric == &[(0xfffe,3)]));
        assert!(!writes[1].preserves_private_fixed_byte(MirFixedZpSlot(0), &fixed));
        assert!(writes[1].preserves_private_fixed_byte(MirFixedZpSlot(1), &fixed));
        assert!(matches!(writes[2], MirKnownMemoryWrites::Exact(_)));
        assert!(matches!(&writes[3], MirKnownMemoryWrites::Pointee { numeric, .. } if numeric == &[(0x700,2)]));
        assert!(matches!(&writes[4], MirKnownMemoryWrites::Pointee { numeric, .. } if numeric == &[(0x800,257)]), "{:?}", writes[4]);
        for r in &p.routines {
            if r.name.contains("SetBlock") || r.name.contains("MoveBlock") {
                assert!(r.blocks.iter().flat_map(|b| &b.ops).all(|o| !matches!(o, MirOp::SArgsCapture { .. })));
            }
        }
    }
}

#[test]
fn prelude_identity_and_numeric_scratch_collision_remain_explicit() {
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = program("BYTE ARRAY text=\"abc\" PROC Main() SCopy($A0,text) RETURN", runtime);
        let r = p.routines.iter().find(|r| r.name == "Main").unwrap();
        let values = buffers::argument_values(&p, r).unwrap();
        let (block, op_index, call) = r.blocks.iter().flat_map(|b| b.ops.iter().enumerate().map(move |(i,o)|(b.id,i,o)))
            .find(|(_,_,o)| library_invocation(&p,o).is_some()).unwrap();
        let contract = library_invocation(&p,call).unwrap();
        assert_eq!(contract.normal_writes(&p,call,Some((&values,crate::mir6502::analysis::sites::MirSite::Op {block,op_index}))), MirKnownMemoryWrites::Unknown);
    }
}
