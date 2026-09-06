use super::*;

#[path = "../../codegen/test_cpu.rs"]
mod cpu;

fn pair_program(signed: bool, reverse: bool) -> MirProgram {
    let mut routine = txa_direct_store_routine(Vec::new(), MirTerminator::Return, Vec::new());
    routine.temps = (0..4).map(|id| MirTemp { id: MirTempId(id) }).collect();
    let value = |id| MirValue::Def(MirDef::VTemp(MirTempId(id)));
    let binary = |op, id| MirOp::Binary {
        op,
        dst: MirDef::VTemp(MirTempId(id)),
        left: value(0),
        right: value(1),
        width: MirWidth::Word,
        carry_in: None,
        carry_out: MirCarryOut::Ignore,
    };
    let div = binary(
        if signed {
            MirBinaryOp::Div
        } else {
            MirBinaryOp::UDiv
        },
        2,
    );
    let rem = binary(
        if signed {
            MirBinaryOp::Mod
        } else {
            MirBinaryOp::UMod
        },
        3,
    );
    routine.blocks[0].ops = vec![
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(0)),
            src: MirAddr::Direct(MirMem::Absolute(0x6E0)),
            width: MirWidth::Word,
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(1)),
            src: MirAddr::Direct(MirMem::Absolute(0x6E2)),
            width: MirWidth::Word,
        },
        if reverse { rem.clone() } else { div.clone() },
        if reverse { div } else { rem },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x600)),
            src: value(2),
            width: MirWidth::Word,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x602)),
            src: value(3),
            width: MirWidth::Word,
        },
    ];
    let mut program = empty_test_program();
    program.routines = vec![routine];
    program
}

#[test]
fn captured_divmod_pairs_materialize_emit_and_execute_both_result_homes() {
    for signed in [false, true] {
        for reverse in [false, true] {
            for runtime in [
                crate::runtime::Runtime::ActionCart,
                crate::runtime::Runtime::Standalone,
            ] {
                let program = crate::mir6502::materialize_program_with_origin_and_runtime(
                    pair_program(signed, reverse),
                    &Mir6502Config::default(),
                    0x3000,
                    runtime,
                )
                .unwrap();
                let helper = if signed {
                    MirRuntimeHelper::DivMod
                } else {
                    MirRuntimeHelper::UDivMod
                };
                assert!(program.runtime_helpers.iter().any(|h| h.helper == helper));
                crate::mir6502::verify::verify_program(
                    &program,
                    crate::mir6502::MirPhase::PreEmission,
                )
                .unwrap();
                let mut emitter =
                    crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
                let summary =
                    crate::mir6502::emit::emit_program(&program, 0x3000, &mut emitter).unwrap();
                let emission = emitter.finish_with_relocations().unwrap();
                let entry = summary
                    .routine_addresses
                    .iter()
                    .find(|r| r.name == "txa_direct_store")
                    .unwrap()
                    .address;
                for (a, b) in [
                    (65529u16, 3u16),
                    (7, 65533),
                    (65529, 65533),
                    (32768, 65535),
                    (50000, 40000),
                    (65535, 2),
                    (513, 256),
                ] {
                    let mut memory = [0xCCu8; 65536];
                    memory[0x3000..0x3000 + emission.bytes.len()].copy_from_slice(&emission.bytes);
                    memory[0x6E0..0x6E2].copy_from_slice(&a.to_le_bytes());
                    memory[0x6E2..0x6E4].copy_from_slice(&b.to_le_bytes());
                    cpu::run_memory(&mut memory, usize::from(entry));
                    let (a, b) = if signed {
                        (i64::from(a as i16), i64::from(b as i16))
                    } else {
                        (i64::from(a), i64::from(b))
                    };
                    let q = a.abs() / b.abs() * if (a < 0) != (b < 0) { -1 } else { 1 };
                    assert_eq!(u16::from_le_bytes([memory[0x600], memory[0x601]]), q as u16);
                    assert_eq!(
                        u16::from_le_bytes([memory[0x602], memory[0x603]]),
                        (a - q * b) as u16
                    );
                    assert_eq!(memory[0x604], 0xCC);
                }
            }
        }
    }
}

