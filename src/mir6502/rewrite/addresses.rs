//! Fold exclusive captured address arithmetic, retaining every memory evaluation.
use std::collections::BTreeSet;

use super::context::{MirProof, PreHomeRewriteContext};
use super::plan::{MirChangeSet, MirEffectDelta, MirRemovedDefinition, MirRewritePlan};
use crate::mir6502::analysis::address_uses::{MirAddressBase, MirAddressUse};
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::*;

pub(in crate::mir6502) fn discover_address_displacements(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (end, op) in block.ops.iter().enumerate() {
            let Some(access) = MirAddressUse::from_op(op) else {
                continue;
            };
            let MirAddressBase::Captured(mut base) = access.base else {
                continue;
            };
            let mut displacement = access.displacement;
            let mut usage = MirSite::Op {
                block: block.id,
                op_index: end,
            };
            let mut removed = BTreeSet::new();
            loop {
                let MirValue::Def(MirDef::VTemp(temp)) = base else {
                    break;
                };
                let MirProof::Proven(definitions) =
                    context.exclusive_word_temp_definition(temp, usage)
                else {
                    break;
                };
                let site = definitions[0].site;
                let MirSite::Op {
                    block: id,
                    op_index: index,
                } = site
                else {
                    break;
                };
                if id != block.id || index >= end {
                    break;
                }
                let (next, delta) = match &block.ops[index] {
                    MirOp::Move {
                        src: MirValue::Def(MirDef::VTemp(root)),
                        width: MirWidth::Word,
                        ..
                    } => (MirValue::Def(MirDef::VTemp(*root)), 0),
                    MirOp::Binary {
                        op: MirBinaryOp::Add,
                        left: MirValue::Def(MirDef::VTemp(root)),
                        right: MirValue::ConstU16(delta),
                        width: MirWidth::Word,
                        carry_in: None | Some(MirCarryIn::Clear),
                        carry_out: MirCarryOut::Ignore,
                        ..
                    } => (MirValue::Def(MirDef::VTemp(*root)), *delta),
                    _ => break,
                };
                let flags = classify_op(&block.ops[index]).machine.flag_writes;
                if !context.flags_dead_after(flags, site) {
                    break;
                }
                // This first slice does not move address arithmetic across any
                // write, call, machine block or implicit machine-state input.
                if block.ops[index + 1..end].iter().any(|op| {
                    let effects = classify_op(op);
                    effects.memory.may_write_any
                        || effects.memory.has_unknown_effects
                        || !effects.memory.direct_writes.is_empty()
                        || !matches!(effects.memory.structured_writes, MirMemoryEffect::None)
                        || effects.machine.register_reads.a
                        || effects.machine.register_reads.x
                        || effects.machine.register_reads.y
                        || effects.machine.flag_reads.any()
                        || matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. })
                }) {
                    break;
                }
                let MirValue::Def(MirDef::VTemp(root)) = next else {
                    unreachable!()
                };
                let available = context.uses_at(root, site).into_iter().all(|use_site| {
                    [0, 1].into_iter().all(|byte| {
                        let lane = crate::mir6502::analysis::use_def::MirTempLane { temp: root, byte };
                        matches!(context.unique_reaching_definition(use_site, lane),
                            MirProof::Proven(definition) if context.value_available_at(definition,
                                context.point(MirSite::Op { block: block.id, op_index: end })).is_proven())
                    })
                });
                if !available {
                    break;
                }
                removed.insert(index);
                base = MirValue::Def(MirDef::VTemp(root));
                displacement = displacement.wrapping_add(delta);
                usage = site;
            }
            let Some(&start) = removed.first() else {
                continue;
            };
            let mut consumer = op.clone();
            let address = match &mut consumer {
                MirOp::Load { src, .. } => src,
                MirOp::Store { dst, .. } => dst,
                _ => unreachable!(),
            };
            *address = match access.index {
                Some((index, elem_size)) => MirAddr::ComputedIndex {
                    base,
                    index,
                    elem_size,
                    offset: displacement,
                },
                None => MirAddr::Deref {
                    ptr: base,
                    offset: displacement,
                },
            };
            let mut replacement: Vec<_> = (start..end)
                .filter(|i| !removed.contains(i))
                .map(|i| block.ops[i].clone())
                .collect();
            replacement.push(consumer);
            let Some(definitions) = super::pilots::prove_removed_window_definitions(
                block.id,
                &block.ops,
                start,
                end + 1,
                &replacement,
                context,
            ) else {
                continue;
            };
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: start..end + 1,
                replacement,
                removed_defs: definitions
                    .into_iter()
                    .map(|definition| MirRemovedDefinition { definition })
                    .collect(),
                exit_effect_delta: MirEffectDelta::MaterializedIndexConsumer,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: "captured-address-displacement",
                observations: vec![],
                family_priority: 15,
                estimated_byte_saving: 0,
                estimated_cycle_saving: 0,
            });
        }
    }
    plans
}

