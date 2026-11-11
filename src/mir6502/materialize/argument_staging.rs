//! Retain a captured argument's high byte in X without changing its homes,
//! stores, source reads, call, or the machine state at the call boundary.
use super::*;
use crate::mir6502::analysis::{
    argument_staging::MAX_SETUP_OPS, call_predicates::ordinary_source, effects::MirFlagSet,
    known_callees::MirKnownCalleeSummaries,
    record_bases::MirPhysicalMemory, sites::MirSite,
};
use crate::mir6502::ir::{MirBlock, MirRegisterSet, MirRoutine};
use crate::mir6502::rewrite::{
    context::{MirExitStateChange, PostHomeRewriteContext},
    driver::MirPostHomeRewriteDriver,
    plan::MirPostHomeRewritePlan,
    posthome::structural_plan,
};

/// The pre-home witness restricts this first release to its qualified source
/// shape. It is never reused as a post-home machine/placement proof.
pub(in crate::mir6502) fn eligible_routines(program: &MirProgram) -> BTreeSet<RoutineId> {
    use crate::mir6502::analysis::{prehome::PreHomeAnalysisSnapshot, sites::MirRoutineGeneration};
    use crate::mir6502::rewrite::context::PreHomeRewriteContext;
    let mut eligible = BTreeSet::new();
    for original in &program.routines {
        if !original.blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(op,MirOp::Call { args,.. } if args.len() == 3 && args[0].width == MirWidth::Word && args[1].width == MirWidth::Word && args[2].width == MirWidth::Byte)) { continue; }
        let mut routine = original.clone();
        if MirPreHomeRewriteDriver::default()
            .run_fixed_point(
                &mut routine,
                crate::mir6502::rewrite::pilots::discover_call_arg_producers,
            )
            .is_err()
        {
            continue;
        }
        let Ok(snapshot) = PreHomeAnalysisSnapshot::new(&routine, MirRoutineGeneration::initial())
        else {
            continue;
        };
        let context = PreHomeRewriteContext::new(&snapshot);
        if routine.blocks.iter().any(|b| {
            b.ops.iter().enumerate().any(|(i, op)| {
                matches!(op, MirOp::Call { .. })
                    && context
                        .argument_staging(
                            &routine,
                            context.point(MirSite::Op {
                                block: b.id,
                                op_index: i,
                            }),
                        )
                        .is_ok()
            })
        }) {
            eligible.insert(routine.id);
        }
    }
    eligible
}

fn arithmetic_capture(r: &MirRoutine, block: &MirBlock, store: usize) -> Option<MirMem> {
    let start = store.checked_sub(5)?;
    let [
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(low_source),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: low_op @ (MirBinaryOp::Add | MirBinaryOp::Sub),
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(_),
            width: MirWidth::Byte,
            carry_in: Some(initial),
            carry_out: MirCarryOut::Produce,
        },
        MirOp::Store {
            dst: MirAddr::Direct(low_home),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(high_source),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: high_op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(_),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::FromPrevious),
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Store {
            dst: MirAddr::Direct(high_home),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ] = &block.ops[start..=store]
    else {
        return None;
    };
    if low_op != high_op
        || *initial
            != if *low_op == MirBinaryOp::Add {
                MirCarryIn::Clear
            } else {
                MirCarryIn::Set
            }
        || !ordinary_source(r, low_source, MirWidth::Word)
        || offset_mem(low_source, 1) != *high_source
        || !peepholes::mem_is_private_scratch(low_home)
        || !peepholes::mem_is_private_scratch(high_home)
        || low_home == high_home
    {
        return None;
    }
    Some(high_home.clone())
}

fn allowed_gap(r: &MirRoutine, op: &MirOp) -> bool {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A | MirReg::Y),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => ordinary_source(r, mem, MirWidth::Byte) || peepholes::mem_is_private_scratch(mem),
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A | MirReg::Y),
            width: MirWidth::Byte,
            ..
        } => true,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA3 | 0xA4))),
            src: MirValue::Def(MirDef::Reg(MirReg::A | MirReg::Y)),
            width: MirWidth::Byte,
        } => true,
        _ => false,
    }
}

fn call_shape(op: &MirOp, callees: &BTreeSet<RoutineId>) -> bool {
    let MirOp::Call {
        target: MirCallTarget::Routine(target),
        abi,
        args,
        effects,
        ..
    } = op
    else {
        return false;
    };
    !effects.opaque
        && !effects.may_call_os
        && effects.reads == MirRegisterSet::default()
        && callees.contains(target)
        && args.len() == 5
        && args.iter().enumerate().all(|(n, a)| {
            a.width == MirWidth::Byte
                && a.home == super::super::abi::action_arg_home(n as u16, MirWidth::Byte)
        })
        && abi.params == args.iter().map(|a| a.home.clone()).collect::<Vec<_>>()
}

