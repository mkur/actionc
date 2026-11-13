//! Additional physical read proof for virtual homes placed in public zero page.
use super::*;

pub(super) struct Readers<'a> {
    pub program: &'a MirProgram,
    pub memory: &'a MirPhysicalMemory,
    pub known: &'a MirKnownCalleeSummaries,
    pub default_sargs: bool,
    pub origin: u16,
}
impl Readers<'_> {
    /// A default scalar SArgs entry copies A/X/Y and the remaining $A3.. argument
    /// bytes into its frame. Its other reads are stack/inline descriptor bytes
    /// and runtime workspace $11/$82..$85. The generated descriptor is data,
    /// not an executable machine block. Do not generalize this to SET overrides.
    fn scalar_prologue(&self, r: &MirRoutine, address: u16) -> bool {
        if !self.default_sargs || self.origin < 0x400 || r.abi != MirRoutineAbi::Action {
            return false;
        }
        let Some(signature) = &r.scalar_signature else {
            return false;
        };
        let bytes: usize = signature
            .params
            .iter()
            .flatten()
            .map(|w| if *w == MirWidth::Word { 2 } else { 1 })
            .sum();
        if bytes == 0
            || bytes > 256
            || (0xa0..0xa0 + bytes as u16).contains(&address)
            || address == 0x11
            || (0x82..=0x85).contains(&address)
        {
            return false;
        }
        let Some(block) = r.blocks.first() else {
            return false;
        };
        let [
            MirOp::RuntimeHelper {
                helper: MirRuntimeHelper::SArgs,
                ..
            },
            MirOp::MachineBlock { id, effects },
            ..,
        ] = block.ops.as_slice()
        else {
            return false;
        };
        if *effects != MirEffects::default() {
            return false;
        }
        self.program
            .machine_blocks
            .iter()
            .find(|m| m.id == *id)
            .is_some_and(|m| {
                matches!(m.items.as_slice(), [
                MirMachineItem::AddressByte { high: false, .. },
                MirMachineItem::AddressByte { high: true, .. }, MirMachineItem::Byte(n)
            ] if usize::from(*n) + 1 == bytes)
            })
    }

    fn callee_excludes(
        &self,
        id: RoutineId,
        address: u16,
        visiting: &mut BTreeSet<RoutineId>,
    ) -> bool {
        if !visiting.insert(id) {
            return false;
        }
        let result = self
            .program
            .routines
            .iter()
            .find(|r| r.id == id)
            .is_some_and(|r| {
                let prologue = self.scalar_prologue(r, address);
                r.blocks.iter().enumerate().all(|(bi, b)| {
                    b.ops.iter().enumerate().all(|(i, op)| {
                        prologue && bi == 0 && i < 2
                            || self.op_excludes(r.id, op, address, visiting)
                    })
                })
            });
        visiting.remove(&id);
        result
    }

    fn op_excludes(
        &self,
        owner: RoutineId,
        op: &MirOp,
        address: u16,
        visiting: &mut BTreeSet<RoutineId>,
    ) -> bool {
        if let MirOp::Call {
            target: MirCallTarget::Routine(id),
            ..
        } = op
        {
            // Inspect the body so a coarse SArgs summary cannot hide either a
            // harmless generated entry or a genuine indirect source reader.
            return self.callee_excludes(*id, address, visiting);
        }
        let effects = classify_op(op);
        if effects.memory.indirect_reads || effects.memory.opaque || effects.homes.unknown_reads {
            return false;
        }
        if matches!(op, MirOp::MachineBlock { .. }) {
            // Inspectable machine routines can already publish exact reads.
            return self.known.get(owner).is_some_and(|s| match s.reads() {
                MirKnownMemoryReads::Exact(reads) => reads
                    .iter()
                    .all(|m| self.memory.address(owner, m).is_some_and(|a| a != address)),
                MirKnownMemoryReads::Unknown => false,
            });
        }
        effects.memory.direct_reads.iter().all(|read| {
            self.memory
                .address(owner, &read.base)
                .is_some_and(|a| address.wrapping_sub(a) >= read.bytes)
        })
    }

    pub fn definition_unobserved(
        &self,
        routine: &MirRoutine,
        block: MirBlockId,
        start: usize,
        address: u16,
    ) -> bool {
        let Ok(cfg) = crate::mir6502::analysis::cfg::MirCfg::from_routine(routine) else {
            return false;
        };
        let mut pending = vec![(block, start)];
        let mut seen = BTreeSet::new();
        while let Some((id, start)) = pending.pop() {
            if !seen.insert((id, start)) {
                continue;
            }
            let Some(block) = routine.blocks.iter().find(|b| b.id == id) else {
                return false;
            };
            let mut killed = false;
            for op in &block.ops[start..] {
                if !self.op_excludes(routine.id, op, address, &mut BTreeSet::new()) {
                    return false;
                }
                if let MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    width,
                    ..
                } = op
                    && self.memory.address(routine.id, mem).is_some_and(|a| {
                        address.wrapping_sub(a) < if *width == MirWidth::Word { 2 } else { 1 }
                    })
                {
                    killed = true;
                    break;
                }
            }
            if !killed {
                if crate::mir6502::analysis::effects::classify_terminator(&block.terminator)
                    .homes
                    .unknown_reads
                {
                    return false;
                }
                pending.extend(cfg.successors(id).iter().map(|id| (*id, 0)));
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> MirProgram {
        let source =
            include_str!("../../../fixtures/runtime/argument_staging.act").replace("\r\n", "\n");
        let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        crate::mir6502::materialize_program_with_origin(
            crate::mir6502::lower_program(&nir).unwrap(),
            &Mir6502Config::optimized(),
            0x3000,
        )
        .unwrap()
    }

    #[test]
    fn scalar_sargs_contract_is_bounded_and_requires_default_binding_and_descriptor() {
        let program = fixture();
        let measured = measure(
            &program,
            &[&program],
            0x3000,
            crate::runtime::Runtime::ActionCart,
            &Trace::disabled(),
        )
        .unwrap();
        let known = MirKnownCalleeSummaries::analyze_for_rewriting(&program);
        let id = program
            .routines
            .iter()
            .find(|r| r.name == "Capture")
            .unwrap()
            .id;
        for default_sargs in [false, true] {
            let readers = Readers {
                program: &program,
                memory: &measured.memory,
                known: &known,
                default_sargs,
                origin: 0x3000,
            };
            assert_eq!(
                readers.callee_excludes(id, 0xe1, &mut BTreeSet::new()),
                default_sargs
            );
            for address in [0x11, 0x82, 0x85, 0xa0, 0xa4] {
                assert!(!readers.callee_excludes(id, address, &mut BTreeSet::new()));
            }
        }
        let mut wrong = program.clone();
        let r = wrong.routines.iter_mut().find(|r| r.id == id).unwrap();
        let MirOp::MachineBlock { id: machine, .. } = r.blocks[0].ops[1] else {
            panic!();
        };
        wrong
            .machine_blocks
            .iter_mut()
            .find(|m| m.id == machine)
            .unwrap()
            .items[2] = MirMachineItem::Byte(255);
        assert!(
            !Readers {
                program: &wrong,
                memory: &measured.memory,
                known: &known,
                default_sargs: true,
                origin: 0x3000
            }
            .callee_excludes(id, 0xe1, &mut BTreeSet::new())
        );
        let mut observer = program.clone();
        observer
            .routines
            .iter_mut()
            .find(|r| r.id == id)
            .unwrap()
            .blocks[0]
            .ops
            .push(MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::FixedIndirectIndexedY {
                    zp: MirFixedZpSlot(0xac),
                },
                width: MirWidth::Byte,
            });
        assert!(
            !Readers {
                program: &observer,
                memory: &measured.memory,
                known: &known,
                default_sargs: true,
                origin: 0x3000
            }
            .callee_excludes(id, 0xe1, &mut BTreeSet::new())
        );
    }

    #[test]
    fn resolved_or_overridden_helper_cannot_claim_default_binding_provenance() {
        let mut p = fixture();
        let mut proof = Preservation::new(true);
        proof.configure_runtime(&p);
        assert!(!proof.default_sargs());
        for d in &mut p.runtime_helpers {
            if d.helper == MirRuntimeHelper::SArgs {
                d.target = MirRuntimeHelperTarget::Deferred;
            }
        }
        proof.configure_runtime(&p);
        assert!(proof.default_sargs());
    }
}
