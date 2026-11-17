use super::*;
use crate::mir6502::ir::*;

fn nir(source: &str) -> crate::nir::NirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern()).unwrap();
    crate::nir::optimize_program(&crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model))).unwrap()
}
fn lower(source: &str) -> MirProgram { crate::mir6502::lower_program(&nir(source)).unwrap() }
fn materialize(source: &str, runtime: crate::runtime::Runtime) -> MirProgram {
    crate::mir6502::materialize_program_with_origin_and_runtime(lower(source),
        &crate::mir6502::Mir6502Config { enable_small_leaf_inlining: false, ..crate::mir6502::Mir6502Config::optimized() },
        0x3000, runtime).unwrap()
}
fn routine<'a>(p: &'a MirProgram, name: &str) -> &'a MirRoutine {
    p.routines.iter().find(|r| r.name == name).unwrap()
}
fn call_to(p: &MirProgram, id: RoutineId) -> &MirOp {
    p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
        .find(|op| matches!(op, MirOp::Call { target: MirCallTarget::Routine(target), .. } if *target == id)).unwrap()
}

#[test]
fn wide_metadata_retains_one_parameter_and_independent_result_facts() {
    use MirAbiIntegerWidth::{Byte, Word, Long};
    use MirDirectParamAbi::{Scalar, Pointer};
    let p = lower("LONGCARD FUNC ReadBits(CARD n) RETURN(LONGCARD(n))
LONGINT FUNC Signed(LONGINT value BYTE count) RETURN(value)
BYTE FUNC Narrow(LONGCARD value BYTE POINTER p) RETURN(BYTE(value))
PROC Sink(LONGCARD value) RETURN
PROC Main() RETURN");
    for (name, params, result) in [
        ("ReadBits", vec![Scalar(Word)], MirDirectResultAbi::Scalar(Long)),
        ("Signed", vec![Scalar(Long), Scalar(Byte)], MirDirectResultAbi::Scalar(Long)),
        ("Narrow", vec![Scalar(Long), Pointer], MirDirectResultAbi::Scalar(Byte)),
        ("Sink", vec![Scalar(Long)], MirDirectResultAbi::Void),
    ] {
        let r = routine(&p, name);
        assert_eq!(r.frame.params.len(), params.len());
        assert_eq!(r.frame.direct_parameter_abi, Some(params));
        assert_eq!(r.direct_result_abi, Some(result));
        assert!(direct_parameter_frame_matches(r));
    }
    assert!(crate::mir6502::printer::format_routine(routine(&p,"ReadBits")).contains("result-abi long @ $A0/$A1/$A2/$A3"));
    let mut bad = p.clone();
    bad.routines.iter_mut().find(|r| r.name == "Signed").unwrap().frame.params[0].storage_size = 2;
    assert!(crate::mir6502::verify_program(&bad, MirPhase::PreMaterialization).is_err());
    let mut bad = p.clone();
    bad.routines.iter_mut().find(|r| r.name == "ReadBits").unwrap().direct_result_abi = Some(MirDirectResultAbi::Scalar(Word));
    assert!(crate::mir6502::verify_program(&bad, MirPhase::PreMaterialization).is_err());
}

#[test]
fn wide_call_projection_checks_whole_values_and_every_physical_byte() {
    let p = lower("LONGCARD FUNC Mix(BYTE tag LONGCARD value CARD tail) RETURN(value+LONGCARD(tail)+tag)
LONGCARD out PROC Main() out=Mix(3,$12345678,5) RETURN");
    let r = routine(&p,"Mix");
    let call = call_to(&p, r.id);
    let MirOp::Call { args, .. } = call else { unreachable!() };
    let lanes = canonical_call_lanes(r, args).unwrap();
    assert_eq!(lanes, vec![MirParamLane::Reg(MirReg::A),MirParamLane::Reg(MirReg::X),MirParamLane::Reg(MirReg::Y),
        MirParamLane::ZeroPage(MirFixedZpSlot(0xa3)),MirParamLane::ZeroPage(MirFixedZpSlot(0xa4)),
        MirParamLane::ZeroPage(MirFixedZpSlot(0xa5)),MirParamLane::ZeroPage(MirFixedZpSlot(0xa6))]);
    let params = r.frame.direct_parameter_abi.as_ref().unwrap();
    assert!(complete_parameter_prefix(params, &[MirWidth::Byte]));
    assert!(!complete_parameter_prefix(params, &[MirWidth::Byte, MirWidth::Word]));
    assert!(complete_parameter_prefix(params, &[MirWidth::Byte, MirWidth::Word, MirWidth::Word]));
    let mut missing = args.clone(); missing.remove(2);
    assert!(canonical_call_lanes(r, &missing).is_err());
    let mut duplicate = args.clone(); duplicate[2].home = duplicate[1].home.clone();
    assert!(canonical_call_lanes(r, &duplicate).is_err());
    assert_eq!(argument_lanes(96).unwrap().last(), Some(&MirParamLane::ZeroPage(MirFixedZpSlot(0xff))));
    assert!(argument_lanes(97).is_none());
}

#[test]
fn wide_result_layout_survives_expansion_and_rejects_missing_high_declarations() {
    let source = "LONGCARD FUNC Echo(LONGCARD value) RETURN(value)
CARD low LONGCARD full
PROC Main() full=Echo($12345678) low=CARD(Echo($87654321)) Echo(0) RETURN";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        for p in [lower(source), materialize(source, runtime)] {
            let r = routine(&p,"Echo");
            let calls: Vec<_> = p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
                .filter(|op| matches!(op, MirOp::Call { target: MirCallTarget::Routine(id), .. } if *id == r.id)).collect();
            assert!(!calls.is_empty());
            for call in calls {
                assert!(validate_direct_call_result(r, call).is_ok());
                for mutation in 0..4 {
                    let mut bad = call.clone();
                    let MirOp::Call { abi, .. } = &mut bad else { unreachable!() };
                    match mutation {
                        0 => abi.additional_results.clear(),
                        1 => abi.additional_results[0].home = MirResultHome::ReturnSlot { offset: 1 },
                        2 => abi.additional_results[0].width = MirWidth::Byte,
                        _ => abi.result = None,
                    }
                    assert!(validate_direct_call_result(r, &bad).is_err());
                }
            }
        }
    }
}

