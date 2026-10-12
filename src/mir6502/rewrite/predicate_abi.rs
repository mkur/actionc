//! A small atomic routine transaction for a normalized predicate diamond.
use super::context::PreHomeRewriteContext;
use crate::mir6502::analysis::call_predicates::{physical_source, predicate_call};
use crate::mir6502::analysis::effects::MirFlagSet;
use crate::mir6502::analysis::sites::{MirProgramPoint, MirRoutineGeneration, MirSite};
use crate::mir6502::ir::*;
use crate::mir6502::materialize::MaterializeLayout;

#[derive(Debug, Clone)]
pub(in crate::mir6502) struct PredicateAbiPlan {
    pub generation: MirRoutineGeneration,
    original: MirRoutine,
    candidate: MirRoutine,
}

pub(in crate::mir6502) fn discover_predicate_abi(
    r: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    point: MirProgramPoint,
    layout: &MaterializeLayout,
) -> Option<PredicateAbiPlan> {
    if !context.valid_point(point) {
        return None;
    }
    let (temp, argument) = predicate_call(r, context, point.site)?;
    let MirSite::Op {
        block,
        op_index: call_index,
    } = point.site
    else {
        return None;
    };
    let index = r.blocks.iter().position(|b| b.id == block)?;
    let call = &r.blocks[index].ops[call_index];
    let MirOp::Call { args, .. } = call else {
        return None;
    };
    // This schedule stages WORD, WORD, BYTE. Register-carried Booleans and
    // other layouts retain their ordinary carrier until separately proven.
    if argument != 2
        || args.len() != 3
        || args[0].home
            != (MirArgHome::RegisterPair {
                lo: MirReg::A,
                hi: MirReg::X,
            })
        || args[1].home
            != (MirArgHome::BytePair {
                lo: Box::new(MirArgHome::Reg(MirReg::Y)),
                hi: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA3))),
            })
        || args[2].home != MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA4))
    {
        return None;
    }
    for arg in &args[..2] {
        let MirValue::Word { lo, .. } = &arg.value else {
            return None;
        };
        let MirValue::PointerCell(source) = lo.as_ref() else {
            return None;
        };
        if arg.width != MirWidth::Word
            || super::call_predicates::source_value(source, MirWidth::Word).as_ref()
                != Some(&arg.value)
            || !physical_source(r, layout, source, MirWidth::Word, call)
        {
            return None;
        }
    }
    let compare_site = MirSite::Op {
        block,
        op_index: call_index - 1,
    };
    if !context.flags_dead_after(MirFlagSet::all(), compare_site)
        || !context
            .register_dead_after(MirReg::A, context.point(compare_site))
            .is_proven()
    {
        return None;
    }
    let mut candidate = r.clone();
    let next = candidate
        .blocks
        .iter()
        .map(|b| b.id.0)
        .max()?
        .checked_add(1)?;
    let yes = MirBlockId(next);
    let no = MirBlockId(next.checked_add(1)?);
    let join = MirBlockId(next.checked_add(2)?);
    let mut tail = candidate.blocks[index].ops.split_off(call_index);
    let compare = candidate.blocks[index].ops.pop()?;
    let MirOp::Compare {
        op,
        left,
        right,
        width: MirWidth::Byte,
        signed: false,
        ..
    } = &compare
    else {
        return None;
    };
    let old_terminator = candidate.blocks[index].terminator.clone();
    candidate.blocks[index].terminator =
        crate::mir6502::materialize::compare_branch::materialize_byte_compare_branch(
            &mut candidate.blocks[index].ops,
            *op,
            left.clone(),
            right.clone(),
            yes,
            no,
        );
    let slot = MirMem::FixedZeroPage(MirFixedZpSlot(0xA4));
    let MirOp::Call { args, .. } = &mut tail[0] else {
        unreachable!()
    };
    args[argument].value = MirValue::PointerCell(slot.clone());
    for (id, value) in [(yes, 1), (no, 0)] {
        candidate.blocks.push(MirBlock {
            id,
            label: format!("cmp_arg_{}", id.0),
            params: vec![],
            ops: vec![MirOp::Store {
                dst: MirAddr::Direct(slot.clone()),
                src: MirValue::ConstU8(value),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge {
                target: join,
                args: vec![],
            }),
        });
    }
    candidate.blocks.push(MirBlock {
        id: join,
        label: format!("cmp_arg_join_{}", join.0),
        params: vec![],
        ops: tail,
        terminator: old_terminator,
    });
    if !diamond_delta_is_justified(r, &candidate, index, call_index, temp) {
        return None;
    }
    Some(PredicateAbiPlan {
        generation: context.generation(),
        original: r.clone(),
        candidate,
    })
}

