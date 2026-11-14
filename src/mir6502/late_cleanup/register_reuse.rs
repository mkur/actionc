//! Expose fixed-pair carry chains to the shared post-home value analyses.
use super::*;
use crate::mir6502::{
    analysis::{
        machine_values::{MirMachineMemoryMap, MirMachineValue},
        sites::MirSite,
    },
    rewrite::{
        context::{MirExitStateChange, MirProof, PostHomeRewriteContext},
        plan::MirPostHomeRewritePlan,
        posthome::structural_plan,
    },
};

#[derive(Default)]
pub(super) struct ReuseStats {
    pub expanded_operations: usize,
    pub removed_reloads: usize,
    pub register_transfers: usize,
}

pub(super) fn apply(
    program: &mut MirProgram,
    memory: Option<&MirPhysicalMemory>,
    trace: &Trace,
) -> Result<ReuseStats, Declined> {
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(program);
    let mut stats = ReuseStats::default();
    for routine in &mut program.routines {
        if !routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(|op| expansion(op).is_some())
        {
            continue;
        }
        let original = routine.clone();
        let expanded = expand(routine);
        // Every rewrite removes a load or replaces one with a native transfer.
        // Neither rule introduces a load, so this bounds the fixed point.
        let rounds = routine.blocks.iter().map(|b| b.ops.len()).sum::<usize>() + 1;
        let result = MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .with_trace(trace)
            .run_fixed_point_with_memory_map(
                routine,
                &known,
                |r| {
                    MirMachineMemoryMap::from_routine(r, |mem| match mem {
                        MirMem::FixedZeroPage(slot) => Some(u16::from(slot.0)),
                        MirMem::Absolute(address) => Some(*address),
                        _ => memory.and_then(|m| m.address(r.id, mem)),
                    })
                },
                |r, c| {
                    discover_register_reuse(r, c, memory)
                        .into_iter()
                        .take(1)
                        .collect()
                },
            )
            .map_err(|_| Declined::Analysis)?;
        if result.applied == 0 {
            *routine = original;
        } else {
            stats.expanded_operations += expanded;
            stats.removed_reloads += result
                .applied_by_stat
                .get("late-redundant-register-load")
                .copied()
                .unwrap_or(0);
            stats.register_transfers += result
                .applied_by_stat
                .get("late-register-load-transfer")
                .copied()
                .unwrap_or(0);
        }
    }
    Ok(stats)
}

fn expand(routine: &mut MirRoutine) -> usize {
    let mut count = 0;
    for block in &mut routine.blocks {
        let mut indices = Vec::with_capacity(block.ops.len());
        let mut ops = Vec::new();
        for op in &block.ops {
            indices.push(ops.len());
            if let Some(expanded) = expansion(op) {
                count += 1;
                ops.extend(expanded);
            } else {
                ops.push(op.clone());
            }
        }
        if let MirTerminator::Branch {
            cond: MirCond::FusedCompare { producer, .. },
            ..
        } = &mut block.terminator
        {
            // Verified producers belong to this block; expansion changes indices.
            producer.op_index = indices[producer.op_index];
        }
        block.ops = ops;
    }
    count
}

