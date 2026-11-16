use crate::mir6502::{self, ir::*, analysis::effects::classify_op};

fn program(origin: u16) -> MirProgram {
    let source = "BYTE out0 CARD out1 BYTE out2,out3
PROC Capture(BYTE a CARD b BYTE c,d)
 out0=a out1=b out2=c out3=d RETURN
PROC Main() Capture(1,$2345,6,7) RETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    mir6502::materialize_program_with_origin(mir6502::lower_program(&nir).unwrap(),
        &mir6502::Mir6502Config::default(), origin).unwrap()
}

#[test]
fn sargs_capture_preserves_bytes_relocations_and_frame_protection() {
    for origin in [0x3000, 0x41c7] {
        let p = program(origin);
        mir6502::verify_program(&p, MirPhase::PreEmission).unwrap();
        assert!(p.machine_blocks.is_empty());
        let r = p.routines.iter().find(|r| r.name == "Capture").unwrap();
        let [MirOp::SArgsCapture { byte_count: 5, effects, .. }, ..] = &r.blocks[0].ops[..]
            else { panic!("missing typed capture"); };
        let summary = classify_op(&r.blocks[0].ops[0]);
        assert_eq!(effects.memory_reads, MirMemoryEffect::Unknown);
        assert_eq!(effects.memory_writes, MirMemoryEffect::Unknown);
        for reg in [MirReg::A, MirReg::X, MirReg::Y] { assert!(summary.reads_reg(reg)); }
        assert!(!summary.removable_when_results_dead);
        let control = mir6502::analysis::storage_ownership::ObjectAddressControl::new(&p);
        for slot in &r.frame.params {
            let MirStorageBase::Param(id) = slot.base else { panic!() };
            assert!(!control.compiler_controls_address(r.id, &MirMem::Param { id, offset: 0 }));
        }

        // Reconstruct the former representation solely as an emission oracle.
        let mut old = p.clone();
        let r = old.routines.iter_mut().find(|r| r.name == "Capture").unwrap();
        let name = r.frame.params[0].name.clone().unwrap();
        let id = MirMachineBlockId(0);
        old.machine_blocks.push(MirMachineBlock { id, source_span: None, items: vec![
            MirMachineItem::AddressByte { high: false, name: name.clone() },
            MirMachineItem::AddressByte { high: true, name }, MirMachineItem::Byte(4),
        ] });
        r.blocks[0].ops.splice(0..1, [MirOp::RuntimeHelper {
            helper: MirRuntimeHelper::SArgs,
            args: super::helper_args(&MirRuntimeHelper::SArgs), result: None,
            additional_results: vec![], effects: effects.clone(),
        }, MirOp::MachineBlock { id, effects: MirEffects::default() }]);
        let emit = |p: &MirProgram| {
            let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
            mir6502::emit::emit_program(p, origin, &mut emitter).unwrap();
            emitter.finish_with_relocations().unwrap()
        };
        let (before, after) = (emit(&old), emit(&p));
        assert_eq!(before.bytes, after.bytes);
        assert_eq!(before.relocations, after.relocations);
    }
}

#[test]
fn sargs_capture_rejects_invalid_targets_counts_frames_and_effects() {
    let p = program(0x3000);
    for case in 0..6 {
        let mut invalid = p.clone();
        let r = invalid.routines.iter_mut().find(|r| r.name == "Capture").unwrap();
        let MirOp::SArgsCapture { first_param, byte_count, effects } = &mut r.blocks[0].ops[0]
            else { panic!() };
        match case {
            0 => *first_param = crate::nir::ParamId(999),
            1 => *byte_count = 0,
            2 => *byte_count = 257,
            3 => effects.memory_writes = MirMemoryEffect::None,
            4 => r.frame.params[1].offset = 1,
            _ => r.frame.params[1].storage_size = 1,
        }
        assert!(mir6502::verify_program(&invalid, MirPhase::PreEmission).unwrap_err().iter()
            .any(|d| d.message.contains("SArgs capture")), "case {case}");
    }
    assert!(mir6502::verify_program(&p, MirPhase::PreMaterialization).unwrap_err().iter()
        .any(|d| d.message.contains("SArgs capture")));
}
