//! Candidate-local call facts for resident storage. The borrowed program fixes
//! the representation; construction never imports another candidate's summaries.
use super::*;
use crate::mir6502::analysis::known_callees::{
    MirCalleeEntryFact, MirKnownCalleeSummaries, MirKnownMemoryWrites,
};
use std::collections::BTreeMap;

pub(in crate::mir6502) struct ResidenceCalls<'a> {
    program: &'a MirProgram,
    known: MirKnownCalleeSummaries,
    entries: BTreeMap<RoutineId, Option<MirCalleeEntryFact>>,
    stage: MirCalleeEntryStage,
}

impl<'a> ResidenceCalls<'a> {
    pub fn new(program: &'a MirProgram, stage: MirCalleeEntryStage) -> Self {
        Self {
            program,
            known: MirKnownCalleeSummaries::analyze_for_rewriting(program),
            entries: BTreeMap::new(),
            stage,
        }
    }

    pub fn writes(&mut self, call: &MirOp) -> Result<&MirKnownMemoryWrites, &'static str> {
        if self.stage != MirCalleeEntryStage::ActualEntry {
            return Err("residence writes require expanded callee facts");
        }
        let MirOp::Call {
            target: MirCallTarget::Routine(target),
            args,
            ..
        } = call
        else {
            return Err("unknown or indirect residence call");
        };
        let callee = self
            .program
            .routines
            .iter()
            .find(|r| r.id == *target)
            .ok_or("missing residence callee")?;
        if callee.abi != MirRoutineAbi::Action
            || callee.blocks.iter().flat_map(|b| &b.ops).any(|op| {
                matches!(
                    op,
                    MirOp::Call { .. }
                        | MirOp::RuntimeHelper { .. }
                        | MirOp::MachineBlock { .. }
                        | MirOp::Barrier { .. }
                )
            })
        {
            return Err("unsupported residence callee");
        }
        let entry = self
            .entries
            .entry(*target)
            .or_insert_with(|| MirCalleeEntryFact::analyze(self.program, *target, self.stage))
            .as_ref()
            .ok_or("unknown residence callee entry")?;
        let mut outgoing = Vec::new();
        for arg in args {
            let lanes = super::super::abi::physical_lanes(&arg.home, arg.width)
                .ok_or("unsupported residence argument")?;
            let expected = super::super::abi::physical_lanes(
                &super::super::abi::action_arg_home(outgoing.len() as u16, arg.width),
                arg.width,
            )
            .ok_or("unsupported residence ABI")?;
            if lanes != expected {
                return Err("noncanonical residence arguments");
            }
            outgoing.extend(lanes);
        }
        handoff::inputs_supplied(entry, &outgoing)?;
        let writes = self
            .known
            .get(*target)
            .ok_or("unknown residence callee writes")?
            .writes();
        if matches!(writes, MirKnownMemoryWrites::Unknown) {
            return Err("unknown residence callee writes");
        }
        Ok(writes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program() -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(
            "BYTE FUNC Touch(BYTE x) RETURN(x+1) CARD FUNC Hold(CARD keep) Touch(7) RETURN(keep)"
        ).unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let n = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let mut p = crate::mir6502::lower_program(&n).unwrap();
        for (r, byte) in p.routines.iter_mut().zip([0xe8, 0xe4]) {
            let MirStorageBase::Param(param) = r.frame.params[0].base else {
                panic!()
            };
            r.frame.parameter_homes = Some(vec![MirParamBinding {
                param,
                home: MirParamHome::ZeroPage(MirFixedZpSlot(byte)),
            }]);
            r.frame.fixed_zero_page.extend(
                (0..r.frame.params[0].storage_size).map(|i| MirFixedZpSlot(byte + i as u8)),
            );
        }
        crate::mir6502::materialize_program(p, &crate::mir6502::Mir6502Config::default()).unwrap()
    }
    fn verify(p: &MirProgram) -> Vec<MirDiagnostic> {
        let r = p.routines.iter().find(|r| r.name == "Hold").unwrap();
        verify_lifetimes(
            r,
            Some(&mut ResidenceCalls::new(
                p,
                MirCalleeEntryStage::ActualEntry,
            )),
        )
    }
    fn store(byte: u8) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(byte))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }
    }
    #[test]
    fn caller_lifetime_uses_current_callee_writes_and_absolute_aliases() {
        let p = program();
        assert!(verify(&p).is_empty());
        for slot in [0xe4, 0xe5] {
            for absolute in [false, true] {
                let mut bad = p.clone();
                let mut write = store(slot);
                if absolute {
                    let MirOp::Store { dst, .. } = &mut write else {
                        unreachable!()
                    };
                    *dst = MirAddr::Direct(MirMem::Absolute(slot as u16));
                }
                bad.routines[0].blocks[0].ops.insert(0, write);
                assert!(
                    !verify(&bad).is_empty(),
                    "slot={slot:x} absolute={absolute}"
                );
            }
        }
        let mut moved = p.clone();
        moved.routines[0].frame.parameter_homes.as_mut().unwrap()[0].home =
            MirParamHome::ZeroPage(MirFixedZpSlot(0xe5));
        let param = moved.routines[0].frame.parameter_homes.as_ref().unwrap()[0].param;
        // A symbolic callee capture must follow the changed binding.
        moved.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Param {
                    id: param,
                    offset: 0,
                }),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify(&moved).is_empty());
        let mut virtual_write = p.clone();
        virtual_write.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(99))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify(&virtual_write).is_empty());
        virtual_write.routines[0]
            .frame
            .zero_page_allocations
            .push(MirZpAllocation {
                slot: MirZpSlot(99),
                start: MirFixedZpSlot(0xe9),
                size: 1,
            });
        assert!(verify(&virtual_write).is_empty());
        virtual_write.routines[0]
            .frame
            .zero_page_allocations
            .last_mut()
            .unwrap()
            .start = MirFixedZpSlot(0xe4);
        assert!(!verify(&virtual_write).is_empty());
    }
    #[test]
    fn setup_clobbers_and_extra_callee_inputs_are_not_preservation() {
        let p = program();
        let mut setup = p.clone();
        let r = &mut setup.routines[1];
        let (block, index) = r
            .blocks
            .iter()
            .enumerate()
            .find_map(|(b, block)| {
                block
                    .ops
                    .iter()
                    .position(|o| matches!(o, MirOp::Call { .. }))
                    .map(|i| (b, i))
            })
            .unwrap();
        r.blocks[block].ops.insert(index, store(0xe4));
        assert!(!verify(&setup).is_empty());
        let mut extra = p.clone();
        extra.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xe6))),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify(&extra).is_empty());
        let mut unknown = p.clone();
        let MirOp::Call { target, .. } = &mut unknown.routines[1].blocks[block].ops[index] else {
            panic!()
        };
        *target = MirCallTarget::Routine(RoutineId(999));
        assert!(!verify(&unknown).is_empty());
    }
}