fn discover(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    physical: &MirPhysicalMemory,
    callees: &BTreeSet<RoutineId>,
    observations: &mut BTreeSet<(MirBlockId, usize, &'static str)>,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    let nz = MirFlagSet {
        n: true,
        z: true,
        ..MirFlagSet::default()
    };
    for block in &routine.blocks {
        for store in 5..block.ops.len() {
            let Some(home) = arithmetic_capture(routine, block, store) else {
                continue;
            };
            let Some(address) = physical
                .address(routine.id, &home)
                .filter(|_| physical.ordinary(routine.id, &home))
            else {
                continue;
            };
            let mut index = store + 1;
            while index < block.ops.len()
                && index - store <= MAX_SETUP_OPS
                && allowed_gap(routine, &block.ops[index])
            {
                index += 1;
            }
            let Some(MirOp::Load {
                dst: MirDef::Reg(MirReg::X),
                src: MirAddr::Direct(source),
                width: MirWidth::Byte,
            }) = block.ops.get(index)
            else {
                continue;
            };
            if *source != home {
                continue;
            }
            let Some(MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(last),
                width: MirWidth::Byte,
            }) = block.ops.get(index + 1)
            else {
                continue;
            };
            if !ordinary_source(routine, last, MirWidth::Byte)
                || !block
                    .ops
                    .get(index + 2)
                    .is_some_and(|o| call_shape(o, callees))
            {
                continue;
            }
            observations.insert((block.id, store, "argument-staging-candidate"));
            if index + 2 - (store - 5) + 1 > MAX_SETUP_OPS {
                observations.insert((block.id, store, "argument-staging-blocked-budget"));
                continue;
            }
            // Every intervening write is explicit. Resolve aliases with the
            // emitted physical map, not with logical scratch names.
            let safe = block.ops[store + 1..index].iter().all(|op| {
                super::super::analysis::effects::classify_op(op)
                    .memory
                    .direct_writes
                    .iter()
                    .all(|range| {
                        physical
                            .address(routine.id, &range.base)
                            .is_some_and(|a| address.wrapping_sub(a) >= range.bytes)
                    })
            });
            if !safe {
                observations.insert((block.id, store, "argument-staging-blocked-overlap"));
                continue;
            }
            let after_store = context.point(MirSite::Op {
                block: block.id,
                op_index: store,
            });
            let after_reload = context.point(MirSite::Op {
                block: block.id,
                op_index: index,
            });
            if !context
                .register_dead_after(MirReg::X, after_store)
                .is_proven()
                || !context.flags_dead_after(nz, after_store).is_proven()
                || !context.flags_dead_after(nz, after_reload).is_proven()
            {
                observations.insert((block.id, store, "argument-staging-blocked-machine-state"));
                continue;
            }
            // Keep the original source/callee admission policy. The generic
            // witness also certifies this existing transfer; leaving the Y
            // reload outside the mutation produces the identical final MIR.
            let Ok(proof)=context.argument_x_forwarding(after_store,physical) else {continue;};
            let exit=MirExitStateChange {flags:nz,..MirExitStateChange::default()};
            if let Some(mut plan)=structural_plan(routine,context,block.id,proof.shape.range(),
                crate::mir6502::analysis::argument_x_forwarding::replacement(routine,&proof.shape).unwrap(),
                exit,"argument-staging-selected",40) {
                plan.witness=Some(crate::mir6502::rewrite::plan::MirPostHomeWitness::ArgumentXForwarding(proof));
                plans.push(plan);
            }
        }
    }
    // Machine-value assumptions may overlap even when the operation windows
    // do not. Rebuild the snapshot after every selected schedule.
    plans.into_iter().take(1).collect()
}

