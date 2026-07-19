use crate::mir6502::diagnostics::MirDiagnostic;
use crate::mir6502::ir::{
    MirBlock, MirBlockId, MirDef, MirEdge, MirOp, MirRoutine, MirTemp, MirTempId, MirTerminator,
    MirValue, MirWidth,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParallelCopy {
    dst: MirTempId,
    src: MirValue,
    width: MirWidth,
}

pub(super) fn lower_block_arguments(routine: &mut MirRoutine) -> Result<(), MirDiagnostic> {
    if !routine.blocks.iter().any(|block| {
        !block.params.is_empty()
            || terminator_edges(&block.terminator).any(|edge| !edge.args.is_empty())
    }) {
        return Ok(());
    }

    let params = routine
        .blocks
        .iter()
        .map(|block| (block.id, block.params.clone()))
        .collect::<BTreeMap<_, _>>();
    split_conditional_argument_edges(routine)?;

    let mut next_temp = routine
        .temps
        .iter()
        .map(|temp| temp.id.0)
        .max()
        .map_or(Ok(0), |id| {
            id.checked_add(1)
                .ok_or_else(|| MirDiagnostic::routine(&routine.name, "MIR temp id space exhausted"))
        })?;

    for block in &mut routine.blocks {
        let MirTerminator::Jump(edge) = &mut block.terminator else {
            continue;
        };
        if edge.args.is_empty() {
            continue;
        }
        let Some(target_params) = params.get(&edge.target) else {
            return Err(MirDiagnostic::block(
                &routine.name,
                &block.label,
                format!("block-argument target `b{}` does not exist", edge.target.0),
            ));
        };
        if edge.args.len() != target_params.len() {
            return Err(MirDiagnostic::block(
                &routine.name,
                &block.label,
                format!(
                    "block-argument edge supplies {} argument(s), expected {}",
                    edge.args.len(),
                    target_params.len()
                ),
            ));
        }

        let copies = target_params
            .iter()
            .zip(std::mem::take(&mut edge.args))
            .map(|(param, arg)| ParallelCopy {
                dst: param.dest,
                src: arg.value,
                width: param.width,
            })
            .collect();
        block.ops.extend(resolve_parallel_copies(
            copies,
            &mut routine.temps,
            &mut next_temp,
            &routine.name,
        )?);
    }

    for block in &mut routine.blocks {
        block.params.clear();
    }
    Ok(())
}

fn split_conditional_argument_edges(routine: &mut MirRoutine) -> Result<(), MirDiagnostic> {
    let mut next_block =
        routine
            .blocks
            .iter()
            .map(|block| block.id.0)
            .max()
            .map_or(Ok(0), |id| {
                id.checked_add(1).ok_or_else(|| {
                    MirDiagnostic::routine(&routine.name, "MIR block id space exhausted")
                })
            })?;
    let mut blocks = Vec::with_capacity(routine.blocks.len());
    for mut block in std::mem::take(&mut routine.blocks) {
        let mut split_blocks = Vec::new();
        if let MirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } = &mut block.terminator
        {
            split_branch_edge(
                then_edge,
                "then",
                &block.label,
                &mut next_block,
                &mut split_blocks,
                &routine.name,
            )?;
            split_branch_edge(
                else_edge,
                "else",
                &block.label,
                &mut next_block,
                &mut split_blocks,
                &routine.name,
            )?;
        }
        blocks.push(block);
        blocks.extend(split_blocks);
    }
    routine.blocks = blocks;
    Ok(())
}

fn split_branch_edge(
    edge: &mut MirEdge,
    arm: &str,
    source_label: &str,
    next_block: &mut u32,
    blocks: &mut Vec<MirBlock>,
    routine_name: &str,
) -> Result<(), MirDiagnostic> {
    if edge.args.is_empty() {
        return Ok(());
    }
    let id = MirBlockId(*next_block);
    *next_block = next_block
        .checked_add(1)
        .ok_or_else(|| MirDiagnostic::routine(routine_name, "MIR block id space exhausted"))?;
    let target_edge = std::mem::replace(edge, MirEdge::plain(id));
    blocks.push(MirBlock {
        id,
        label: format!("{source_label}.{arm}_args_{}", id.0),
        params: Vec::new(),
        ops: Vec::new(),
        terminator: MirTerminator::Jump(target_edge),
    });
    Ok(())
}

