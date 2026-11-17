use super::*;
use crate::mir6502::{self as mir, materialize::bounded_native_word_shift};

#[test]
fn word_shift_admission_uses_the_native_recipe_boundary() {
    for direction in ["LSH", "RSH"] {
        for count in ["0", "1", "4", "5", "keep"] {
            let n=super::tests::nir(&format!(
                "CARD FUNC Shift(CARD x,keep) x=x {direction} {count} RETURN(x+keep) PROC Main() RETURN"));
            let p=mir::lower_program(&n).unwrap();
            let r=p.routines.iter().find(|r|r.name=="Shift").unwrap();
            let supported=matches!(count,"0"|"1"|"4");
            assert_eq!(check(&ObjectAddressControl::new(&p),r,false,true,None).is_ok(),supported,
                "{direction} {count}");
            if supported {
                let proposal=super::super::propose(&p,true).unwrap();
                let r=proposal.routines.iter().find(|r|r.name=="Shift").unwrap();
                assert!(r.frame.resident_bytes().iter().any(|(m,_)|matches!(m,MirMem::Param{..})));
                mir::materialize_program(proposal,&Default::default()).unwrap();
            }
        }
    }
    let base=MirOp::Binary { op:MirBinaryOp::Rsh, dst:MirDef::VTemp(MirTempId(0)),
        left:MirValue::ConstU16(0x8001), right:MirValue::ConstU8(1), width:MirWidth::Word,
        carry_in:None, carry_out:MirCarryOut::Ignore };
    assert_eq!(bounded_native_word_shift(&base),Some(1));
    for kind in 0..3 {
        let mut op=base.clone();
        if let MirOp::Binary{carry_in,carry_out,left,..}=&mut op {
            match kind {
                0 => *carry_in=Some(MirCarryIn::FromPrevious),
                1 => *carry_out=MirCarryOut::Produce,
                _ => *left=MirValue::PointerCell(MirMem::Absolute(0x800)),
            }
        }
        assert_eq!(bounded_native_word_shift(&op),None);
    }
}
