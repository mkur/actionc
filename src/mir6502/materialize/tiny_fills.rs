//! Cost-bounded selection of closed, constant byte fills before home allocation.
//!
//! This recognizes typed MIR computation, not variant constructors or source
//! names. Only a fixed window of an ordinary local object is eligible. The
//! pointer and word counter must have no uses outside the matched computation.

use std::collections::BTreeSet;

use crate::mir6502::analysis::{
    cfg::MirCfg,
    effects::{classify_op, classify_terminator},
};
use crate::mir6502::ir::*;
use crate::nir::LocalId;

const MAX_BYTES: u16 = 8;

struct Plan {
    preheader: usize,
    first_op: usize,
    header: MirBlockId,
    body: MirBlockId,
    exit: MirBlockId,
    target: LocalId,
    start: u16,
    end: u16,
    value: u8,
    controls: [LocalId; 2],
}

pub(super) fn select(program: &mut MirProgram) -> Vec<(RoutineId, usize)> {
    let mut selected = Vec::new();
    for index in 0..program.routines.len() {
        let mut count = 0;
        while let Some(plan) = discover(program, index) {
            let routine = &mut program.routines[index];
            let preheader = &mut routine.blocks[plan.preheader];
            preheader.ops.truncate(plan.first_op);
            preheader
                .ops
                .extend((plan.start..=plan.end).map(|offset| MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Local {
                        id: plan.target,
                        offset,
                    }),
                    src: MirValue::ConstU8(plan.value),
                    width: MirWidth::Byte,
                }));
            preheader.terminator = MirTerminator::Jump(MirEdge::plain(plan.exit));
            routine
                .blocks
                .retain(|b| b.id != plan.header && b.id != plan.body);
            routine.frame.locals.retain(|slot| {
                !matches!(slot.base, MirStorageBase::Local(id) if plan.controls.contains(&id))
            });
            count += 1;
        }
        if count != 0 {
            selected.push((program.routines[index].id, count));
        }
    }
    selected
}

fn discover(program: &MirProgram, routine_index: usize) -> Option<Plan> {
    let r = &program.routines[routine_index];
    if !matches!(r.abi, MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry) {
        return None;
    }
    let cfg = MirCfg::from_routine(r).ok()?;
    // Machine code can refer to frame cells or observe machine state that is
    // not part of this logical fill contract. Keep its original computation.
    if r.blocks
        .iter()
        .flat_map(|b| &b.ops)
        .any(|op| matches!(op, MirOp::MachineBlock { .. }))
    {
        return None;
    }
    for (index, preheader) in r.blocks.iter().enumerate() {
        if let Some(plan) = candidate(program, r, &cfg, index, preheader) {
            return Some(plan);
        }
    }
    None
}

