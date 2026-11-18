//! Identity of a reviewed direct-call entry. The review is an assertion about
//! caller obligations. It does not modify the instruction-level input summary.
use super::*;
use crate::asm6502::InlineAsmRelocationKind;
use crate::mir6502::ir::{MirDirectParamAbi, MirDirectResultAbi, MirInlineAsmTarget};

pub(in crate::mir6502) fn abi_inputs_review_fingerprint(
    program: &MirProgram,
    routine: &MirRoutine,
) -> Option<u64> {
    body_fingerprint(program, routine, crate::callee6502::ENTRY_VERSION)
}

pub(in crate::mir6502) fn reviewed_contract<'a>(
    program: &MirProgram,
    routine: &'a MirRoutine,
) -> Option<&'a crate::callee6502::CallFacts<crate::nir::SymbolId>> {
    let review = routine.frame.callee_review.as_ref()?;
    let params = routine.frame.direct_parameter_abi.as_ref()?;
    if !review.valid_buffer_signature(&params.iter().map(|p| p.width().bytes()).collect::<Vec<_>>()) {
        return None;
    }
    let [block] = routine.blocks.as_slice() else {
        return None;
    };
    let [MirOp::MachineBlock { id, .. }] = block.ops.as_slice() else {
        return None;
    };
    let machine = program.machine_blocks.iter().find(|m| m.id == *id)?;
    if body_fingerprint(program, routine, review.version()) != Some(review.fingerprint)
        || !review.dependencies_match(|id| {
            if !machine.items.iter().any(|item| matches!(item, MirMachineItem::Relocation {
                target: MirInlineAsmTarget::Memory(MirMem::Global { id: seen, offset: 0 }), .. } if seen == id)) { return None; }
            Some(program.globals.iter().find(|g| g.id == *id)?.storage_size)
        }) { return None; }
    Some(&review.facts)
}

