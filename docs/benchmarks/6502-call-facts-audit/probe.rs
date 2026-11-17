// Audit only: included temporarily inside analysis::known_callees.
use super::*;
use crate::mir6502 as mir;
use serde_json::{json, Value};
use mir::ir::*;

fn entry(f: Option<&MirCalleeEntryFact>) -> Value {
    f.map_or(Value::Null, |f| json!({"registers":format!("{:?}",f.inputs.registers),
        "flags":format!("{:?}",f.inputs.flags),"fixed":f.inputs.fixed,
        "defined_results":f.defined_results,
        "current":f.is_current(&CURRENT.with(|p|p.borrow().clone().unwrap()), MirCalleeEntryStage::ActualEntry)}))
}
thread_local! { static CURRENT: std::cell::RefCell<Option<MirProgram>> = const {std::cell::RefCell::new(None)}; }

fn selected(n: &crate::nir::NirProgram, config: &mir::Mir6502Config) -> mir::preparation::Executable {
    let t=mir::trace::Trace::disabled();
    selected_traced(n,config,&t)
}
fn selected_traced(n: &crate::nir::NirProgram, config: &mir::Mir6502Config, t: &mir::trace::Trace) -> mir::preparation::Executable {
    let (p,resume)=mir::prepare_program_for_emission(n,0x3000,config,crate::runtime::Runtime::ActionCart,&t).unwrap();
    let base=mir::finalize_output(n,p,0x3000,crate::runtime::Runtime::ActionCart,&t).unwrap();
    mir::parameter_homes::select_output(n,base,resume,config,0x3000,crate::runtime::Runtime::ActionCart,&t)
}
fn nir(source: &str) -> crate::nir::NirProgram {
    let ast=crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model=crate::semantic::analyze_with_options(&ast,crate::semantic::SemanticOptions::modern()).unwrap();
    crate::nir::optimize_program(&crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast,&model))).unwrap()
}
fn inventory(p: &MirProgram, output: Option<&crate::codegen::CodegenOutput>) -> Value {
    CURRENT.with(|s|*s.borrow_mut()=Some(p.clone()));
    let known=MirKnownCalleeSummaries::analyze_for_rewriting(p);
    let mut acyclic=MirAcyclicEntryAnalysis::new(p);
    let mut residence=mir::parameter_homes::ResidenceCalls::with_acyclic(p,MirCalleeEntryStage::ActualEntry);
    let mut leaf_residence=mir::parameter_homes::ResidenceCalls::new(p,MirCalleeEntryStage::ActualEntry);
    let routines:Vec<_>=p.routines.iter().map(|r| {
        let k=known.get(r.id).unwrap();
        let f=acyclic.analyze(r.id);
        let cfg=mir::analysis::cfg::MirCfg::from_routine(r).unwrap();
        let leaf=MirCalleeEntryFact::analyze(p,r.id,MirCalleeEntryStage::ActualEntry);
        let range=output.and_then(|o|o.map.routine_ranges.iter().find(|rr|rr.name==r.name));
        let assembly=output.zip(range).map(|(o,rr)|crate::codegen::format_listing_with_origin(
            &o.bytes[usize::from(rr.start-o.origin)..usize::from(rr.end-o.origin)],rr.start));
        let calls:Vec<_>=r.blocks.iter().flat_map(|b|b.ops.iter().enumerate()).filter_map(|(index,o)| {
            let MirOp::Call {target,..}=o else{return None};
            Some(json!({"op":index,"target":format!("{target:?}"),
                "acyclic_inputs":format!("{:?}",residence.inputs(o)),
                "acyclic_writes":format!("{:?}",residence.writes(o)),
                "leaf_inputs":format!("{:?}",leaf_residence.inputs(o)),
                "leaf_writes":format!("{:?}",leaf_residence.writes(o))}))
        }).collect();
        let comparisons:Vec<_>=r.blocks.iter().flat_map(|b|b.ops.iter()).filter(|o|matches!(o,MirOp::Compare {..}))
            .map(|o|json!({"op":format!("{o:?}"),"effects":format!("{:?}",classify_op(o).machine)})).collect();
        json!({"id":r.id.0,"name":r.name,"abi":format!("{:?}",r.abi),
            "scalar_signature":format!("{:?}",r.scalar_signature),"direct_parameter_abi":format!("{:?}",r.frame.direct_parameter_abi),
            "direct_result_abi":format!("{:?}",r.direct_result_abi),
            "declared_effects":format!("{:?}",r.effects),"leaf_entry":entry(leaf.as_ref()),"acyclic_entry":entry(f.as_deref()),
            "acyclic_failure":format!("{:?}",acyclic.failure(r.id)),
            "legacy_ignores_flags":known.scalar_entry_ignores_flags(r.id),
            "precise_entry_flags":format!("{:?}",mir::analysis::entry_flags::MirKnownCalleeEntryFlags::analyze(r)),
            "machine_entry":format!("{:?}",mir::analysis::machine_liveness::MirMachineLiveness::entry_inputs(r,&cfg)),
            "reads":format!("{:?}",k.reads()),"writes":format!("{:?}",k.writes()),
            "preserves_e4":k.writes().preserves_fixed_byte(MirFixedZpSlot(0xe4), known.fixed_storage()),
            "preserves_a0":k.writes().preserves_fixed_byte(MirFixedZpSlot(0xa0), known.fixed_storage()),
            "preserves_e8":k.writes().preserves_fixed_byte(MirFixedZpSlot(0xe8), known.fixed_storage()),
            "preserves_e9":k.writes().preserves_fixed_byte(MirFixedZpSlot(0xe9), known.fixed_storage()),
            "preserves_ea":k.writes().preserves_fixed_byte(MirFixedZpSlot(0xea), known.fixed_storage()),
            "storage":output.map(|o|o.map.storage_symbols.iter().filter(|s|
                s.scope==crate::codegen::CodegenSymbolScope::Routine(r.name.clone()))
                .map(|s|json!({"name":s.name,"kind":format!("{:?}",s.kind),"address":s.address,"size":s.size})).collect::<Vec<_>>()),
            "accumulator":format!("{:?}",k.accumulator()),"zn":format!("{:?}",k.zn()),
            "calls":calls,"comparisons":comparisons,"assembly":assembly,
            "mir":mir::printer::format_program_filtered(p,|id|id==r.id)})
    }).collect();
    CURRENT.with(|s|*s.borrow_mut()=None);
    json!({"routines":routines,"globals":format!("{:#?}",p.globals),
        "packaged_bytes":output.map(|o|crate::codegen::format_load_file(o).len())})
}
fn simple_routine(id:u32,ops:Vec<MirOp>) -> MirRoutine {
    MirRoutine {id:RoutineId(id),name:format!("r{id}"),abi:MirRoutineAbi::Action,inline:Default::default(),
        direct_result_abi: None,
        scalar_signature:Some(MirScalarSignature {params:vec![],result:vec![]}),
        frame:MirFrame {direct_parameter_abi:Some(vec![]),..Default::default()},temps:vec![],effects:MirEffects::default(),
        blocks:vec![MirBlock {id:MirBlockId(0),label:"b0".into(),params:vec![],ops,terminator:MirTerminator::Return}]}
}
fn synthetic() -> Value {
    let mut result=vec![];
    for kind in [MirMemoryRegionKind::Global(crate::nir::SymbolId(0)),MirMemoryRegionKind::Static(crate::nir::SymbolId(0)),
        MirMemoryRegionKind::Local(crate::nir::LocalId(0)),MirMemoryRegionKind::Param(crate::nir::ParamId(0)),
        MirMemoryRegionKind::AbsoluteRange,MirMemoryRegionKind::ZeroPage] {
        let effect=MirMemoryEffect::Regions(vec![MirMemoryRegion {kind:kind.clone(),offset:3,size:2}]);
        let mut reads=MirKnownMemoryReads::exact();let mut writes=MirKnownMemoryWrites::exact();
        summarize_structured_reads(&effect,false,&mut reads);summarize_structured_writes(&effect,false,&mut writes);
        let expected=match kind {
            MirMemoryRegionKind::Global(id)=>MirMem::Global {id,offset:3},
            MirMemoryRegionKind::Static(id)=>MirMem::Static {id,offset:3},
            MirMemoryRegionKind::Local(id)=>MirMem::Local {id,offset:3},
            MirMemoryRegionKind::Param(id)=>MirMem::Param {id,offset:3},
            MirMemoryRegionKind::AbsoluteRange=>MirMem::Absolute(3),
            MirMemoryRegionKind::ZeroPage=>MirMem::FixedZeroPage(MirFixedZpSlot(3)),_=>unreachable!()};
        result.push(json!({"kind":format!("{kind:?}"),"offset":3,"size":2,
            "reads":format!("{reads:?}"),"writes":format!("{writes:?}"),"expected_byte":format!("{expected:?}"),
            "may_write_expected_byte":writes.may_write_mem(&expected, &MirFixedStorage::default())}));
    }
    // A write through a pointer reads its carrier but not the pointee.
    // Write uncertainty must remain independent of those finite reads.
    let store=MirOp::StoreIndirect { consumer:MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {lo:MirFixedZpSlot(0xac)}),offset:0,
        src:MirValue::Def(MirDef::Reg(MirReg::A)) };
    let routine=simple_routine(0,vec![store]);
    let mut reads=MirKnownMemoryReads::exact();summarize_op_reads(&routine.blocks[0].ops[0],&routine,&MirKnownCalleeSummaries::default(),&mut reads);
    assert_eq!(reads,MirKnownMemoryReads::Exact(vec![absolute_mem(0xac),absolute_mem(0xad)]));
    let mut writes=MirKnownMemoryWrites::exact();
    summarize_op_writes(&routine.blocks[0].ops[0],&routine,&MirKnownCalleeSummaries::default(),&mut writes);
    assert_eq!(writes,MirKnownMemoryWrites::Unknown);
    let zp=MirMem::FixedZeroPage(MirFixedZpSlot(0xe4));
    let home=mir::analysis::effects::MirHomeByte::FixedZeroPage(MirFixedZpSlot(0xe4));
    let store_site=mir::analysis::sites::MirSite::Op {block:MirBlockId(0),op_index:0};
    let mut may_writes=vec![];
    for effect in [MirMemoryEffect::Unknown,MirMemoryEffect::Regions(vec![MirMemoryRegion {
        kind:MirMemoryRegionKind::ZeroPage,offset:0xe4,size:1}])] {
        let call=MirOp::Call {target:MirCallTarget::Routine(RoutineId(1)),
            abi:MirCallAbi {params:vec![],result:None,additional_results:vec![],clobbers:Default::default(),preserves:Default::default()},
            args:vec![],result:None,additional_results:vec![],effects:MirEffects {memory_writes:effect.clone(),..Default::default()}};
        let routine=simple_routine(0,vec![
            MirOp::Store {dst:MirAddr::Direct(zp.clone()),src:MirValue::Def(MirDef::Reg(MirReg::A)),width:MirWidth::Byte},call,
            MirOp::Load {dst:MirDef::Reg(MirReg::A),src:MirAddr::Direct(zp.clone()),width:MirWidth::Byte}]);
        let cfg=mir::analysis::cfg::MirCfg::from_routine(&routine).unwrap();
        let live=mir::analysis::home_liveness::MirHomeLiveness::analyze(&routine,&cfg);
        let defs=mir::analysis::home_definitions::MirHomeDefinitions::analyze(&routine,&cfg);
        may_writes.push(json!({"effect":format!("{effect:?}"),"live_after_store":format!("{:?}",live.live_after(home,store_site)),
            "definition_dead_after_store":format!("{:?}",defs.definition_dead_after(home,store_site,store_site)),
            "call_effects":format!("{:?}",classify_op(&routine.blocks[0].ops[1]))}));
    }
    let source="BYTE FUNC Fetch(CARD p) BYTE POINTER q q=BYTE POINTER(p) RETURN(q^) BYTE FUNC Middle(CARD p) RETURN(Fetch(p))";
    let e=selected(&nir(source),&mir::Mir6502Config {enable_small_leaf_inlining:false,..mir::Mir6502Config::optimized()});
    let mut p=e.program;
    let target=p.routines.iter().find(|r|r.name=="Middle").unwrap().id;
    let middle=p.routines.iter_mut().find(|r|r.id==target).unwrap();
    middle.blocks[0].ops.push(MirOp::Load {dst:MirDef::Reg(MirReg::A),
        src:MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xa0))),width:MirWidth::Byte});
    CURRENT.with(|s|*s.borrow_mut()=Some(p.clone()));
    let with_signature=entry(MirAcyclicEntryAnalysis::new(&p).analyze(target).as_deref());
    let fetch=p.routines.iter_mut().find(|r|r.name=="Fetch").unwrap();
    fetch.scalar_signature=None;
    fetch.frame.direct_parameter_abi=Some(vec![MirDirectParamAbi::Pointer]);
    CURRENT.with(|s|*s.borrow_mut()=Some(p.clone()));
    let without_signature=entry(MirAcyclicEntryAnalysis::new(&p).analyze(target).as_deref());
    assert_eq!(with_signature["fixed"],without_signature["fixed"]);
    p.routines.iter_mut().find(|r|r.name=="Fetch").unwrap().direct_result_abi=None;
    CURRENT.with(|s|*s.borrow_mut()=Some(p.clone()));
    let without_result=entry(MirAcyclicEntryAnalysis::new(&p).analyze(target).as_deref());
    assert!(without_result["fixed"].as_array().unwrap().contains(&json!(0xa0)));
    CURRENT.with(|s|*s.borrow_mut()=None);
    json!({"may_write_liveness":may_writes,"typed_result_composition":{"integer_parameter":with_signature,"pointer_parameter":without_signature,"unknown_result":without_result},
        "structured_regions":result,"store_only_reads":format!("{reads:?}"),"store_only_writes":format!("{writes:?}"),
        "store_only_effects":format!("{:?}",classify_op(&routine.blocks[0].ops[0]).memory)})
}

