//! Snapshot proofs for an exclusive numeric predicate's direct call.
#![allow(dead_code)] // Selection is enabled in the next slice.
use super::effects::classify_op;
use super::sites::{MirProgramPoint, MirSite};
use super::use_def::MirDefSite;
use crate::mir6502::ir::*;
use crate::mir6502::materialize::MaterializeLayout;
use crate::mir6502::rewrite::context::{MirProof, PreHomeRewriteContext};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct ArgumentSnapshot {
    pub call: MirProgramPoint,
    pub argument: usize,
    pub source: MirMem,
    pub width: MirWidth,
    pub definitions: Vec<MirDefSite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum SnapshotBlocker {
    Site,
    Predicate,
    Storage,
    Flow,
    Barrier,
    Placement,
}

pub(in crate::mir6502) fn ordinary_source(r: &MirRoutine, mem: &MirMem, width: MirWidth) -> bool {
    let bytes = if width == MirWidth::Word { 2 } else { 1 };
    let (slots, offset) = match mem {
        MirMem::Param { offset, .. } => (&r.frame.params, *offset),
        MirMem::Local { offset, .. } => (&r.frame.locals, *offset),
        _ => return false,
    };
    slots.iter().any(|slot| {
        let same = match (&slot.base, mem) {
            (MirStorageBase::Param(a), MirMem::Param { id: b, .. }) => a == b,
            (MirStorageBase::Local(a), MirMem::Local { id: b, .. }) => a == b,
            _ => false,
        };
        same && slot.scalar_width.is_some()
            && slot.init.is_none()
            && offset
                .checked_add(bytes)
                .is_some_and(|end| end <= slot.storage_size)
    })
}

pub(in crate::mir6502) fn pure_region_op(r: &MirRoutine, op: &MirOp) -> bool {
    match op {
        MirOp::Load {
            dst: MirDef::VTemp(_),
            src: MirAddr::Direct(mem),
            width,
        } => ordinary_source(r, mem, *width),
        MirOp::LoadImm {
            dst: MirDef::VTemp(_),
            ..
        } => true,
        MirOp::Compare {
            dst: MirCondDest::Temp(_),
            left,
            right,
            width,
            ..
        } => {
            fn safe(r: &MirRoutine, v: &MirValue, width: MirWidth) -> bool {
                match v {
                    MirValue::Def(MirDef::VTemp(_))
                    | MirValue::ConstU8(_)
                    | MirValue::ConstU16(_) => true,
                    MirValue::PointerCell(mem) => ordinary_source(r, mem, width),
                    _ => false,
                }
            }
            safe(r, left, *width) && safe(r, right, *width)
        }
        // Calls are barriers even when a callee summary says no effects.
        _ => false,
    }
}

pub(in crate::mir6502) fn predicate_call(
    r: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    site: MirSite,
) -> Option<(MirTempId, usize)> {
    let MirSite::Op { block, op_index } = site else {
        return None;
    };
    let b = r.blocks.iter().find(|b| b.id == block)?;
    let MirOp::Call {
        target: MirCallTarget::Routine(_),
        args,
        ..
    } = b.ops.get(op_index)?
    else {
        return None;
    };
    let MirOp::Compare {
        dst: MirCondDest::Temp(temp),
        op: MirCompareOp::Eq | MirCompareOp::Ne,
        width: MirWidth::Byte,
        right: MirValue::ConstU8(0),
        ..
    } = b.ops.get(op_index.checked_sub(1)?)?
    else {
        return None;
    };
    let compare_site = MirSite::Op {
        block,
        op_index: op_index - 1,
    };
    if !pure_region_op(r, &b.ops[op_index - 1])
        || !context
            .exclusive_byte_temp_flow(*temp, compare_site, site)
            .is_proven()
    {
        return None;
    }
    let indexes = args
        .iter()
        .enumerate()
        .filter(|(_, a)| a.value == MirValue::Def(MirDef::VTemp(*temp)))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let [argument] = indexes.as_slice() else {
        return None;
    };
    (args[*argument].width == MirWidth::Byte).then_some((*temp, *argument))
}

/// Logical storage identity and physical staging interference are separate
/// obligations. This first schedule writes only register/fixed-ZP argument
/// homes and reserved ZP scratch, so every ordinary source must be above $FF.
pub(in crate::mir6502) fn physical_source(
    r: &MirRoutine,
    layout: &MaterializeLayout,
    source: &MirMem,
    width: MirWidth,
    call: &MirOp,
) -> bool {
    fn home(h: &MirArgHome) -> bool {
        match h {
            MirArgHome::Reg(_) | MirArgHome::RegisterPair { .. } | MirArgHome::FixedZeroPage(_) => {
                true
            }
            MirArgHome::BytePair { lo, hi } => home(lo) && home(hi),
            _ => false,
        }
    }
    let MirOp::Call { args, .. } = call else {
        return false;
    };
    ordinary_source(r, source, width)
        && args.iter().all(|a| home(&a.home))
        && layout.mem_address(r.id, source).is_some_and(|a| {
            a >= 0x100 && a.checked_add(u16::from(width == MirWidth::Word)).is_some()
        })
}

pub(in crate::mir6502) fn argument_snapshot(
    r: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    call: MirProgramPoint,
    argument: usize,
    layout: &MaterializeLayout,
) -> Result<ArgumentSnapshot, SnapshotBlocker> {
    if !context.valid_point(call) {
        return Err(SnapshotBlocker::Site);
    }
    let MirSite::Op { block, op_index } = call.site else {
        return Err(SnapshotBlocker::Site);
    };
    let b = r
        .blocks
        .iter()
        .find(|b| b.id == block)
        .ok_or(SnapshotBlocker::Site)?;
    let (_, predicate) = predicate_call(r, context, call.site).ok_or(SnapshotBlocker::Predicate)?;
    if argument >= predicate {
        return Err(SnapshotBlocker::Predicate);
    }
    let call_op = &b.ops[op_index];
    let MirOp::Call { args, .. } = call_op else {
        unreachable!()
    };
    let arg = args.get(argument).ok_or(SnapshotBlocker::Site)?;
    let MirValue::Def(MirDef::VTemp(temp)) = arg.value else {
        return Err(SnapshotBlocker::Flow);
    };
    let definitions = match arg.width {
        MirWidth::Word => match context.exclusive_word_temp_definition(temp, call.site) {
            MirProof::Proven(defs) => defs.to_vec(),
            _ => return Err(SnapshotBlocker::Flow),
        },
        MirWidth::Byte => {
            let sites = b
                .ops
                .iter()
                .enumerate()
                .filter(|(_, op)| {
                    classify_op(op)
                        .logical
                        .temp_defs
                        .iter()
                        .any(|d| d.temp() == temp)
                })
                .map(|(i, _)| MirSite::Op { block, op_index: i })
                .collect::<Vec<_>>();
            let [site] = sites.as_slice() else {
                return Err(SnapshotBlocker::Flow);
            };
            match context.exclusive_byte_temp_flow(temp, *site, call.site) {
                MirProof::Proven(def) => vec![def],
                _ => return Err(SnapshotBlocker::Flow),
            }
        }
    };
    let MirSite::Op {
        block: producer_block,
        op_index: producer,
    } = definitions[0].site
    else {
        return Err(SnapshotBlocker::Flow);
    };
    if producer_block != block || producer >= op_index - 1 {
        return Err(SnapshotBlocker::Flow);
    }
    let MirOp::Load {
        dst: MirDef::VTemp(id),
        src: MirAddr::Direct(source),
        width,
    } = &b.ops[producer]
    else {
        return Err(SnapshotBlocker::Storage);
    };
    if *id != temp || *width != arg.width || !ordinary_source(r, source, *width) {
        return Err(SnapshotBlocker::Storage);
    }
    if !b.ops[producer + 1..op_index]
        .iter()
        .all(|op| pure_region_op(r, op))
    {
        return Err(SnapshotBlocker::Barrier);
    }
    if !physical_source(r, layout, source, *width, call_op) {
        return Err(SnapshotBlocker::Placement);
    }
    Ok(ArgumentSnapshot {
        call,
        argument,
        source: source.clone(),
        width: *width,
        definitions,
    })
}

#[cfg(test)]
pub(in crate::mir6502) mod tests {
    use super::*;
    use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
    use crate::mir6502::analysis::sites::MirRoutineGeneration;

    pub(in crate::mir6502) fn program() -> MirProgram {
        let source = include_str!("../../../surveys/tn/reproducers/captured-call-arguments.act")
            .replace("\r\n", "\n");
        let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(&semir)).unwrap();
        crate::mir6502::lower_program(&nir).unwrap()
    }

    fn prove(
        p: &MirProgram,
        generation: MirRoutineGeneration,
        point_generation: MirRoutineGeneration,
        origin: u16,
    ) -> Result<ArgumentSnapshot, SnapshotBlocker> {
        let r = &p.routines[1];
        let snapshot = PreHomeAnalysisSnapshot::new(r, generation).unwrap();
        let context = PreHomeRewriteContext::new(&snapshot);
        let index = r.blocks[0]
            .ops
            .iter()
            .position(|op| matches!(op, MirOp::Call { .. }))
            .unwrap();
        context.argument_snapshot(
            r,
            MirProgramPoint {
                generation: point_generation,
                site: MirSite::Op {
                    block: r.blocks[0].id,
                    op_index: index,
                },
            },
            1,
            &MaterializeLayout::new(p, origin),
        )
    }

    #[test]
    fn call_predicate_snapshot_requires_complete_exclusive_flow_and_current_generation() {
        let p = program();
        let g = MirRoutineGeneration::initial();
        let proof = prove(&p, g, g, 0x3000).unwrap();
        assert_eq!(proof.definitions.len(), 2);
        assert_eq!(proof.width, MirWidth::Word);
        assert_eq!(prove(&p, g.next(), g, 0x3000), Err(SnapshotBlocker::Site));
        assert_eq!(prove(&p, g, g, 0x80), Err(SnapshotBlocker::Placement));
        let mut extra = p.clone();
        extra.routines[1].blocks[0].ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x800)),
            src: MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 1,
            }),
            width: MirWidth::Byte,
        });
        assert_eq!(prove(&extra, g, g, 0x3000), Err(SnapshotBlocker::Flow));
    }

    #[test]
    fn call_predicate_snapshot_rejects_calls_alias_writes_and_observable_reads() {
        let p = program();
        let g = MirRoutineGeneration::initial();
        let blockers = vec![
            p.routines[1].blocks[0]
                .ops
                .iter()
                .find(|op| matches!(op, MirOp::Call { .. }))
                .unwrap()
                .clone(),
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Param {
                    id: crate::nir::ParamId(1),
                    offset: 1,
                }),
                src: MirValue::ConstU8(0),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(9)),
                src: MirAddr::Direct(MirMem::Absolute(0xD20A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(9)),
                src: MirAddr::Deref {
                    ptr: MirValue::ConstU16(0x7000),
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
        ];
        for blocker in blockers {
            let mut changed = p.clone();
            changed.routines[1].blocks[0].ops.insert(2, blocker.clone());
            let r = &changed.routines[1];
            let snapshot = PreHomeAnalysisSnapshot::new(r, g).unwrap();
            let context = PreHomeRewriteContext::new(&snapshot);
            let point = context.point(MirSite::Op {
                block: r.blocks[0].id,
                op_index: 5,
            });
            assert!(
                context
                    .argument_snapshot(r, point, 1, &MaterializeLayout::new(&changed, 0x3000))
                    .is_err(),
                "{blocker:?}"
            );
        }
    }
}