fn resolve_parallel_copies(
    mut pending: Vec<ParallelCopy>,
    temps: &mut Vec<MirTemp>,
    next_temp: &mut u32,
    routine_name: &str,
) -> Result<Vec<MirOp>, MirDiagnostic> {
    pending.retain(|copy| !is_identity_copy(copy));
    let mut ops = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        if let Some(index) = pending.iter().position(|copy| {
            !pending
                .iter()
                .any(|candidate| value_references_temp(&candidate.src, copy.dst))
        }) {
            let copy = pending.remove(index);
            ops.push(copy_op(copy));
            continue;
        }

        let cycle_temp = pending[0].dst;
        let cycle_width = pending[0].width;
        let scratch = MirTempId(*next_temp);
        *next_temp = next_temp
            .checked_add(1)
            .ok_or_else(|| MirDiagnostic::routine(routine_name, "MIR temp id space exhausted"))?;
        temps.push(MirTemp { id: scratch });
        ops.push(MirOp::Move {
            dst: MirDef::VTemp(scratch),
            src: MirValue::Def(MirDef::VTemp(cycle_temp)),
            width: cycle_width,
        });
        for copy in &mut pending {
            replace_temp(&mut copy.src, cycle_temp, scratch);
        }
    }
    Ok(ops)
}

fn copy_op(copy: ParallelCopy) -> MirOp {
    MirOp::Move {
        dst: MirDef::VTemp(copy.dst),
        src: copy.src,
        width: copy.width,
    }
}

fn is_identity_copy(copy: &ParallelCopy) -> bool {
    matches!(copy.src, MirValue::Def(MirDef::VTemp(id)) if id == copy.dst)
}

fn value_references_temp(value: &MirValue, temp: MirTempId) -> bool {
    match value {
        MirValue::Def(MirDef::VTemp(id) | MirDef::VTempByte { id, .. }) => *id == temp,
        MirValue::Word { lo, hi } => {
            value_references_temp(lo, temp) || value_references_temp(hi, temp)
        }
        _ => false,
    }
}

fn replace_temp(value: &mut MirValue, old: MirTempId, new: MirTempId) {
    match value {
        MirValue::Def(MirDef::VTemp(id)) if *id == old => *id = new,
        MirValue::Def(MirDef::VTempByte { id, .. }) if *id == old => *id = new,
        MirValue::Word { lo, hi } => {
            replace_temp(lo, old, new);
            replace_temp(hi, old, new);
        }
        _ => {}
    }
}

