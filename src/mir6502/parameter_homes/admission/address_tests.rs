use super::*;
use crate::mir6502::{self as mir, trace::Trace};

#[test]
fn symbolic_addresses_admit_private_siblings_but_keep_exposure_and_numeric_exclusions() {
    for (table, static_address) in [("BYTE ARRAY table=[5 9 17 33]", false),
        ("BYTE ARRAY table=$800", false), ("BYTE ARRAY table=[5 9 17 33]", true)] {
        let text=format!("CARD reserve=$A0,reserve2=$A2 BYTE guard=$E0 {table}
BYTE FUNC Fetch(BYTE POINTER p) RETURN(p(1))
CARD FUNC Hold(CARD keep) BYTE b b=Fetch(table) RETURN(keep+b)
CARD FUNC Literal(CARD keep) BYTE b b=Fetch(\"Hi\") RETURN(keep+b)
PROC Expose(CARD keep) CARD address=$710 address=@keep Fetch(table) RETURN
PROC Main() RETURN");
        let n=super::tests::nir(&text);
        let mut input=mir::lower_program(&n).unwrap();
        // Source string arguments lower directly to StaticAddr. Exercise the
        // equivalent structured LeaAddr explicitly, using the real static ID.
        if static_address {
            let id=input.statics[0].id;
            let op=input.routines.iter_mut().find(|r|r.name=="Hold").unwrap()
                .blocks.iter_mut().flat_map(|b|&mut b.ops)
                .find(|op|matches!(op,MirOp::LeaAddr{target:MirMem::Global{..},..})).unwrap();
            if let MirOp::LeaAddr{target,..}=op { *target=MirMem::Static{id,offset:0}; }
        }
        let selected=mir::materialize_program(input.clone(),&Default::default()).unwrap();
        let addresses=ObjectAddressControl::new(&input);
        for name in ["Hold","Literal"] {
            let r=input.routines.iter().find(|r|r.name==name).unwrap();
            assert!(check(&addresses,r,true,true,Some(&selected)).is_ok(),"{name}");
        }
        let p=cross_call::propose_with_scope(&input,&selected,true,true,true,&Trace::disabled()).unwrap();
        for name in ["Hold","Literal"] {
            let r=p.routines.iter().find(|r|r.name==name).unwrap();
            assert!(r.frame.resident_bytes().iter().any(|(m,_)|matches!(m,MirMem::Param{..})),"{name}");
            assert!(r.frame.resident_bytes().iter().all(|(_,s)|s.0!=0xe0));
        }
        let exposed=p.routines.iter().find(|r|r.name=="Expose").unwrap();
        assert!(exposed.frame.resident_bytes().iter().all(|(m,_)|!matches!(m,MirMem::Param{..})));
        mir::materialize_program(p,&Default::default()).unwrap();
    }
}
