//! Expose fixed-pair carry chains to the shared post-home value analyses.
use super::*;

/// Exactly the ordered instructions emitted for the compound operation.
/// Admission is intentionally independent of source names and allocation.
#[allow(dead_code)]
fn expansion(op: &MirOp) -> Option<Vec<MirOp>> {
    let MirOp::OffsetPointerByIndirectByte {
        op,
        dst: MirMem::FixedZeroPage(dst),
        source,
        offset,
    } = op
    else {
        return None;
    };
    let MirPointerPair::Fixed { lo } = source.pointer_pair() else {
        return None;
    };
    if source.preserves_index_in_y()
        || *offset > 255
        || lo.0 == 255
        || dst.0 == 255
        || (u16::from(dst.0) < u16::from(lo.0) + 2 && u16::from(lo.0) < u16::from(dst.0) + 2)
    {
        return None;
    }
    let carry = match op {
        MirBinaryOp::Add => MirCarryIn::Clear,
        MirBinaryOp::Sub => MirCarryIn::Set,
        _ => return None,
    };
    let load = |slot| MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
        width: MirWidth::Byte,
    };
    let store = |slot| MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    };
    Some(vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::Y),
            value: *offset,
            width: MirWidth::Byte,
        },
        load(lo.0),
        MirOp::BinaryDirectIndexedByte {
            op: *op,
            source: MirByteIndexedSource::FixedIndirectY { zp: lo },
            carry_in: Some(carry),
            carry_out: MirCarryOut::Produce,
        },
        store(dst.0),
        load(lo.0 + 1),
        MirOp::Binary {
            op: *op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(0),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::FromPrevious),
            carry_out: MirCarryOut::Ignore,
        },
        store(dst.0 + 1),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(op: MirBinaryOp, offset: u16) -> MirOp {
        MirOp::OffsetPointerByIndirectByte {
            op,
            dst: MirMem::FixedZeroPage(MirFixedZpSlot(0xAC)),
            source: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                lo: MirFixedZpSlot(0xAE),
            }),
            offset,
        }
    }

    fn program(ops: Vec<MirOp>) -> MirProgram {
        let mut p = super::super::tests::program(ops);
        p.routines[0].frame.fixed_zero_page = (0xAC..=0xAF).map(MirFixedZpSlot).collect();
        p
    }

    fn bytes(p: &MirProgram) -> Vec<u8> {
        super::super::super::verify_program(p, MirPhase::PreEmission).unwrap();
        let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
        emit::emit_program_with_reference_programs_traced(
            p,
            0x3000,
            &mut emitter,
            true,
            &[],
            &Trace::disabled(),
        )
        .unwrap();
        emitter.finish().unwrap()
    }

    #[test]
    fn expansion_preserves_emitted_bytes_and_carry_effects() {
        for op in [MirBinaryOp::Add, MirBinaryOp::Sub] {
            for offset in [0, 1, 255] {
                let compound = operation(op, offset);
                let expanded = expansion(&compound).unwrap();
                assert_eq!(
                    bytes(&program(vec![compound])),
                    bytes(&program(expanded.clone()))
                );
                let effects = classify_op(&expanded[2]);
                assert!(effects.machine.definitely_overwrites_carry);
                assert!(effects.machine.definitely_overwrites_overflow);
                assert!(effects.memory.indirect_reads);
                assert!(!effects.memory.indirect_writes);
                assert!(classify_op(&expanded[5]).machine.uses_previous_carry);
            }
        }
    }

    #[test]
    fn expansion_and_verifier_reject_invalid_carry_or_pointer_shapes() {
        assert!(expansion(&operation(MirBinaryOp::Mul, 0)).is_none());
        assert!(expansion(&operation(MirBinaryOp::Add, 256)).is_none());
        for destination in [0xAD, 0xAE, 0xAF, 0xFF] {
            let mut op = operation(MirBinaryOp::Add, 0);
            if let MirOp::OffsetPointerByIndirectByte { dst, .. } = &mut op {
                *dst = MirMem::FixedZeroPage(MirFixedZpSlot(destination));
            }
            assert!(expansion(&op).is_none());
        }
        for op in [MirBinaryOp::Add, MirBinaryOp::Sub] {
            for carry in [
                None,
                Some(MirCarryIn::FromPrevious),
                Some(if op == MirBinaryOp::Add {
                    MirCarryIn::Set
                } else {
                    MirCarryIn::Clear
                }),
            ] {
                let mut expanded = expansion(&operation(op, 0)).unwrap();
                if let MirOp::BinaryDirectIndexedByte { carry_in, .. } = &mut expanded[2] {
                    *carry_in = carry;
                }
                assert!(
                    super::super::super::verify_program(&program(expanded), MirPhase::PreEmission)
                        .is_err()
                );
            }
        }
    }
}
