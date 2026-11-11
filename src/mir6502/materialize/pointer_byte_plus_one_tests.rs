use super::*;
use crate::mir6502::analysis::{
    effects::{MirFlagSet, classify_op},
    pointer_byte_plus_one::{self as facts, tests::baseline},
};

fn target() -> MirOp {
    MirOp::OffsetPointerByIndirectBytePlusOne {
        dst: MirMem::FixedZeroPage(MirFixedZpSlot(0xAC)),
        source: INDEX_POINTER_PAIR,
        offset: 0,
    }
}
fn program_with_target() -> MirProgram {
    let mut p = baseline();
    let r = p.routines.iter_mut().find(|r| r.name == "Forward").unwrap();
    let b = &mut r.blocks[0];
    let i = b
        .ops
        .iter()
        .position(|o| matches!(o, MirOp::OffsetPointerByIndirectByte { .. }))
        .unwrap();
    b.ops.splice(i..i + 2, [target()]);
    p
}

#[test]
fn pointer_byte_plus_one_selected_form_verifies_prints_and_declares_complete_effects() {
    let p = program_with_target();
    crate::mir6502::verify_program(&p, MirPhase::PreEmission).unwrap();
    assert!(
        crate::mir6502::format_program(&p)
            .contains("offset_pointer_by_indirect_byte_plus_one.w fixed_zp $AC (zp$AE),y +0")
    );
    assert!(
        format!(
            "{:?}",
            crate::mir6502::verify_program(&p, MirPhase::PreMaterialization).unwrap_err()
        )
        .contains("pointer byte plus one requires post-home")
    );
    let effects = classify_op(&target());
    assert!(effects.memory.indirect_reads && effects.memory.has_unknown_effects);
    assert!(effects.homes.unknown_reads);
    assert!(!effects.memory.indirect_writes && !effects.memory.may_write_any);
    assert_eq!(effects.memory.definite_writes.len(), 2);
    assert!(effects.may_clobber_reg_compat(MirReg::A));
    assert!(effects.may_clobber_reg_compat(MirReg::X));
    assert!(effects.may_clobber_reg_compat(MirReg::Y));
    assert_eq!(effects.machine.flag_clobbers, MirFlagSet::all());
    for bad in [
        MirOp::OffsetPointerByIndirectBytePlusOne {
            dst: MirMem::FixedZeroPage(MirFixedZpSlot(0xAD)),
            source: INDEX_POINTER_PAIR,
            offset: 0,
        },
        MirOp::OffsetPointerByIndirectBytePlusOne {
            dst: MirMem::FixedZeroPage(MirFixedZpSlot(0xFF)),
            source: INDEX_POINTER_PAIR,
            offset: 0,
        },
        MirOp::OffsetPointerByIndirectBytePlusOne {
            dst: MirMem::Absolute(0x680),
            source: INDEX_POINTER_PAIR,
            offset: 0,
        },
        MirOp::OffsetPointerByIndirectBytePlusOne {
            dst: MirMem::FixedZeroPage(MirFixedZpSlot(0xAC)),
            source: INDEX_POINTER_PAIR,
            offset: 256,
        },
    ] {
        let mut p = p.clone();
        let r = p.routines.iter_mut().find(|r| r.name == "Forward").unwrap();
        let o = r.blocks[0]
            .ops
            .iter_mut()
            .find(|o| matches!(o, MirOp::OffsetPointerByIndirectBytePlusOne { .. }))
            .unwrap();
        *o = bad;
        assert!(
            format!(
                "{:?}",
                crate::mir6502::verify_program(&p, MirPhase::PreEmission).unwrap_err()
            )
            .contains("pointer byte plus one requires post-home")
        );
    }
}

#[test]
fn pointer_byte_plus_one_machine_values_discard_the_previous_x_carrier() {
    use crate::mir6502::analysis::{
        posthome::PostHomeAnalysisSnapshot,
        sites::{MirRoutineGeneration, MirSite},
    };
    use crate::mir6502::rewrite::context::PostHomeRewriteContext;
    let mut p = program_with_target();
    let r = p.routines.iter_mut().find(|r| r.name == "Forward").unwrap();
    let i = r.blocks[0]
        .ops
        .iter()
        .position(|o| matches!(o, MirOp::OffsetPointerByIndirectBytePlusOne { .. }))
        .unwrap();
    r.blocks[0].ops.insert(
        i,
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 77,
            width: MirWidth::Byte,
        },
    );
    let snap = PostHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
    let ctx = PostHomeRewriteContext::new(&snap);
    assert!(
        ctx.register_value_at(
            MirReg::X,
            ctx.point(MirSite::Op {
                block: r.blocks[0].id,
                op_index: i + 1
            })
        )
        .is_proven()
    );
    assert!(
        !ctx.register_value_at(
            MirReg::X,
            ctx.point(MirSite::Op {
                block: r.blocks[0].id,
                op_index: i + 2
            })
        )
        .is_proven()
    );
}

