//! Normal-return writes through native pointers. Only residence can use the
//! distinction between an ordinary pointee and the physical bytes of a caller.
use super::*;
use crate::mir6502::ir::{MirAddr, MirAddressConsumer, MirPointerPair, MirWidth};

/// A selected indirect instruction either sets Y from its operand or uses the
/// live byte in Y. Keep the latter's full 256 possible offsets when necessary.
fn destination(op: &MirOp) -> Option<(MirPointerPair, u16, u16, bool)> {
    let (consumer, offset, width) = match op {
        MirOp::StoreIndirect {
            consumer, offset, ..
        } => (*consumer, *offset, 1),
        MirOp::CopyIndirectWord {
            destination,
            destination_offset,
            ..
        }
        | MirOp::CopyDirectWordToIndirect {
            destination,
            destination_offset,
            ..
        } => (*destination, *destination_offset, 2),
        MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY { zp },
            width,
            ..
        } => {
            return Some((
                MirPointerPair::Fixed { lo: *zp },
                0,
                crate::mir6502::abi::action_arg_width_bytes(*width),
                true,
            ));
        }
        MirOp::Store {
            dst: MirAddr::IndirectIndexedY { zp },
            width,
            ..
        } => {
            return Some((
                MirPointerPair::Virtual(*zp),
                0,
                crate::mir6502::abi::action_arg_width_bytes(*width),
                true,
            ));
        }
        _ => return None,
    };
    match consumer {
        MirAddressConsumer::IndirectIndexedY(pair) => Some((pair, offset, width, false)),
        MirAddressConsumer::LiveIndirectIndexedY(pair) if offset == 0 => {
            Some((pair, offset, width, true))
        }
        _ => None,
    }
}

fn fixed_pair(routine: &MirRoutine, pair: MirPointerPair) -> Option<u8> {
    match pair {
        MirPointerPair::Fixed { lo } => Some(lo.0),
        MirPointerPair::Virtual(slot) => routine
            .frame
            .zero_page_allocations
            .iter()
            .find(|a| a.slot == slot && a.size >= 2)
            .map(|a| a.start.0),
    }
}

/// A reviewed producer initialized both bytes of a source-owned pointer.
/// Intervening calls must preserve the pair. A scratch pair of unknown origin
/// cannot borrow the source-pointer exception.
fn reviewed_pointer(
    program: &MirProgram,
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    site: MirSite,
    lo: u8,
) -> bool {
    let Some(hi) = lo.checked_add(1) else {
        return false;
    };
    if !program.globals.iter().any(|g| {
        g.storage_size == 2
            && known.fixed_storage.address(&MirMem::Global {
                id: g.id,
                offset: 0,
            }) == Some(u16::from(lo))
    }) {
        return false;
    }
    let MirSite::Op { block, op_index } = site else {
        return false;
    };
    let Some(block) = routine.blocks.iter().find(|b| b.id == block) else {
        return false;
    };
    for op in block.ops[..op_index].iter().rev() {
        if let MirOp::Call { target, .. } = op {
            let Some(summary) = known.for_target(target) else {
                return false;
            };
            if summary.reviewed_facts().is_some_and(|f| {
                f.defines_on_return.contains(&lo) && f.defines_on_return.contains(&hi)
            }) {
                return true;
            }
            if !summary
                .writes()
                .preserves_fixed_pair(MirFixedZpSlot(lo), &known.fixed_storage)
            {
                return false;
            }
        } else {
            let mut writes = MirKnownMemoryWrites::exact();
            summarize_op_writes(op, routine, known, &mut writes);
            if !writes.preserves_fixed_pair(MirFixedZpSlot(lo), &known.fixed_storage) {
                return false;
            }
        }
    }
    false
}

