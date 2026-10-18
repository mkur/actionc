//! Delay inert payload reads into already-proven disjoint result transfers.
//! This is a separate transaction: failure preserves store/home elision.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn improve(
    selected: &mut MirProgram,
    emitted: &mut MirProgram,
    candidates: &[Candidate],
    protected: &BTreeMap<RoutineId, Vec<MirMem>>,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    stats: &mut MirPeepholeStats,
) {
    let mut proposal = selected.clone();
    // Home elision may already have removed some constructor allocations.
    let mut protected: BTreeMap<_, Vec<_>> = protected
        .iter()
        .map(|(&id, cells)| {
            let r = selected.routines.iter().find(|r| r.id == id).unwrap();
            let cells = cells
                .iter()
                .filter(|cell| {
                    !matches!(cell, MirMem::Local { id, .. }
                    if !r.frame.locals.iter().any(|s| s.base == MirStorageBase::Local(*id)))
                })
                .cloned()
                .collect();
            (id, cells)
        })
        .collect();
    let mut counts = BTreeMap::<RoutineId, usize>::new();
    for c in candidates {
        let r = proposal
            .routines
            .iter()
            .find(|r| r.id == c.routine)
            .unwrap();
        let Some((ops, sources, count)) = select(&proposal, r, c) else {
            continue;
        };
        let r = proposal
            .routines
            .iter_mut()
            .find(|r| r.id == c.routine)
            .unwrap();
        r.blocks
            .iter_mut()
            .find(|b| b.id == c.return_block)
            .unwrap()
            .ops = ops;
        protected.entry(c.routine).or_default().extend(sources);
        *counts.entry(c.routine).or_default() += count;
    }
    if counts.is_empty() {
        return;
    }
    let materialized = crate::mir6502::verify_program(&proposal, MirPhase::PreMaterialization)
        .ok()
        .and_then(|()| {
            crate::mir6502::materialize_resolved_program_without_copy_costing(
                proposal.clone(),
                config,
                origin,
                runtime,
                false,
            )
            .ok()
        });
    let accepted = materialized.as_ref().is_some_and(|new| {
        Cost::measure(emitted, origin)
            .zip(Cost::measure(new, origin))
            .is_some_and(|(old, new)| {
                new.bytes < old.bytes && counts.keys().all(|&id| new.does_not_grow(&old, id))
            })
            && occupied(new).is_subset(&occupied(emitted))
            && private_cells::physical_cells_unaliased(emitted, origin, &protected)
            && private_cells::physical_cells_unaliased(new, origin, &protected)
    });
    for (id, count) in counts {
        stats.record_many(
            id,
            if accepted {
                "constructor-payload-bytes-forwarded"
            } else {
                "constructor-payload-declined-cost-or-alias"
            },
            count,
        );
    }
    if accepted {
        *selected = proposal;
        *emitted = materialized.unwrap();
    }
}

fn lane(value: &MirValue) -> Option<MirTempLane> {
    match value {
        MirValue::Def(MirDef::VTemp(id)) => Some(MirTempLane { temp: *id, byte: 0 }),
        MirValue::Def(MirDef::VTempByte { id, byte }) => Some(MirTempLane {
            temp: *id,
            byte: *byte,
        }),
        _ => None,
    }
}

fn offset_cell(mem: &MirMem, byte: u16) -> Option<MirMem> {
    match mem {
        MirMem::Local { id, offset } => Some(MirMem::Local {
            id: *id,
            offset: offset.checked_add(byte)?,
        }),
        MirMem::Param { id, offset } => Some(MirMem::Param {
            id: *id,
            offset: offset.checked_add(byte)?,
        }),
        _ => None,
    }
}

