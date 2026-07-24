use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::known_callees::MirKnownCalleeSummaries;
use crate::mir6502::analysis::machine_values::{MirMachineValue, MirMachineValueAvailability};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirAddr, MirDef, MirOp, MirProgram, MirReg, MirValue, MirWidth};

use super::stats::MirPeepholeStats;

pub(super) fn record_xy_reload_candidates(program: &MirProgram, stats: &mut MirPeepholeStats) {
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
                let Some((reg, loaded, kind)) = loaded_index_register_value(op, &values, site)
                else {
                    continue;
                };
                let Ok(Some(incoming)) = values.register_at(site, reg) else {
                    continue;
                };
                if incoming != loaded {
                    continue;
                }
                let rule = match reg {
                    MirReg::X => "machine-value-x-reload-candidate",
                    MirReg::Y => "machine-value-y-reload-candidate",
                    MirReg::A => unreachable!("only X/Y loads are classified"),
                };
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

fn loaded_index_register_value(
    op: &MirOp,
    values: &MirMachineValueAvailability,
    site: MirSite,
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
        } => move_source_value(src, values, site).map(|value| (*reg, value, "move")),
        _ => None,
    }
}

fn move_source_value(
    value: &MirValue,
    values: &MirMachineValueAvailability,
    site: MirSite,
) -> Option<MirMachineValue> {
    match value {
        MirValue::Def(MirDef::Reg(reg)) => values.register_at(site, *reg).ok().flatten(),
        MirValue::ConstU8(value) => Some(MirMachineValue::ConstU8(*value)),
        MirValue::ConstU16(value) => u8::try_from(*value).ok().map(MirMachineValue::ConstU8),
        MirValue::PointerCell(mem) => Some(MirMachineValue::DirectMem(mem.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirBlockId, MirEffects, MirFrame, MirMem, MirRoutine, MirRoutineAbi, MirSpillId,
        MirTerminator, RoutineId,
    };

    #[test]
    fn census_reports_store_then_reload_of_same_x_value() {
        let spill = |id| MirMem::Spill {
            id: MirSpillId(id),
            offset: 0,
        };
        let destination = spill(2);
        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "ReloadX".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: Vec::new(),
                blocks: vec![MirBlock {
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
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };
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
}
