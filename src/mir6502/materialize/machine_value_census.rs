#[cfg(test)]
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::known_callees::MirKnownCalleeSummaries;
#[cfg(test)]
use crate::mir6502::analysis::machine_values::MirMachineValueAvailability;
use crate::mir6502::analysis::machine_values::{MirMachineMemoryMap, MirMachineValue};
use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::ir::{MirAddr, MirDef, MirMem, MirOp, MirProgram, MirReg, MirValue, MirWidth};
use crate::mir6502::rewrite::context::{MirProof, PostHomeRewriteContext};

use super::layout::MaterializeLayout;
use super::stats::MirPeepholeStats;

#[cfg(test)]
fn record_register_reload_candidates(program: &MirProgram, stats: &mut MirPeepholeStats) {
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
                let Some((reg, loaded, kind)) =
                    loaded_register_value(op, |reg| values.register_at(site, reg).ok().flatten())
                else {
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

pub(super) fn fold_redundant_register_reloads(
    program: &mut MirProgram,
    layout: &MaterializeLayout,
    stats: &mut MirPeepholeStats,
) {
    let known_callees = MirKnownCalleeSummaries::analyze(program);
    for routine in &mut program.routines {
        let removals =
            {
                let memory_map =
                    MirMachineMemoryMap::from_routine(routine, |mem| match mem {
                        MirMem::FixedZeroPage(slot) => Some(u16::from(slot.0)),
                        MirMem::ZeroPage(slot) => routine
                            .frame
                            .zero_page_allocations
                            .iter()
                            .find_map(|allocation| {
                                (allocation.slot == *slot).then_some(u16::from(allocation.start.0))
                            }),
                        _ if layout.mem_has_absolute_backing(mem) => None,
                        _ => layout.mem_address(routine.id, mem),
                    });
                let snapshot = PostHomeAnalysisSnapshot::new_with_known_callees_and_memory_map(
                    routine,
                    MirRoutineGeneration::initial(),
                    &known_callees,
                    &memory_map,
                )
                .expect("post-home program was verified before register reload folding");
                let context = PostHomeRewriteContext::new(&snapshot);
                let mut removals = Vec::new();
                for block in &routine.blocks {
                    for (op_index, op) in block.ops.iter().enumerate() {
                        let site = MirSite::Op {
                            block: block.id,
                            op_index,
                        };
                        let point = context.point(site);
                        let Some((reg, loaded, kind)) = loaded_register_value(op, |source| {
                            match context.register_value_at(source, point) {
                                MirProof::Proven(value) => Some(value),
                                MirProof::Blocked(_) => None,
                            }
                        }) else {
                            continue;
                        };
                        let MirProof::Proven(incoming) = context.register_value_at(reg, point)
                        else {
                            continue;
                        };
                        if incoming != loaded || !load_read_is_removable(op, layout) {
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
                            stats.record(routine.id, "machine-value-register-reload-flags-live");
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

fn loaded_register_value(
    op: &MirOp,
    mut register_value: impl FnMut(MirReg) -> Option<MirMachineValue>,
) -> Option<(MirReg, MirMachineValue, &'static str)> {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(reg),
            src,
            width: MirWidth::Byte,
        } => load_source_value(src, &mut register_value).map(|(value, kind)| (*reg, value, kind)),
        MirOp::LoadImm {
            dst: MirDef::Reg(reg),
            value,
            width: MirWidth::Byte,
        } => u8::try_from(*value)
            .ok()
            .map(|value| (*reg, MirMachineValue::ConstU8(value), "immediate")),
        MirOp::Move {
            dst: MirDef::Reg(reg),
            src,
            width: MirWidth::Byte,
        } => move_source_value(src, &mut register_value).map(|value| (*reg, value, "move")),
        _ => None,
    }
}

fn load_source_value(
    source: &MirAddr,
    register_value: &mut impl FnMut(MirReg) -> Option<MirMachineValue>,
) -> Option<(MirMachineValue, &'static str)> {
    let indexed = |base: &MirMem, index| {
        Some((
            MirMachineValue::IndexedMem {
                base: base.clone(),
                index: Box::new(index),
            },
            "indexed",
        ))
    };
    match source {
        MirAddr::Direct(mem) => Some((MirMachineValue::DirectMem(mem.clone()), "direct")),
        MirAddr::ZeroPageIndexedX { base } => {
            indexed(&MirMem::ZeroPage(*base), register_value(MirReg::X)?)
        }
        MirAddr::AbsoluteIndexedX { base } => indexed(base, register_value(MirReg::X)?),
        MirAddr::AbsoluteIndexedY { base } => indexed(base, register_value(MirReg::Y)?),
        MirAddr::Label(_)
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::ComputedIndex { .. }
        | MirAddr::PointerCell { .. }
        | MirAddr::PointerIndex { .. }
        | MirAddr::Deref { .. } => None,
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

fn load_read_is_removable(op: &MirOp, layout: &MaterializeLayout) -> bool {
    match op {
        MirOp::Load {
            src: MirAddr::Direct(mem),
            ..
        }
        | MirOp::Load {
            src: MirAddr::AbsoluteIndexedX { base: mem } | MirAddr::AbsoluteIndexedY { base: mem },
            ..
        }
        | MirOp::Move {
            src: MirValue::PointerCell(mem),
            ..
        } => layout.mem_allows_pure_read_reordering(mem),
        MirOp::Load {
            src: MirAddr::ZeroPageIndexedX { .. },
            ..
        } => true,
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
        MirReg::A => "machine-value-a-reload-candidate",
        MirReg::X => "machine-value-x-reload-candidate",
        MirReg::Y => "machine-value-y-reload-candidate",
    }
}

fn reload_elision_rule(reg: MirReg) -> &'static str {
    match reg {
        MirReg::A => "machine-value-a-reload-elided",
        MirReg::X => "machine-value-x-reload-elided",
        MirReg::Y => "machine-value-y-reload-elided",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirBlockId, MirCompareOp, MirCond, MirCondDest, MirEdge, MirEffects, MirFlagTest,
        MirFrame, MirGlobal, MirGlobalBacking, MirMem, MirRoutine, MirRoutineAbi, MirSpillId,
        MirTerminator, RoutineId,
    };
    use crate::nir::SymbolId;

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

        record_register_reload_candidates(&program, &mut stats);

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
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);

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
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);

        assert_eq!(program.routines[0].blocks[0].ops.len(), 3);
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-register-reload-flags-live"),
            1
        );
    }

    #[test]
    fn definition_cleanup_removes_store_exposed_by_late_reload_folding() {
        let mut program = store_reload_program(MirTerminator::Return, Vec::new());
        let mut stats = MirPeepholeStats::default();
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);
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

    fn y_reload_program(source: MirMem, intervening: MirOp, across_edge: bool) -> MirProgram {
        let reload = MirOp::Load {
            dst: MirDef::Reg(MirReg::Y),
            src: MirAddr::Direct(source.clone()),
            width: MirWidth::Byte,
        };
        let (entry_ops, entry_terminator, extra_blocks) = if across_edge {
            (
                vec![reload.clone(), intervening],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                vec![MirBlock {
                    id: MirBlockId(1),
                    label: "reload".to_string(),
                    params: Vec::new(),
                    ops: vec![reload],
                    terminator: MirTerminator::Return,
                }],
            )
        } else {
            (
                vec![reload.clone(), intervening, reload],
                MirTerminator::Return,
                Vec::new(),
            )
        };
        let mut blocks = vec![MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: entry_ops,
            terminator: entry_terminator,
        }];
        blocks.extend(extra_blocks);
        MirProgram {
            statics: Vec::new(),
            globals: vec![MirGlobal {
                id: SymbolId(0),
                name: "indexed_base".to_string(),
                kind: "array".to_string(),
                width: None,
                storage_size: 256,
                backing: MirGlobalBacking::Ordinary { offset: 0 },
                init: None,
            }],
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "ReloadY".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    spills: vec![MirSpillId(1)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks,
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        }
    }

    fn indexed_y_store() -> MirOp {
        MirOp::Store {
            dst: MirAddr::AbsoluteIndexedY {
                base: MirMem::Global {
                    id: SymbolId(0),
                    offset: 0,
                },
            },
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }
    }

    fn indexed_compare_program(successor_ops: Vec<MirOp>) -> MirProgram {
        let index = MirMem::Spill {
            id: MirSpillId(1),
            offset: 0,
        };
        let array = |offset| MirMem::Global {
            id: SymbolId(0),
            offset,
        };
        MirProgram {
            statics: Vec::new(),
            globals: vec![MirGlobal {
                id: SymbolId(0),
                name: "array".to_string(),
                kind: "array".to_string(),
                width: None,
                storage_size: 256,
                backing: MirGlobalBacking::Ordinary { offset: 0 },
                init: None,
            }],
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "IndexedCompare".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    spills: vec![MirSpillId(1)],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![
                    MirBlock {
                        id: MirBlockId(0),
                        label: "compare".to_string(),
                        params: Vec::new(),
                        ops: vec![
                            MirOp::Load {
                                dst: MirDef::Reg(MirReg::Y),
                                src: MirAddr::Direct(index),
                                width: MirWidth::Byte,
                            },
                            MirOp::CompareDirectIndexedBytes {
                                dst: MirCondDest::Flags,
                                op: MirCompareOp::Lt,
                                left: array(1),
                                right: array(0),
                                signed: false,
                            },
                        ],
                        terminator: MirTerminator::Branch {
                            cond: MirCond::FlagTest(MirFlagTest::CClear),
                            then_edge: MirEdge::plain(MirBlockId(1)),
                            else_edge: MirEdge::plain(MirBlockId(2)),
                        },
                    },
                    MirBlock {
                        id: MirBlockId(1),
                        label: "taken".to_string(),
                        params: Vec::new(),
                        ops: successor_ops,
                        terminator: MirTerminator::Return,
                    },
                    MirBlock {
                        id: MirBlockId(2),
                        label: "exit".to_string(),
                        params: Vec::new(),
                        ops: Vec::new(),
                        terminator: MirTerminator::Return,
                    },
                ],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        }
    }

    #[test]
    fn fold_reuses_comparison_loaded_indexed_a_and_unchanged_y_on_taken_edge() {
        let index = MirMem::Spill {
            id: MirSpillId(1),
            offset: 0,
        };
        let mut program = indexed_compare_program(vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(index),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::AbsoluteIndexedY {
                    base: MirMem::Global {
                        id: SymbolId(0),
                        offset: 1,
                    },
                },
                width: MirWidth::Byte,
            },
        ]);
        let mut stats = MirPeepholeStats::default();
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);

        assert!(program.routines[0].blocks[1].ops.is_empty());
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-a-reload-elided"),
            1
        );
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-y-reload-elided"),
            1
        );
    }

    #[test]
    fn fold_keeps_indexed_a_reload_after_aliasing_store() {
        let array = |offset| MirMem::Global {
            id: SymbolId(0),
            offset,
        };
        let mut program = indexed_compare_program(vec![
            MirOp::Store {
                dst: MirAddr::Direct(array(1)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::AbsoluteIndexedY { base: array(1) },
                width: MirWidth::Byte,
            },
        ]);
        let mut stats = MirPeepholeStats::default();
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);

        assert_eq!(program.routines[0].blocks[1].ops.len(), 2);
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-a-reload-elided"),
            0
        );
    }

    #[test]
    fn fold_reuses_volatile_load_value_but_never_deletes_a_volatile_read() {
        let mut program = store_reload_program(MirTerminator::Return, Vec::new());
        program.routines[0].blocks[0].ops = vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::Absolute(0xD20A)),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::X),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::X),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::Absolute(0xD20A)),
                width: MirWidth::Byte,
            },
        ];
        let mut stats = MirPeepholeStats::default();
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);

        let ops = &program.routines[0].blocks[0].ops;
        assert_eq!(ops.len(), 3);
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(
                    op,
                    MirOp::Load {
                        src: MirAddr::Direct(MirMem::Absolute(0xD20A)),
                        ..
                    }
                ))
                .count(),
            2
        );
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-x-reload-elided"),
            1
        );
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-a-reload-elided"),
            0
        );
    }

    #[test]
    fn fold_removes_y_reload_across_disjoint_indexed_store_and_cfg_edge() {
        let mut program = y_reload_program(
            MirMem::Spill {
                id: MirSpillId(1),
                offset: 0,
            },
            indexed_y_store(),
            true,
        );
        let mut stats = MirPeepholeStats::default();
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);

        assert!(program.routines[0].blocks[1].ops.is_empty());
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-y-reload-elided"),
            1
        );
    }

    #[test]
    fn fold_keeps_y_reload_when_indexed_store_can_reach_source() {
        let mut program = y_reload_program(
            MirMem::Global {
                id: SymbolId(0),
                offset: 255,
            },
            indexed_y_store(),
            false,
        );
        let mut stats = MirPeepholeStats::default();
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);

        assert_eq!(program.routines[0].blocks[0].ops.len(), 3);
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-y-reload-elided"),
            0
        );
    }

    #[test]
    fn fold_keeps_y_reload_across_unknown_indirect_write() {
        let mut program = y_reload_program(
            MirMem::Spill {
                id: MirSpillId(1),
                offset: 0,
            },
            MirOp::StoreIndirect {
                consumer: crate::mir6502::ir::MirAddressConsumer::IndirectIndexedY(
                    crate::mir6502::ir::MirPointerPair::Fixed {
                        lo: crate::mir6502::ir::MirFixedZpSlot(0xAC),
                    },
                ),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                offset: 0,
            },
            false,
        );
        let mut stats = MirPeepholeStats::default();
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);

        assert_eq!(program.routines[0].blocks[0].ops.len(), 3);
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-y-reload-elided"),
            0
        );
    }

    #[test]
    fn fold_keeps_y_reload_across_absolute_memory_write() {
        let mut program = y_reload_program(
            MirMem::Spill {
                id: MirSpillId(1),
                offset: 0,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0xd000)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            false,
        );
        let mut stats = MirPeepholeStats::default();
        let layout = MaterializeLayout::new(&program, 0x3000);

        fold_redundant_register_reloads(&mut program, &layout, &mut stats);

        assert_eq!(program.routines[0].blocks[0].ops.len(), 3);
        assert_eq!(
            stats.count_for(RoutineId(0), "machine-value-y-reload-elided"),
            0
        );
    }
}
