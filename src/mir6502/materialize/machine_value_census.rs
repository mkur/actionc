#[cfg(test)]
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::known_callees::MirKnownCalleeSummaries;
use crate::mir6502::analysis::machine_values::MirMachineValue;
#[cfg(test)]
use crate::mir6502::analysis::machine_values::MirMachineValueAvailability;
use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::ir::{MirAddr, MirDef, MirMem, MirOp, MirProgram, MirReg, MirValue, MirWidth};
use crate::mir6502::rewrite::context::{MirProof, PostHomeRewriteContext};

use super::stats::MirPeepholeStats;

#[cfg(test)]
fn record_xy_reload_candidates(program: &MirProgram, stats: &mut MirPeepholeStats) {
    let known_callees = MirKnownCalleeSummaries::analyze(program);
    for routine in &program.routines {
        let cfg = MirCfg::from_routine(routine)
            .expect("post-home program was verified before machine-value telemetry");
        let values =
            MirMachineValueAvailability::analyze_with_known_callees(routine, &cfg, &known_callees);
        for block in &routine.blocks {
            for (op_index, op) in block.ops.iter().enumerate() {
                let site = MirSite::Op {
                    block: block.id,
                    op_index,
                };
                let Some((reg, loaded, kind)) = loaded_index_register_value(op, |reg| {
                    values.register_at(site, reg).ok().flatten()
                }) else {
                    continue;
                };
                let Ok(Some(incoming)) = values.register_at(site, reg) else {
                    continue;
                };
                if incoming != loaded {
                    continue;
                }
                let rule = reload_candidate_rule(reg);
                stats.record(routine.id, rule);
                stats.record_site(
                    routine.id,
                    rule,
                    format!(
                        "block=b{} op=#{} kind={kind} value={loaded:?}",
                        block.id.0, op_index
                    ),
                );
            }
        }
    }
}

pub(super) fn fold_redundant_xy_reloads(program: &mut MirProgram, stats: &mut MirPeepholeStats) {
    let known_callees = MirKnownCalleeSummaries::analyze(program);
    for routine in &mut program.routines {
        let removals = {
            let snapshot = PostHomeAnalysisSnapshot::new_with_known_callees(
                routine,
                MirRoutineGeneration::initial(),
                &known_callees,
            )
            .expect("post-home program was verified before X/Y reload folding");
            let context = PostHomeRewriteContext::new(&snapshot);
            let mut removals = Vec::new();
            for block in &routine.blocks {
                for (op_index, op) in block.ops.iter().enumerate() {
                    let site = MirSite::Op {
                        block: block.id,
                        op_index,
                    };
                    let point = context.point(site);
                    let Some((reg, loaded, kind)) = loaded_index_register_value(op, |source| {
                        match context.register_value_at(source, point) {
                            MirProof::Proven(value) => Some(value),
                            MirProof::Blocked(_) => None,
                        }
                    }) else {
                        continue;
                    };
                    let MirProof::Proven(incoming) = context.register_value_at(reg, point) else {
                        continue;
                    };
                    if incoming != loaded || !load_read_is_removable(op) {
                        continue;
                    }
                    let rule = reload_candidate_rule(reg);
                    stats.record(routine.id, rule);
                    stats.record_site(
                        routine.id,
                        rule,
                        format!(
                            "block=b{} op=#{} kind={kind} value={loaded:?}",
                            block.id.0, op_index
                        ),
                    );
                    let written_flags = classify_op(op).machine.flag_writes;
                    if written_flags.any()
                        && !context.flags_dead_after(written_flags, point).is_proven()
                    {
                        stats.record(routine.id, "machine-value-xy-reload-flags-live");
                        continue;
                    }
                    removals.push((block.id, op_index, reg));
                }
            }
            removals
        };

        for block in &mut routine.blocks {
            let mut removed = removals
                .iter()
                .filter(|(candidate_block, _, _)| *candidate_block == block.id)
                .map(|(_, op_index, reg)| (*op_index, *reg))
                .collect::<Vec<_>>();
            removed.sort_unstable_by_key(|(op_index, _)| *op_index);
            for (op_index, reg) in removed.into_iter().rev() {
                block.ops.remove(op_index);
                stats.record(routine.id, reload_elision_rule(reg));
            }
        }
    }
}

fn loaded_index_register_value(
    op: &MirOp,
    mut register_value: impl FnMut(MirReg) -> Option<MirMachineValue>,
) -> Option<(MirReg, MirMachineValue, &'static str)> {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(reg @ (MirReg::X | MirReg::Y)),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => Some((*reg, MirMachineValue::DirectMem(mem.clone()), "direct")),
        MirOp::LoadImm {
            dst: MirDef::Reg(reg @ (MirReg::X | MirReg::Y)),
            value,
            width: MirWidth::Byte,
        } => u8::try_from(*value)
            .ok()
            .map(|value| (*reg, MirMachineValue::ConstU8(value), "immediate")),
        MirOp::Move {
            dst: MirDef::Reg(reg @ (MirReg::X | MirReg::Y)),
            src,
            width: MirWidth::Byte,
        } => move_source_value(src, &mut register_value).map(|value| (*reg, value, "move")),
        _ => None,
    }
}

