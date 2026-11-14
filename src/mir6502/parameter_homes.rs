//! Parameter identity and placement are separate. Selection is introduced in
//! subsequent slices; this module owns the common binding invariants.
use super::{diagnostics::MirDiagnostic, ir::*};
use std::collections::BTreeSet;

/// Address uses expose the parameter object, unlike dereferencing its value.
pub(super) fn exposed(program: &MirProgram, routine: &MirRoutine, id: crate::nir::ParamId) -> bool {
    use super::analysis::effects::{classify_op, classify_terminator};
    let target = MirDataRelocationTarget::Param {
        routine: routine.id,
        id,
    };
    let image_has = |image: &MirDataImage| image.relocations.iter().any(|r| r.target == target);
    program.statics.iter().any(|s| image_has(&s.image))
        || program.globals.iter().any(|g| match &g.init {
            Some(MirGlobalInit::Bytes { image, .. }) => image_has(image),
            Some(MirGlobalInit::Descriptor { backing, .. }) => image_has(&backing.image),
            _ => false,
        })
        || program.routines.iter().flat_map(|r| r.frame.params.iter().chain(&r.frame.locals)).any(|s| match &s.init {
            Some(MirStorageInit::Bytes { image, .. }) => image_has(image),
            Some(MirStorageInit::Descriptor { backing, .. }) => image_has(&backing.image),
            _ => false,
        })
        || routine.blocks.iter().any(|b| {
            b.ops.iter().map(classify_op).map(|e| e.memory.address_references)
                .chain(std::iter::once(classify_terminator(&b.terminator).memory.address_references))
                .flatten().any(|m| matches!(m, MirMem::Param { id: p, .. } if p == id))
            // Machine payloads can refer to parameters by relocation or raw bytes.
            || b.ops.iter().any(|op| matches!(op, MirOp::MachineBlock { .. }))
        })
}

pub(super) fn verify_storage(program: &MirProgram, routine: &MirRoutine) -> Vec<MirDiagnostic> {
    let Some(bindings) = &routine.frame.parameter_homes else {
        return vec![];
    };
    let mut errors = Vec::new();
    for b in bindings {
        if b.home == MirParamHome::Inline {
            continue;
        }
        if exposed(program, routine, b.param)
            || !matches!(
                routine.abi,
                MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry
            )
        {
            errors.push(MirDiagnostic::routine(
                &routine.name,
                "exposed parameter cannot change its inline home",
            ));
        }
        if let MirParamHome::ZeroPage(start) = b.home {
            let width = routine
                .frame
                .params
                .iter()
                .find(|s| s.base == MirStorageBase::Param(b.param))
                .map_or(0, |s| s.storage_size);
            if program.globals.iter().any(|g| match g.backing {
                MirGlobalBacking::Absolute(address) => {
                    u32::from(address) < u32::from(start.0) + u32::from(width)
                        && u32::from(start.0) < u32::from(address) + u32::from(g.storage_size)
                }
                _ => false,
            }) {
                errors.push(MirDiagnostic::routine(
                    &routine.name,
                    "parameter binding overlaps source-owned storage",
                ));
            }
        }
    }
    errors
}