pub(in crate::mir6502) fn body_fingerprint(
    program: &MirProgram,
    routine: &MirRoutine,
    version: &[u8],
) -> Option<u64> {
    let [block] = routine.blocks.as_slice() else {
        return None;
    };
    let [MirOp::MachineBlock { id, effects }] = block.ops.as_slice() else {
        return None;
    };
    if block.terminator != MirTerminator::Unreachable
        || effects.opaque
        || effects.may_call_os
        || routine.effects.opaque
        || routine.effects.may_call_os
        || !crate::mir6502::abi::direct_parameter_frame_matches(routine)
    {
        return None;
    }
    let machine = program.machine_blocks.iter().find(|m| m.id == *id)?;
    use crate::callee6502::{BodyItem, fingerprint};
    let params = routine
        .frame
        .direct_parameter_abi
        .as_ref()?
        .iter()
        .map(|abi| match abi {
            MirDirectParamAbi::Scalar(w) => w.bytes() as u8,
            MirDirectParamAbi::Pointer => b'P',
        })
        .collect::<Vec<_>>();
    let result = match routine.direct_result_abi? {
        MirDirectResultAbi::Void => 0,
        MirDirectResultAbi::Scalar(w) => w.bytes() as u8,
        MirDirectResultAbi::Pointer => b'P',
    };
    let mut items = Vec::with_capacity(machine.items.len());
    for item in &machine.items {
        match item {
            MirMachineItem::Byte(b) => items.push(BodyItem::Byte(*b)),
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Low8 | InlineAsmRelocationKind::Byte8,
                target: MirInlineAsmTarget::Absolute(address),
                addend: 0,
                requires_zero_page: true,
                ..
            } => items.push(BodyItem::Byte(u8::try_from(*address).ok()?)),
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Absolute16,
                target: MirInlineAsmTarget::Absolute(address),
                addend: 0,
                requires_zero_page: false,
                ..
            } => {
                for b in address.to_le_bytes() {
                    items.push(BodyItem::Byte(b));
                }
            }
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Absolute16,
                target: MirInlineAsmTarget::Memory(MirMem::Global { id, offset: 0 }),
                addend: 0,
                requires_zero_page: false,
                ..
            } => {
                let global = program.globals.iter().find(|g| g.id == *id)?;
                use crate::mir6502::ir::MirGlobalInit;
                let array = match global.init.as_ref()? {
                    MirGlobalInit::Bytes { array: Some(a), .. }
                    | MirGlobalInit::ZeroFill { array: Some(a), .. } => a,
                    _ => return None,
                };
                if array.pointer_backed
                    || array.address_initializer.is_some()
                    || !matches!(global.backing, MirGlobalBacking::Ordinary { .. })
                {
                    return None;
                }
                items.push(BodyItem::Global {
                    name: &global.name,
                    extent: global.storage_size,
                });
            }
            _ => return None,
        }
    }
    fingerprint(&params, result, &items, version)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = "BYTE ARRAY tab(4)=[1 2 3 4]\n;@actionc reviewed-6502 0000000000000000\n;@actionc abi-inputs-only\n;@actionc reads-only tab[0..3]\n;@actionc writes-only none\nBYTE FUNC Leaf=*(BYTE ch)\nASM\ntax\nlda tab,x\nrts\nENDASM\nPROC Main() RETURN";
    fn lower(source: &str) -> Result<crate::nir::NirProgram, String> {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap())
            .map_err(|e| format!("{e:?}"))?;
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .map_err(|e| format!("{e:?}"))?;
        Ok(crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
    }
    #[test]
    fn reviewed_contract_tracks_objects_and_independent_directions() {
        let nir = lower(SOURCE).unwrap();
        crate::nir::verify_program(&nir).unwrap();
        let mir = crate::mir6502::lower_program(&nir).unwrap();
        let hash =
            body_fingerprint(&mir, &mir.routines[0], crate::callee6502::CONTRACT_VERSION).unwrap();
        let src = SOURCE.replace("0000000000000000", &format!("{hash:016x}"));
        for source in [src.clone(), src.replace('\n', "\r\n")] {
            let mut p = crate::mir6502::lower_program(&lower(&source).unwrap()).unwrap();
            let f = reviewed_contract(&p, &p.routines[0]).unwrap();
            assert!(f.abi_inputs_only);
            assert_eq!(f.writes.as_deref(), Some([].as_slice()));
            assert_eq!(f.reads.as_ref().unwrap()[0].global, Some(p.globals[0].id));
            p.globals[0].backing = MirGlobalBacking::Ordinary { offset: 50 };
            assert!(reviewed_contract(&p, &p.routines[0]).is_some());
            p.globals[0].backing = MirGlobalBacking::Absolute(0xd000);
            assert!(reviewed_contract(&p, &p.routines[0]).is_none());
        }
        let missing = src.replace(";@actionc writes-only none", "");
        let p = crate::mir6502::lower_program(&lower(&missing).unwrap()).unwrap();
        assert_eq!(reviewed_contract(&p, &p.routines[0]).unwrap().writes, None);
        for stale in [
            src.replace("tax", "tay"),
            src.replace("BYTE ch", "CARD ch"),
            src.replace("tab(4)", "tab(5)"),
        ] {
            let p = crate::mir6502::lower_program(&lower(&stale).unwrap()).unwrap();
            assert!(reviewed_contract(&p, &p.routines[0]).is_none());
        }
        for bad in [
            src.replace("0..3", "0..4"),
            src.replace("tab[", "missing["),
            src.replace("0..3", "3..0"),
        ] {
            assert!(lower(&bad).is_err());
        }
        let mut invalid = nir;
        invalid.routines[0]
            .entry
            .callee_review
            .as_mut()
            .unwrap()
            .facts
            .reads
            .as_mut()
            .unwrap()[0]
            .end = 4;
        assert!(crate::nir::verify_program(&invalid).is_err());
    }
}
