//! Contracts for selected implementations. Capture semantics stay independent
//! of the helper's return-address/descriptor implementation.
use super::ir::*;
pub(super) mod signatures;
mod invocation;
pub(super) mod buffers;
pub(super) mod library;
pub(super) use library::library_invocation;
pub(super) use invocation::{invocation, helper_invocation};
use std::collections::BTreeSet;
use std::sync::Arc;

impl MirRuntimeImplementation {
    pub(super) fn capture(
        program: &MirProgram,
        target: &MirRuntimeHelperTarget,
        helper: Option<MirRuntimeHelper>,
    ) -> Option<Arc<Self>> {
        let mut pending = BTreeSet::new();
        match target {
            MirRuntimeHelperTarget::Routine(id) => {
                pending.insert(*id);
            }
            MirRuntimeHelperTarget::KnownAbsolute(_) => {}
            MirRuntimeHelperTarget::Deferred => return None,
        }
        let mut fallthroughs = Vec::new();
        let mut visited = BTreeSet::new();
        let mut routines = Vec::new();
        let mut machines = Vec::new();
        while let Some(id) = pending.pop_first() {
            if !visited.insert(id) {
                continue;
            }
            let routine = program.routines.iter().find(|r| r.id == id)?;
            for op in routine.blocks.iter().flat_map(|b| &b.ops) {
                if let MirOp::Call {
                    target: MirCallTarget::Routine(id),
                    ..
                } = op
                {
                    pending.insert(*id);
                }
                if let MirOp::MachineBlock { id, .. } = op {
                    let machine = program.machine_blocks.iter().find(|m| m.id == *id)?;
                    for item in &machine.items {
                        if let MirMachineItem::Relocation {
                            target: MirInlineAsmTarget::Routine(id),
                            ..
                        } = item
                        {
                            pending.insert(*id);
                        }
                    }
                    if !machines.contains(machine) {
                        machines.push(machine.clone());
                    }
                }
            }
            // The reviewed assembly catalog includes adjacent entry aliases
            // and fallthroughs (SetSign/SS1, Zero/SetBlock). Preserve adjacency
            // as well as the dependency body when checking evidence freshness.
            if let [block] = routine.blocks.as_slice()
                && matches!(block.terminator, MirTerminator::Unreachable) {
                let falls_through = if block.ops.is_empty() { true }
                    else if let Some(MirOp::MachineBlock { id, .. }) = block.ops.last() {
                        let machine = program.machine_blocks.iter().find(|m| m.id == *id)?;
                        !matches!(crate::mir6502::analysis::known_callees::machine_block_fallthrough(machine),
                            crate::mir6502::analysis::known_callees::MachineBlockFallthrough::Stops)
                    } else { false };
                if falls_through {
                    let index = program.routines.iter().position(|r| r.id == routine.id)?;
                    let next = program.routines.get(index + 1)?;
                    fallthroughs.push((routine.id, next.id));
                    pending.insert(next.id);
                }
            }
            routines.push(routine.clone());
        }
        Some(Arc::new(Self {
            helper,
            fallthroughs,
            target: target.clone(),
            routines,
            machines,
        }))
    }

    pub(super) fn is_current(&self, program: &MirProgram, target: &MirRuntimeHelperTarget) -> bool {
        self.target == *target
            && self.fallthroughs.iter().all(|(from, to)| program.routines.windows(2)
                .any(|pair| pair[0].id == *from && pair[1].id == *to))
            && self
                .routines
                .iter()
                .all(|r| program.routines.iter().any(|now| now == r))
            && self
                .machines
                .iter()
                .all(|m| program.machine_blocks.iter().any(|now| now == m))
    }
}

