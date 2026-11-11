//! Incoming flag requirements, independent of incidental exit flags.
use super::{cfg::MirCfg, effects::MirFlagSet, machine_liveness::MirMachineLiveness};
use crate::mir6502::ir::{MirOp, MirRoutine};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::mir6502) enum MirKnownCalleeEntryFlags {
    #[default]
    Unknown,
    Known {
        may_observe: MirFlagSet,
    },
}
impl MirKnownCalleeEntryFlags {
    pub(in crate::mir6502) fn analyze(routine: &MirRoutine) -> Self {
        if routine.scalar_signature.is_none()
            || routine.effects.opaque
            || routine.effects.reads.flags
            || routine.effects.preserves.flags
            || routine
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|o| matches!(o, MirOp::MachineBlock { .. }))
        {
            return Self::Unknown;
        }
        let Ok(cfg) = MirCfg::from_routine(routine) else {
            return Self::Unknown;
        };
        MirMachineLiveness::entry_flag_inputs(routine, &cfg)
            .map_or(Self::Unknown, |may_observe| Self::Known { may_observe })
    }
    pub(in crate::mir6502) fn unobserved(self, changed: MirFlagSet) -> bool {
        match self {
            Self::Unknown => !changed.any(),
            Self::Known { may_observe: live } => {
                !(changed.c && live.c
                    || changed.n && live.n
                    || changed.z && live.z
                    || changed.v && live.v)
            }
        }
    }
}
#[cfg(test)]
#[path = "entry_flags_tests.rs"]
mod tests;
