use super::*;
use crate::mir6502::{self as mir, trace::{Mir6502Trace, Trace}};

const SOURCE: &str = "LONGCARD input=$700,result=$900
CARD FUNC Hold(CARD keep BYTE count) result=input RSH count RETURN(keep+1)
PROC Main() RETURN";
fn programs(source: &str) -> (MirProgram,MirProgram) {
    let p=mir::lower_program(&super::tests::nir(source)).unwrap();
    let selected=mir::materialize_program(p.clone(),&Default::default()).unwrap();
    (p,selected)
}
fn hold(p: &MirProgram) -> &MirRoutine { p.routines.iter().find(|r|r.name=="Hold").unwrap() }

#[test]
fn helper_admission_requires_current_contracts_and_counts_each_invocation() {
    let (input,selected)=programs(SOURCE);
    let addresses=ObjectAddressControl::new(&input);
    assert!(check(&addresses,hold(&input),true,true,Some(&selected)).is_ok());
    assert_eq!(check(&addresses,hold(&input),true,true,None).unwrap_err().reason,Reason::HelperContract);
    let proposed=cross_call::propose_with_scope(&input,&selected,false,true,true,&Trace::disabled()).unwrap();
    assert!(hold(&proposed).frame.parameter_homes.as_ref().unwrap().iter()
        .any(|b| b.param==crate::nir::ParamId(0) && matches!(b.home,MirParamHome::ZeroPage(_))));
    assert!(cross_call::propose_with_scope(&input,&selected,true,false,true,&Trace::disabled()).is_none());
    let mut stale=selected.clone();
    stale.runtime_helpers.iter_mut().find(|h|h.helper==MirRuntimeHelper::Rsh32).unwrap()
        .target=MirRuntimeHelperTarget::KnownAbsolute(0x1234);
    assert_eq!(check(&addresses,hold(&input),true,true,Some(&stale)).unwrap_err().reason,Reason::HelperContract);
    let mut malformed=input.clone();
    let op=malformed.routines.iter_mut().find(|r|r.name=="Hold").unwrap().blocks.iter_mut()
        .flat_map(|b|&mut b.ops).find(|op|matches!(op,MirOp::RuntimeHelper{..})).unwrap();
    if let MirOp::RuntimeHelper{args,..}=op { args.pop(); }
    assert_eq!(check(&ObjectAddressControl::new(&malformed),hold(&malformed),true,true,Some(&selected)).unwrap_err().reason,Reason::HelperContract);
    let (input,selected)=programs(&SOURCE.replace("RETURN(keep+1)","result=input LSH count RETURN(keep+1)"));
    assert!(cross_call::propose_with_scope(&input,&selected,false,true,true,&Trace::disabled()).is_none());
    assert!(cross_call::propose_with_scope(&input,&selected,true,true,true,&Trace::disabled()).is_some());
}

#[test]
fn helper_result_collisions_reject_both_word_lanes_and_restore_selected_artifact() {
    let n=super::tests::nir(SOURCE);
    let config=mir::Mir6502Config{enable_small_leaf_inlining:false,..mir::Mir6502Config::optimized()};
    let runtime=crate::runtime::Runtime::ActionCart;
    let session=Mir6502Trace::default(); let trace=session.context();
    let (p,resume)=mir::prepare_program_for_emission(&n,0x3000,&config,runtime,&trace).unwrap();
    let resume=resume.unwrap();
    let executable=mir::finalize_output(&n,p,0x3000,runtime,&trace).unwrap();
    let selected=SelectedPlacement{input:Arc::new(resume.core.checkpoint.input.program.clone()),executable};
    for start in [0xc3,0xc7] {
        let mut proposal=cross_call::propose_with_scope(&selected.input,&selected.executable.program,true,true,true,&trace).unwrap();
        let r=proposal.routines.iter_mut().find(|r|r.name=="Hold").unwrap();
        r.frame.parameter_homes.as_mut().unwrap().iter_mut().find(|b|b.param==crate::nir::ParamId(0)).unwrap()
            .home=MirParamHome::ZeroPage(MirFixedZpSlot(start));
        for b in [start,start+1] { if !r.frame.fixed_zero_page.contains(&MirFixedZpSlot(b)) {r.frame.fixed_zero_page.push(MirFixedZpSlot(b));} }
        let expected=selected.executable.clone();
        let baseline=SelectedPlacement{input:Arc::clone(&selected.input),executable:expected.clone()};
        let result=try_proposal(&n,baseline,&resume,proposal,"cross-call-homes",&config,0x3000,runtime,&trace);
        assert!(Arc::ptr_eq(&result.input,&selected.input));
        assert_eq!(result.executable.program,expected.program);
        assert_eq!(result.executable.output,expected.output);
        assert_eq!(result.executable.memory,expected.memory);
        assert_eq!(result.executable.costs,expected.costs);
        assert_eq!(result.executable.trace_snapshot,expected.trace_snapshot);
    }
    assert_eq!(session.manifest()["work"]["cross_call_home_trials"],2);
    assert_eq!(session.events().iter().filter(|e|e.kind=="cross-call-home-result"&&e.data["reason"]=="invalid-trial").count(),2);
}
