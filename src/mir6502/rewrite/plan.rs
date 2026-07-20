#![allow(dead_code)] // Additional rewrite families extend these declarations.

use std::collections::BTreeSet;
use std::ops::Range;

use crate::mir6502::analysis::sites::MirRoutineGeneration;
use crate::mir6502::analysis::use_def::MirDefSite;
use crate::mir6502::ir::{MirBlockId, MirFixedZpSlot, MirOp, MirReg, MirWidth};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) enum MirFactClass {
    Cfg,
    Reachability,
    Dominance,
    TempUseDef,
    ReachingDefinitions,
    TempLiveness,
    HomeLiveness,
    MachineLiveness,
    MemoryEffects,
    LayoutFacts,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirChangeSet {
    pub invalidates: BTreeSet<MirFactClass>,
}

impl MirChangeSet {
    pub(in crate::mir6502) fn prehome_operation_change() -> Self {
        Self {
            invalidates: BTreeSet::from([
                MirFactClass::TempUseDef,
                MirFactClass::ReachingDefinitions,
                MirFactClass::TempLiveness,
                MirFactClass::MemoryEffects,
                MirFactClass::MachineLiveness,
            ]),
        }
    }

    pub(in crate::mir6502) fn invalidates(&self, fact: MirFactClass) -> bool {
        self.invalidates.contains(&fact)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::mir6502) enum MirEffectDelta {
    /// Original and replacement have identical non-logical effects.
    #[default]
    Unchanged,
    /// A pre-home selection makes an operation's eventual result register
    /// explicit and routes a consumer through it. Other effects stay equal.
    SelectedResultRegister(MirReg),
    /// A call result remains in its ABI return slot and the next call reads it
    /// there instead of through a transient logical temp.
    ForwardedReturnSlot {
        base: MirFixedZpSlot,
        width: MirWidth,
    },
    /// Abstract call-argument expressions were selected into explicit ABI
    /// staging operations while preserving the calls and their effects.
    MaterializedCallArguments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) struct MirRemovedDefinition {
    pub definition: MirDefSite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirRewritePlan {
    pub generation: MirRoutineGeneration,
    pub block: MirBlockId,
    pub range: Range<usize>,
    pub replacement: Vec<MirOp>,
    pub removed_defs: Vec<MirRemovedDefinition>,
    pub exit_effect_delta: MirEffectDelta,
    pub change_set: MirChangeSet,
    pub stat: &'static str,
    pub observations: Vec<(&'static str, usize)>,
    pub family_priority: u16,
    pub estimated_byte_saving: u16,
    pub estimated_cycle_saving: u16,
}

impl MirRewritePlan {
    pub(in crate::mir6502) fn removed_operation_count(&self) -> usize {
        self.range.len().saturating_sub(self.replacement.len())
    }
}