/// Undo the certified three-block diamond and require exact equality with the
/// original routine. Both paths define 0/1, each reaches the single retained
/// call, and no foreign block, incoming edge, call effect or operand is changed.
fn diamond_delta_is_justified(
    original: &MirRoutine,
    candidate: &MirRoutine,
    index: usize,
    call_index: usize,
    temp: MirTempId,
) -> bool {
    let n = original.blocks.len();
    if candidate.blocks.len() != n + 3 {
        return false;
    }
    let mut restored = candidate.clone();
    let yes = &candidate.blocks[n];
    let no = &candidate.blocks[n + 1];
    let join = &candidate.blocks[n + 2];
    let slot = MirMem::FixedZeroPage(MirFixedZpSlot(0xA4));
    for (branch, value) in [(yes, 1), (no, 0)] {
        if !branch.params.is_empty()
            || branch.ops
                != [MirOp::Store {
                    dst: MirAddr::Direct(slot.clone()),
                    src: MirValue::ConstU8(value),
                    width: MirWidth::Byte,
                }]
            || branch.terminator
                != MirTerminator::Jump(MirEdge {
                    target: join.id,
                    args: vec![],
                })
        {
            return false;
        }
    }
    if !join.params.is_empty() || join.terminator != original.blocks[index].terminator {
        return false;
    }
    let mut tail = join.ops.clone();
    let Some(MirOp::Call { args, .. }) = tail.first_mut() else {
        return false;
    };
    if args.len() != 3 || args[2].value != MirValue::PointerCell(slot) {
        return false;
    }
    args[2].value = MirValue::Def(MirDef::VTemp(temp));
    if tail != original.blocks[index].ops[call_index..] {
        return false;
    }
    let original_compare = &original.blocks[index].ops[call_index - 1];
    let MirOp::Compare { op, .. } = original_compare else {
        return false;
    };
    let mut flags_compare = original_compare.clone();
    let MirOp::Compare { dst, .. } = &mut flags_compare else {
        unreachable!()
    };
    *dst = MirCondDest::Flags;
    let prefix = &candidate.blocks[index];
    if prefix.ops.last() != Some(&flags_compare)
        || prefix.ops[..prefix.ops.len() - 1] != original.blocks[index].ops[..call_index - 1]
        || prefix.terminator
            != (MirTerminator::Branch {
                cond: MirCond::FlagTest(if *op == MirCompareOp::Ne {
                    MirFlagTest::ZClear
                } else {
                    MirFlagTest::ZSet
                }),
                then_edge: MirEdge {
                    target: yes.id,
                    args: vec![],
                },
                else_edge: MirEdge {
                    target: no.id,
                    args: vec![],
                },
            })
    {
        return false;
    }
    restored.blocks.truncate(n);
    restored.blocks[index] = original.blocks[index].clone();
    restored == *original
}