#[cfg(test)]
mod tests {
    use super::super::driver::MirPreHomeRewriteDriver;
    use super::*;
    fn value(id: u32) -> MirValue {
        MirValue::Def(MirDef::VTemp(MirTempId(id)))
    }
    fn routine() -> MirRoutine {
        MirRoutine {
            scalar_signature: None,
            inline: Default::default(),
            id: RoutineId(0),
            name: "address".into(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: vec![],
            effects: MirEffects::default(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".into(),
                params: vec![],
                terminator: MirTerminator::Return,
                ops: vec![
                    MirOp::Load {
                        dst: MirDef::VTemp(MirTempId(0)),
                        src: MirAddr::Direct(MirMem::Absolute(0x600)),
                        width: MirWidth::Word,
                    },
                    MirOp::Binary {
                        op: MirBinaryOp::Add,
                        dst: MirDef::VTemp(MirTempId(1)),
                        left: value(0),
                        right: MirValue::ConstU16(255),
                        width: MirWidth::Word,
                        carry_in: None,
                        carry_out: MirCarryOut::Ignore,
                    },
                    MirOp::Move {
                        dst: MirDef::VTemp(MirTempId(2)),
                        src: value(1),
                        width: MirWidth::Word,
                    },
                    MirOp::Load {
                        dst: MirDef::VTemp(MirTempId(3)),
                        src: MirAddr::ComputedIndex {
                            base: value(2),
                            index: MirValue::ConstU8(1),
                            elem_size: 1,
                            offset: 1,
                        },
                        width: MirWidth::Byte,
                    },
                ],
            }],
        }
    }
    fn run(r: &mut MirRoutine) -> usize {
        MirPreHomeRewriteDriver::default()
            .run_fixed_point_by_key(
                r,
                discover_address_displacements,
                super::super::pilots::pointer_rewrite_rank,
            )
            .unwrap()
            .applied
    }
    #[test]
    fn folds_captured_offsets_without_repeating_the_load() {
        let mut r = routine();
        assert_eq!(run(&mut r), 1);
        assert_eq!(r.blocks[0].ops.len(), 2);
        assert!(matches!(
            &r.blocks[0].ops[0],
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Absolute(0x600)),
                ..
            }
        ));
        assert!(
            matches!(&r.blocks[0].ops[1], MirOp::Load { src: MirAddr::ComputedIndex { base, offset: 256, .. }, .. } if *base == value(0))
        );
    }
    #[test]
    fn rejects_writes_multiple_users_and_live_carry() {
        for barrier in [
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x600)),
                src: MirValue::ConstU16(0),
                width: MirWidth::Word,
            },
            MirOp::Store {
                dst: MirAddr::Deref {
                    ptr: value(0),
                    offset: 0,
                },
                src: MirValue::ConstU8(0),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: MirDef::VTemp(MirTempId(4)),
                src: value(2),
                width: MirWidth::Word,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::VTemp(MirTempId(4)),
                left: MirValue::ConstU8(0),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            },
        ] {
            let mut r = routine();
            r.blocks[0].ops.insert(3, barrier);
            run(&mut r);
            // A harmless copy may disappear; the offset-producing add must stay.
            assert!(r.blocks[0].ops.iter().any(|op| matches!(
                op,
                MirOp::Binary {
                    dst: MirDef::VTemp(MirTempId(1)),
                    ..
                }
            )));
        }
    }
    #[test]
    fn rejects_escaped_addresses_and_source_redefinitions() {
        let mut r = routine();
        r.blocks[0].ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x602)),
            src: value(2),
            width: MirWidth::Word,
        });
        assert_eq!(run(&mut r), 0);
        let mut r = routine();
        r.blocks[0].ops.insert(
            3,
            MirOp::Move {
                dst: MirDef::VTemp(MirTempId(0)),
                src: MirValue::ConstU16(0),
                width: MirWidth::Word,
            },
        );
        run(&mut r);
        assert!(
            r.blocks[0]
                .ops
                .iter()
                .any(|op| matches!(op, MirOp::Binary { .. }))
        );
    }
}
