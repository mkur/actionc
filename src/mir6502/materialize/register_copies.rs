//! Diagnostics and rewrites for direct byte copies through the accumulator.
use super::{layout::MaterializeLayout, stats::MirPeepholeStats};
use crate::mir6502::analysis::effects::MirFlagSet;
use crate::mir6502::analysis::known_callees::MirKnownCalleeSummaries;
use crate::mir6502::analysis::machine_values::{MirMachineMemoryMap, MirMachineValue};
use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::ir::*;
use crate::mir6502::rewrite::context::{MirProof, PostHomeRewriteContext};

pub(super) fn memory_map(routine: &MirRoutine, layout: &MaterializeLayout) -> MirMachineMemoryMap {
    MirMachineMemoryMap::from_routine(routine, |mem| match mem {
        MirMem::FixedZeroPage(slot) => Some(u16::from(slot.0)),
        MirMem::ZeroPage(slot) => routine
            .frame
            .zero_page_allocations
            .iter()
            .find_map(|a| (a.slot == *slot).then_some(u16::from(a.start.0))),
        _ if layout.mem_has_absolute_backing(mem) => None,
        _ => layout.mem_address(routine.id, mem),
    })
}

pub(super) fn removable_read(mem: &MirMem, layout: &MaterializeLayout) -> bool {
    !layout.mem_has_absolute_backing(mem) && layout.mem_allows_pure_read_reordering(mem)
}

/// A single final-program census. Counts are candidates, never additive savings.
pub(super) fn record_candidates(
    program: &MirProgram,
    layout: &MaterializeLayout,
    stats: &mut MirPeepholeStats,
) {
    let callees = MirKnownCalleeSummaries::analyze(program);
    for routine in &program.routines {
        let map = memory_map(routine, layout);
        let Ok(snapshot) = PostHomeAnalysisSnapshot::new_with_known_callees_and_memory_map(
            routine,
            MirRoutineGeneration::initial(),
            &callees,
            &map,
        ) else {
            continue;
        };
        let context = PostHomeRewriteContext::new(&snapshot);
        for block in &routine.blocks {
            for (index, pair) in block.ops.windows(2).enumerate() {
                if !matches!(
                    &pair[1],
                    MirOp::Store {
                        dst: MirAddr::Direct(_),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    }
                ) {
                    continue;
                }
                let point = context.point(MirSite::Op {
                    block: block.id,
                    op_index: index,
                });
                let (kind, available) = match &pair[0] {
                    MirOp::Move {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirValue::Def(MirDef::Reg(reg @ (MirReg::X | MirReg::Y))),
                        width: MirWidth::Byte,
                    } => ("register-copy-transfer-candidate", Some(*reg)),
                    MirOp::Load {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirAddr::Direct(mem),
                        width: MirWidth::Byte,
                    } => {
                        if !removable_read(mem, layout) {
                            stats.record(routine.id, "register-copy-blocked-observable-read");
                            continue;
                        }
                        let value = MirMachineValue::DirectMem(mem.clone());
                        let available = [MirReg::A, MirReg::X, MirReg::Y].into_iter().find(|reg|
                            matches!(context.register_value_at(*reg, point), MirProof::Proven(ref v) if *v == value));
                        ("register-copy-load-candidate", available)
                    }
                    _ => continue,
                };
                stats.record(routine.id, kind);
                stats.record_site(
                    routine.id,
                    kind,
                    format!(
                        "block=b{} op=#{} available={available:?}",
                        block.id.0, index
                    ),
                );
                if available.is_none() {
                    stats.record(routine.id, "register-copy-blocked-value");
                    continue;
                }
                let end = context.point(MirSite::Op {
                    block: block.id,
                    op_index: index + 1,
                });
                if !context.register_dead_after(MirReg::A, end).is_proven() {
                    stats.record(routine.id, "register-copy-live-accumulator");
                }
                if !context
                    .flags_dead_after(
                        MirFlagSet {
                            n: true,
                            z: true,
                            ..Default::default()
                        },
                        end,
                    )
                    .is_proven()
                {
                    stats.record(routine.id, "register-copy-live-nz");
                }
            }
        }
    }
}