fn candidate(
    program: &MirProgram,
    r: &MirRoutine,
    cfg: &MirCfg,
    preheader_index: usize,
    preheader: &MirBlock,
) -> Option<Plan> {
    let first = preheader.ops.len().checked_sub(4)?;
    let [
        MirOp::LeaAddr {
            dst: MirDef::VTemp(address),
            target: MirMem::Local { id: target, offset },
            width: MirWidth::Word,
        },
        MirOp::Move {
            dst: MirDef::VTemp(pointer),
            src: MirValue::Def(MirDef::VTemp(moved)),
            width: MirWidth::Word,
        },
        MirOp::Store {
            dst:
                MirAddr::Direct(MirMem::Local {
                    id: pointer_cell,
                    offset: 0,
                }),
            src: MirValue::Def(MirDef::VTemp(stored)),
            width: MirWidth::Word,
        },
        MirOp::Store {
            dst:
                MirAddr::Direct(MirMem::Local {
                    id: counter,
                    offset: 0,
                }),
            src: initial,
            width: MirWidth::Word,
        },
    ] = &preheader.ops[first..]
    else {
        return None;
    };
    if moved != address
        || stored != pointer
        || target == pointer_cell
        || target == counter
        || pointer_cell == counter
    {
        return None;
    }
    let initial = constant(initial)?;
    let header = block(r, jump(&preheader.terminator)?)?;
    let [
        MirOp::Load {
            dst: MirDef::VTemp(index),
            src:
                MirAddr::Direct(MirMem::Local {
                    id: loaded_counter,
                    offset: 0,
                }),
            width: MirWidth::Word,
        },
        MirOp::Compare {
            dst: MirCondDest::Temp(condition),
            op: MirCompareOp::Le,
            left: MirValue::Def(MirDef::VTemp(compared)),
            right: bound,
            width: MirWidth::Word,
            signed: false,
        },
    ] = header.ops.as_slice()
    else {
        return None;
    };
    let end = constant(bound)?;
    let count = end.checked_sub(initial)?.checked_add(1)?;
    if loaded_counter != counter
        || compared != index
        || !(1..=MAX_BYTES).contains(&count)
        || end == u16::MAX
    {
        return None;
    }
    let MirTerminator::Branch {
        cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(tested))),
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return None;
    };
    if tested != condition || !then_edge.args.is_empty() || !else_edge.args.is_empty() {
        return None;
    }
    let body = block(r, then_edge.target)?;
    let [
        MirOp::Load {
            dst: MirDef::VTemp(base),
            src:
                MirAddr::Direct(MirMem::Local {
                    id: loaded_pointer,
                    offset: 0,
                }),
            width: MirWidth::Word,
        },
        MirOp::Load {
            dst: MirDef::VTemp(subscript),
            src:
                MirAddr::Direct(MirMem::Local {
                    id: indexed_counter,
                    offset: 0,
                }),
            width: MirWidth::Word,
        },
        MirOp::Store {
            dst:
                MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(used_base)),
                    index: MirValue::Def(MirDef::VTemp(used_index)),
                    elem_size: 1,
                    offset: displacement,
                },
            src: fill,
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::VTemp(previous),
            src:
                MirAddr::Direct(MirMem::Local {
                    id: incremented_counter,
                    offset: 0,
                }),
            width: MirWidth::Word,
        },
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::VTemp(next),
            left: MirValue::Def(MirDef::VTemp(added)),
            right: step,
            width: MirWidth::Word,
            carry_in: None | Some(MirCarryIn::Clear),
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Store {
            dst:
                MirAddr::Direct(MirMem::Local {
                    id: updated_counter,
                    offset: 0,
                }),
            src: MirValue::Def(MirDef::VTemp(updated)),
            width: MirWidth::Word,
        },
    ] = body.ops.as_slice()
    else {
        return None;
    };
    if loaded_pointer != pointer_cell
        || indexed_counter != counter
        || incremented_counter != counter
        || updated_counter != counter
        || used_base != base
        || used_index != subscript
        || added != previous
        || updated != next
        || constant(step)? != 1
        || jump(&body.terminator)? != header.id
        || cfg.predecessors(header.id) != &BTreeSet::from([preheader.id, body.id])
        || cfg.predecessors(body.id) != &BTreeSet::from([header.id])
        || [preheader.id, header.id, body.id].contains(&else_edge.target)
        || r.blocks
            .first()
            .is_some_and(|b| b.id == header.id || b.id == body.id)
        || !header.params.is_empty()
        || !body.params.is_empty()
    {
        return None;
    }
    let value = u8::try_from(constant(fill)?).ok()?;
    let start = offset.checked_add(*displacement)?.checked_add(initial)?;
    let end = start.checked_add(count - 1)?;
    let target_slot = local(r, *target)?;
    if !target_slot.mutable
        || end >= target_slot.storage_size
        || matches!(
            target_slot.init,
            Some(MirStorageInit::Descriptor { .. } | MirStorageInit::RoutineAddress { .. })
        )
    {
        return None;
    }
    let controls = [*pointer_cell, *counter];
    for id in controls {
        let slot = local(r, id)?;
        if slot.storage_size != 2 || slot.init.is_some() || exposed_in_data(program, r.id) {
            return None;
        }
    }
    if r.frame
        .locals
        .iter()
        .any(|slot| matches!(slot.base, MirStorageBase::LocalAlias { .. }))
    {
        return None;
    }
    // All definitions removed by this transaction must have their uses inside
    // it. Likewise, control cells must not escape through addresses, aliases,
    // effect regions or executable references elsewhere in the routine.
    let removed = |b: MirBlockId, i: usize| {
        b == header.id || b == body.id || (b == preheader.id && i >= first)
    };
    let definitions = r
        .blocks
        .iter()
        .flat_map(|b| {
            b.ops
                .iter()
                .enumerate()
                .filter_map(|(i, op)| removed(b.id, i).then_some(op))
        })
        .flat_map(|op| {
            classify_op(op)
                .logical
                .temp_defs
                .into_iter()
                .map(|a| a.temp())
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    let temps = definitions.iter().copied().collect::<BTreeSet<_>>();
    if temps.len() != definitions.len() {
        return None;
    }
    for b in &r.blocks {
        if b.params.iter().any(|p| temps.contains(&p.dest)) {
            return None;
        }
        for (i, op) in b.ops.iter().enumerate() {
            if removed(b.id, i) {
                continue;
            }
            let effects = classify_op(op);
            if effects.logical.temp_uses.iter().any(|a| temps.contains(&a.temp()))
                || effects.logical.temp_defs.iter().any(|a| temps.contains(&a.temp()))
                || effects.memory.direct_references.iter().any(|m| control(m, &controls))
                // An escaped neighboring home can reach these cells through
                // pointer arithmetic. The matched address has only bounded
                // in-object uses; all other frame-address exposure is blocked.
                || effects.memory.address_references.iter().any(|m| matches!(m, MirMem::Local { .. } | MirMem::Param { .. }))
                || effects.memory.direct_reads.iter().chain(&effects.memory.direct_writes).any(|a| control(&a.base, &controls))
                || [&effects.memory.structured_reads, &effects.memory.structured_writes].iter().any(|effect| {
                    matches!(effect, MirMemoryEffect::Regions(regions) if regions.iter().any(|region| matches!(region.kind, MirMemoryRegionKind::Local(id) if controls.contains(&id))))
                })
            { return None; }
        }
        if b.id != header.id {
            let effects = classify_terminator(&b.terminator);
            if effects
                .logical
                .temp_uses
                .iter()
                .any(|a| temps.contains(&a.temp()))
                || effects
                    .memory
                    .direct_references
                    .iter()
                    .any(|m| control(m, &controls))
                || effects
                    .memory
                    .address_references
                    .iter()
                    .any(|m| matches!(m, MirMem::Local { .. } | MirMem::Param { .. }))
            {
                return None;
            }
        }
    }
    // At most one immediate load plus eight absolute stores (26 bytes). The
    // matched word-counter loop needs at least 30 bytes even before pointer
    // arithmetic and its back edge; use this conservative size-only budget.
    if 2 + 3 * count >= 30 {
        return None;
    }
    Some(Plan {
        preheader: preheader_index,
        first_op: first,
        header: header.id,
        body: body.id,
        exit: else_edge.target,
        target: *target,
        start,
        end,
        value,
        controls,
    })
}

fn local(r: &MirRoutine, id: LocalId) -> Option<&MirStorageSlot> {
    r.frame
        .locals
        .iter()
        .find(|slot| slot.base == MirStorageBase::Local(id) && slot.offset == 0)
}
fn control(mem: &MirMem, ids: &[LocalId; 2]) -> bool {
    matches!(mem, MirMem::Local { id, .. } if ids.contains(id))
}
fn constant(value: &MirValue) -> Option<u16> {
    match value {
        MirValue::ConstU8(n) => Some(u16::from(*n)),
        MirValue::ConstU16(n) => Some(*n),
        _ => None,
    }
}
fn block(r: &MirRoutine, id: MirBlockId) -> Option<&MirBlock> {
    r.blocks.iter().find(|b| b.id == id)
}
fn jump(t: &MirTerminator) -> Option<MirBlockId> {
    match t {
        MirTerminator::Jump(edge) if edge.args.is_empty() => Some(edge.target),
        _ => None,
    }
}

fn exposed_in_data(program: &MirProgram, routine: RoutineId) -> bool {
    let image = |image: &MirDataImage| {
        image
            .relocations
            .iter()
            .any(|r| matches!(r.target, MirDataRelocationTarget::Local { routine: owner, .. } | MirDataRelocationTarget::Param { routine: owner, .. } if owner == routine))
    };
    let init = |init: &MirStorageInit| match init {
        MirStorageInit::Bytes { image: data, .. } => image(data),
        MirStorageInit::Descriptor { backing, .. } => image(&backing.image),
        _ => false,
    };
    program.statics.iter().any(|s| image(&s.image))
        || program.globals.iter().any(|g| match &g.init {
            Some(MirGlobalInit::Bytes { image: data, .. }) => image(data),
            Some(MirGlobalInit::Descriptor { backing, .. }) => image(&backing.image),
            _ => false,
        })
        || program
            .routines
            .iter()
            .flat_map(|r| r.frame.params.iter().chain(&r.frame.locals))
            .any(|s| s.init.as_ref().is_some_and(init))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        nir,
        semantic::{self, SemanticOptions},
    };

    fn lower(source: &str, optimized: bool) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
        let raw = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        let nir = if optimized {
            nir::optimize_program(&raw).unwrap()
        } else {
            raw
        };
        crate::mir6502::lower_program(&nir).unwrap()
    }

    fn variant(bytes: u16, optimized: bool) -> MirProgram {
        lower(
            &format!(
                "TYPE Payload=[BYTE ARRAY bytes({bytes})]\nTYPE Choice=VARIANT [EMPTY FULL [Payload payload]]\nChoice result\nChoice FUNC Make()\nRETURN(Choice.EMPTY)\nPROC Main()\nresult=Make()\nRETURN"
            ),
            optimized,
        )
    }

    #[test]
    fn tiny_fills_select_bounded_constructor_gaps_in_raw_and_optimized_mir() {
        for optimized in [false, true] {
            for bytes in [1, 2, 3, 8, 9, 257] {
                let mut program = variant(bytes, optimized);
                let selected: usize = select(&mut program).iter().map(|(_, n)| n).sum();
                assert_eq!(
                    selected,
                    usize::from((2..=8).contains(&bytes)),
                    "{bytes}, optimized={optimized}"
                );
                crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
                assert!(select(&mut program).is_empty(), "idempotent selection");
            }
        }
    }

    #[test]
    fn tiny_fills_handle_nonzero_values_and_page_crossing_offsets() {
        let mut program = variant(3, false);
        let ri = program
            .routines
            .iter()
            .position(|r| r.name == "Make")
            .unwrap();
        let plan = discover(&program, ri).unwrap();
        let r = &mut program.routines[ri];
        r.name = "UnrelatedFill".into();
        r.frame
            .locals
            .iter_mut()
            .find(|s| s.base == MirStorageBase::Local(plan.target))
            .unwrap()
            .storage_size = 300;
        if let MirOp::Store { src, .. } = r.blocks[plan.preheader].ops.last_mut().unwrap() {
            *src = MirValue::ConstU16(254);
        }
        let header = r.blocks.iter_mut().find(|b| b.id == plan.header).unwrap();
        if let MirOp::Compare { right, .. } = &mut header.ops[1] {
            *right = MirValue::ConstU16(256);
        }
        let body = r.blocks.iter_mut().find(|b| b.id == plan.body).unwrap();
        if let MirOp::Store { src, .. } = &mut body.ops[2] {
            *src = MirValue::ConstU8(0xA5);
        }
        assert_eq!(
            select(&mut program).iter().map(|(_, n)| n).sum::<usize>(),
            1
        );
        let routine = &program.routines[ri];
        let writes = routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|op| match op {
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Local { offset, .. }),
                    src: MirValue::ConstU8(0xA5),
                    width: MirWidth::Byte,
                } => Some(*offset),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(writes, [254, 255, 256]);
        crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
    }

    #[test]
    fn tiny_fills_reduce_emitted_size_through_the_full_byte_budget() {
        fn size(program: MirProgram) -> usize {
            let config = crate::mir6502::Mir6502Config {
                enable_tiny_constant_fills: false,
                ..Default::default()
            };
            let materialized = crate::mir6502::materialize_program_with_origin_and_runtime(
                program,
                &config,
                0x3000,
                crate::runtime::Runtime::ActionCart,
            )
            .unwrap();
            let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
            crate::mir6502::emit_program(&materialized, &mut emitter).unwrap();
            emitter.finish_with_relocations().unwrap().bytes.len()
        }
        for bytes in 2..=8 {
            let before = variant(bytes, true);
            let mut after = before.clone();
            assert_eq!(select(&mut after).iter().map(|(_, n)| n).sum::<usize>(), 1);
            let (before, after) = (size(before), size(after));
            assert!(
                after + 40 <= before,
                "{bytes} byte fill: {before} -> {after}"
            );
        }
    }

    #[test]
    fn tiny_fills_reject_observable_controls_effects_and_unproven_windows() {
        for rejection in [
            "counter read",
            "pointer address",
            "alias",
            "data relocation",
            "volatile",
            "call",
            "outside object",
            "absolute",
            "signed",
            "stride",
            "wrapping bound",
            "entry bypass",
            "escaped temp",
        ] {
            let mut program = variant(2, false);
            let ri = program
                .routines
                .iter()
                .position(|r| r.name == "Make")
                .unwrap();
            let plan = discover(&program, ri).unwrap();
            let r = &mut program.routines[ri];
            let temp = MirTempId(r.temps.iter().map(|t| t.id.0).max().unwrap() + 1);
            r.temps.push(MirTemp { id: temp });
            let exit = r.blocks.iter().position(|b| b.id == plan.exit).unwrap();
            let header = r.blocks.iter().position(|b| b.id == plan.header).unwrap();
            let body = r.blocks.iter().position(|b| b.id == plan.body).unwrap();
            match rejection {
                "counter read" => r.blocks[exit].ops.insert(
                    0,
                    MirOp::Load {
                        dst: MirDef::VTemp(temp),
                        src: MirAddr::Direct(MirMem::Local {
                            id: plan.controls[1],
                            offset: 0,
                        }),
                        width: MirWidth::Word,
                    },
                ),
                "pointer address" => r.blocks[exit].ops.insert(
                    0,
                    MirOp::LeaAddr {
                        dst: MirDef::VTemp(temp),
                        target: MirMem::Local {
                            id: plan.controls[0],
                            offset: 0,
                        },
                        width: MirWidth::Word,
                    },
                ),
                "alias" => {
                    let mut slot = local(r, plan.controls[1]).unwrap().clone();
                    slot.base = MirStorageBase::LocalAlias {
                        id: LocalId(100),
                        target: plan.controls[1],
                    };
                    r.frame.locals.push(slot);
                }
                "data relocation" => {
                    let slot = r
                        .frame
                        .locals
                        .iter_mut()
                        .find(|s| s.base == MirStorageBase::Local(plan.target))
                        .unwrap();
                    slot.init = Some(MirStorageInit::Bytes {
                        image: MirDataImage {
                            bytes: vec![0; 3],
                            relocations: vec![MirDataRelocation {
                                offset: 0,
                                kind: MirDataRelocationKind::Word16,
                                target: MirDataRelocationTarget::Local {
                                    routine: r.id,
                                    id: plan.controls[0],
                                },
                                addend: 0,
                                span: crate::source::Span::new(0, 0),
                            }],
                        },
                        zero_fill: 0,
                        mutable: true,
                        section: String::new(),
                    });
                }
                "volatile" => r.blocks[body].ops.insert(
                    0,
                    MirOp::Barrier {
                        effects: MirEffects::default(),
                    },
                ),
                "call" => {
                    let call = lower("PROC F() RETURN PROC Main() F() RETURN", false)
                        .routines
                        .into_iter()
                        .flat_map(|r| r.blocks)
                        .flat_map(|b| b.ops)
                        .find(|op| matches!(op, MirOp::Call { .. }))
                        .unwrap();
                    r.blocks[body].ops.insert(0, call);
                }
                "outside object" => {
                    r.frame
                        .locals
                        .iter_mut()
                        .find(|s| s.base == MirStorageBase::Local(plan.target))
                        .unwrap()
                        .storage_size = 2
                }
                "absolute" => {
                    r.frame
                        .locals
                        .iter_mut()
                        .find(|s| s.base == MirStorageBase::Local(plan.target))
                        .unwrap()
                        .base = MirStorageBase::Absolute(0xD000)
                }
                "signed" => {
                    if let MirOp::Compare { signed, .. } = &mut r.blocks[header].ops[1] {
                        *signed = true;
                    }
                }
                "stride" => {
                    if let MirOp::Store {
                        dst: MirAddr::ComputedIndex { elem_size, .. },
                        ..
                    } = &mut r.blocks[body].ops[2]
                    {
                        *elem_size = 2;
                    }
                }
                "wrapping bound" => {
                    if let MirOp::Store { src, .. } =
                        r.blocks[plan.preheader].ops.last_mut().unwrap()
                    {
                        *src = MirValue::ConstU16(65534);
                    }
                    if let MirOp::Compare { right, .. } = &mut r.blocks[header].ops[1] {
                        *right = MirValue::ConstU16(65535);
                    }
                }
                "entry bypass" => r.blocks.swap(0, header),
                "escaped temp" => {
                    let MirOp::Load { dst, .. } = r.blocks[body].ops[0].clone() else {
                        unreachable!()
                    };
                    r.blocks[exit].ops.insert(
                        0,
                        MirOp::Move {
                            dst: MirDef::VTemp(temp),
                            src: MirValue::Def(dst),
                            width: MirWidth::Word,
                        },
                    );
                }
                _ => unreachable!(),
            }
            let before = program.clone();
            assert!(select(&mut program).is_empty(), "{rejection}");
            assert_eq!(
                program, before,
                "rejected transaction changed MIR: {rejection}"
            );
        }
    }
}
