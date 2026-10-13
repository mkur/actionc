//! Natural-loop and complete-lane proofs for pre-home computation motion.
use super::effects::{MirFlagSet, classify_op};
use super::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::ir::*;
use crate::mir6502::rewrite::context::{MirProof, PreHomeRewriteContext};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirNaturalLoop {
    pub header: MirBlockId,
    pub preheader: MirBlockId,
    pub nodes: BTreeSet<MirBlockId>,
}

/// Header dominance identifies backedges; the reverse closure must have one
/// outside entry, whose unconditional edge executes the original header even
/// on a zero-trip loop. No CFG changes or speculative data reads are needed.
pub(in crate::mir6502) fn natural_loop(
    r: &MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    header: MirBlockId,
) -> Option<MirNaturalLoop> {
    let cfg = c.cfg();
    let backedges: Vec<_> = cfg
        .predecessors(header)
        .iter()
        .copied()
        .filter(|&p| c.block_dominates(header, p))
        .collect();
    if backedges.is_empty() {
        return None;
    }
    let mut nodes = BTreeSet::from([header]);
    let mut pending = backedges;
    while let Some(id) = pending.pop() {
        if !c.block_dominates(header, id) {
            return None;
        }
        if nodes.insert(id) {
            pending.extend(cfg.predecessors(id).iter().copied());
        }
    }
    let outside: Vec<_> = cfg
        .predecessors(header)
        .difference(&nodes)
        .copied()
        .collect();
    let [preheader] = outside.as_slice() else {
        return None;
    };
    let pre = r.blocks.iter().find(|b| b.id == *preheader)?;
    if !c.block_dominates(*preheader, header)
        || !matches!(&pre.terminator,MirTerminator::Jump(edge) if edge.target==header && edge.args.is_empty())
        || r.blocks
            .iter()
            .any(|b| nodes.contains(&b.id) && !b.params.is_empty())
    {
        return None;
    }
    Some(MirNaturalLoop {
        header,
        preheader: *preheader,
        nodes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirLoopMotionPlan {
    pub generation: MirRoutineGeneration,
    pub region: MirNaturalLoop,
    /// Original header positions, in dependency order. The operations themselves
    /// stay unchanged and are inserted before the preheader's existing jump.
    pub operations: BTreeSet<usize>,
    pub original: Vec<MirOp>,
}

fn arithmetic(op: &MirOp) -> bool {
    match op {
        MirOp::Binary {
            op,
            dst: MirDef::VTemp(_),
            left,
            right,
            width,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        } => {
            let value = |v: &MirValue| {
                matches!(
                    v,
                    MirValue::ConstU8(_)
                        | MirValue::ConstU16(_)
                        | MirValue::Def(MirDef::VTemp(_))
                        | MirValue::Def(MirDef::VTempByte { .. })
                )
            };
            value(left)
                && value(right)
                && match op {
                    MirBinaryOp::Add
                    | MirBinaryOp::Sub
                    | MirBinaryOp::And
                    | MirBinaryOp::Or
                    | MirBinaryOp::Xor => true,
                    MirBinaryOp::Lsh | MirBinaryOp::Rsh => {
                        let count = match right {
                            MirValue::ConstU8(n) => u16::from(*n),
                            MirValue::ConstU16(n) => *n,
                            _ => return false,
                        };
                        count < if *width == MirWidth::Byte { 8 } else { 16 }
                    }
                    _ => false,
                }
        }
        _ => false,
    }
}

pub(in crate::mir6502) fn might_match(r: &MirRoutine) -> bool {
    r.blocks.iter().any(|b| {
        matches!(b.terminator, MirTerminator::Branch { .. }) && b.ops.iter().any(arithmetic)
    })
}

fn local_reload_is_invariant(
    r: &MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    f: &MirNaturalLoop,
    op: &MirOp,
) -> bool {
    let MirOp::Load {
        src: MirAddr::Direct(mem @ MirMem::Local { id, offset }),
        dst: MirDef::VTemp(_),
        width,
    } = op
    else {
        return false;
    };
    let bytes = if *width == MirWidth::Word { 2 } else { 1 };
    if !r.frame.locals.iter().any(|s| {
        s.base == MirStorageBase::Local(*id)
            && s.storage == MirStorageClass::Scalar
            && s.scalar_width == Some(*width)
            && offset
                .checked_add(bytes)
                .is_some_and(|end| end <= s.storage_size)
    }) {
        return false;
    }
    // An alias or exposed frame can make a stable captured local reachable
    // through another identity. Keep the entire frame unexposed in this slice.
    if r.frame
        .locals
        .iter()
        .any(|s| matches!(s.base,MirStorageBase::LocalAlias {target,..} if target==*id))
        || r.blocks.iter().flat_map(|b| &b.ops).any(|op| {
            classify_op(op)
                .memory
                .address_references
                .iter()
                .any(|m| matches!(m, MirMem::Local { .. } | MirMem::Param { .. }))
        })
    {
        return false;
    }
    let mut stores = vec![];
    for b in &r.blocks {
        for (i, op) in b.ops.iter().enumerate() {
            let effect = classify_op(op);
            if effect
                .memory
                .direct_writes
                .iter()
                .any(|w| crate::mir6502::analysis::comparison_loops::same_object(&w.base, mem))
            {
                if !matches!(op,MirOp::Store {dst:MirAddr::Direct(target),width:w,..} if target==mem && w==width)
                    || f.nodes.contains(&b.id)
                {
                    return false;
                }
                stores.push((b.id, i));
            }
        }
    }
    let [(block, _)] = stores.as_slice() else {
        return false;
    };
    c.block_dominates(*block, f.preheader)
}

fn safe_loop(r: &MirRoutine, f: &MirNaturalLoop) -> bool {
    r.blocks
        .iter()
        .filter(|b| f.nodes.contains(&b.id))
        .flat_map(|b| &b.ops)
        .all(|op| {
            let e = classify_op(op);
            !matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) && !e.memory.indirect_writes
                && !e.memory.has_unknown_effects
                && !e.memory.may_write_any
                && e.memory.direct_writes.iter().all(|w| {
                    matches!(
                        w.base,
                        MirMem::Local { .. } | MirMem::Param { .. } | MirMem::Spill { .. }
                    )
                })
        })
}

fn add_dependencies(
    r: &MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    f: &MirNaturalLoop,
    index: usize,
    indices: &mut BTreeSet<usize>,
    visiting: &mut BTreeSet<usize>,
) -> Option<()> {
    if indices.contains(&index) {
        return Some(());
    }
    if !visiting.insert(index) {
        return None;
    }
    let header = r.blocks.iter().find(|b| b.id == f.header)?;
    let op = header.ops.get(index)?;
    if !arithmetic(op) && !local_reload_is_invariant(r, c, f, op) {
        return None;
    }
    let site = MirSite::Op {
        block: f.header,
        op_index: index,
    };
    let effects = classify_op(op);
    // Removing an abstract arithmetic definition must not remove a live flag
    // producer; inserting it must not replace incoming machine state either.
    if !c.flags_dead_after(effects.machine.flag_writes, site) {
        return None;
    }
    for access in &effects.logical.temp_uses {
        let temp = access.temp();
        let definitions = c.temp_definitions(temp);
        let first = definitions.first()?;
        if definitions.iter().any(|d| d.site != first.site) {
            return None;
        }
        let uses = c.uses_at(temp, site);
        for usage in uses {
            for d in definitions
                .iter()
                .filter(|d| usage.requirement.requires(d.lane))
            {
                if !matches!(c.unique_reaching_definition(usage,d.lane),MirProof::Proven(reaching) if reaching==*d)
                {
                    return None;
                }
                if let MirSite::Op { block, op_index } = d.site {
                    if block == f.header {
                        if op_index >= index {
                            return None;
                        }
                        add_dependencies(r, c, f, op_index, indices, visiting)?;
                        continue;
                    }
                }
                if f.nodes.contains(&d.site.block())
                    || !c
                        .value_available_at(*d, c.point(MirSite::Terminator { block: f.preheader }))
                        .is_proven()
                {
                    return None;
                }
            }
        }
    }
    let (MirOp::Binary {
        dst: MirDef::VTemp(result),
        ..
    }
    | MirOp::Load {
        dst: MirDef::VTemp(result),
        ..
    }) = op
    else {
        return None;
    };
    let defs = c.temp_definitions(*result);
    if defs.is_empty() || defs.iter().any(|d| d.site != site) {
        return None;
    }
    for usage in c.temp_uses(*result) {
        for d in defs.iter().filter(|d| usage.requirement.requires(d.lane)) {
            if !matches!(c.unique_reaching_definition(usage,d.lane),MirProof::Proven(reaching) if reaching==*d)
            {
                return None;
            }
        }
    }
    indices.insert(index);
    visiting.remove(&index);
    Some(())
}

/// Immutable fact and plan creation. Only total arithmetic and existing private
/// reloads move; source captures, stores and the complete CFG remain unchanged.
pub(in crate::mir6502) fn prove(
    r: &MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    header: MirBlockId,
    index: usize,
) -> Option<MirLoopMotionPlan> {
    let f = natural_loop(r, c, header)?;
    if !safe_loop(r, &f)
        || !c.branch_machine_state_dead(f.preheader).is_proven()
        || !c.flags_dead_after(
            MirFlagSet::all(),
            MirSite::Terminator { block: f.preheader },
        )
    {
        return None;
    }
    let b = r.blocks.iter().find(|b| b.id == header)?;
    if !arithmetic(b.ops.get(index)?) {
        return None;
    }
    let mut indices = BTreeSet::new();
    add_dependencies(r, c, &f, index, &mut indices, &mut BTreeSet::new())?;
    Some(MirLoopMotionPlan {
        generation: c.generation(),
        region: f,
        original: indices.iter().map(|&i| b.ops[i].clone()).collect(),
        operations: indices,
    })
}

pub(in crate::mir6502) fn discover_captured_bound_motion(
    r: &MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirLoopMotionPlan> {
    r.blocks
        .iter()
        .filter(|b| matches!(b.terminator, MirTerminator::Branch { .. }))
        .flat_map(|b| (0..b.ops.len()).filter_map(|i| prove(r, c, b.id, i)))
        .collect()
}

#[cfg(test)]
pub(in crate::mir6502) mod tests {
    use super::*;
    pub(in crate::mir6502) fn program(source: &str) -> MirProgram {
        let tokens = crate::lexer::tokenize(source).unwrap();
        let ast = crate::parser::parse(&tokens).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let sem = crate::semantic::ir::lower_program(&ast, &model);
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(&sem)).unwrap();
        crate::mir6502::lower_program(&nir).unwrap()
    }
    pub(in crate::mir6502) fn captured() -> MirProgram {
        program(
            "CARD FUNC Count(CARD bound)\n CARD ordinal\n LET CARD capacity=bound\n ordinal=0\n WHILE ordinal<(capacity RSH 3) DO ordinal==+1 OD\nRETURN(ordinal)\nPROC Main()\nRETURN\n",
        )
    }
    pub(in crate::mir6502) fn plans(r: &MirRoutine) -> Vec<MirLoopMotionPlan> {
        let snapshot =
            super::super::prehome::PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial())
                .unwrap();
        discover_captured_bound_motion(r, &PreHomeRewriteContext::new(&snapshot))
    }

    #[test]
    fn motion_proves_both_lanes_and_moves_only_the_existing_capture_reload() {
        let p = captured();
        let r = &p.routines[0];
        let facts = plans(r);
        assert_eq!(facts.len(), 1, "{}", crate::mir6502::format_program(&p));
        let f = &facts[0];
        assert_eq!(f.operations.len(), 2);
        let header = r.blocks.iter().find(|b| b.id == f.region.header).unwrap();
        assert!(matches!(
            header.ops[*f.operations.first().unwrap()],
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Local { .. }),
                width: MirWidth::Word,
                ..
            }
        ));
        assert!(matches!(
            header.ops[*f.operations.last().unwrap()],
            MirOp::Binary {
                op: MirBinaryOp::Rsh,
                width: MirWidth::Word,
                ..
            }
        ));
        assert!(f.region.nodes.contains(&f.region.header));
        assert!(!f.region.nodes.contains(&f.region.preheader));
    }

    #[test]
    fn motion_rejects_aliases_exposed_captures_and_loop_barriers() {
        let p = captured();
        let original = &p.routines[0];
        let f = plans(original).remove(0);
        let load = &original
            .blocks
            .iter()
            .find(|b| b.id == f.region.header)
            .unwrap()
            .ops[*f.operations.first().unwrap()];
        let MirOp::Load {
            src: MirAddr::Direct(MirMem::Local { id, .. }),
            ..
        } = load
        else {
            panic!()
        };
        for case in 0..7 {
            let mut r = original.clone();
            match case {
                0 => {
                    r.frame
                        .locals
                        .iter_mut()
                        .find(|s| s.base == MirStorageBase::Local(*id))
                        .unwrap()
                        .base = MirStorageBase::Absolute(0x4000);
                }
                1 => r.blocks[0].ops.push(MirOp::Move {
                    dst: MirDef::VTemp(MirTempId(999)),
                    src: MirValue::StorageAddrByte {
                        mem: MirMem::Local { id: *id, offset: 0 },
                        byte: 0,
                    },
                    width: MirWidth::Byte,
                }),
                2 => r
                    .blocks
                    .iter_mut()
                    .find(|b| b.id != f.region.header && f.region.nodes.contains(&b.id))
                    .unwrap()
                    .ops
                    .push(MirOp::Store {
                        dst: MirAddr::PointerCell {
                            ptr: MirMem::Param {
                                id: crate::nir::ParamId(0),
                                offset: 0,
                            },
                            offset: 0,
                        },
                        src: MirValue::ConstU8(0),
                        width: MirWidth::Byte,
                    }),
                3 => r
                    .blocks
                    .iter_mut()
                    .find(|b| b.id == f.region.header)
                    .unwrap()
                    .ops
                    .insert(
                        0,
                        MirOp::Barrier {
                            effects: Default::default(),
                        },
                    ),
                4 => r
                    .blocks
                    .iter_mut()
                    .find(|b| b.id == f.region.header)
                    .unwrap()
                    .ops
                    .insert(
                        0,
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Absolute(0xD000)),
                            src: MirValue::ConstU8(0),
                            width: MirWidth::Byte,
                        },
                    ),
                5 => r
                    .blocks
                    .iter_mut()
                    .find(|b| b.id == f.region.header)
                    .unwrap()
                    .ops
                    .insert(
                        0,
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Local { id: *id, offset: 1 }),
                            src: MirValue::ConstU8(0),
                            width: MirWidth::Byte,
                        },
                    ),
                _ => {
                    r.frame
                        .locals
                        .iter_mut()
                        .find(|s| s.base == MirStorageBase::Local(*id))
                        .unwrap()
                        .storage = MirStorageClass::Record
                }
            }
            assert!(plans(&r).is_empty(), "case {case}");
        }
        for source in [
            "BYTE n PROC Touch() n==+1 RETURN CARD FUNC Count(CARD bound) CARD ordinal LET CARD capacity=bound ordinal=0 WHILE ordinal<(capacity RSH 3) DO Touch() ordinal==+1 OD RETURN(ordinal) PROC Main() RETURN",
            "CARD FUNC Count(CARD POINTER source) CARD ordinal ordinal=0 WHILE ordinal<(source^ RSH 3) DO ordinal==+1 OD RETURN(ordinal) PROC Main() RETURN",
        ] {
            let p = program(source);
            assert!(plans(p.routines.iter().find(|r| r.name == "Count").unwrap()).is_empty());
        }
    }

    #[test]
    fn motion_rejects_conditional_preheaders_redefinitions_and_live_flags() {
        let p = captured();
        let original = &p.routines[0];
        let f = plans(original).remove(0);
        for case in 0..3 {
            let mut r = original.clone();
            match case {
                0 => {
                    let exit = r.blocks.last().unwrap().id;
                    let pre = r
                        .blocks
                        .iter_mut()
                        .find(|b| b.id == f.region.preheader)
                        .unwrap();
                    pre.terminator = MirTerminator::Branch {
                        cond: MirCond::BoolValue(MirValue::ConstU8(1)),
                        then_edge: MirEdge::plain(f.region.header),
                        else_edge: MirEdge::plain(exit),
                    };
                }
                1 => {
                    let header = r.blocks.iter().find(|b| b.id == f.region.header).unwrap();
                    let MirOp::Load {
                        dst: MirDef::VTemp(temp),
                        ..
                    } = &header.ops[*f.operations.first().unwrap()]
                    else {
                        panic!()
                    };
                    let temp = *temp;
                    r.blocks[0].ops.push(MirOp::LoadImm {
                        dst: MirDef::VTempByte { id: temp, byte: 1 },
                        value: 0,
                        width: MirWidth::Byte,
                    });
                }
                _ => {
                    let header = r
                        .blocks
                        .iter_mut()
                        .find(|b| b.id == f.region.header)
                        .unwrap();
                    header.ops.truncate(*f.operations.last().unwrap() + 1);
                    if let MirTerminator::Branch { cond, .. } = &mut header.terminator {
                        *cond = MirCond::FlagTest(MirFlagTest::CSet);
                    }
                }
            }
            assert!(plans(&r).is_empty(), "case {case}");
        }
    }
}

