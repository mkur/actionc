use super::*;
use crate::mir6502::{self as mir, materialize::word_shift_supports_source_homes};

#[test]
fn word_shift_admission_uses_the_native_recipe_boundary() {
    for direction in ["LSH", "RSH"] {
        for count in ["0", "1", "4", "5", "7", "8", "9", "16", "keep"] {
            let n = super::tests::nir(&format!(
                "CARD FUNC Shift(CARD x,keep) x=x {direction} {count} RETURN(x+keep) PROC Main() RETURN"
            ));
            let p = mir::lower_program(&n).unwrap();
            let r = p.routines.iter().find(|r| r.name == "Shift").unwrap();
            let supported =
                matches!(count, "0" | "1" | "4" | "8" | "9" | "16");
            assert_eq!(
                check(&ObjectAddressControl::new(&p), r, false, true, None).is_ok(),
                supported,
                "{direction} {count}"
            );
            if supported {
                let proposal = super::super::propose(&p, true, 0x3000).unwrap();
                let r = proposal
                    .routines
                    .iter()
                    .find(|r| r.name == "Shift")
                    .unwrap();
                assert!(
                    r.frame
                        .resident_bytes()
                        .iter()
                        .any(|(m, _)| matches!(m, MirMem::Param { .. }))
                );
                mir::materialize_program(proposal, &Default::default()).unwrap();
            }
        }
    }
}

#[test]
fn native_word_shift_homes_require_independent_lanes_and_unused_carry() {
    for count in [1, 8] {
        let word = MirDef::VTemp(MirTempId(0));
        let value = MirValue::ConstU16(0x8001);
        let admits = |dst, left, carry_in, carry_out| {
            word_shift_supports_source_homes(&MirOp::Binary {
                op: MirBinaryOp::Rsh,
                dst,
                left,
                right: MirValue::ConstU8(count),
                width: MirWidth::Word,
                carry_in,
                carry_out,
            })
        };
        let ignore = MirCarryOut::Ignore;
        assert!(admits(word.clone(), value.clone(), None, ignore));
        assert!(!admits(
            word.clone(),
            value.clone(),
            Some(MirCarryIn::FromPrevious),
            ignore
        ));
        assert!(!admits(
            word.clone(),
            value.clone(),
            None,
            MirCarryOut::Produce
        ));
        assert!(!admits(
            word,
            MirValue::PointerCell(MirMem::Absolute(0x800)),
            None,
            ignore
        ));
        assert!(!admits(
            MirDef::VTempByte {
                id: MirTempId(0),
                byte: 1
            },
            value,
            None,
            ignore
        ));
    }
}