fn inert(p: &MirProgram, r: &MirRoutine, op: &MirOp) -> bool {
    match op {
        MirOp::Load {
            dst: MirDef::VTemp(_) | MirDef::VTempByte { .. },
            src: MirAddr::Direct(mem @ (MirMem::Local { .. } | MirMem::Param { .. })),
            width,
        } => range_in_bounds(p, r, mem, width_bytes(*width)),
        MirOp::LoadImm {
            dst: MirDef::VTemp(_) | MirDef::VTempByte { .. },
            ..
        }
        | MirOp::Move {
            dst: MirDef::VTemp(_) | MirDef::VTempByte { .. },
            ..
        }
        | MirOp::Extend {
            dst: MirDef::VTemp(_) | MirDef::VTempByte { .. },
            ..
        }
        | MirOp::Truncate {
            dst: MirDef::VTemp(_) | MirDef::VTempByte { .. },
            ..
        } => {
            let e = classify_op(op).memory;
            e.direct_reads.is_empty()
                && e.direct_writes.is_empty()
                && !e.indirect_reads
                && !e.indirect_writes
                && !e.opaque
                && !e.has_unknown_effects
                && e.structured_reads == MirMemoryEffect::None
                && e.structured_writes == MirMemoryEffect::None
        }
        _ => false,
    }
}

// Pointer-to-CARD casts are width-preserving virtual moves. Follow a small
// same-block chain, requiring a single definition for every lane at each step.
fn loaded_chain(
    b: &MirBlock,
    defs: &MirTempUseDefIndex,
    mut temp: MirTempId,
    mut end: usize,
) -> Option<(Vec<(MirTempId, usize)>, MirMem, u16)> {
    let mut chain = Vec::new();
    let mut expected_width = None;
    for _ in 0..=8 {
        let site = defs.unique_definition(MirTempLane { temp, byte: 0 })?.site;
        let MirSite::Op { block, op_index } = site else {
            return None;
        };
        if block != b.id || op_index >= end {
            return None;
        }
        let (id, width) = match &b.ops[op_index] {
            MirOp::Load {
                dst: MirDef::VTemp(id),
                width,
                ..
            }
            | MirOp::Move {
                dst: MirDef::VTemp(id),
                width,
                ..
            } => (*id, *width),
            _ => return None,
        };
        if id != temp
            || expected_width.is_some_and(|w| w != width)
            || (0..width_bytes(width)).any(|byte| {
                defs.unique_definition(MirTempLane {
                    temp,
                    byte: byte as u8,
                })
                .is_none_or(|def| def.site != site)
            })
        {
            return None;
        }
        chain.push((temp, op_index));
        match &b.ops[op_index] {
            MirOp::Load {
                src: MirAddr::Direct(mem),
                ..
            } => {
                return Some((chain, mem.clone(), width_bytes(width)));
            }
            MirOp::Move {
                src: MirValue::Def(MirDef::VTemp(source)),
                ..
            } => {
                temp = *source;
                expected_width = Some(width);
                end = op_index;
            }
            _ => return None,
        }
    }
    None
}

