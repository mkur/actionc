use super::*;
use crate::mir6502::{self as mir, trace::{Mir6502Trace, Trace}};

pub(super) fn nir(source: &str) -> crate::nir::NirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern()).unwrap();
    crate::nir::optimize_program(&crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model))).unwrap()
}

#[test]
fn admission_reports_structural_and_operation_reasons_without_changing_proposals() {
    let source = "PROC Touch() RETURN
PROC Empty() Touch() RETURN
CARD FUNC Wide(CARD a,b,c) Touch() RETURN(a)
CARD FUNC Shift(CARD a) Touch() RETURN(a RSH a)
CARD FUNC Indexed(CARD POINTER p CARD n) Touch() RETURN(p(n)+n)
CARD FUNC Hold(CARD keep) Touch() RETURN(keep+1)
PROC Main() RETURN";
    let n = nir(source);
    let p = mir::lower_program(&n).unwrap();
    let addresses = ObjectAddressControl::new(&p);
    for (name, reason) in [("Empty",Reason::NoObjects),("Wide",Reason::CaptureExtent),
        ("Shift",Reason::Operation),("Indexed",Reason::PointerArithmetic)] {
        let r = p.routines.iter().find(|r| r.name == name).unwrap();
        let rejection = check(&addresses,r,true,true,None).unwrap_err();
        assert_eq!(rejection.reason,reason,"{name}");
        assert_eq!(rejection.site.is_some(),matches!(reason,Reason::Operation|Reason::PointerArithmetic));
    }
    let r = p.routines.iter().find(|r| r.name == "Hold").unwrap();
    assert!(check(&addresses,r,true,true,None).is_ok());
    assert_eq!(check(&addresses,r,false,true,None).unwrap_err().reason,Reason::CallsDisabled);
    let selected = mir::materialize_program(p.clone(),&Default::default()).unwrap();
    let session = Mir6502Trace::default();
    let on = cross_call::propose_with_scope(&p,&selected,true,true,true,&session.context());
    let off = cross_call::propose_with_scope(&p,&selected,true,true,true,&Trace::disabled());
    assert_eq!(on,off);
    assert!(session.events().iter().any(|e| e.kind=="cross-call-ineligible"
        && e.data["routine"]=="Empty" && e.data["reason"]=="no candidate source objects"));
    assert!(session.events().iter().any(|e| e.kind=="cross-call-ineligible"
        && e.data["routine"]=="Indexed" && e.data["operation"].as_str().unwrap().contains("Binary")));
    let config = mir::Mir6502Config { enable_small_leaf_inlining:false,..mir::Mir6502Config::optimized() };
    let runtime = crate::runtime::Runtime::ActionCart;
    let (p,resume) = mir::prepare_program_for_emission(&n,0x3000,&config,runtime,&session.context()).unwrap();
    let base = mir::finalize_output(&n,p,0x3000,runtime,&session.context()).unwrap();
    let on = select_output(&n,base,resume,&config,0x3000,runtime,&session.context());
    let off = mir::generate_output_with_config_and_runtime(&n,0x3000,&config,runtime).unwrap();
    assert_eq!(on.output,off);
}