#[test]
fn wide_entries_compose_direct_and_sargs_captures_and_preservation() {
    use crate::mir6502::analysis::known_callees::{MirAcyclicEntryAnalysis, MirCalleeEntryStage};
    use crate::mir6502::parameter_homes::ResidenceCalls;
    use std::collections::BTreeSet;
    let source = "LONGCARD result=$900
LONGCARD FUNC Echo(LONGCARD value) RETURN(value)
LONGINT FUNC Shifted(BYTE tag LONGINT value) RETURN(value+LONGINT(tag))
PROC Middle() result=Echo($12345678) result=LONGCARD(Shifted(1,-2)) RETURN
BYTE FUNC Hold(BYTE keep) Middle() RETURN(keep)
PROC Main() RETURN";
    for runtime in [crate::runtime::Runtime::ActionCart, crate::runtime::Runtime::Standalone] {
        let p = materialize(source,runtime);
        let mut entries = MirAcyclicEntryAnalysis::new(&p);
        for (name, fixed) in [("Echo", BTreeSet::from([0xa3])),("Shifted",BTreeSet::from([0xa3,0xa4]))] {
            let r = routine(&p,name);
            let entry = entries.analyze(r.id).unwrap_or_else(|| panic!("{name}: {:?}", entries.failure(r.id)));
            assert_eq!(entry.inputs.fixed, fixed);
            assert_eq!(entry.defined_results, BTreeSet::from([0xa0,0xa1,0xa2,0xa3]));
            let mut stale = p.clone();
            stale.routines.iter_mut().find(|r| r.name == name).unwrap().direct_result_abi = Some(MirDirectResultAbi::Scalar(MirAbiIntegerWidth::Word));
            assert!(!entry.is_current(&stale, MirCalleeEntryStage::ActualEntry));
        }
        assert!(routine(&p,"Shifted").blocks[0].ops.iter().any(|o| matches!(o,MirOp::SArgsCapture {byte_count:5,..})));
        let middle = routine(&p,"Middle");
        assert!(entries.analyze(middle.id).unwrap().inputs.fixed.is_empty());
        let mut calls = ResidenceCalls::with_acyclic(&p,MirCalleeEntryStage::ActualEntry);
        let writes = calls.writes(call_to(&p,middle.id)).unwrap();
        let fixed = calls.fixed_storage();
        assert!(writes.preserves_fixed_byte(MirFixedZpSlot(0xe4),&fixed));
        for byte in [0xa2,0xa3,0xa4] { assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(byte),&fixed)); }
    }
}

#[test]
fn wide_high_result_bytes_remain_live_and_observable_after_capture_expansion() {
    use crate::mir6502::analysis::{cfg::MirCfg, effects::MirHomeByte,
        home_liveness::{MirHomeLiveness, call_return_home_uses}, sites::MirSite};
    let p = materialize("LONGCARD FUNC Echo(LONGCARD value) RETURN(value) LONGCARD out PROC Main() out=Echo($12345678) RETURN", crate::runtime::Runtime::ActionCart);
    let r = routine(&p, "Echo");
    let live = MirHomeLiveness::analyze(r, &MirCfg::from_routine(r).unwrap());
    let boundary = call_return_home_uses(&p, call_to(&p,r.id));
    for byte in [0xa0,0xa1,0xa2,0xa3] {
        let home = MirHomeByte::FixedZeroPage(MirFixedZpSlot(byte));
        assert!(boundary.contains(home));
        for b in r.blocks.iter().filter(|b| b.terminator == MirTerminator::Return) {
            assert!(live.live_after(home, MirSite::Terminator {block:b.id}).unwrap());
        }
    }
}