#[test]
fn divmod_fusion_rejects_effects_rereads_mismatched_inputs_and_carry() {
    for reject in 0..5 {
        let mut program = pair_program(false, false);
        let layout = MaterializeLayout::new(&program, 0x3000);
        let routine = &mut program.routines[0];
        match reject {
            0 => {
                routine.blocks[0].ops.insert(
                    3,
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x600)),
                        src: MirValue::ConstU8(1),
                        width: MirWidth::Byte,
                    },
                );
            }
            1 => {
                if let MirOp::Binary { right, .. } = &mut routine.blocks[0].ops[3] {
                    *right = MirValue::ConstU16(3);
                }
            }
            2 => {
                for op in &mut routine.blocks[0].ops[2..4] {
                    if let MirOp::Binary { left, .. } = op {
                        *left = MirValue::PointerCell(MirMem::Absolute(0x6E0));
                    }
                }
            }
            3 => {
                if let MirOp::Binary { op, .. } = &mut routine.blocks[0].ops[3] {
                    *op = MirBinaryOp::Mod;
                }
            }
            _ => {
                if let MirOp::Binary { carry_in, .. } = &mut routine.blocks[0].ops[3] {
                    *carry_in = Some(MirCarryIn::FromPrevious);
                }
            }
        }
        let before = routine.clone();
        assert_eq!(
            runtime::fuse_adjacent_divmod(
                routine,
                &Mir6502Config::default(),
                &layout,
                &mut Vec::new()
            ),
            0
        );
        assert_eq!(*routine, before);
    }
}

#[test]
fn narrow_selection_requires_unsigned_proof_and_preserves_signatures() {
    let widths = BTreeMap::new();
    let wide = MirValue::Def(MirDef::VTemp(MirTempId(0)));
    let byte = MirValue::Word {
        lo: Box::new(MirValue::Def(MirDef::VTempByte {
            id: MirTempId(1),
            byte: 0,
        })),
        hi: Box::new(MirValue::ConstU8(0)),
    };
    for (op, left, right, expected) in [
        (
            MirBinaryOp::Div,
            wide.clone(),
            byte.clone(),
            MirRuntimeHelper::Div,
        ),
        (
            MirBinaryOp::UDiv,
            wide.clone(),
            wide.clone(),
            MirRuntimeHelper::UDiv,
        ),
        (
            MirBinaryOp::UDiv,
            wide.clone(),
            MirValue::ConstU16(256),
            MirRuntimeHelper::UDiv,
        ),
        (
            MirBinaryOp::UDiv,
            wide.clone(),
            byte.clone(),
            MirRuntimeHelper::DivU16U8,
        ),
        (
            MirBinaryOp::UMod,
            wide.clone(),
            byte.clone(),
            MirRuntimeHelper::ModU16U8,
        ),
        (
            MirBinaryOp::UDiv,
            byte.clone(),
            byte,
            MirRuntimeHelper::DivU8,
        ),
    ] {
        let selection = runtime::helper_for_typed_binary(
            op,
            MirWidth::Word,
            &left,
            &right,
            &widths,
            false,
            true,
            &[],
        )
        .unwrap();
        assert_eq!(selection.helper, expected);
        assert!(runtime::helper_implements_binary(
            selection.helper,
            op,
            MirWidth::Word,
            &left,
            &right,
            &widths
        ));
    }
}

#[test]
fn private_arithmetic_signatures_reject_malformed_homes_widths_and_effects() {
    let program = crate::mir6502::materialize_program_with_origin_and_runtime(
        pair_program(true, false),
        &Mir6502Config::default(),
        0x3000,
        crate::runtime::Runtime::ActionCart,
    )
    .unwrap();
    for fault in 0..5 {
        let mut invalid = program.clone();
        let declaration = invalid
            .runtime_helpers
            .iter_mut()
            .find(|helper| helper.helper == MirRuntimeHelper::DivMod)
            .unwrap();
        match fault {
            0 => declaration.abi.params.clear(),
            1 => declaration.additional_results[0].width = MirWidth::Byte,
            2 => declaration.effects = MirEffects::default(),
            _ => {
                let call = invalid
                    .routines
                    .iter_mut()
                    .flat_map(|r| &mut r.blocks)
                    .flat_map(|b| &mut b.ops)
                    .find(|op| {
                        matches!(
                            op,
                            MirOp::RuntimeHelper {
                                helper: MirRuntimeHelper::DivMod,
                                ..
                            }
                        )
                    })
                    .unwrap();
                if let MirOp::RuntimeHelper {
                    args,
                    additional_results,
                    ..
                } = call
                {
                    if fault == 3 {
                        args.clear();
                    } else {
                        additional_results.clear();
                    }
                }
            }
        }
        let diagnostics =
            crate::mir6502::verify::verify_program(&invalid, crate::mir6502::MirPhase::PreEmission)
                .expect_err("malformed private helper contract must be rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("runtime helper"))
        );
    }
}