#[cfg(test)]
mod natural_loop_tests {
    use super::tests::{captured, plans};
    use super::*;
    #[test]
    fn natural_loop_merges_backedges_and_rejects_a_side_entry() {
        let p = captured();
        let mut r = p.routines[0].clone();
        let f = plans(&r).remove(0);
        let latch = r
            .blocks
            .iter()
            .find(|b| b.id != f.region.header && f.region.nodes.contains(&b.id))
            .unwrap()
            .id;
        let new = MirBlockId(99);
        r.blocks
            .iter_mut()
            .find(|b| b.id == latch)
            .unwrap()
            .terminator = MirTerminator::Branch {
            cond: MirCond::BoolValue(MirValue::ConstU8(1)),
            then_edge: MirEdge::plain(f.region.header),
            else_edge: MirEdge::plain(new),
        };
        r.blocks.push(MirBlock {
            id: new,
            label: "second_latch".into(),
            params: vec![],
            ops: vec![],
            terminator: MirTerminator::Jump(MirEdge::plain(f.region.header)),
        });
        let facts = plans(&r);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].region.nodes.contains(&new));
        r.blocks
            .iter_mut()
            .find(|b| b.id == f.region.preheader)
            .unwrap()
            .terminator = MirTerminator::Branch {
            cond: MirCond::BoolValue(MirValue::ConstU8(1)),
            then_edge: MirEdge::plain(f.region.header),
            else_edge: MirEdge::plain(new),
        };
        assert!(plans(&r).is_empty());
    }
}