fn apply(
    baseline: &MirProgram,
    physical: &MirPhysicalMemory,
    eligible: &BTreeSet<RoutineId>,
) -> Option<(MirProgram, MirPeepholeStats, BTreeSet<RoutineId>)> {
    let callees = baseline
        .routines
        .iter()
        .filter(|r| {
            r.scalar_signature.as_ref().is_some_and(|s| {
                s.params
                    == vec![
                        vec![MirWidth::Word],
                        vec![MirWidth::Word],
                        vec![MirWidth::Byte],
                    ]
                    && s.result.len() <= 1
            })
        })
        .map(|r| r.id)
        .collect::<BTreeSet<_>>();
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(baseline);
    let mut proposed = baseline.clone();
    let mut stats = MirPeepholeStats::default();
    let mut affected = BTreeSet::new();
    for r in &mut proposed.routines {
        if !eligible.contains(&r.id) {
            continue;
        }
        if !r
            .blocks
            .iter()
            .any(|b| (5..b.ops.len()).any(|i| arithmetic_capture(r, b, i).is_some()))
        {
            continue;
        }
        let mut observations = BTreeSet::new();
        // Each commit removes one X reload. The number of original operations
        // plus one therefore bounds convergence even though op count is equal.
        let rounds = r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() + 1;
        let result = MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .run_fixed_point_with_physical_memory(
                r,
                &summaries,
                physical,
                |r, c| discover(r, c, physical, &callees, &mut observations),
            )
            .ok()?;
        for (_, _, stat) in observations {
            stats.record(r.id, stat);
        }
        if result.applied > 0 {
            affected.insert(r.id);
        }
        record_prehome_rewrite_result(r.id, result, &mut stats);
    }
    Some((proposed, stats, affected))
}

fn image_size(p: &MirProgram, origin: u16, runtime: crate::runtime::Runtime) -> Option<usize> {
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = super::super::emit::emit_program(p, origin, &mut emitter).ok()?;
    let output = super::super::codegen_output(
        emitter.finish_with_relocations().ok()?,
        origin,
        summary,
        runtime,
        p,
    );
    Some(crate::codegen::format_load_file(&output).len())
}

pub(in crate::mir6502) fn select_costed(
    baseline: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    eligible: &BTreeSet<RoutineId>,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    if !baseline.routines.iter().any(|r| {
        r.blocks
            .iter()
            .any(|b| (5..b.ops.len()).any(|i| arithmetic_capture(r, b, i).is_some()))
    }) {
        return Ok(baseline);
    }
    let Some(physical) = register_copies::physical_memory(&baseline, &baseline, origin) else {
        return Ok(baseline);
    };
    let Some((mut proposed, mut stats, affected)) = apply(&baseline, &physical, eligible) else {
        return Ok(baseline);
    };
    if affected.is_empty() {
        maybe_report_peepholes(&baseline, &stats, config);
        return Ok(baseline);
    }
    let old = address_values::Cost::measure(&baseline, origin);
    let cost = address_values::Cost::measure(&proposed, origin);
    let mut accepted = affected
        .iter()
        .copied()
        .filter(|&id| {
            old.as_ref()
                .zip(cost.as_ref())
                .is_some_and(|(old, new)| new.does_not_grow(old, id))
        })
        .collect::<BTreeSet<_>>();
    for r in &mut proposed.routines {
        if affected.contains(&r.id) && !accepted.contains(&r.id) {
            *r = baseline
                .routines
                .iter()
                .find(|old| old.id == r.id)
                .unwrap()
                .clone();
        }
    }
    let replayed = register_copies::physical_memory(&proposed, &baseline, origin)
        .and_then(|map| apply(&baseline, &map, eligible))
        .is_some_and(|(mut replay, _, _)| {
            for r in &mut replay.routines {
                if affected.contains(&r.id) && !accepted.contains(&r.id) {
                    *r = baseline
                        .routines
                        .iter()
                        .find(|old| old.id == r.id)
                        .unwrap()
                        .clone();
                }
            }
            replay == proposed
        });
    let final_ok = replayed
        && super::super::verify_program(&proposed, MirPhase::PreEmission).is_ok()
        && baseline
            .routines
            .iter()
            .zip(&proposed.routines)
            .all(|(a, b)| a.frame == b.frame)
        && old
            .as_ref()
            .zip(address_values::Cost::measure(&proposed, origin))
            .is_some_and(|(old, new)| {
                new.bytes <= old.bytes && accepted.iter().all(|&id| new.does_not_grow(old, id))
            })
        && image_size(&baseline, origin, runtime)
            .zip(image_size(&proposed, origin, runtime))
            .is_some_and(|(old, new)| new < old);
    if !final_ok {
        accepted.clear();
    }
    for id in affected {
        stats.record_many(
            id,
            if accepted.contains(&id) {
                "argument-staging-applied"
            } else {
                "argument-staging-declined-final-proof-or-cost"
            },
            stats.count_for(id, "argument-staging-selected"),
        );
    }
    maybe_report_peepholes(
        if accepted.is_empty() {
            &baseline
        } else {
            &proposed
        },
        &stats,
        config,
    );
    Ok(if accepted.is_empty() {
        baseline
    } else {
        proposed
    })
}

#[cfg(test)]
#[path = "argument_staging_tests.rs"]
mod tests;

mod x_forwarding;
pub(in crate::mir6502) use x_forwarding::select_costed as select_x_forwarding_costed;
