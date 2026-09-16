//! Conservative native instruction selection. Every live value has invocation
//! storage or a verified per-domain pointer home; scratch is dead at calls.
mod allocation;
mod code;
mod select;

use super::*;
pub use allocation::{AllocatedFrame, Location, Slot};
pub use code::{Code, Fixup, Label, Target};

#[derive(Debug, Clone)]
pub struct MachineRoutine {
    pub id: RoutineId,
    pub frame: AllocatedFrame,
    pub code: Code,
}

#[derive(Debug, Clone)]
pub struct MachineProgram {
    pub routines: Vec<MachineRoutine>,
}

pub fn materialize(program: &Mir65816Program) -> Result<MachineProgram, String> {
    verify_program(program).map_err(|e| format!("invalid MIR65816: {e:?}"))?;
    if program.call_convention != Mir65816CallConvention::Native {
        return Err(
            "65816 emission requires wdc-65816-native; small-model emission is unsupported".into(),
        );
    }
    let contract = program.native_abi.as_ref().ok_or("missing native ABI")?;
    if !contract.unsupported_signatures.is_empty() {
        return Err(format!(
            "signature outside native ABI v1: {:?}",
            contract.unsupported_signatures
        ));
    }
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
        routines.push(select::routine(routine).map_err(|e| format!("{}: {e}", routine.name))?);
    }
    Ok(MachineProgram { routines })
}