pub(super) fn selected_helper(
    program: &MirProgram,
    helper: MirRuntimeHelper,
) -> Option<&MirRuntimeHelperDecl> {
    let declaration = program
        .runtime_helpers
        .iter()
        .find(|d| d.helper == helper)?;
    if declaration.binding.as_ref()?.helper != Some(helper)
        || declaration.abi != signatures::helper_abi_for(helper)
        || declaration.effects != signatures::helper_effects(&helper)
        || declaration.additional_results != signatures::helper_additional_results(helper) {
        return None;
    }
    declaration
        .binding
        .as_ref()?
        .is_current(program, &declaration.target)
        .then_some(declaration)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CaptureSummary {
    pub inputs: Vec<MirArgHome>,
    pub definitions: Vec<MirMem>,
    pub scratch: Vec<MirMem>,
    pub state_reads: Vec<MirMem>,
}

pub(super) fn sargs_capture(
    program: &MirProgram,
    routine: &MirRoutine,
    op: &MirOp,
) -> Option<CaptureSummary> {
    let MirOp::SArgsCapture {
        first_param,
        byte_count,
        effects,
    } = op
    else {
        return None;
    };
    selected_helper(program, MirRuntimeHelper::SArgs)?;
    // The ABI's contiguous $A0-based area must fit in ZP. The descriptor's
    // wider encoding alone does not establish a supported argument range.
    if !(5..=96).contains(byte_count)
        || routine.frame.parameter_homes.is_some()
        || !matches!(
            routine.abi,
            MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry
        )
        || routine.frame.params.first()?.base != MirStorageBase::Param(*first_param)
        || *effects != super::materialize::helper_effects(&MirRuntimeHelper::SArgs)
    {
        return None;
    }
    let mut definitions = Vec::new();
    for slot in &routine.frame.params {
        let MirStorageBase::Param(id) = slot.base else {
            return None;
        };
        if slot.offset != 0 || slot.storage_size != slot.abi_byte_width()? {
            return None;
        }
        definitions.extend((0..slot.storage_size).map(|offset| MirMem::Param { id, offset }));
    }
    if definitions.len() != usize::from(*byte_count) {
        return None;
    }
    Some(CaptureSummary {
        inputs: (0..*byte_count)
            .map(|offset| super::abi::action_arg_home(offset, MirWidth::Byte))
            .collect(),
        definitions,
        scratch: (0x82..=0x85)
            .chain(0xa0..=0xa2)
            .map(|b| MirMem::FixedZeroPage(MirFixedZpSlot(b)))
            .collect(),
        state_reads: vec![MirMem::FixedZeroPage(MirFixedZpSlot(0x11))],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn program(source: &str, runtime: crate::runtime::Runtime) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model))).unwrap();
        crate::mir6502::materialize_program_with_origin_and_runtime(
            crate::mir6502::lower_program(&nir).unwrap(),
            &super::super::Mir6502Config { enable_small_leaf_inlining: false,
                ..super::super::Mir6502Config::optimized() },
            0x3000, runtime).unwrap()

    }

    #[test]
    fn sargs_summary_covers_all_lanes_and_rejects_invalid_or_stale_capture() {
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            let p = program(
                "BYTE out PROC Capture(BYTE a CARD w BYTE POINTER p) out=a RETURN PROC Main() RETURN",
                runtime,
            );
            let r = p.routines.iter().find(|r| r.name == "Capture").unwrap();
            let op = r
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .find(|op| matches!(op, MirOp::SArgsCapture { .. }))
                .unwrap();
            let summary = sargs_capture(&p, r, op).unwrap();
            assert_eq!(
                summary.inputs,
                vec![
                    MirArgHome::Reg(MirReg::A),
                    MirArgHome::Reg(MirReg::X),
                    MirArgHome::Reg(MirReg::Y),
                    MirArgHome::FixedZeroPage(MirFixedZpSlot(0xa3)),
                    MirArgHome::FixedZeroPage(MirFixedZpSlot(0xa4))
                ]
            );
            assert_eq!(summary.definitions.len(), 5);
            assert!(summary.definitions.contains(&MirMem::Param {
                id: match r.frame.params[2].base {
                    MirStorageBase::Param(id) => id,
                    _ => unreachable!(),
                },
                offset: 1
            }));
            assert_eq!(summary.scratch.len(), 7);
            assert_eq!(
                summary.state_reads,
                vec![MirMem::FixedZeroPage(MirFixedZpSlot(0x11))]
            );
            for count in [0, 4, 6, 97, 256] {
                let mut bad = op.clone();
                if let MirOp::SArgsCapture { byte_count, .. } = &mut bad {
                    *byte_count = count;
                }
                assert!(sargs_capture(&p, r, &bad).is_none());
            }
            let mut bad = p.clone();
            bad.runtime_helpers
                .iter_mut()
                .find(|d| d.helper == MirRuntimeHelper::SArgs)
                .unwrap()
                .target = MirRuntimeHelperTarget::KnownAbsolute(0x1234);
            assert!(sargs_capture(&bad, r, op).is_none());
            let mut bad = p.clone();
            let d = bad
                .runtime_helpers
                .iter_mut()
                .find(|d| d.helper == MirRuntimeHelper::SArgs)
                .unwrap();
            d.binding = None;
            assert!(sargs_capture(&bad, r, op).is_none());
            if let MirRuntimeHelperTarget::Routine(id) =
                selected_helper(&p, MirRuntimeHelper::SArgs).unwrap().target
            {
                let mut bad = p.clone();
                bad.routines.iter_mut().find(|r| r.id == id).unwrap().blocks[0]
                    .ops
                    .clear();
                assert!(sargs_capture(&bad, r, op).is_none());
            }
        }
        let p = program(
            "CARD out PROC Small(CARD a,b) out=a RETURN PROC Main() RETURN",
            crate::runtime::Runtime::ActionCart,
        );
        assert!(
            !p.routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .any(|op| matches!(op, MirOp::SArgsCapture { .. }))
        );
    }
}

#[cfg(test)]
mod entry_tests;

impl std::fmt::Debug for MirRuntimeImplementation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirRuntimeImplementation").field("helper", &self.helper)
            .field("target", &self.target).field("fallthroughs", &self.fallthroughs)
            .field("routines", &self.routines.iter().map(|r| r.id).collect::<Vec<_>>())
            .field("machines", &self.machines.iter().map(|m| m.id).collect::<Vec<_>>()).finish()
    }
}

#[cfg(test)]
mod helper_tests;

#[cfg(test)]
mod library_tests;
