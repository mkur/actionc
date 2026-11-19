//! Identity of a reviewed direct-call entry. The review is an assertion about
//! caller obligations. It does not modify the instruction-level input summary.
use super::*;
use crate::callee6502::ReviewDependencies;
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
    reviewed_match(program, routine, &mut ReviewDependencies::new()).map(|r| &r.facts)
}

fn reviewed_match<'a>(program: &MirProgram, routine: &'a MirRoutine, dependencies: &mut ReviewDependencies<RoutineId>)
    -> Option<&'a crate::callee6502::Review<crate::nir::SymbolId>> {
    let review = routine.frame.callee_review.as_ref()?;
    let params = routine.frame.direct_parameter_abi.as_ref()?;
    if !review.valid_buffer_signature(&params.iter().map(|p| p.width().bytes()).collect::<Vec<_>>())
    {
        return None;
    }
    let [block] = routine.blocks.as_slice() else {
        return None;
    };
    let [MirOp::MachineBlock { id, .. }] = block.ops.as_slice() else {
        return None;
    };
    let machine = program.machine_blocks.iter().find(|m| m.id == *id)?;
    if machine_fingerprint(program, routine, review.version(), dependencies) != Some(review.fingerprint)
        || !review.dependencies_match(|id| {
            if !machine.items.iter().any(|item| matches!(item, MirMachineItem::Relocation {
                target: MirInlineAsmTarget::Memory(MirMem::Global { id: seen, offset: 0 }), .. } if seen == id)) { return None; }
            Some(program.globals.iter().find(|g| g.id == *id)?.storage_size)
        }) { return None; }
    Some(review)
}

pub(in crate::mir6502) fn body_fingerprint(
    program: &MirProgram,
    routine: &MirRoutine,
    version: &[u8],
) -> Option<u64> {
    machine_fingerprint(program, routine, version, &mut ReviewDependencies::new())
}

fn machine_fingerprint(program: &MirProgram, routine: &MirRoutine, version: &[u8],
    dependencies: &mut ReviewDependencies<RoutineId>) -> Option<u64> {
    let [block] = routine.blocks.as_slice() else {
        return None;
    };
    let [MirOp::MachineBlock { id, effects }] = block.ops.as_slice() else {
        return None;
    };
    if !routine.frame.locals.is_empty()
        || !routine.frame.contains_source_asm
        || block.terminator != MirTerminator::Unreachable
        || (routine.effects.opaque && !effects.opaque)
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
                kind: InlineAsmRelocationKind::Byte8, target: MirInlineAsmTarget::Absolute(value),
                addend: 0, requires_zero_page: false, ..
            } => items.push(BodyItem::Byte(u8::try_from(*value).ok()?)),
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
            MirMachineItem::Relocation { kind, target: MirInlineAsmTarget::InlineOffset(offset), addend, requires_zero_page, .. } if effects.opaque => {
                items.push(BodyItem::Local { kind: *kind, offset: *offset, addend: *addend, requires_zp: *requires_zero_page });
            }
            MirMachineItem::Relocation { .. } if effects.opaque => return None,
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Absolute16, target: MirInlineAsmTarget::Routine(id),
                addend: 0, requires_zero_page: false, ..
            } => {
                let callee = program.routines.iter().find(|r| r.id == *id)?;
                if program.routines.iter().any(|r| r.id != *id && r.name.eq_ignore_ascii_case(&callee.name)) { return None; }
                let contract = dependencies.resolve(*id, |deps| {
                    reviewed_match(program, callee, deps)?.dependency_hash(|id| {
                        program.globals.iter().find(|g| g.id == *id).map(|g| g.name.clone())
                    })
                })?;
                items.push(BodyItem::Call { name: &callee.name, contract });
            }
            MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Absolute16,
                target: MirInlineAsmTarget::Memory(MirMem::Global { id, offset: 0 }),
                addend,
                requires_zero_page: false,
                ..
            } => {
                let global = program.globals.iter().find(|g| g.id == *id)?;
                if program.globals.iter().any(|other| {
                    other.id != global.id && other.name.eq_ignore_ascii_case(&global.name)
                }) {
                    return None;
                }
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
                    addend: *addend,
                });
            }
            _ => return None,
        }
    }
    if effects.opaque {
        let hash = crate::callee6502::opaque_fingerprint(&params, result, &items, version)?;
        return Some(routine.frame.callee_review.as_ref()?.bind_precondition(hash));
    }
    if (effects.may_call_os || routine.effects.may_call_os) && !items.iter().any(|i| matches!(i, BodyItem::Call { .. })) {
        return None;
    }
    let hash = fingerprint(&params, result, &items, version)?;
    if version == crate::callee6502::RANGE_VERSION {
        Some(routine.frame.callee_review.as_ref()?.bind_precondition(hash))
    } else { Some(hash) }
}