fn terminator_edges(terminator: &MirTerminator) -> impl Iterator<Item = &MirEdge> {
    let edges = match terminator {
        MirTerminator::Jump(edge) => [Some(edge), None],
        MirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => [Some(then_edge), Some(else_edge)],
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => [None, None],
    };
    edges.into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirAddr, MirBlockParam, MirCond, MirEffects, MirFrame, MirMem, MirPhase, MirProgram,
        MirRoutineAbi, MirValue,
    };

    fn routine(blocks: Vec<MirBlock>, temp_count: u32) -> MirRoutine {
        MirRoutine {
            id: crate::mir6502::ir::RoutineId(0),
            name: "Merge".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: (0..temp_count)
                .map(|id| MirTemp { id: MirTempId(id) })
                .collect(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn param(id: u32, width: MirWidth) -> MirBlockParam {
        MirBlockParam {
            dest: MirTempId(id),
            width,
        }
    }

    fn arg(value: MirValue, width: MirWidth) -> crate::mir6502::ir::MirEdgeArg {
        crate::mir6502::ir::MirEdgeArg { value, width }
    }

    #[test]
    fn conditional_argument_copy_gets_its_own_edge_block() {
        let mut routine = routine(
            vec![
                MirBlock {
                    id: MirBlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Branch {
                        cond: MirCond::BoolValue(MirValue::ConstU8(1)),
                        then_edge: MirEdge {
                            target: MirBlockId(1),
                            args: vec![arg(MirValue::ConstU8(7), MirWidth::Byte)],
                        },
                        else_edge: MirEdge::plain(MirBlockId(2)),
                    },
                },
                MirBlock {
                    id: MirBlockId(1),
                    label: "then".to_string(),
                    params: vec![param(0, MirWidth::Byte)],
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                },
                MirBlock {
                    id: MirBlockId(2),
                    label: "else".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                },
            ],
            1,
        );

        lower_block_arguments(&mut routine).unwrap();

        let MirTerminator::Branch { then_edge, .. } = &routine.blocks[0].terminator else {
            panic!("entry branch");
        };
        assert_eq!(then_edge, &MirEdge::plain(MirBlockId(3)));
        assert_eq!(
            routine.blocks[1].ops,
            vec![MirOp::Move {
                dst: MirDef::VTemp(MirTempId(0)),
                src: MirValue::ConstU8(7),
                width: MirWidth::Byte,
            }]
        );
        assert_eq!(
            routine.blocks[1].terminator,
            MirTerminator::Jump(MirEdge::plain(MirBlockId(1)))
        );
        assert!(routine.blocks.iter().all(|block| block.params.is_empty()));
    }

    #[test]
    fn parallel_swap_uses_target_managed_scratch() {
        let mut routine = routine(
            vec![MirBlock {
                id: MirBlockId(0),
                label: "loop".to_string(),
                params: vec![param(0, MirWidth::Byte), param(1, MirWidth::Byte)],
                ops: Vec::new(),
                terminator: MirTerminator::Jump(MirEdge {
                    target: MirBlockId(0),
                    args: vec![
                        arg(MirValue::Def(MirDef::VTemp(MirTempId(1))), MirWidth::Byte),
                        arg(MirValue::Def(MirDef::VTemp(MirTempId(0))), MirWidth::Byte),
                    ],
                }),
            }],
            2,
        );

        lower_block_arguments(&mut routine).unwrap();

        assert_eq!(routine.temps.last(), Some(&MirTemp { id: MirTempId(2) }));
        assert_eq!(
            routine.blocks[0].ops,
            vec![
                MirOp::Move {
                    dst: MirDef::VTemp(MirTempId(2)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                    width: MirWidth::Byte,
                },
                MirOp::Move {
                    dst: MirDef::VTemp(MirTempId(0)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    width: MirWidth::Byte,
                },
                MirOp::Move {
                    dst: MirDef::VTemp(MirTempId(1)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                    width: MirWidth::Byte,
                },
            ]
        );
    }

    #[test]
    fn word_and_pointer_values_remain_typed_parallel_copies() {
        let mut routine = routine(
            vec![
                MirBlock {
                    id: MirBlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Jump(MirEdge {
                        target: MirBlockId(1),
                        args: vec![
                            arg(MirValue::ConstU16(0x1234), MirWidth::Word),
                            arg(
                                MirValue::StaticAddr(crate::nir::SymbolId(4)),
                                MirWidth::Word,
                            ),
                        ],
                    }),
                },
                MirBlock {
                    id: MirBlockId(1),
                    label: "join".to_string(),
                    params: vec![param(0, MirWidth::Word), param(1, MirWidth::Word)],
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                },
            ],
            2,
        );

        lower_block_arguments(&mut routine).unwrap();

        assert_eq!(
            routine.blocks[0].ops,
            vec![
                MirOp::Move {
                    dst: MirDef::VTemp(MirTempId(0)),
                    src: MirValue::ConstU16(0x1234),
                    width: MirWidth::Word,
                },
                MirOp::Move {
                    dst: MirDef::VTemp(MirTempId(1)),
                    src: MirValue::StaticAddr(crate::nir::SymbolId(4)),
                    width: MirWidth::Word,
                },
            ]
        );
    }

    #[test]
    fn materialization_preserves_a_temp_used_only_by_a_conditional_edge() {
        let routine = routine(
            vec![
                MirBlock {
                    id: MirBlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: vec![MirOp::Move {
                        dst: MirDef::VTemp(MirTempId(0)),
                        src: MirValue::ConstU8(9),
                        width: MirWidth::Byte,
                    }],
                    terminator: MirTerminator::Branch {
                        cond: MirCond::BoolValue(MirValue::ConstU8(1)),
                        then_edge: MirEdge {
                            target: MirBlockId(1),
                            args: vec![arg(
                                MirValue::Def(MirDef::VTemp(MirTempId(0))),
                                MirWidth::Byte,
                            )],
                        },
                        else_edge: MirEdge::plain(MirBlockId(2)),
                    },
                },
                MirBlock {
                    id: MirBlockId(1),
                    label: "then".to_string(),
                    params: vec![param(1, MirWidth::Byte)],
                    ops: vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x4000)),
                        src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    }],
                    terminator: MirTerminator::Return,
                },
                MirBlock {
                    id: MirBlockId(2),
                    label: "else".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                },
            ],
            2,
        );
        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![routine],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        let materialized =
            crate::mir6502::materialize_program(program, &crate::mir6502::Mir6502Config::default())
                .expect("materialize conditional block argument");
        crate::mir6502::verify_program(&materialized, MirPhase::PreEmission)
            .expect("block argument materialization is emission-ready");
        assert!(materialized.routines[0].blocks.iter().any(|block| {
            block.ops.iter().any(|op| {
                matches!(
                    op,
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x4000)),
                        ..
                    }
                )
            })
        }));
    }
}