fn discover_register_reuse(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    memory: Option<&MirPhysicalMemory>,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, op) in block.ops.iter().enumerate() {
            let MirOp::Load {
                dst: MirDef::Reg(dst),
                src: MirAddr::Direct(mem @ MirMem::FixedZeroPage(slot)),
                width: MirWidth::Byte,
            } = op
            else {
                continue;
            };
            // Fixed ZP is ordinary RAM. It need not be privately owned: this
            // removes a read, never a store. Execution aliases remain excluded.
            if memory.is_some_and(|m| !m.execution_disjoint(u16::from(slot.0))) {
                continue;
            }
            let point = context.point(MirSite::Op {
                block: block.id,
                op_index: index,
            });
            let value = MirMachineValue::DirectMem(mem.clone());
            let matches = |reg| matches!(context.register_value_at(reg, point), MirProof::Proven(ref v) if *v == value);
            let mut exit = MirExitStateChange::default();
            let (replacement, stat) = if matches(*dst) {
                exit.flags = classify_op(op).machine.flag_writes;
                (vec![], "late-redundant-register-load")
            } else if *dst == MirReg::A {
                let Some(src) = [MirReg::X, MirReg::Y].into_iter().find(|reg| matches(*reg)) else {
                    continue;
                };
                (
                    vec![MirOp::Move {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirValue::Def(MirDef::Reg(src)),
                        width: MirWidth::Byte,
                    }],
                    "late-register-load-transfer",
                )
            } else {
                continue;
            };
            if let Some(plan) = structural_plan(
                routine,
                context,
                block.id,
                index..index + 1,
                replacement,
                exit,
                stat,
                0,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

/// Exactly the ordered instructions emitted for the compound operation.
/// Admission is intentionally independent of source names and allocation.
fn expansion(op: &MirOp) -> Option<Vec<MirOp>> {
    let MirOp::OffsetPointerByIndirectByte {
        op,
        dst: MirMem::FixedZeroPage(dst),
        source,
        offset,
    } = op
    else {
        return None;
    };
    let MirPointerPair::Fixed { lo } = source.pointer_pair() else {
        return None;
    };
    if source.preserves_index_in_y()
        || *offset > 255
        || lo.0 == 255
        || dst.0 == 255
        || (u16::from(dst.0) < u16::from(lo.0) + 2 && u16::from(lo.0) < u16::from(dst.0) + 2)
    {
        return None;
    }
    let carry = match op {
        MirBinaryOp::Add => MirCarryIn::Clear,
        MirBinaryOp::Sub => MirCarryIn::Set,
        _ => return None,
    };
    let load = |slot| MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
        width: MirWidth::Byte,
    };
    let store = |slot| MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    };
    Some(vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::Y),
            value: *offset,
            width: MirWidth::Byte,
        },
        load(lo.0),
        MirOp::BinaryDirectIndexedByte {
            op: *op,
            source: MirByteIndexedSource::FixedIndirectY { zp: lo },
            carry_in: Some(carry),
            carry_out: MirCarryOut::Produce,
        },
        store(dst.0),
        load(lo.0 + 1),
        MirOp::Binary {
            op: *op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(0),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::FromPrevious),
            carry_out: MirCarryOut::Ignore,
        },
        store(dst.0 + 1),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::{
        posthome::PostHomeAnalysisSnapshot, sites::MirRoutineGeneration,
    };

    fn operation(op: MirBinaryOp, offset: u16) -> MirOp {
        MirOp::OffsetPointerByIndirectByte {
            op,
            dst: MirMem::FixedZeroPage(MirFixedZpSlot(0xAC)),
            source: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                lo: MirFixedZpSlot(0xAE),
            }),
            offset,
        }
    }

    fn program(ops: Vec<MirOp>) -> MirProgram {
        let mut p = super::super::tests::program(ops);
        p.routines[0].frame.fixed_zero_page = (0xAC..=0xAF).map(MirFixedZpSlot).collect();
        p
    }

    fn bytes(p: &MirProgram) -> Vec<u8> {
        super::super::super::verify_program(p, MirPhase::PreEmission).unwrap();
        let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
        emit::emit_program_with_reference_programs_traced(
            p,
            0x3000,
            &mut emitter,
            true,
            &[],
            &Trace::disabled(),
        )
        .unwrap();
        emitter.finish().unwrap()
    }

    fn store(slot: u8, reg: MirReg) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
            src: MirValue::Def(MirDef::Reg(reg)),
            width: MirWidth::Byte,
        }
    }

    fn entry() -> MirProgram {
        program(vec![
            store(0xAE, MirReg::A),
            store(0xAF, MirReg::X),
            operation(MirBinaryOp::Add, 0),
        ])
    }

    fn plans(p: &MirProgram) -> Vec<MirPostHomeRewritePlan> {
        let r = &p.routines[0];
        let map = MirMachineMemoryMap::from_routine(r, |m| match m {
            MirMem::FixedZeroPage(s) => Some(s.0 as u16),
            MirMem::Absolute(a) => Some(*a),
            _ => None,
        });
        let known = MirKnownCalleeSummaries::default();
        let snapshot = PostHomeAnalysisSnapshot::new_with_known_callees_and_memory_map(
            r,
            MirRoutineGeneration::initial(),
            &known,
            &map,
        )
        .unwrap();
        discover_register_reuse(r, &PostHomeRewriteContext::new(&snapshot), None)
    }

    #[test]
    fn terminal_transaction_removes_reload_and_uses_x_with_fixed_frames() {
        let p = entry();
        let trace = crate::mir6502::trace::Mir6502Trace::default();
        let selected = super::super::select(
            p.clone(),
            &super::super::tests::clean(&p),
            None,
            &Mir6502Config {
                enable_late_register_reuse: true,
                ..Mir6502Config::optimized()
            },
            0x3000,
            crate::runtime::Runtime::ActionCart,
            &trace.context(),
        )
        .unwrap();
        assert_eq!(bytes(&p).len() - bytes(&selected).len(), 3);
        assert_eq!(p.routines[0].frame, selected.routines[0].frame);
        let events = trace.events();
        let result = &events
            .iter()
            .find(|e| e.kind == "late-cleanup-result")
            .unwrap()
            .data;
        assert_eq!(result["accepted"], true, "{result}");
        assert_eq!(result["expanded_operations"], 1);
        assert_eq!(result["removed_reloads"], 1);
        assert_eq!(result["register_transfers"], 1);
        assert_eq!(trace.manifest()["work"]["emission_invocations"], 2);
        super::super::tests::assert_closed_attempts(&events);
        let mut again = selected.clone();
        assert_eq!(
            apply(&mut again, None, &Trace::disabled())
                .unwrap()
                .expanded_operations,
            0
        );
        assert_eq!(again, selected);
    }

    #[test]
    fn reuse_keeps_live_flags_and_invalidates_physical_aliases_and_barriers() {
        let load = super::super::tests::load(MirMem::FixedZeroPage(MirFixedZpSlot(0xAE)));
        let mut p = program(vec![
            store(0xAE, MirReg::A),
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::Y),
                value: 0,
                width: MirWidth::Byte,
            },
            load.clone(),
        ]);
        p.routines[0].blocks[0].terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::ZSet),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(1)),
        };
        p.routines[0].blocks.push(MirBlock {
            id: MirBlockId(1),
            label: "exit".into(),
            params: vec![],
            ops: vec![],
            terminator: MirTerminator::Return,
        });
        assert!(plans(&p).is_empty(), "LDY changed the live N/Z result");
        for clobber in [
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0xAE)),
                src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                width: MirWidth::Byte,
            },
            MirOp::Barrier {
                effects: MirEffects {
                    opaque: true,
                    memory_writes: MirMemoryEffect::Unknown,
                    ..Default::default()
                },
            },
            MirOp::Store {
                dst: MirAddr::FixedIndirectIndexedY {
                    zp: MirFixedZpSlot(0xAC),
                },
                src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                width: MirWidth::Byte,
            },
        ] {
            let p = program(vec![store(0xAE, MirReg::X), clobber, load.clone()]);
            assert!(plans(&p).is_empty());
        }
        // Transfers reproduce N/Z even when those flags are live.
        p.routines[0].blocks[0].ops[0] = store(0xAE, MirReg::X);
        let found = plans(&p);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].stat, "late-register-load-transfer");
        p.routines[0].blocks[0].ops[0] = store(0xAE, MirReg::Y);
        p.routines[0].blocks[0].ops.remove(1);
        assert_eq!(
            plans(&p)[0].replacement,
            [MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                width: MirWidth::Byte
            }]
        );
    }

    #[test]
    fn missing_reuse_preserves_compound_form_and_joins_forget_conflicting_values() {
        let mut p = program(vec![operation(MirBinaryOp::Add, 0)]);
        let old = p.clone();
        assert_eq!(
            apply(&mut p, None, &Trace::disabled())
                .unwrap()
                .expanded_operations,
            0
        );
        assert_eq!(old, p);
        let mut p = entry();
        p.routines[0].blocks[0].ops.pop();
        p.routines[0].blocks[0].terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::ZSet),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        };
        p.routines[0].blocks.push(MirBlock {
            id: MirBlockId(1),
            label: "clobber".into(),
            params: vec![],
            ops: vec![MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::X),
                value: 7,
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(2))),
        });
        p.routines[0].blocks.push(MirBlock {
            id: MirBlockId(2),
            label: "join".into(),
            params: vec![],
            ops: vec![super::super::tests::load(MirMem::FixedZeroPage(
                MirFixedZpSlot(0xAF),
            ))],
            terminator: MirTerminator::Return,
        });
        assert!(plans(&p).is_empty());
    }

    #[test]
    fn expansion_preserves_emitted_bytes_and_carry_effects() {
        for op in [MirBinaryOp::Add, MirBinaryOp::Sub] {
            for offset in [0, 1, 255] {
                let compound = operation(op, offset);
                let expanded = expansion(&compound).unwrap();
                assert_eq!(
                    bytes(&program(vec![compound])),
                    bytes(&program(expanded.clone()))
                );
                let effects = classify_op(&expanded[2]);
                assert!(effects.machine.definitely_overwrites_carry);
                assert!(effects.machine.definitely_overwrites_overflow);
                assert!(effects.memory.indirect_reads);
                assert!(!effects.memory.indirect_writes);
                assert!(classify_op(&expanded[5]).machine.uses_previous_carry);
            }
        }
    }

    #[test]
    fn expansion_and_verifier_reject_invalid_carry_or_pointer_shapes() {
        assert!(expansion(&operation(MirBinaryOp::Mul, 0)).is_none());
        assert!(expansion(&operation(MirBinaryOp::Add, 256)).is_none());
        for destination in [0xAD, 0xAE, 0xAF, 0xFF] {
            let mut op = operation(MirBinaryOp::Add, 0);
            if let MirOp::OffsetPointerByIndirectByte { dst, .. } = &mut op {
                *dst = MirMem::FixedZeroPage(MirFixedZpSlot(destination));
            }
            assert!(expansion(&op).is_none());
        }
        for op in [MirBinaryOp::Add, MirBinaryOp::Sub] {
            for carry in [
                None,
                Some(MirCarryIn::FromPrevious),
                Some(if op == MirBinaryOp::Add {
                    MirCarryIn::Set
                } else {
                    MirCarryIn::Clear
                }),
            ] {
                let mut expanded = expansion(&operation(op, 0)).unwrap();
                if let MirOp::BinaryDirectIndexedByte { carry_in, .. } = &mut expanded[2] {
                    *carry_in = carry;
                }
                assert!(
                    super::super::super::verify_program(&program(expanded), MirPhase::PreEmission)
                        .is_err()
                );
            }
        }
    }
}