pub(super) fn verify_bindings(routine: &MirRoutine) -> Vec<MirDiagnostic> {
    let Some(bindings) = &routine.frame.parameter_homes else {
        return vec![];
    };
    let mut errors = Vec::new();
    let mut fail = |message: &str| errors.push(MirDiagnostic::routine(&routine.name, message));
    let mut ids = BTreeSet::new();
    let mut occupied = BTreeSet::new();
    for binding in bindings {
        if !ids.insert(binding.param) {
            fail("duplicate parameter binding");
        }
        let Some(slot) = routine
            .frame
            .params
            .iter()
            .find(|s| s.base == MirStorageBase::Param(binding.param))
        else {
            fail(
                "parameter binding has no matching Param declaration or conflicts with legacy ABI-only metadata",
            );
            continue;
        };
        let width = slot.storage_size;
        if slot.offset != 0
            || slot.init.is_some()
            || slot.storage != MirStorageClass::Scalar
            || !matches!(
                (slot.scalar_width, width),
                (Some(MirWidth::Byte), 1) | (Some(MirWidth::Word), 2)
            )
        {
            fail("parameter binding requires an uninitialized scalar byte or word at offset zero");
        }
        match &binding.home {
            MirParamHome::Inline | MirParamHome::Absent => {}
            MirParamHome::Registers(regs) => {
                if regs.len() != usize::from(width)
                    || regs
                        .iter()
                        .any(|r| !matches!(r, MirReg::A | MirReg::X | MirReg::Y))
                {
                    fail("parameter register binding requires one A/X/Y register per byte");
                }
                let mut lanes = Vec::new();
                for reg in regs {
                    if lanes.contains(reg) {
                        fail("parameter register lanes overlap");
                    }
                    lanes.push(*reg);
                }
            }
            MirParamHome::ZeroPage(start) => {
                if u16::from(start.0) + width > 256 {
                    fail("parameter pair crosses the zero-page boundary");
                }
                for byte in u16::from(start.0)..u16::from(start.0).saturating_add(width).min(256) {
                    if !occupied.insert(byte) {
                        fail("parameter zero-page bindings overlap");
                    }
                    if !routine
                        .frame
                        .fixed_zero_page
                        .contains(&MirFixedZpSlot(byte as u8))
                    {
                        fail("parameter zero-page binding is not reserved");
                    }
                    if routine.frame.zero_page_allocations.iter().any(|a| {
                        byte >= u16::from(a.start.0)
                            && byte < u16::from(a.start.0) + u16::from(a.size)
                    }) {
                        fail("parameter zero-page binding overlaps a virtual allocation");
                    }
                }
            }
        }
    }
    for slot in &routine.frame.params {
        match slot.base {
            MirStorageBase::Param(id) if ids.contains(&id) => {}
            _ => {
                fail("bound routine has a missing parameter binding or legacy allocation metadata")
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::ParamId;
    fn input() -> MirProgram {
        source("CARD FUNC Echo(CARD x) x==+1 RETURN(x)")
    }
    fn source(text: &str) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        super::super::lower_program(&nir).unwrap()
    }
    fn bind(p: &mut MirProgram, home: MirParamHome) -> ParamId {
        let r = &mut p.routines[0];
        let MirStorageBase::Param(id) = r.frame.params[0].base else {
            panic!()
        };
        r.frame.parameter_homes = Some(vec![MirParamBinding { param: id, home }]);
        id
    }
    #[test]
    fn inline_binding_preserves_planning_addresses_and_prints_identity() {
        let mut p = input();
        let old = super::super::materialize::MaterializeLayout::new(&p, 0x3000);
        let id = bind(&mut p, MirParamHome::Inline);
        assert!(verify_bindings(&p.routines[0]).is_empty());
        let new = super::super::materialize::MaterializeLayout::new(&p, 0x3000);
        let mem = MirMem::Param { id, offset: 0 };
        assert_eq!(
            old.mem_address(p.routines[0].id, &mem),
            new.mem_address(p.routines[0].id, &mem)
        );
        assert!(super::super::format_program(&p).contains("x -> Inline"));
    }
    #[test]
    fn malformed_bindings_are_rejected() {
        let mut p = input();
        bind(&mut p, MirParamHome::Registers(vec![MirReg::A]));
        assert!(!verify_bindings(&p.routines[0]).is_empty());
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xff)));
        assert!(!verify_bindings(&p.routines[0]).is_empty());
        let id = bind(&mut p, MirParamHome::Inline);
        p.routines[0]
            .frame
            .parameter_homes
            .as_mut()
            .unwrap()
            .push(MirParamBinding {
                param: id,
                home: MirParamHome::Absent,
            });
        assert!(!verify_bindings(&p.routines[0]).is_empty());
        p.routines[0].frame.parameter_homes = Some(vec![]);
        assert!(!verify_bindings(&p.routines[0]).is_empty());
        bind(&mut p, MirParamHome::Inline);
        p.routines[0].frame.params[0].base = MirStorageBase::ParamAbiOnly(id);
        assert!(!verify_bindings(&p.routines[0]).is_empty());
    }

    fn materialize(p: MirProgram) -> MirProgram {
        super::super::materialize_program_with_origin_and_runtime(
            p,
            &super::super::Mir6502Config {
                enable_peepholes: false,
                ..Default::default()
            },
            0x3000,
            crate::compiler::Runtime::ActionCart,
        )
        .unwrap()
    }
    fn emit(p: &MirProgram) -> (Vec<u8>, super::super::emit::MirEmissionSummary) {
        let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
        let summary =
            super::super::emit::emit_program_with_physical_memory(p, 0x3000, &mut emitter, true)
                .unwrap();
        (emitter.finish_with_relocations().unwrap().bytes, summary)
    }
    #[test]
    fn inline_and_zero_page_layouts_agree_with_final_emission() {
        let mut p = input();
        let baseline = emit(&materialize(p.clone()));
        let id = bind(&mut p, MirParamHome::Inline);
        let inline = emit(&materialize(p.clone()));
        assert_eq!(baseline.0, inline.0);
        assert_eq!(baseline.1.storage_symbols, inline.1.storage_symbols);
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xe2)));
        p.routines[0]
            .frame
            .fixed_zero_page
            .extend([MirFixedZpSlot(0xe2), MirFixedZpSlot(0xe3)]);
        let p = materialize(p);
        let (_, summary) = emit(&p);
        let layout = super::super::materialize::MaterializeLayout::new(&p, 0x3000);
        for offset in 0..2 {
            let mem = MirMem::Param { id, offset };
            assert_eq!(
                layout.mem_address(p.routines[0].id, &mem),
                Some(0xe2 + offset)
            );
            assert_eq!(
                summary.physical_memory.address(p.routines[0].id, &mem),
                Some(0xe2 + offset)
            );
        }
        let symbol = summary
            .storage_symbols
            .iter()
            .find(|s| s.name == "x")
            .unwrap();
        assert_eq!((symbol.address, symbol.size), (0xe2, 2));
    }

    #[test]
    fn register_binding_never_invents_a_memory_address() {
        let mut p = input();
        let id = bind(&mut p, MirParamHome::Registers(vec![MirReg::A, MirReg::X]));
        let layout = super::super::materialize::MaterializeLayout::new(&p, 0x3000);
        assert_eq!(
            layout.mem_address(p.routines[0].id, &MirMem::Param { id, offset: 0 }),
            None
        );
        let errors = super::super::verify_program(&p, MirPhase::PostHome).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("residual memory reference"))
        );
    }

    #[test]
    fn exposure_and_source_owned_zero_page_reject_relocation() {
        let mut p = source("CARD FUNC Echo(CARD x) RETURN(@x)");
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xe2)));
        p.routines[0]
            .frame
            .fixed_zero_page
            .extend([MirFixedZpSlot(0xe2), MirFixedZpSlot(0xe3)]);
        let errors = super::super::verify_program(&p, MirPhase::PreMaterialization).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("exposed parameter"))
        );

        let mut p = input();
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xe2)));
        p.routines[0]
            .frame
            .fixed_zero_page
            .extend([MirFixedZpSlot(0xe2), MirFixedZpSlot(0xe3)]);
        p.globals.push(MirGlobal {
            id: crate::nir::SymbolId(999),
            name: "reserved".into(),
            kind: "byte".into(),
            width: Some(MirWidth::Byte),
            storage_size: 1,
            backing: MirGlobalBacking::Absolute(0xe3),
            init: None,
        });
        let errors = super::super::verify_program(&p, MirPhase::PreMaterialization).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("source-owned storage"))
        );
    }
}
