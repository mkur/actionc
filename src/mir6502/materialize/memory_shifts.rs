//! Native right shifts of captured private bytes, before physical ZP coloring.
use super::*;
use crate::mir6502::ir::{MirRegisterSet, MirRoutine};
use crate::mir6502::rewrite::context::{MirExitStateChange, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::MirPostHomeRewritePlan;
use crate::mir6502::rewrite::posthome::structural_plan;

const STAT: &str = "private-memory-right-shift-selected";

fn shape(ops: &[MirOp]) -> Option<(MirMem, MirUpdateOp)> {
    let [
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: MirBinaryOp::Rsh,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in,
            ..
        },
        MirOp::Store {
            dst: MirAddr::Direct(target),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ] = ops
    else {
        return None;
    };
    if mem != target || !peepholes::mem_is_private_scratch(mem) {
        return None;
    }
    let update = match carry_in {
        None => MirUpdateOp::Lsr,
        Some(MirCarryIn::FromPrevious) => MirUpdateOp::Ror,
        _ => return None,
    };
    Some((mem.clone(), update))
}

fn discover_private_memory_shifts(
    r: &MirRoutine,
    c: &PostHomeRewriteContext<'_, '_>,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for b in &r.blocks {
        for (index, ops) in b.ops.windows(3).enumerate() {
            let Some((mem, op)) = shape(ops) else {
                continue;
            };
            // C/N/Z and the home value are identical; only A's exit value differs.
            let exit = MirExitStateChange {
                registers: MirRegisterSet {
                    a: true,
                    ..MirRegisterSet::default()
                },
                ..MirExitStateChange::default()
            };
            if let Some(plan) = structural_plan(
                r,
                c,
                b.id,
                index..index + 3,
                vec![MirOp::UpdateMem {
                    op,
                    mem,
                    width: MirWidth::Byte,
                }],
                exit,
                STAT,
                45,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(super) fn run(
    r: &mut MirRoutine,
    stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    // Avoid rebuilding the full shared snapshot for unrelated routines.
    if !r
        .blocks
        .iter()
        .any(|b| b.ops.windows(3).any(|o| shape(o).is_some()))
    {
        return Ok(());
    }
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(r, discover_private_memory_shifts)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &r.name,
                format!("post-home private memory shift failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(r.id, result, stats);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::effects::classify_op;
    use crate::mir6502::ir::*;
    use crate::nir::{LocalId, ParamId, SymbolId};

    fn routine(mem: MirMem, carry: bool, live_a: bool) -> MirRoutine {
        let mut r = MirRoutine {
            id: RoutineId(0),
            name: "Shift".into(),
            abi: MirRoutineAbi::Action,
            inline: Default::default(),
            scalar_signature: None,
            temps: vec![],
            frame: MirFrame::default(),
            blocks: vec![],
            effects: MirEffects::default(),
        };
        if let MirMem::Spill { id, .. } = &mem {
            r.frame.spills.push(*id);
        }
        let mut ops = vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Rsh,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in: carry.then_some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ];
        if live_a {
            ops.push(MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            });
        } else {
            ops.push(MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value: 0,
                width: MirWidth::Byte,
            });
        }
        r.blocks.push(MirBlock {
            id: MirBlockId(0),
            label: "entry".into(),
            params: vec![],
            ops,
            terminator: MirTerminator::Return,
        });
        r
    }

    #[test]
    fn private_memory_shift_requires_dead_a_and_an_exact_owned_byte() {
        for carry in [false, true] {
            let mem = MirMem::Spill {
                id: MirSpillId(0),
                offset: 0,
            };
            let mut r = routine(mem.clone(), carry, false);
            run(&mut r, &mut MirPeepholeStats::default()).unwrap();
            assert!(
                matches!(r.blocks[0].ops.first(),Some(MirOp::UpdateMem{op,mem:home,..})
                if *op==(if carry {MirUpdateOp::Ror} else {MirUpdateOp::Lsr}) && home==&mem)
            );
            let effects = classify_op(&r.blocks[0].ops[0]);
            assert!(!effects.machine.register_writes.a);
            assert!(
                effects.machine.flag_writes.c
                    && effects.machine.flag_writes.z
                    && effects.machine.flag_writes.n
            );
            assert_eq!(effects.machine.flag_reads.c, carry);
            assert_eq!(effects.machine.uses_previous_carry, carry);
            let mut live = routine(mem, carry, true);
            let original = live.clone();
            run(&mut live, &mut MirPeepholeStats::default()).unwrap();
            assert_eq!(live, original);
        }
        for mem in [
            MirMem::Absolute(0xD000),
            MirMem::FixedZeroPage(MirFixedZpSlot(0xE2)),
            MirMem::Global {
                id: SymbolId(0),
                offset: 0,
            },
            MirMem::Local {
                id: LocalId(0),
                offset: 0,
            },
            MirMem::Param {
                id: ParamId(0),
                offset: 0,
            },
        ] {
            let mut r = routine(mem, false, false);
            let original = r.clone();
            run(&mut r, &mut MirPeepholeStats::default()).unwrap();
            assert_eq!(r, original);
        }
        for bad in 0..3 {
            let mut r = routine(
                MirMem::Spill {
                    id: MirSpillId(0),
                    offset: 0,
                },
                false,
                false,
            );
            match bad {
                0 => {
                    if let MirOp::Binary { right, .. } = &mut r.blocks[0].ops[1] {
                        *right = MirValue::ConstU8(2)
                    }
                }
                1 => {
                    if let MirOp::Store { dst, .. } = &mut r.blocks[0].ops[2] {
                        *dst = MirAddr::Direct(MirMem::Spill {
                            id: MirSpillId(1),
                            offset: 0,
                        })
                    }
                }
                _ => {
                    if let MirOp::Binary { carry_in, .. } = &mut r.blocks[0].ops[1] {
                        *carry_in = Some(MirCarryIn::Clear)
                    }
                }
            }
            let original = r.clone();
            run(&mut r, &mut MirPeepholeStats::default()).unwrap();
            assert_eq!(r, original);
        }
    }

    #[test]
    fn private_memory_shift_preserves_live_flags_and_rejects_successor_a_uses() {
        let mem = MirMem::Spill {
            id: MirSpillId(0),
            offset: 0,
        };
        let mut r = routine(mem.clone(), true, true);
        let tail = r.blocks[0].ops.pop().unwrap();
        r.blocks[0].terminator = MirTerminator::Jump(MirEdge {
            target: MirBlockId(1),
            args: vec![],
        });
        r.blocks.push(MirBlock {
            id: MirBlockId(1),
            label: "successor".into(),
            params: vec![],
            ops: vec![tail],
            terminator: MirTerminator::Return,
        });
        let original = r.clone();
        run(&mut r, &mut MirPeepholeStats::default()).unwrap();
        assert_eq!(r, original);
        let mut r = routine(mem, false, false);
        r.blocks[0].ops.pop();
        r.blocks[0].terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::CSet),
            then_edge: MirEdge {
                target: MirBlockId(1),
                args: vec![],
            },
            else_edge: MirEdge {
                target: MirBlockId(1),
                args: vec![],
            },
        };
        r.blocks.push(MirBlock {
            id: MirBlockId(1),
            label: "exit".into(),
            params: vec![],
            ops: vec![],
            terminator: MirTerminator::Return,
        });
        run(&mut r, &mut MirPeepholeStats::default()).unwrap();
        assert!(matches!(
            r.blocks[0].ops.as_slice(),
            [MirOp::UpdateMem {
                op: MirUpdateOp::Lsr,
                ..
            }]
        ));
    }

    #[test]
    fn private_memory_shift_verifier_rejects_wrong_phases_widths_and_update_forms() {
        use crate::mir6502::verify::verify_program;
        let mem = MirMem::Spill {
            id: MirSpillId(0),
            offset: 0,
        };
        let mut r = routine(mem.clone(), false, false);
        r.blocks[0].ops = vec![
            MirOp::Store {
                dst: MirAddr::Direct(mem.clone()),
                src: MirValue::ConstU8(0x81),
                width: MirWidth::Byte,
            },
            MirOp::UpdateMem {
                op: MirUpdateOp::Lsr,
                mem: mem.clone(),
                width: MirWidth::Byte,
            },
        ];
        r.abi = MirRoutineAbi::ProgramEntry;
        let p = MirProgram {
            statics: vec![],
            globals: vec![],
            routines: vec![r],
            machine_blocks: vec![],
            runtime_helpers: vec![],
        };
        verify_program(&p, MirPhase::PostHome).unwrap();
        assert!(verify_program(&p, MirPhase::PreMaterialization).is_err());
        assert!(crate::mir6502::format_program(&p).contains("lsr.b"));
        for op in [
            MirOp::UpdateMem {
                op: MirUpdateOp::Ror,
                mem: mem.clone(),
                width: MirWidth::Word,
            },
            MirOp::UpdateReg {
                op: MirUpdateOp::Lsr,
                reg: MirReg::X,
            },
            MirOp::UpdateIndexedMem {
                op: MirUpdateOp::Ror,
                base: MirMem::Absolute(0x5000),
            },
        ] {
            let mut broken = p.clone();
            broken.routines[0].blocks[0].ops[1] = op;
            assert!(verify_program(&broken, MirPhase::PostHome).is_err());
        }
    }
}