fn fragment(fused: bool, offset: u16, origin: u16) -> (Vec<u8>, usize) {
    let mut p = baseline();
    let r = p.routines.iter_mut().find(|r| r.name == "Forward").unwrap();
    let i = r.blocks[0]
        .ops
        .iter()
        .position(|o| matches!(o, MirOp::OffsetPointerByIndirectByte { .. }))
        .unwrap();
    let mut ops = if fused {
        vec![MirOp::OffsetPointerByIndirectBytePlusOne {
            dst: MirMem::FixedZeroPage(MirFixedZpSlot(0xAC)),
            source: INDEX_POINTER_PAIR,
            offset,
        }]
    } else {
        let mut ops = r.blocks[0].ops[i..i + 2].to_vec();
        if let MirOp::OffsetPointerByIndirectByte { offset: k, .. } = &mut ops[0] {
            *k = offset;
        }
        ops
    };
    for byte in [0u8, 1] {
        ops.push(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAC + byte))),
            width: MirWidth::Byte,
        });
        ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x800 + u16::from(byte))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        });
    }
    r.blocks[0].ops = ops;
    r.blocks[0].terminator = MirTerminator::Return;
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = crate::mir6502::emit::emit_program(&p, origin, &mut emitter).unwrap();
    let output = crate::mir6502::codegen_output(
        emitter.finish_with_relocations().unwrap(),
        origin,
        summary,
        crate::compiler::Runtime::ActionCart,
        &p,
    );
    let entry = output
        .routine_addresses
        .iter()
        .find(|r| r.name == "Forward")
        .unwrap()
        .address as usize;
    (output.bytes, entry)
}

#[test]
fn pointer_byte_plus_one_emission_exhausts_low_bytes_and_preserves_entry_alias_reads() {
    for fused in [false, true] {
        let (bytes, entry) = fragment(fused, 0, 0x3000);
        if fused {
            let at = entry - 0x3000;
            assert_eq!(
                &bytes[at..at + 21],
                &[
                    0xA0, 0, 0xA5, 0xAE, 0x18, 0x71, 0xAE, 0xAA, 0xA5, 0xAF, 0x69, 0, 0xE8, 0x86,
                    0xAC, 0x85, 0xAD, 0xD0, 2, 0xE6, 0xAD
                ]
            );
        }
        let mut memory = [0u8; 65536];
        memory[0x3000..0x3000 + bytes.len()].copy_from_slice(&bytes);
        for low in 0..=255u16 {
            for b in 0..=255u16 {
                let p = 0x7000 + low;
                memory[0xAE..=0xAF].copy_from_slice(&p.to_le_bytes());
                memory[p as usize] = b as u8;
                memory[0xAC..=0xAD].fill(0xD6);
                memory[0x802] = 0xA5;
                leaf_test_cpu::run_memory(&mut memory, entry);
                assert_eq!(
                    u16::from_le_bytes([memory[0x800], memory[0x801]]),
                    p.wrapping_add(b).wrapping_add(1)
                );
                assert_eq!(&memory[0xAE..=0xAF], &p.to_le_bytes());
                assert_eq!(memory[p as usize], b as u8);
                assert_eq!(memory[0x802], 0xA5);
            }
        }
        for offset in [0, 1, 255] {
            let (bytes, entry) = fragment(fused, offset, 0x41C7);
            for p in [
                0u16, 1, 0xFF, 0x100, 0x7FFF, 0x8000, 0xFF00, 0xFFFF, 0xA0, 0xA3, 0xAC, 0xAD, 0xAE,
                0xAF,
            ] {
                for b in [0u8, 1, 127, 128, 254, 255] {
                    let mut memory = [0xD6; 65536];
                    memory[0x41C7..0x41C7 + bytes.len()].copy_from_slice(&bytes);
                    let address = p.wrapping_add(offset) as usize;
                    memory[address] = b;
                    memory[0xAE..=0xAF].copy_from_slice(&p.to_le_bytes());
                    // Aliasing a carrier constrains the byte read; preserve the
                    // resulting entry state rather than inventing two values.
                    let actual = memory[address];
                    let expected = p.wrapping_add(u16::from(actual)).wrapping_add(1);
                    leaf_test_cpu::run_memory(&mut memory, entry);
                    assert_eq!(
                        u16::from_le_bytes([memory[0x800], memory[0x801]]),
                        expected,
                        "{p:04X}/{offset}/{b}/{fused}"
                    );
                    assert_eq!(&memory[0xAE..=0xAF], &p.to_le_bytes());
                }
            }
        }
    }
    assert!(facts::valid_pairs(
        MirFixedZpSlot(0xAC),
        INDEX_POINTER_PAIR,
        255
    ));
    // Place the repair branch's fallthrough and target on opposite pages.
    // Exercise both its taken path and the increment-wrap repair path.
    let (_, entry) = fragment(true, 0, 0x3000);
    let entry_offset = entry - 0x3000;
    for desired_entry in [0x41EBusize, 0x41EC, 0x41ED] {
        let origin = (desired_entry - entry_offset) as u16;
        for fused in [false, true] {
            let (bytes, entry) = fragment(fused, 0, origin);
            assert_eq!(entry, desired_entry);
            for p in [0x7000u16, 0x70FF, 0xFFFF] {
                for b in [0u8, 1, 127, 128, 254, 255] {
                    let mut memory = [0xD6; 65536];
                    let at = usize::from(origin);
                    memory[at..at + bytes.len()].copy_from_slice(&bytes);
                    memory[usize::from(p)] = b;
                    memory[0xAE..=0xAF].copy_from_slice(&p.to_le_bytes());
                    leaf_test_cpu::run_memory(&mut memory, entry);
                    assert_eq!(
                        u16::from_le_bytes([memory[0x800], memory[0x801]]),
                        p.wrapping_add(u16::from(b)).wrapping_add(1),
                        "{entry:04X}/{p:04X}/{b}/{fused}"
                    );
                    assert_eq!(&memory[0xAE..=0xAF], &p.to_le_bytes());
                }
            }
        }
    }
}
