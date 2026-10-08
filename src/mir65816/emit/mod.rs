//! Conservative native instruction selection. Values have invocation storage,
//! a verified domain home or a bounded accumulator lifetime; scratch dies at calls.
mod allocation;
mod analysis;
mod call_flow;
mod coalescing;
mod copies;
mod effects;
mod forwarding;
mod home_demand;
pub(super) mod layout;
mod liveness;
mod loop_x;
mod mixed;
mod mixed_copies;
mod placement;
mod pointer_coalescing;
mod pointer_copies;
#[cfg(feature = "native65816-state-proof")]
pub mod proof;
mod replay;
mod resources;
mod rewrite;
pub(crate) mod scalar;
mod select;
mod selected;
mod state;
mod tracked;
#[cfg(any(test, feature = "native65816-state-proof"))]
mod work;

use super::*;
pub use allocation::{AllocatedFrame, Location, Slot};
pub use tracked::{
    Code, ConditionalBranch, Fixup, JumpEncoding, Label, LocalJump, MirTransfer, Target,
};

#[derive(Debug, Clone)]
pub struct MachineRoutine {
    pub id: RoutineId,
    pub frame: AllocatedFrame,
    pub code: Code,
}

#[derive(Debug, Clone)]
pub struct MachineProgram {
    /// The verified, legalized graph used by allocation, emission and linking.
    pub prepared: Mir65816Program,
    pub routines: Vec<MachineRoutine>,
    pub stack_checks: bool,
}

pub fn materialize(program: &Mir65816Program) -> Result<MachineProgram, String> {
    materialize_with_stack_checks(program, true)
}

pub fn materialize_with_stack_checks(
    program: &Mir65816Program,
    stack_checks: bool,
) -> Result<MachineProgram, String> {
    materialize_inner(program, false, stack_checks)
}

fn materialize_inner(
    program: &Mir65816Program,
    trace: bool,
    stack_checks: bool,
) -> Result<MachineProgram, String> {
    materialize_path(
        program,
        trace,
        stack_checks,
        #[cfg(feature = "native65816-state-proof")]
        true,
    )
}
fn materialize_path(
    program: &Mir65816Program,
    trace: bool,
    stack_checks: bool,
    #[cfg(feature = "native65816-state-proof")] replay: bool,
) -> Result<MachineProgram, String> {
    verify_program(program).map_err(|e| format!("invalid MIR65816: {e:?}"))?;
    if program.call_convention != Mir65816CallConvention::Native {
        return Err(
            "65816 emission requires wdc-65816-native; small-model emission is unsupported".into(),
        );
    }
    let contract = program.native_abi.as_ref().ok_or("missing native ABI")?;
    if !contract.unsupported_signatures.is_empty() {
        return Err(format!(
            "signature outside native ABI v2: {:?}",
            contract.unsupported_signatures
        ));
    }
    let prepared = arithmetic::prepare(program)?;
    let program = &prepared;
    for routine in &program.routines {
        if !routine.entry.external && routine.helper.is_none() {
            // Logical facts precede every physical placement path, including
            // forwarding. Stage 1 validates them without changing selection.
            super::analysis::RoutineAnalysis::new(routine)
                .map_err(|e| format!("{}: invalid logical MIR: {e}", routine.name))?;
        }
    }
    let forwarding = forwarding::plans(program);
    let mut routines = Vec::new();
    for routine in &program.routines {
        // External Action! interfaces are linked to explicitly described assembly.
        if routine.entry.external {
            continue;
        }
        if routine.entry.placement != crate::nir::NirRoutinePlacement::Relocatable {
            return Err(format!(
                "{}: fixed/current-location routine placement is unsupported",
                routine.name
            ));
        }
        if let Some(plan) = forwarding.get(&routine.id) {
            routines.push(plan.emit(program, routine, trace)?);
            continue;
        }
        routines.push(
            select::routine_with_data(
                routine,
                &program.data,
                trace,
                stack_checks,
                #[cfg(feature = "native65816-state-proof")]
                replay,
            )
            .map_err(|e| format!("{}: {e}", routine.name))?,
        );
    }
    Ok(MachineProgram {
        prepared,
        routines,
        stack_checks,
    })
}

#[cfg(test)]
mod state_tests;