pub(in crate::mir6502) fn apply_predicate_abi(
    routine: &mut MirRoutine,
    current_generation: MirRoutineGeneration,
    plan: PredicateAbiPlan,
) -> bool {
    if current_generation != plan.generation
        || *routine != plan.original
        || crate::mir6502::analysis::cfg::MirCfg::from_routine(&plan.candidate).is_err()
    {
        return false;
    }
    *routine = plan.candidate;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
    use crate::mir6502::rewrite::driver::MirPreHomeRewriteDriver;

    fn selected() -> MirProgram {
        let mut p = crate::mir6502::analysis::call_predicates::tests::program();
        let layout = MaterializeLayout::new(&p, 0x3000);
        MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut p.routines[1], |r, c| {
                super::super::call_predicates::discover_argument_snapshots(r, c, &layout)
            })
            .unwrap();
        p
    }

    #[test]
    fn call_predicate_abi_transaction_checks_generation_exact_diamond_and_both_paths() {
        let p = selected();
        let r = &p.routines[1];
        let layout = MaterializeLayout::new(&p, 0x3000);
        let g = MirRoutineGeneration::initial();
        let s = PreHomeAnalysisSnapshot::new(r, g).unwrap();
        let c = PreHomeRewriteContext::new(&s);
        let b = &r.blocks[0];
        let i = b
            .ops
            .iter()
            .position(|op| matches!(op, MirOp::Call { .. }))
            .unwrap();
        let point = c.point(MirSite::Op {
            block: b.id,
            op_index: i,
        });
        let plan =
            discover_predicate_abi(r, &c, point, &layout).expect("admitted fixed-home predicate");
        let mut stale = r.clone();
        assert!(!apply_predicate_abi(&mut stale, g.next(), plan.clone()));
        assert_eq!(stale, *r);
        let mut wrong = plan.candidate.clone();
        let MirOp::Store { src, .. } = &mut wrong.blocks[r.blocks.len()].ops[0] else {
            unreachable!()
        };
        *src = MirValue::ConstU8(2);
        assert!(!diamond_delta_is_justified(r, &wrong, 0, i, MirTempId(3)));
        let mut applied = r.clone();
        assert!(apply_predicate_abi(&mut applied, g, plan));
        assert_eq!(
            applied
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|op| matches!(op, MirOp::Call { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn call_predicate_abi_rejects_extra_boolean_uses_and_slot_aliases() {
        for extra_use in [true, false] {
            let mut p = selected();
            let r = &mut p.routines[1];
            let b = &mut r.blocks[0];
            if extra_use {
                b.ops.push(MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x800)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                    width: MirWidth::Byte,
                });
            } else {
                let MirOp::Call { args, .. } = b
                    .ops
                    .iter_mut()
                    .find(|op| matches!(op, MirOp::Call { .. }))
                    .unwrap()
                else {
                    unreachable!()
                };
                args[0].value = MirValue::Word {
                    lo: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
                        MirFixedZpSlot(0xA4),
                    ))),
                    hi: Box::new(MirValue::ConstU8(0)),
                };
            }
            let r = &p.routines[1];
            let s = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
            let c = PreHomeRewriteContext::new(&s);
            let i = r.blocks[0]
                .ops
                .iter()
                .position(|op| matches!(op, MirOp::Call { .. }))
                .unwrap();
            assert!(
                discover_predicate_abi(
                    r,
                    &c,
                    c.point(MirSite::Op {
                        block: r.blocks[0].id,
                        op_index: i
                    }),
                    &MaterializeLayout::new(&p, 0x3000)
                )
                .is_none()
            );
        }
    }

    #[test]
    fn call_predicate_abi_rejects_live_machine_state_and_other_argument_homes() {
        for live_machine in [true, false] {
            let mut p = selected();
            let r = &mut p.routines[1];
            let call = r.blocks[0]
                .ops
                .iter_mut()
                .find(|op| matches!(op, MirOp::Call { .. }))
                .unwrap();
            let MirOp::Call {
                args, abi, effects, ..
            } = call
            else {
                unreachable!()
            };
            if live_machine {
                abi.clobbers.a = false;
                abi.preserves.a = true;
                effects.clobbers.a = false;
                effects.preserves.a = true;
                r.blocks[0].ops.push(MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x800)),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                });
            } else {
                args[2].home = MirArgHome::Reg(MirReg::Y);
            }
            let r = &p.routines[1];
            let s = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
            let c = PreHomeRewriteContext::new(&s);
            let i = r.blocks[0]
                .ops
                .iter()
                .position(|op| matches!(op, MirOp::Call { .. }))
                .unwrap();
            assert!(
                discover_predicate_abi(
                    r,
                    &c,
                    c.point(MirSite::Op {
                        block: r.blocks[0].id,
                        op_index: i
                    }),
                    &MaterializeLayout::new(&p, 0x3000)
                )
                .is_none()
            );
        }
    }
}