#[test]
#[ignore = "diagnostic audit, temporary test-only include"]
fn audit_call_facts() {
    let root=std::path::Path::new("build/call-facts-audit-20261116");
    let mut cases=vec![];
    for (name,source) in [
        ("global_zp","BYTE state=$E8 BYTE FUNC ReadState() RETURN(state) BYTE FUNC Hold(BYTE keep) ReadState() RETURN(keep+1)"),
        ("global_ram","BYTE state=$600 BYTE FUNC ReadState() RETURN(state) BYTE FUNC Hold(BYTE keep) ReadState() RETURN(keep+1)"),
        ("global_alias_zp","BYTE state=$E8 BYTE alias=state BYTE FUNC ReadState() RETURN(alias) BYTE FUNC Hold(BYTE keep) ReadState() RETURN(keep+1)"),
        ("compare_byte","BYTE FUNC Check(BYTE x) IF x<40 THEN RETURN(1) FI RETURN(0)"),
        ("compare_word","BYTE FUNC Check(CARD x) IF x<$2800 THEN RETURN(1) FI RETURN(0)"),
        ("integer_result","BYTE FUNC Fetch(CARD p) BYTE POINTER q q=BYTE POINTER(p) RETURN(q^) BYTE FUNC Middle(CARD p) RETURN(Fetch(p))"),
        ("pointer_result","BYTE FUNC Fetch(BYTE POINTER p) RETURN(p^) BYTE FUNC Middle(CARD p) RETURN(Fetch(BYTE POINTER(p)))"),
        ("global_write_ram","BYTE state=$600 BYTE FUNC Touch() state=1 RETURN(0) BYTE FUNC Hold(BYTE keep) Touch() RETURN(keep+1)"),
        ("global_write_zp","BYTE state=$E8 BYTE FUNC Touch() state=1 RETURN(0) BYTE FUNC Hold(BYTE keep) Touch() RETURN(keep+1)"),
        ("global_write_alias_zp","CARD state=$E8 CARD alias=state BYTE FUNC Touch() alias=$1234 RETURN(0) BYTE FUNC Hold(BYTE keep) Touch() RETURN(keep+1)"),
        ("store_only","PROC WriteByte(BYTE POINTER p BYTE value) p^=value RETURN PROC Forward(BYTE POINTER p BYTE value) WriteByte(p,value) RETURN"),
        ("read_modify_write","PROC AddByte(BYTE POINTER p BYTE value) p^=p^+value RETURN"),
        ("asm_leaf",";@actionc clobbers $AC/$AD $A0\nBYTE FUNC Fetch=*(CARD p)\nASM\n sta $AC\n stx $AD\n ldy #0\n lda ($AC),y\n sta $A0\n rts\nENDASM\nBYTE FUNC Middle(CARD p) RETURN(Fetch(p))"),
    ] {
        let n=nir(source);
        let config=mir::Mir6502Config {enable_small_leaf_inlining:false,..mir::Mir6502Config::optimized()};
        let e=selected(&n,&config);
        let facts=inventory(&e.program,Some(&e.output));
        if matches!(name, "compare_byte" | "compare_word") {
            let check=facts["routines"].as_array().unwrap().iter().find(|r|r["name"]=="Check").unwrap();
            assert_eq!(check["acyclic_entry"]["flags"], json!(format!("{:?}",mir::analysis::effects::MirFlagSet::default())));
            assert_eq!(check["legacy_ignores_flags"],json!(true));
        }
        if name.starts_with("global_write_") {
            let touch=facts["routines"].as_array().unwrap().iter().find(|r|r["name"]=="Touch").unwrap();
            assert_eq!(touch["preserves_e4"],json!(true));
            assert_eq!(touch["preserves_a0"],json!(false));
            assert_eq!(touch["preserves_e8"],json!(name=="global_write_ram"));
            assert_eq!(touch["preserves_e9"],json!(name!="global_write_alias_zp"));
            assert_eq!(touch["preserves_ea"],json!(true));
        }
        if matches!(name, "store_only" | "read_modify_write") {
            let known=MirKnownCalleeSummaries::analyze_for_rewriting(&e.program);
            for r in &e.program.routines {
                let summary=known.get(r.id).unwrap();
                assert_eq!(matches!(summary.reads(),MirKnownMemoryReads::Exact(_)), name=="store_only", "{}: {:?}",r.name,summary.reads());
                assert_eq!(summary.writes(),&MirKnownMemoryWrites::Unknown);
            }
        }
        std::fs::write(root.join(format!("{name}.act")),source).unwrap();
        std::fs::write(root.join(format!("{name}.xex")),crate::codegen::format_load_file(&e.output)).unwrap();
        cases.push(json!({"case":name,"source":source,"facts":facts}));
        println!("audited {name}");
    }
    std::fs::write(root.join("controls.json"),serde_json::to_string_pretty(&cases).unwrap()).unwrap();
    std::fs::write(root.join("synthetic.json"),serde_json::to_string_pretty(&synthetic()).unwrap()).unwrap();
    let loaded=crate::includes::load_program_with_expanded_source("samples/tn/modern/TN.ACT").unwrap();
    let model=crate::semantic::analyze_with_options(&loaded.program,crate::semantic::SemanticOptions::modern()).unwrap();
    let n=crate::nir::optimize_program(&crate::nir::lower_program(&crate::semantic::ir::lower_program(&loaded.program,&model))).unwrap();
    let started=std::time::Instant::now();
    let e=selected(&n,&mir::Mir6502Config::optimized());
    let compile_seconds=started.elapsed().as_secs_f64();
    let image=crate::codegen::format_load_file(&e.output);
    let facts=inventory(&e.program,Some(&e.output));
    // Keep evidence even when an assertion identifies a changed fact.
    std::fs::write(root.join("tn.json"),serde_json::to_string_pretty(&facts).unwrap()).unwrap();
    std::fs::write(root.join("tn.xex"),&image).unwrap();
    if std::env::var_os("ACTIONC_CALL_FACT_PROFILE").is_some() {
        let counts=std::rc::Rc::new(std::cell::RefCell::new(std::collections::BTreeMap::new()));
        let t=mir::trace::Trace::with_counts(counts.clone());
        let started=std::time::Instant::now();
        let counted=selected_traced(&n,&mir::Mir6502Config::optimized(),&t);
        let counted_seconds=started.elapsed().as_secs_f64();
        assert_eq!(image,crate::codegen::format_load_file(&counted.output));
        std::fs::write(root.join("tn-profile.json"),serde_json::to_string_pretty(&json!({
            "compile_seconds":compile_seconds,"counted_compile_seconds":counted_seconds,
            "work":&*counts.borrow(),"counting_preserves_image":true,
            "scope":"MIR preparation, parameter-home selection and output; frontend excluded; debug opt-level=1"
        })).unwrap()).unwrap();
    }
    if std::env::var_os("ACTIONC_CALL_FACT_DETAILS").is_some() {
        let session=mir::trace::Mir6502Trace::new(mir::trace::TraceOptions {
            routines:vec!["__facts_only__".into()],max_bytes:64*1024*1024,..Default::default()
        });
        let detailed=selected_traced(&n,&mir::Mir6502Config::optimized(),&session.context());
        assert_eq!(image,crate::codegen::format_load_file(&detailed.output));
        let events:Vec<_>=session.events().into_iter().filter(|e|e.kind.starts_with("core-callee-")).collect();
        std::fs::write(root.join("tn-contract-events.json"),serde_json::to_string_pretty(&json!({
            "events":events,"manifest":session.manifest(),"tracing_preserves_image":true
        })).unwrap()).unwrap();
    }
    let routine=|name|facts["routines"].as_array().unwrap().iter().find(|r|r["name"]==name).unwrap();
    assert_eq!(routine("Next")["leaf_entry"]["defined_results"],json!([0xa0,0xa1]));
    assert_eq!(routine("Next")["acyclic_entry"]["defined_results"],json!([0xa0,0xa1]));
    assert_eq!(routine("Ord")["acyclic_entry"]["fixed"],json!([0xa3]));
    assert_eq!(routine("Items")["acyclic_entry"]["fixed"],json!([]));
    for name in ["ArenaCursor", "Alloc"] {
        assert_eq!(routine(name)["acyclic_entry"]["flags"],json!(format!("{:?}",mir::analysis::effects::MirFlagSet::default())));
        assert_eq!(routine(name)["acyclic_entry"]["fixed"],json!([]));
        assert_eq!(routine(name)["acyclic_entry"]["defined_results"],json!([0xa0,0xa1]));
    }
    println!("audited all {} selected TN routines",e.program.routines.len());
}