#[cfg(test)]
mod tn_reviews;

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
    fn opaque_contract_matches_payload_local_relocations_and_abi_in_both_backends() {
        let source = ";@actionc reviewed-6502 0000000000000000
;@actionc abi-inputs-only
;@actionc preserves $EA-$ED
;@actionc returns A=$A0
BYTE FUNC Leaf=*()
ASM OPAQUE
lda #>done-1
pha
lda #<done-1
pha
rts
done:
lda #7
sta $A0
rts
ENDASM
PROC Main() Leaf() RETURN";
        let p = crate::mir6502::lower_program(&lower(source).unwrap()).unwrap();
        let hash = body_fingerprint(&p, &p.routines[0], crate::callee6502::CONTRACT_VERSION).unwrap();
        let source = source.replace("0000000000000000", &format!("{hash:016x}"));
        for (src, matched) in [
            (source.clone(), true),
            (source.replace("lda #7", "lda #8"), false),
            (source.replace("#<done-1", "#<done"), false),
            (source.replace("#>done", "#<done"), false),
            (source.replace("BYTE FUNC", "CARD FUNC"), false),
        ] {
            let ast = crate::parser::parse(&crate::lexer::tokenize(&src).unwrap()).unwrap();
            let model = crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern()).unwrap();
            let sem = crate::semantic::ir::lower_program(&ast, &model);
            let p = crate::mir6502::lower_program(&crate::nir::lower_program(&sem)).unwrap();
            let mir = reviewed_contract(&p, &p.routines[0]);
            let classic = crate::codegen::projected_review_for_test(&sem, "Leaf");
            assert_eq!(mir.is_some(), matched, "MIR: {src}");
            assert_eq!(classic.is_some(), matched, "classic: {src}");
            if let (Some(mir), Some(classic)) = (mir, classic) {
                assert!(mir.abi_inputs_only && classic.abi_inputs_only);
                assert_eq!(mir.preserves_zp, classic.preserves_zp);
                assert!(mir.writes.is_none() && classic.writes.is_none());
            }
        }
    }

    #[test]
    fn typed_jsr_binds_both_backends_to_current_child_guarantees() {
        let source = ";@actionc reviewed-6502 1111111111111111
;@actionc writes-only none
PROC Child=*()
ASM
rts
ENDASM
;@actionc reviewed-6502 2222222222222222
;@actionc abi-inputs-only
;@actionc entry-reads abi
PROC Parent=*(BYTE n)
ASM
pha
jsr Child
pla
beq done
rts
done:
rts
ENDASM
PROC Main() Parent(1) RETURN";
        let fingerprint = |src: &str, index| {
            let mir = crate::mir6502::lower_program(&lower(src).unwrap()).unwrap();
            body_fingerprint(&mir, &mir.routines[index], crate::callee6502::CONTRACT_VERSION).unwrap()
        };
        let src = source.replace("1111111111111111", &format!("{:016x}", fingerprint(source, 0)));
        let src = src.replace("2222222222222222", &format!("{:016x}", fingerprint(&src, 1)));
        for (src, valid) in [
            (src.clone(), true),
            (src.replace("writes-only none", "writes-only $A0"), false),
            (src.replacen("ASM
rts", "ASM
nop
rts", 1), false),
            (src.replace("jsr Child", "jsr $3456"), false),
            (src.replace("jsr Child", "jsr Parent"), false),
        ] {
            let ast = crate::parser::parse(&crate::lexer::tokenize(&src).unwrap()).unwrap();
            let model = crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern()).unwrap();
            let sem = crate::semantic::ir::lower_program(&ast, &model);
            let mir = crate::mir6502::lower_program(&crate::nir::lower_program(&sem)).unwrap();
            assert_eq!(crate::codegen::projected_review_for_test(&sem, "Parent").is_some(), valid, "{src}");
            assert_eq!(reviewed_contract(&mir, &mir.routines[1]).is_some(), valid, "{src}");
            assert_eq!(MirCalleeEntryFact::analyze(&mir, mir.routines[1].id, MirCalleeEntryStage::ActualEntry).is_some(), valid);
        }
    }
    #[test]
    fn native_direct_call_producers_publish_their_own_generated_facts() {
        let source = "BYTE g=$0600 BYTE FUNC Read() RETURN(g) PROC Main() g=Read()+1 RETURN";
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let sem = crate::semantic::ir::lower_program(&ast, &model);
        assert!(crate::codegen::projected_review_for_test(&sem, "Read").is_none());
        let classic = crate::codegen::generate_semir_profile_with_origin(
            &sem,
            0x3000,
            crate::codegen::CodegenProfile::Modern,
        )
        .unwrap();
        assert!(
            classic.map.optimizations.iter().any(|o| o.kind
                == crate::codegen::CodegenOptimizationKind::CallResultMaterializationRemoved)
        );
        let nir = crate::nir::lower_program(&sem);
        let mir = crate::mir6502::materialize_program(
            crate::mir6502::lower_program(&nir).unwrap(),
            &Default::default(),
        )
        .unwrap();
        let summary = MirKnownCalleeSummaries::analyze(&mir);
        let leaf = summary.get(mir.routines[0].id).unwrap();
        assert!(leaf.reviewed_facts().is_none());
        let facts = leaf.call_facts();
        assert!(facts.reads.unwrap().iter().any(
            |r| r.global == Some(mir.globals[0].id) || (r.global.is_none() && r.start == 0x600)
        ));
        assert!(
            facts
                .writes
                .unwrap()
                .iter()
                .any(|r| r.global.is_none() && r.start == 0xa0)
        );
    }

    #[test]
    fn classic_and_mir_publish_the_same_reviewed_fields() {
        let source = "BYTE ARRAY tab(4)=[1 2 3 4]\nBYTE ARRAY s\n;@actionc reviewed-6502 314ad810833478c4\n;@actionc abi-inputs-only\n;@actionc reads-only tab[0..3]\n;@actionc writes-only $A0\n;@actionc preserves $AC/$AD $AE/$AF\n;@actionc returns A=$A0\nBYTE FUNC Leaf=*(BYTE ch)\nASM\n and #3\n tax\n lda tab,x\n sta $A0\n rts\nENDASM\nPROC Paint()\n s(1)=Leaf(s(1))\nRETURN\nPROC Main() Paint() RETURN\n";
        for (src, valid) in [
            (source.to_owned(), true),
            (source.replace("and #3", "and #1"), false),
            (source.replace("tab(4)", "tab(5)"), false),
            (source.replace("BYTE ch", "CARD ch"), false),
            (source.replace("Leaf=*", "Leaf"), false),
        ] {
            let ast = crate::parser::parse(&crate::lexer::tokenize(&src).unwrap()).unwrap();
            let model = crate::semantic::analyze_with_options(
                &ast,
                crate::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            let sem = crate::semantic::ir::lower_program(&ast, &model);
            let classic = crate::codegen::projected_review_for_test(&sem, "Leaf");
            let nir = crate::nir::lower_program(&sem);
            let p = crate::mir6502::lower_program(&nir).unwrap();
            let f = reviewed_contract(&p, &p.routines[0]);
            assert_eq!(classic.is_some(), valid);
            assert_eq!(f.is_some(), valid);
            if let Some(classic) = classic {
                // Normalize the sole referenced table across the two ID spaces.
                let view = crate::callee6502::Review {
                    fingerprint: 0,
                    facts: classic.clone(),
                };
                let table = classic.reads.as_ref().unwrap()[0].global.unwrap();
                let common = view
                    .map_ids(|id| (*id == table).then_some(p.globals[0].id))
                    .unwrap();
                assert_eq!(Some(&common.facts), f);
                let known = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
                assert_eq!(
                    known.get(p.routines[0].id).unwrap().call_facts(),
                    common.facts
                );
            }
        }
    }

    #[test]
    fn tn_calcadr_review_matches_both_backends_and_binds_table_offset() {
        use crate::semantic::ir::{SemItem, SemProgram, SemStmt};
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/tn/modern/TN.ACT");
        let ast = crate::includes::load_program_with_includes(path).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let mut sem = crate::semantic::ir::lower_program(&ast, &model);
        let project = |sem: &SemProgram| {
            let classic = crate::codegen::projected_review_for_test(sem, "CalcAdr");
            let mir = crate::mir6502::lower_program(&crate::nir::lower_program(sem)).unwrap();
            let routine = mir.routines.iter().find(|r| r.name == "CalcAdr").unwrap();
            let facts = reviewed_contract(&mir, routine).cloned();
            // CalcAdr declares only absolute ranges, independent of backend IDs.
            assert_eq!(
                classic.map(|facts| crate::callee6502::Review {
                    fingerprint: 0,
                    facts
                }
                .map_ids(|_| None)
                .unwrap()
                .facts),
                facts
            );
            facts
        };
        let facts = project(&sem).expect("CalcAdr review matches the maintained TN library");
        assert!(facts.abi_inputs_only && facts.physical_abi_inputs && facts.preserves_regs.y);
        assert_eq!(facts.defines_on_return, BTreeSet::from([0xe6, 0xe7]));
        assert_eq!(facts.reads, None);
        assert_eq!(
            facts.writes.as_deref(),
            Some(
                [crate::callee6502::DataRange {
                    global: None,
                    start: 0xe6,
                    end: 0xe7,
                }]
                .as_slice()
            )
        );

        let routine = sem
            .modules
            .iter_mut()
            .flat_map(|m| &mut m.items)
            .find_map(|item| match item {
                SemItem::Routine(r) if r.symbol.name == "CalcAdr" => Some(r),
                _ => None,
            })
            .unwrap();
        let [SemStmt::InlineAsm { program, .. }] = routine.body.as_mut_slice() else {
            panic!()
        };
        program
            .relocations
            .iter_mut()
            .find(|r| r.addend == 24)
            .unwrap()
            .addend = 23;
        assert!(project(&sem).is_none());
    }

    #[test]
    fn reviewed_return_definitions_require_consistent_fixed_writes() {
        let source = ";@actionc reviewed-6502 0000000000000000
;@actionc writes-only $E6-$E7
;@actionc defines-on-return $E6 $E7
PROC Leaf=*()
ASM
sta $E6
sta $E7
rts
ENDASM
PROC Main() RETURN";
        let mut nir = lower(source).unwrap();
        crate::nir::verify_program(&nir).unwrap();
        let review = nir.routines[0].entry.callee_review.as_mut().unwrap();
        assert_eq!(review.facts.defines_on_return, BTreeSet::from([0xe6, 0xe7]));
        review.facts.defines_on_return.insert(0xe8);
        assert!(crate::nir::verify_program(&nir).is_err());
        assert!(lower(&source.replace("writes-only $E6-$E7", "writes-only $E6")).is_err());
        let unreviewed = source.replace(";@actionc reviewed-6502 0000000000000000", "")
            .replace(";@actionc writes-only $E6-$E7", "");
        assert!(lower(&unreviewed).unwrap_err().contains("defines-on-return requires reviewed-6502"));
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
        let missing_reads = src.replace(";@actionc reads-only tab[0..3]", "");
        let p = crate::mir6502::lower_program(&lower(&missing_reads).unwrap()).unwrap();
        assert_eq!(reviewed_contract(&p, &p.routines[0]).unwrap().reads, None);
        let inventory =
            machine_storage_observations(&p.machine_blocks[0], &p.routines[0], &p).unwrap();
        assert_eq!(inventory.reads.len(), 1);
        assert_eq!(
            inventory.reads[0].address.base,
            MirMem::Global {
                id: p.globals[0].id,
                offset: 0
            }
        );
        assert_eq!(
            inventory.reads[0].extent,
            super::super::observations::ObservationExtent::Bytes(256)
        );
        assert!(inventory.writes.is_empty());
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