pub(super) fn store(
    program: &MirProgram,
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    values: &MirMachineValueAvailability,
    site: MirSite,
    op: &MirOp,
) -> Option<MirKnownMemoryWrites> {
    let (pair, mut offset, mut bytes, indexed) = destination(op)?;
    let lo = fixed_pair(routine, pair)?;
    let hi = lo.wrapping_add(1);
    if indexed {
        if let Some(MirMachineValue::ConstU8(y)) = values.register_at(site, MirReg::Y).ok()? {
            offset = offset.wrapping_add(u16::from(y));
        } else {
            bytes = bytes.checked_add(255)?;
        }
    }
    let byte = |slot| {
        values
            .fixed_zero_page_value_at(site, MirFixedZpSlot(slot))
            .ok()
            .flatten()
    };
    let numeric = match (byte(lo), byte(hi)) {
        (Some(MirMachineValue::ConstU8(l)), Some(MirMachineValue::ConstU8(h))) => {
            let start = u16::from_le_bytes([l, h]).wrapping_add(offset);
            if bytes > 1
                && [lo, hi]
                    .iter()
                    .any(|s| u16::from(*s).wrapping_sub(start) < bytes)
            {
                return None;
            }
            vec![(start, bytes)]
        }
        _ if reviewed_pointer(program, routine, known, site, lo) => Vec::new(),
        _ => return None,
    };
    let effects = classify_op(op);
    if effects.memory.opaque || !effects.logical.temp_defs.is_empty() {
        return None;
    }
    let mut writes = MirKnownMemoryWrites::Pointee {
        fixed: Vec::new(),
        numeric,
        ordinary: true,
        caller_dependent: false,
    };
    record_physical_writes(routine, effects, &mut writes);
    Some(writes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{MirDef, MirValue};

    fn materialized(source: &str) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        crate::mir6502::materialize_program(
            crate::mir6502::lower_program(&nir).unwrap(),
            &Default::default(),
        )
        .unwrap()
    }

    #[test]
    fn pointer_writes_keep_general_memory_unknown_and_physical_stores_visible() {
        let mut p = materialized("BYTE v PROC Draw() v=1 RETURN PROC Main() Draw() RETURN");
        let pair = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(0xac),
        });
        let cell = |slot| MirMem::FixedZeroPage(MirFixedZpSlot(slot));
        let store = |slot, n| MirOp::Store {
            dst: MirAddr::Direct(cell(slot)),
            src: MirValue::ConstU8(n),
            width: MirWidth::Byte,
        };
        for op in [
            MirOp::StoreIndirect {
                consumer: pair,
                offset: 1,
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
            },
            MirOp::CopyDirectWordToIndirect {
                source: cell(0xa0),
                destination: pair,
                destination_offset: 1,
            },
        ] {
            let bytes = if matches!(op, MirOp::StoreIndirect { .. }) {
                1
            } else {
                2
            };
            p.routines[0].blocks[0].ops = vec![
                store(0xac, 0xff),
                store(0xad, 0xff),
                store(0xe4, 9),
                op,
                store(0xe4, 0),
            ];
            let known = MirKnownCalleeSummaries::analyze_exit_facts(&p, false, Some(&p), true);
            let writes = known.get(p.routines[0].id).unwrap().writes();
            assert!(
                matches!(writes, MirKnownMemoryWrites::Pointee { numeric, .. } if numeric == &[(0, bytes)]),
                "{writes:?}"
            );
            assert!(writes.may_write_mem(&cell(0xed), &known.fixed_storage));
            assert!(
                !writes.preserves_private_fixed_byte(MirFixedZpSlot(0xe4), &known.fixed_storage)
            );
            assert!(
                writes.preserves_private_fixed_byte(MirFixedZpSlot(0xed), &known.fixed_storage)
            );
        }
    }

    #[test]
    fn tn_screen_store_uses_program_pointer_and_keeps_reviewed_scratch() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/tn/modern/TN.ACT");
        let ast = crate::includes::load_program_with_includes(path).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let p = crate::mir6502::materialize_program(
            crate::mir6502::lower_program(&nir).unwrap(),
            &Default::default(),
        )
        .unwrap();
        let putchar = p.routines.iter().find(|r| r.name == "Putchar").unwrap();
        let known = MirKnownCalleeSummaries::analyze_exit_facts(&p, false, Some(&p), true);
        let writes = known.get(putchar.id).unwrap().writes();
        assert!(
            matches!(writes, MirKnownMemoryWrites::Pointee { ordinary: true, .. }),
            "{writes:?}"
        );
        for slot in [0xa0, 0xe6, 0xe7, 0xff] {
            assert!(
                !writes.preserves_private_fixed_byte(MirFixedZpSlot(slot), &known.fixed_storage)
            );
        }
        assert!(writes.preserves_private_fixed_byte(MirFixedZpSlot(0xa8), &known.fixed_storage));
        assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(0xa8), &known.fixed_storage));
    }
}
