//! Observe existing proof and transaction results; these records grant no permission.
use super::{Trace, homes};
use crate::mir6502::analysis::{
    effects::MirHomeByte,
    sites::{MirRoutineGeneration, MirSite},
};
use crate::mir6502::rewrite::{
    context::MirProofBlocker,
    plan::{MirPostHomeRewritePlan, MirRewritePlan},
};
use crate::mir6502::{MirRoutine, RoutineId};
use serde_json::{Value, json};

pub(in crate::mir6502) fn site(site: MirSite) -> Value {
    match site {
        MirSite::BlockEntry { block } => json!({"kind":"entry","block":block.0}),
        MirSite::Op { block, op_index } => json!({"kind":"op","block":block.0,"op":op_index}),
        MirSite::Terminator { block } => json!({"kind":"terminator","block":block.0}),
    }
}

pub(in crate::mir6502) fn home(routine: RoutineId, byte: MirHomeByte) -> homes::Home {
    use crate::mir6502::MirMem;
    match byte {
        MirHomeByte::Spill { id, offset } => homes::memory(routine, &MirMem::Spill { id, offset }),
        MirHomeByte::VirtualZeroPage { slot, offset } => {
            let mut home = homes::memory(routine, &MirMem::ZeroPage(slot));
            home.offset = offset;
            home
        }
        MirHomeByte::FixedZeroPage(slot) => homes::memory(routine, &MirMem::FixedZeroPage(slot)),
    }
}

impl Trace {
    pub(in crate::mir6502) fn driver_round(
        &self,
        r: &MirRoutine,
        generation: MirRoutineGeneration,
        phase: &'static str,
    ) {
        if self.wants(r.id) {
            self.event("rewrite-generation", ||json!({"routine":r.id.0,"generation":generation.number(),"phase":phase,
                "blocks":r.blocks.iter().map(|b|json!({"block":b.id.0,"ops":b.ops.len()})).collect::<Vec<_>>() }));
        }
    }
    pub(in crate::mir6502) fn prehome_plan(
        &self,
        r: &MirRoutine,
        p: &MirRewritePlan,
        outcome: &'static str,
    ) {
        if self.wants(r.id) {
            self.event("rewrite", ||json!({"routine":r.id.0,"generation":p.generation.number(),"phase":"pre-home",
                "family":p.stat,"outcome":outcome,"block":p.block.0,"range":[p.range.start,p.range.end],"replacement_ops":p.replacement.len(),
                "removed_definitions":p.removed_defs.iter().map(|d|json!({"temp":d.definition.lane.temp.0,"byte":d.definition.lane.byte,"site":site(d.definition.site)})).collect::<Vec<_>>() }));
        }
    }
    pub(in crate::mir6502) fn posthome_plan(
        &self,
        r: &MirRoutine,
        p: &MirPostHomeRewritePlan,
        outcome: &'static str,
    ) {
        if self.wants(r.id) {
            self.event("rewrite", ||json!({"routine":r.id.0,"generation":p.generation.number(),"phase":"post-home",
                "family":p.stat,"outcome":outcome,"block":p.block.0,"range":[p.range.start,p.range.end],"replacement_ops":p.replacement.len(),
                "removed_definitions":p.removed_homes.iter().map(|d|json!({"home":home(r.id,d.home),"store":site(d.store)})).collect::<Vec<_>>() }));
        }
    }
    pub(in crate::mir6502) fn blocker(
        &self,
        r: &MirRoutine,
        generation: MirRoutineGeneration,
        family: &'static str,
        point: MirSite,
        blocker: &MirProofBlocker,
    ) {
        if !self.wants(r.id) {
            return;
        }
        self.event("rewrite-blocked", || {
            let details=match blocker {
                MirProofBlocker::HomeDefinitionLive{home:byte,store,end} => json!({"home":home(r.id,*byte),"store":site(*store),"window_end":site(*end)}),
                MirProofBlocker::HomeLive{home:byte,point} => json!({"home":home(r.id,*byte),"point":site(*point)}),
                MirProofBlocker::ReplacementHomeRead{home:byte,op_index} => json!({"home":home(r.id,*byte),"replacement_op":op_index}),
                _ => json!({"display":format!("{blocker:?}")}),
            };
            json!({"routine":r.id.0,"generation":generation.number(),"family":family,"point":site(point),"reason":blocker.category(),"details":details})
        });
    }
}