fn select(
    p: &MirProgram,
    r: &MirRoutine,
    c: &Candidate,
) -> Option<(Vec<MirOp>, Vec<MirMem>, usize)> {
    let b = r.blocks.iter().find(|b| b.id == c.return_block)?;
    if !matches!(b.terminator, MirTerminator::Return) {
        return None;
    }
    // Recognize only the exact suffix emitted by captured-copy forwarding.
    // The caller census in the parent proof, not the pointer's ABI position,
    // proves these stores disjoint from ordinary storage in this routine.
    let start = b.ops.len().checked_sub(usize::from(c.bytes))?;
    let setup = start.checked_sub(1)?;
    let MirOp::MaterializeAddress { consumer, .. } = &b.ops[setup] else {
        return None;
    };
    for (offset, op) in b.ops[start..].iter().enumerate() {
        if !matches!(op, MirOp::StoreIndirect { consumer: dst, offset: n, .. }
            if dst == consumer && usize::from(*n) == offset)
        {
            return None;
        }
    }
    let defs = MirTempUseDefIndex::from_routine(r);
    let mut loads = BTreeMap::new();
    for op in &b.ops[start..] {
        let MirOp::StoreIndirect { src, .. } = op else {
            unreachable!()
        };
        let Some(lane) = lane(src) else {
            continue;
        };
        if loads.contains_key(&lane.temp) {
            continue;
        }
        let Some((chain, mem, bytes)) = loaded_chain(b, &defs, lane.temp, setup) else {
            continue;
        };
        let op_index = chain.last()?.1;
        if setup - op_index > 64 || offset_cell(&mem, 0).is_none()
            || !range_in_bounds(p, r, &mem, bytes)
            || !b.ops[op_index..setup].iter().all(|op| inert(p, r, op))
            || chain.iter().any(|&(id, _)| defs.uses_of_temp(id).iter().any(|usage| {
                let MirSite::Op { block, op_index } = usage.site else { return true; };
                block != b.id || !(chain.iter().any(|&(_, i)| i == op_index)
                    || (op_index >= start && matches!(&b.ops[op_index], MirOp::StoreIndirect { src, .. }
                        if self::lane(src).is_some_and(|l| l.temp == id && u16::from(l.byte) < bytes))))
            }))
        { continue; }
        for (id, op_index) in chain {
            loads.insert(id, (op_index, mem.clone(), bytes));
        }
    }
    if loads.is_empty() {
        return None;
    }
    let removed: BTreeSet<_> = loads.values().map(|(index, _, _)| *index).collect();
    let mut sources = Vec::new();
    for (_, mem, bytes) in loads.values() {
        for byte in 0..*bytes {
            sources.push(offset_cell(mem, byte)?);
        }
    }
    let mut count = 0;
    let mut ops = Vec::new();
    for (i, op) in b.ops.iter().enumerate() {
        if removed.contains(&i) {
            continue;
        }
        let mut op = op.clone();
        if i >= start {
            if let MirOp::StoreIndirect { src, .. } = &mut op {
                if let Some(lane) = lane(src) {
                    if let Some((_, mem, _)) = loads.get(&lane.temp) {
                        *src = MirValue::PointerCell(offset_cell(mem, u16::from(lane.byte))?);
                        count += 1;
                    }
                }
            }
        }
        ops.push(op);
    }
    Some((ops, sources, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "TYPE Choice=VARIANT [EMPTY FOUND [CARD value]] Choice result Choice FUNC Make(CARD n) RETURN(Choice.FOUND(n)) PROC Main() result=Make(1234) RETURN";

    fn lower(source: &str, optimized: bool) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let raw = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let nir = if optimized {
            crate::nir::optimize_program(&raw).unwrap()
        } else {
            raw
        };
        crate::mir6502::lower_program(&nir).unwrap()
    }

    fn staged(source: &str) -> (MirProgram, Candidate) {
        let mut p = lower(source, false);
        let c = discover(&p, &Mir6502Config::default()).candidates.remove(0);
        let r = p.routines.iter_mut().find(|r| r.id == c.routine).unwrap();
        let bi = r
            .blocks
            .iter()
            .position(|b| b.id == c.return_block)
            .unwrap();
        let oi = r.blocks[bi].ops.len() - 1;
        let MirOp::CopyBytes { destination, .. } = &r.blocks[bi].ops[oi] else {
            panic!()
        };
        let bytes = copy_values::captured_bytes(
            r,
            &MirCfg::from_routine(r).unwrap(),
            &MirTempUseDefIndex::from_routine(r),
            &temp_widths::collect_routine_temp_widths(r),
            bi,
            oi,
            c.local,
            0,
            c.bytes,
        )
        .unwrap();
        let stores =
            super::super::super::copies::copy_captured_values(destination, &bytes).unwrap();
        r.blocks[bi].ops.splice(oi..=oi, stores);
        r.blocks[bi].ops.retain(|op| {
            !matches!(op,
            MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, .. }), .. } if *id == c.local)
        });
        crate::mir6502::verify_program(&p, MirPhase::PreMaterialization).unwrap();
        (p, c)
    }

    #[test]
    fn constructor_payload_forwarding_saves_staging_for_byte_and_word_reads() {
        for optimized in [false, true] {
            for source in [
                SOURCE.to_string(),
                SOURCE.replace("CARD", "BYTE").replace("1234", "42"),
                SOURCE
                    .replace("Make(CARD n)", "Make(BYTE POINTER n)")
                    .replace("FOUND(n)", "FOUND(CARD(n))")
                    .replace("Make(1234)", "Make(BYTE POINTER(1234))"),
                SOURCE.replace(
                    "RETURN(Choice.FOUND(n))",
                    "CARD saved saved=n RETURN(Choice.FOUND(saved))",
                ),
            ] {
                let p = lower(&source, optimized);
                let mut off = Mir6502Config::default();
                off.enable_constructor_payload_forwarding = false;
                let old = crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000)
                    .unwrap();
                let new = crate::mir6502::materialize_program_with_origin(
                    p,
                    &Mir6502Config::default(),
                    0x3000,
                )
                .unwrap();
                assert!(
                    Cost::measure(&new, 0x3000).unwrap().bytes
                        < Cost::measure(&old, 0x3000).unwrap().bytes,
                    "{source}, optimized={optimized}"
                );
                assert!(occupied(&new).is_subset(&occupied(&old)));
            }
        }
    }

    #[test]
    fn constructor_payload_forwarding_rejects_changed_reads_and_additional_uses() {
        for reason in [
            "write",
            "pointer write",
            "call",
            "barrier",
            "extra use",
            "redefinition",
            "out of bounds",
            "absolute",
            "global",
            "budget",
        ] {
            let (mut p, c) = staged(SOURCE);
            let r = p.routines.iter_mut().find(|r| r.id == c.routine).unwrap();
            let b = r
                .blocks
                .iter_mut()
                .find(|b| b.id == c.return_block)
                .unwrap();
            let end = b.ops.len();
            let MirOp::StoreIndirect { src, .. } = &b.ops[end - 1] else {
                panic!()
            };
            let temp = lane(src).unwrap().temp;
            let index = b
                .ops
                .iter()
                .position(
                    |op| matches!(op, MirOp::Load { dst: MirDef::VTemp(id), .. } if *id == temp),
                )
                .unwrap();
            let MirOp::Load {
                src: MirAddr::Direct(mem),
                ..
            } = b.ops[index].clone()
            else {
                panic!()
            };
            let inserted = match reason {
                "write" => Some(MirOp::Store {
                    dst: MirAddr::Direct(mem),
                    src: MirValue::ConstU16(99),
                    width: MirWidth::Word,
                }),
                "pointer write" => Some(MirOp::Store {
                    dst: MirAddr::Deref {
                        ptr: MirValue::ConstU16(0x9000),
                        offset: 0,
                    },
                    src: MirValue::ConstU8(99),
                    width: MirWidth::Byte,
                }),
                "call" => Some(MirOp::Call {
                    target: MirCallTarget::Routine(c.routine),
                    abi: MirCallAbi {
                        params: vec![],
                        result: None,
                        additional_results: vec![],
                        clobbers: MirRegisterSet::default(),
                        preserves: MirRegisterSet::default(),
                    },
                    effects: MirEffects::default(),
                    args: vec![],
                    result: None,
                    additional_results: vec![],
                }),
                "barrier" => Some(MirOp::Barrier {
                    effects: MirEffects::default(),
                }),
                "extra use" => Some(MirOp::Move {
                    dst: MirDef::VTemp(MirTempId(900)),
                    src: MirValue::Def(MirDef::VTemp(temp)),
                    width: MirWidth::Word,
                }),
                "redefinition" => Some(b.ops[index].clone()),
                "out of bounds" | "absolute" | "global" => {
                    let MirOp::Load { src, .. } = &mut b.ops[index] else {
                        panic!()
                    };
                    *src = MirAddr::Direct(match reason {
                        "absolute" => MirMem::Absolute(0xD000),
                        "global" => MirMem::Global {
                            id: p.globals[0].id,
                            offset: 0,
                        },
                        _ => offset_cell(&mem, 1).unwrap(),
                    });
                    None
                }
                "budget" => {
                    for _ in 0..65 {
                        b.ops.insert(
                            index + 1,
                            MirOp::LoadImm {
                                dst: MirDef::VTemp(MirTempId(900)),
                                value: 0,
                                width: MirWidth::Byte,
                            },
                        );
                    }
                    None
                }
                _ => unreachable!(),
            };
            if let Some(op) = inserted {
                b.ops.insert(index + 1, op);
            }
            let r = p.routines.iter().find(|r| r.id == c.routine).unwrap();
            assert!(select(&p, r, &c).is_none(), "{reason}");
        }
    }

    #[test]
    fn constructor_payload_cast_chains_require_private_width_preserving_uses() {
        let source = SOURCE
            .replace("Make(CARD n)", "Make(BYTE POINTER n)")
            .replace("FOUND(n)", "FOUND(CARD(n))")
            .replace("Make(1234)", "Make(BYTE POINTER(1234))");
        for reason in ["width", "extra use", "chain budget"] {
            let (mut p, c) = staged(&source);
            let r = p.routines.iter().find(|r| r.id == c.routine).unwrap();
            assert!(select(&p, r, &c).is_some());
            let r = p.routines.iter_mut().find(|r| r.id == c.routine).unwrap();
            let b = r
                .blocks
                .iter_mut()
                .find(|b| b.id == c.return_block)
                .unwrap();
            let index = b
                .ops
                .iter()
                .position(|op| matches!(op, MirOp::Move { .. }))
                .unwrap();
            let MirOp::Move {
                src: MirValue::Def(MirDef::VTemp(root)),
                ..
            } = b.ops[index]
            else {
                panic!()
            };
            match reason {
                "width" => {
                    let MirOp::Move { width, .. } = &mut b.ops[index] else {
                        panic!()
                    };
                    *width = MirWidth::Byte;
                }
                "extra use" => b.ops.insert(
                    index,
                    MirOp::Move {
                        dst: MirDef::VTemp(MirTempId(900)),
                        src: MirValue::Def(MirDef::VTemp(root)),
                        width: MirWidth::Word,
                    },
                ),
                "chain budget" => {
                    let mut previous = root;
                    for n in 0..8 {
                        let next = MirTempId(900 + n);
                        b.ops.insert(
                            index + n as usize,
                            MirOp::Move {
                                dst: MirDef::VTemp(next),
                                src: MirValue::Def(MirDef::VTemp(previous)),
                                width: MirWidth::Word,
                            },
                        );
                        previous = next;
                    }
                    let MirOp::Move { src, .. } = &mut b.ops[index + 8] else {
                        panic!()
                    };
                    *src = MirValue::Def(MirDef::VTemp(previous));
                }
                _ => unreachable!(),
            }
            let r = p.routines.iter().find(|r| r.id == c.routine).unwrap();
            assert!(select(&p, r, &c).is_none(), "{reason}");
        }
    }

    #[test]
    fn constructor_payload_alias_rejection_retains_constructor_savings() {
        let mut p = lower(SOURCE, false);
        let mut off = Mir6502Config::default();
        off.enable_constructor_payload_forwarding = false;
        off.enable_constructor_home_elision = false;
        let baseline =
            crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
        let r = baseline.routines.iter().find(|r| r.name == "Make").unwrap();
        let MirStorageBase::Param(id) = r.frame.params[1].base else {
            panic!()
        };
        let address = super::super::super::MaterializeLayout::new(&baseline, 0x3000)
            .mem_address(r.id, &MirMem::Param { id, offset: 0 })
            .unwrap();
        let mut alias = p
            .globals
            .iter()
            .find(|g| g.storage_size > 0)
            .unwrap()
            .clone();
        alias.id = crate::nir::SymbolId(900);
        alias.name = "payload_alias".into();
        alias.storage_size = 1;
        alias.width = Some(MirWidth::Byte);
        alias.backing = MirGlobalBacking::Absolute(address);
        alias.init = None;
        p.globals.push(alias);
        let before =
            crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
        let mut on = off.clone();
        on.enable_constructor_payload_forwarding = true;
        let after =
            crate::mir6502::materialize_program_with_origin(p.clone(), &on, 0x3000).unwrap();
        assert_eq!(
            before, after,
            "payload rejection must retain store-only output"
        );
        off.enable_constructor_store_elision = false;
        let staged = crate::mir6502::materialize_program_with_origin(p, &off, 0x3000).unwrap();
        assert!(
            Cost::measure(&after, 0x3000).unwrap().bytes
                < Cost::measure(&staged, 0x3000).unwrap().bytes
        );
    }
}
