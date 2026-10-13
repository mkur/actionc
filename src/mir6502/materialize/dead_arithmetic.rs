//! Remove dead machine arithmetic without suppressing source memory captures.
use super::*;
use crate::mir6502::analysis::effects::MirFlagSet;
use crate::mir6502::ir::{MirRegisterSet, MirRoutine};
use crate::mir6502::rewrite::context::{MirExitStateChange, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::MirPostHomeRewritePlan;
use crate::mir6502::rewrite::posthome::structural_plan;

fn pure_bitwise(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::Binary {
            op: MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(_) | MirValue::Def(MirDef::Reg(_)),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        }
    )
}

pub(in crate::mir6502) fn discover_dead_register_arithmetic(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = vec![];
    for block in &routine.blocks {
        for (index, op) in block.ops.iter().enumerate() {
            if !pure_bitwise(op) {
                continue;
            }
            let exit = MirExitStateChange {
                registers: MirRegisterSet {
                    a: true,
                    ..Default::default()
                },
                flags: MirFlagSet {
                    n: true,
                    z: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            if let Some(plan) = structural_plan(
                routine,
                context,
                block.id,
                index..index + 1,
                vec![],
                exit,
                "dead-register-bitwise-removed",
                3,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(super) fn run(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
    stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    if !routine
        .blocks
        .iter()
        .any(|b| b.ops.iter().any(pure_bitwise))
    {
        return Ok(());
    }
    // The same fixed point removes newly exposed pure reloads. Absolute reads
    // stay governed by the existing conservative load classifier.
    let result = MirPostHomeRewriteDriver::default()
        .run_fixed_point(routine, |r, c| {
            let mut plans = discover_dead_register_arithmetic(r, c);
            plans.extend(peepholes::discover_dead_register_writes(r, c, layout));
            plans
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home dead register arithmetic failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, stats);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::*;
    use crate::nir::ParamId;

    fn fixture() -> (MirRoutine, MaterializeLayout) {
        let mut program = MirProgram {
            globals: vec![],
            statics: vec![],
            routines: vec![],
            machine_blocks: vec![],
            runtime_helpers: vec![],
        };
        let r = MirRoutine {
            scalar_signature: None,
            inline: Default::default(),
            id: RoutineId(0),
            name: "Masked".into(),
            abi: MirRoutineAbi::Action,
            frame: Default::default(),
            temps: vec![],
            effects: Default::default(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".into(),
                params: vec![],
                ops: vec![
                    MirOp::Load {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirAddr::Direct(MirMem::Param {
                            id: ParamId(0),
                            offset: 1,
                        }),
                        width: MirWidth::Byte,
                    },
                    MirOp::Binary {
                        op: MirBinaryOp::And,
                        dst: MirDef::Reg(MirReg::A),
                        left: MirValue::Def(MirDef::Reg(MirReg::A)),
                        right: MirValue::ConstU8(0),
                        width: MirWidth::Byte,
                        carry_in: None,
                        carry_out: MirCarryOut::Ignore,
                    },
                    MirOp::LoadImm {
                        dst: MirDef::Reg(MirReg::Y),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::LoadImm {
                        dst: MirDef::Reg(MirReg::A),
                        value: 1,
                        width: MirWidth::Byte,
                    },
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                        width: MirWidth::Byte,
                    },
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5001)),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    },
                ],
                terminator: MirTerminator::Return,
            }],
        };
        program.routines.push(r.clone());
        (r, MaterializeLayout::new(&program, 0x3000))
    }

    #[test]
    fn dead_bitwise_removes_captured_high_reload_but_keeps_observable_reads() {
        let (mut r, layout) = fixture();
        run(&mut r, &layout, &mut MirPeepholeStats::default()).unwrap();
        assert_eq!(r.blocks[0].ops.len(), 4);
        let (mut r, layout) = fixture();
        if let MirOp::Load { src, .. } = &mut r.blocks[0].ops[0] {
            *src = MirAddr::Direct(MirMem::Absolute(0xD20A));
        }
        run(&mut r, &layout, &mut MirPeepholeStats::default()).unwrap();
        assert_eq!(r.blocks[0].ops.len(), 5);
        assert!(matches!(
            r.blocks[0].ops[0],
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Absolute(0xD20A)),
                ..
            }
        ));
    }

    #[test]
    fn dead_bitwise_keeps_live_result_or_flags_across_successors() {
        for live_flags in [false, true] {
            let (mut r, layout) = fixture();
            r.blocks[0].ops.truncate(2);
            r.blocks[0].terminator = MirTerminator::Jump(MirEdge::plain(MirBlockId(1)));
            r.blocks.push(MirBlock {
                id: MirBlockId(1),
                label: "use".into(),
                params: vec![],
                ops: if live_flags {
                    vec![]
                } else {
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    }]
                },
                terminator: if live_flags {
                    MirTerminator::Branch {
                        cond: MirCond::FlagTest(MirFlagTest::ZSet),
                        then_edge: MirEdge::plain(MirBlockId(2)),
                        else_edge: MirEdge::plain(MirBlockId(2)),
                    }
                } else {
                    MirTerminator::Return
                },
            });
            r.blocks.push(MirBlock {
                id: MirBlockId(2),
                label: "exit".into(),
                params: vec![],
                ops: vec![],
                terminator: MirTerminator::Return,
            });
            let before = r.clone();
            run(&mut r, &layout, &mut MirPeepholeStats::default()).unwrap();
            assert_eq!(r, before);
        }
    }

    #[test]
    fn dead_bitwise_rejects_memory_operands_and_carry_arithmetic() {
        let (r, layout) = fixture();
        for case in 0..4 {
            let mut r = r.clone();
            if let MirOp::Binary {
                op,
                right,
                carry_out,
                carry_in,
                ..
            } = &mut r.blocks[0].ops[1]
            {
                match case {
                    0 => *right = MirValue::PointerCell(MirMem::Absolute(0xD20A)),
                    1 => *op = MirBinaryOp::Add,
                    2 => *carry_out = MirCarryOut::Produce,
                    _ => *carry_in = Some(MirCarryIn::FromPrevious),
                }
            }
            let before = r.clone();
            run(&mut r, &layout, &mut MirPeepholeStats::default()).unwrap();
            assert_eq!(r, before);
        }
    }
}