fn move_source_value(
    value: &MirValue,
    register_value: &mut impl FnMut(MirReg) -> Option<MirMachineValue>,
) -> Option<MirMachineValue> {
    match value {
        MirValue::Def(MirDef::Reg(reg)) => register_value(*reg),
        MirValue::ConstU8(value) => Some(MirMachineValue::ConstU8(*value)),
        MirValue::ConstU16(value) => u8::try_from(*value).ok().map(MirMachineValue::ConstU8),
        MirValue::PointerCell(mem) => Some(MirMachineValue::DirectMem(mem.clone())),
        _ => None,
    }
}

fn load_read_is_removable(op: &MirOp) -> bool {
    match op {
        MirOp::Load {
            src: MirAddr::Direct(mem),
            ..
        }
        | MirOp::Move {
            src: MirValue::PointerCell(mem),
            ..
        } => !matches!(mem, MirMem::Absolute(_)),
        MirOp::LoadImm { .. }
        | MirOp::Move {
            src: MirValue::Def(MirDef::Reg(_)) | MirValue::ConstU8(_) | MirValue::ConstU16(_),
            ..
        } => true,
        _ => false,
    }
}

fn reload_candidate_rule(reg: MirReg) -> &'static str {
    match reg {
        MirReg::X => "machine-value-x-reload-candidate",
        MirReg::Y => "machine-value-y-reload-candidate",
        MirReg::A => unreachable!("only X/Y loads are classified"),
    }
}

fn reload_elision_rule(reg: MirReg) -> &'static str {
    match reg {
        MirReg::X => "machine-value-x-reload-elided",
        MirReg::Y => "machine-value-y-reload-elided",
        MirReg::A => unreachable!("only X/Y loads are classified"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirBlockId, MirCond, MirEdge, MirEffects, MirFlagTest, MirFrame, MirMem,
        MirRoutine, MirRoutineAbi, MirSpillId, MirTerminator, RoutineId,
    };

    fn store_reload_program(terminator: MirTerminator, extra_blocks: Vec<MirBlock>) -> MirProgram {
        let spill = |id| MirMem::Spill {
            id: MirSpillId(id),
            offset: 0,
        };
        let destination = spill(2);
        let mut blocks = vec![MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::X),
                    src: MirAddr::Direct(spill(1)),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(destination.clone()),
                    src: MirValue::Def(MirDef::Reg(MirReg::X)),
                    width: MirWidth::Byte,
                },
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::X),
                    src: MirAddr::Direct(destination),
                    width: MirWidth::Byte,
                },
            ],
            terminator,
        }];
        blocks.extend(extra_blocks);
        MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "ReloadX".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks,
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        }
    }

    #[test]
    fn census_reports_store_then_reload_of_same_x_value() {
        let program = store_reload_program(MirTerminator::Return, Vec::new());
        let mut stats = MirPeepholeStats::default();

        record_xy_reload_candidates(&program, &mut stats);

        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-x-reload-candidate"),
            1
        );
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-y-reload-candidate"),
            0
        );
    }

    #[test]
    fn fold_removes_redundant_x_reload_when_result_flags_are_dead() {
        let mut program = store_reload_program(MirTerminator::Return, Vec::new());
        let mut stats = MirPeepholeStats::default();

        fold_redundant_xy_reloads(&mut program, &mut stats);

        assert_eq!(program.routines[0].blocks[0].ops.len(), 2);
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-x-reload-elided"),
            1
        );
    }

    #[test]
    fn fold_keeps_redundant_x_reload_when_result_flags_feed_branch() {
        let branch = MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::ZSet),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        };
        let exit = |id| MirBlock {
            id: MirBlockId(id),
            label: format!("exit{id}"),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        };
        let mut program = store_reload_program(branch, vec![exit(1), exit(2)]);
        let mut stats = MirPeepholeStats::default();

        fold_redundant_xy_reloads(&mut program, &mut stats);

        assert_eq!(program.routines[0].blocks[0].ops.len(), 3);
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-xy-reload-flags-live"),
            1
        );
    }

    #[test]
    fn definition_cleanup_removes_store_exposed_by_late_reload_folding() {
        let mut program = store_reload_program(MirTerminator::Return, Vec::new());
        let mut stats = MirPeepholeStats::default();

        fold_redundant_xy_reloads(&mut program, &mut stats);
        super::super::run_analyzed_dead_private_scratch_stores(
            &mut program.routines[0],
            &mut stats,
        )
        .unwrap();

        assert_eq!(program.routines[0].blocks[0].ops.len(), 1);
        assert_eq!(
            stats.count_for(RoutineId(0), "dead-private-scratch-store"),
            1
        );
    }
}
