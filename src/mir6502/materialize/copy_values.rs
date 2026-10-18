//! Reuse already captured bytes instead of reading and staging a small copy.
//! Constructor stores remain observable; no lifetime or no-alias assumption is
//! made about the destination. Final emission must prove that the trade saves
//! space without increasing routine homes or zero-page demand.
use super::{address_values::Cost, copies, tiny_fills};
use crate::mir6502::{
    Mir6502Config, MirDiagnostic,
    analysis::{
        cfg::MirCfg,
        effects::classify_op,
        use_def::{MirTempLane, MirTempUseDefIndex},
    },
    ir::*,
};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: u16 = 8;
const MAX_OPS: usize = 64;
const MAX_BLOCKS: usize = 4;

pub(in crate::mir6502) fn materialize_costed(
    original: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let mut off = config.clone();
    off.enable_copy_value_forwarding = false;
    let fallback = |p, reporting| {
        crate::mir6502::materialize_resolved_program_without_copy_costing(
            p, &off, origin, runtime, reporting,
        )
    };
    // Avoid another materialization for programs without small local copies.
    if !original
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .any(|op| {
            matches!(
                op,
                MirOp::CopyBytes {
                    source: MirAddr::Direct(MirMem::Local { .. }),
                    size: 1..=MAX_BYTES,
                    source_volatile: false,
                    destination_volatile: false,
                    ..
                }
            )
        })
    {
        return fallback(original, report);
    }
    let mut selected = original.clone();
    let tiny_selected = if config.enable_tiny_constant_fills {
        tiny_fills::select(&mut selected)
    } else {
        Vec::new()
    };
    let candidates = select(&mut selected);
    if candidates.is_empty() {
        return fallback(original, report);
    }
    crate::mir6502::verify_program(&selected, MirPhase::PreMaterialization)?;
    let baseline = fallback(original.clone(), false)?;
    let Ok(proposed) = fallback(selected.clone(), false) else {
        return fallback(original, report);
    };
    let accepted = match (
        Cost::measure(&baseline, origin),
        Cost::measure(&proposed, origin),
    ) {
        (Some(old), Some(new)) => {
            new.bytes < old.bytes
                && candidates.keys().all(|&id| new.does_not_grow(&old, id))
                && occupied_zero_page(&proposed).is_subset(&occupied_zero_page(&baseline))
        }
        _ => false,
    };
    if report {
        let mut stats = super::MirPeepholeStats::default();
        if accepted {
            for (routine, count) in tiny_selected {
                stats.record_many(routine, "tiny-constant-fill-selected", count);
            }
        }
        for (&routine, &count) in &candidates {
            stats.record_many(
                routine,
                if accepted {
                    "aggregate-copy-values-forwarded"
                } else {
                    "aggregate-copy-values-declined-final-cost"
                },
                count,
            );
        }
        super::maybe_report_peepholes(&original, &stats, config);
    }
    if report && super::stats::reporting_enabled(config) {
        fallback(if accepted { selected } else { original }, true)
    } else {
        Ok(if accepted { proposed } else { baseline })
    }
}

fn occupied_zero_page(program: &MirProgram) -> BTreeSet<u8> {
    program
        .routines
        .iter()
        .flat_map(|r| {
            r.frame.fixed_zero_page.iter().map(|s| s.0).chain(
                r.frame
                    .zero_page_allocations
                    .iter()
                    .flat_map(|a| (0..a.size).map(move |n| a.start.0.wrapping_add(n))),
            )
        })
        .collect()
}

fn select(program: &mut MirProgram) -> BTreeMap<RoutineId, usize> {
    let mut counts = BTreeMap::new();
    for routine in &mut program.routines {
        // A selected copy exposes ordinary stores for a following snapshot
        // copy. Re-prove against the new routine each round. Every successful
        // round removes at least one CopyBytes and never creates another.
        loop {
            let Ok(cfg) = MirCfg::from_routine(routine) else {
                break;
            };
            let definitions = MirTempUseDefIndex::from_routine(routine);
            let widths = super::temp_widths::collect_routine_temp_widths(routine);
            // Plans use one immutable routine. Apply from the end so earlier
            // instruction indices and all lifetime proofs remain valid.
            let plans = routine
                .blocks
                .iter()
                .enumerate()
                .flat_map(|(block, b)| {
                    let (cfg, definitions, widths, routine) =
                        (&cfg, &definitions, &widths, &*routine);
                    b.ops.iter().enumerate().filter_map(move |(op_index, op)| {
                        let MirOp::CopyBytes {
                            destination,
                            source: MirAddr::Direct(MirMem::Local { id, offset }),
                            size: 1..=MAX_BYTES,
                            source_volatile: false,
                            destination_volatile: false,
                        } = op
                        else {
                            return None;
                        };
                        let MirOp::CopyBytes { size, .. } = op else {
                            unreachable!()
                        };
                        if classify_op(op).machine.register_reads != MirRegisterSet::default() {
                            return None;
                        }
                        let slot = routine
                            .frame
                            .locals
                            .iter()
                            .find(|s| s.base == MirStorageBase::Local(*id) && s.offset == 0)?;
                        if offset.checked_add(*size)? > slot.storage_size
                            || matches!(
                                slot.init,
                                Some(
                                    MirStorageInit::Descriptor { .. }
                                        | MirStorageInit::RoutineAddress { .. }
                                )
                            )
                        {
                            return None;
                        }
                        let bytes = captured_bytes(
                            routine,
                            cfg,
                            definitions,
                            widths,
                            block,
                            op_index,
                            *id,
                            *offset,
                            *size,
                        )?;
                        let replacement = copies::copy_captured_values(destination, &bytes)?;
                        Some((block, op_index, replacement))
                    })
                })
                .collect::<Vec<_>>();
            if plans.is_empty() {
                break;
            }
            *counts.entry(routine.id).or_default() += plans.len();
            for (block, op_index, replacement) in plans.into_iter().rev() {
                routine.blocks[block]
                    .ops
                    .splice(op_index..=op_index, replacement);
            }
        }
    }
    counts
}

#[allow(clippy::too_many_arguments)]
fn captured_bytes(
    routine: &MirRoutine,
    cfg: &MirCfg,
    definitions: &MirTempUseDefIndex,
    widths: &BTreeMap<MirTempId, MirWidth>,
    block: usize,
    op_index: usize,
    source: crate::nir::LocalId,
    first: u16,
    size: u16,
) -> Option<Vec<MirValue>> {
    let mut bytes = vec![None; usize::from(size)];
    let mut current = &routine.blocks[block];
    let mut end = op_index;
    let mut visited = BTreeSet::new();
    let mut remaining = MAX_OPS;
    loop {
        if visited.len() == MAX_BLOCKS || !visited.insert(current.id) {
            return None;
        }
        for op in current.ops[..end].iter().rev() {
            remaining = remaining.checked_sub(1)?;
            if let MirOp::Store {
                dst: MirAddr::Direct(MirMem::Local { id, offset }),
                src,
                width,
            } = op
            {
                if *id != source {
                    return None;
                }
                let width = match width {
                    MirWidth::Byte => 1,
                    MirWidth::Word => 2,
                };
                for lane in 0..width {
                    let address = offset.checked_add(lane)?;
                    if let Some(index) = address.checked_sub(first).filter(|&n| n < size) {
                        let byte = &mut bytes[usize::from(index)];
                        if byte.is_none() {
                            *byte =
                                Some(captured_lane(src, lane as u8, width, definitions, widths)?);
                        }
                    }
                }
                if bytes.iter().all(Option::is_some) {
                    return bytes.into_iter().collect();
                }
                continue;
            }
            // A read of hardware/unknown memory, any write, or an ordering
            // barrier ends the proof. In particular a call may change even
            // unescaped routine-static storage through reentry.
            let effect = classify_op(op);
            if matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) || effect.memory.opaque
                || effect.memory.has_unknown_effects
                || effect.memory.indirect_reads
                || effect.memory.indirect_writes
                || effect.memory.may_write_any
                || !effect.memory.direct_writes.is_empty()
                || effect.memory.structured_reads != MirMemoryEffect::None
                || effect.memory.structured_writes != MirMemoryEffect::None
                || effect
                    .memory
                    .direct_reads
                    .iter()
                    .any(|r| !matches!(r.base, MirMem::Local { .. } | MirMem::Param { .. }))
            {
                return None;
            }
        }
        // Only follow an unconditional, unique entry. No joins, loop back
        // edges or block arguments can substitute another captured value.
        if !current.params.is_empty() || routine.blocks.first()?.id == current.id {
            return None;
        }
        let predecessors = cfg.predecessors(current.id);
        if predecessors.len() != 1 {
            return None;
        }
        let previous = routine
            .blocks
            .iter()
            .find(|b| predecessors.contains(&b.id))?;
        if !matches!(&previous.terminator, MirTerminator::Jump(edge) if edge.target == current.id && edge.args.is_empty())
        {
            return None;
        }
        current = previous;
        end = current.ops.len();
    }
}

fn captured_lane(
    value: &MirValue,
    lane: u8,
    width: u16,
    definitions: &MirTempUseDefIndex,
    widths: &BTreeMap<MirTempId, MirWidth>,
) -> Option<MirValue> {
    match value {
        MirValue::ConstU8(n) => Some(MirValue::ConstU8(if lane == 0 { *n } else { 0 })),
        MirValue::ConstU16(n) => Some(MirValue::ConstU8((n >> (lane * 8)) as u8)),
        MirValue::Def(MirDef::VTemp(id)) => {
            if lane == 1 && widths.get(id) == Some(&MirWidth::Byte) {
                return Some(MirValue::ConstU8(0));
            }
            definitions.unique_definition(MirTempLane {
                temp: *id,
                byte: lane,
            })?;
            Some(if width == 1 && widths.get(id) == Some(&MirWidth::Byte) {
                value.clone()
            } else {
                MirValue::Def(MirDef::VTempByte {
                    id: *id,
                    byte: lane,
                })
            })
        }
        MirValue::Def(MirDef::VTempByte { id, byte }) if lane == 0 => {
            definitions.unique_definition(MirTempLane {
                temp: *id,
                byte: *byte,
            })?;
            Some(value.clone())
        }
        MirValue::Word { lo, hi } => {
            captured_lane(if lane == 0 { lo } else { hi }, 0, 1, definitions, widths)
        }
        // Registers and PointerCell operands are live reads, not snapshots.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    const CHOICE: &str = "TYPE Choice=VARIANT [EMPTY FOUND [CARD address]] Choice result Choice FUNC Make(CARD n) IF n=0 THEN RETURN(Choice.EMPTY) FI RETURN(Choice.FOUND(n)) PROC Main() result=Make(1234) RETURN";
    const RECORD: &str = "TYPE Data=[BYTE a,b,c] Data FUNC Make(BYTE n) Data piece piece.a=n piece.b=22 piece.c=33 RETURN(piece) PROC Main() RETURN";

    #[test]
    fn aggregate_capture_provenance_is_verified_and_independent_of_names() {
        for optimized in [false, true] {
            let mut program = lower(CHOICE, optimized);
            let routine = program
                .routines
                .iter()
                .position(|r| r.name == "Make")
                .unwrap();
            let local = program.routines[routine]
                .frame
                .locals
                .iter()
                .position(|s| s.purpose == MirStoragePurpose::AggregateCapture)
                .unwrap();
            program.routines[routine].frame.locals[local].name = Some("ordinary_name".into());
            crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
            for malformed in 0..5 {
                let mut invalid = program.clone();
                let slot = &mut invalid.routines[routine].frame.locals[local];
                match malformed {
                    0 => slot.storage = MirStorageClass::Scalar,
                    1 => slot.offset = 1,
                    2 => slot.scalar_width = Some(MirWidth::Byte),
                    3 => slot.base = MirStorageBase::Absolute(0x9000),
                    _ => {
                        slot.init = Some(MirStorageInit::ZeroFill {
                            bytes: slot.storage_size,
                            mutable: true,
                            section: "data".into(),
                        })
                    }
                }
                let errors = crate::mir6502::verify_program(&invalid, MirPhase::PreMaterialization)
                    .unwrap_err();
                assert!(
                    errors
                        .iter()
                        .any(|e| e.message.contains("aggregate capture"))
                );
            }
        }
    }

    #[test]
    fn copy_values_forward_complete_snapshots_and_keep_constructor_writes() {
        for optimized in [false, true] {
            let mut program = lower(CHOICE, optimized);
            tiny_fills::select(&mut program);
            let before = program.routines.iter().find(|r| r.name == "Make").unwrap();
            let id = before.id;
            let stores = |r: &MirRoutine| {
                r.blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .filter(|op| {
                        matches!(
                            op,
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Local { .. }),
                                ..
                            }
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let original = stores(before);
            assert_eq!(select(&mut program).get(&id), Some(&2));
            let after = program.routines.iter().find(|r| r.id == id).unwrap();
            assert_eq!(stores(after), original);
            assert!(
                !after
                    .blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .any(|op| matches!(op, MirOp::CopyBytes { .. }))
            );
            crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
            assert!(select(&mut program).is_empty());
        }
    }

    #[test]
    fn copy_values_reject_missing_bytes_effects_and_live_reads() {
        for reason in [
            "missing byte",
            "call",
            "barrier",
            "pointer write",
            "absolute read",
            "live cell",
            "register",
            "register destination",
            "redefined temp",
            "source volatile",
            "destination volatile",
            "out of bounds",
        ] {
            let mut program = lower(RECORD, false);
            let r = program
                .routines
                .iter_mut()
                .find(|r| r.name == "Make")
                .unwrap();
            let b = r
                .blocks
                .iter_mut()
                .find(|b| b.ops.iter().any(|op| matches!(op, MirOp::CopyBytes { .. })))
                .unwrap();
            let copy = b
                .ops
                .iter()
                .position(|op| matches!(op, MirOp::CopyBytes { .. }))
                .unwrap();
            let store = b
                .ops
                .iter()
                .position(|op| {
                    matches!(
                        op,
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Local { .. }),
                            ..
                        }
                    )
                })
                .unwrap();
            let MirOp::Store { src: value, .. } = b.ops[store].clone() else {
                unreachable!()
            };
            match reason {
                "missing byte" => {
                    b.ops.remove(store);
                }
                "call" => {
                    let call = lower("PROC F() RETURN PROC Main() F() RETURN", false)
                        .routines
                        .into_iter()
                        .flat_map(|r| r.blocks)
                        .flat_map(|b| b.ops)
                        .find(|op| matches!(op, MirOp::Call { .. }))
                        .unwrap();
                    b.ops.insert(copy, call);
                }
                "barrier" => b.ops.insert(copy, copies::volatile_barrier()),
                "pointer write" => b.ops.insert(
                    copy,
                    MirOp::Store {
                        dst: MirAddr::Deref {
                            ptr: MirValue::ConstU16(0x800),
                            offset: 0,
                        },
                        src: MirValue::ConstU8(0),
                        width: MirWidth::Byte,
                    },
                ),
                "absolute read" => {
                    let temp = MirTempId(r.temps.iter().map(|t| t.id.0).max().unwrap() + 1);
                    r.temps.push(MirTemp { id: temp });
                    b.ops.insert(
                        copy,
                        MirOp::Load {
                            dst: MirDef::VTemp(temp),
                            src: MirAddr::Direct(MirMem::Absolute(0xD000)),
                            width: MirWidth::Byte,
                        },
                    );
                }
                "live cell" | "register" => {
                    if let MirOp::Store { src, .. } = &mut b.ops[store] {
                        *src = if reason == "live cell" {
                            MirValue::PointerCell(MirMem::Absolute(0x800))
                        } else {
                            MirValue::Def(MirDef::Reg(MirReg::A))
                        };
                    }
                }
                "register destination" => {
                    if let MirOp::CopyBytes { destination, .. } = &mut b.ops[copy] {
                        *destination = MirAddr::Deref {
                            ptr: MirValue::Word {
                                lo: Box::new(MirValue::Def(MirDef::Reg(MirReg::A))),
                                hi: Box::new(MirValue::Def(MirDef::Reg(MirReg::X))),
                            },
                            offset: 0,
                        };
                    }
                }
                "redefined temp" => {
                    let MirValue::Def(dst) = value else {
                        panic!("expected captured input")
                    };
                    b.ops.insert(
                        copy,
                        MirOp::Move {
                            dst,
                            src: MirValue::ConstU8(99),
                            width: MirWidth::Byte,
                        },
                    );
                }
                "source volatile" => {
                    if let MirOp::CopyBytes {
                        source_volatile, ..
                    } = &mut b.ops[copy]
                    {
                        *source_volatile = true
                    }
                }
                "destination volatile" => {
                    if let MirOp::CopyBytes {
                        destination_volatile,
                        ..
                    } = &mut b.ops[copy]
                    {
                        *destination_volatile = true
                    }
                }
                "out of bounds" => {
                    if let MirOp::CopyBytes { size, .. } = &mut b.ops[copy] {
                        *size += 1
                    }
                }
                _ => unreachable!(),
            }
            let before = program.clone();
            assert!(select(&mut program).is_empty(), "{reason}");
            assert_eq!(program, before, "{reason}");
        }
    }

    #[test]
    fn copy_values_preserve_last_store_and_enforce_the_byte_budget() {
        for size in [1, 3, 8, 9] {
            let writes = (0..size)
                .map(|i| format!("piece.bytes({i})={i} "))
                .collect::<String>();
            let source = format!(
                "TYPE Data=[BYTE ARRAY bytes({size})] Data FUNC Make() Data piece {writes} piece.bytes(0)=99 RETURN(piece) PROC Main() RETURN"
            );
            let mut program = lower(&source, false);
            let expected = program
                .routines
                .iter()
                .find(|r| r.name == "Make")
                .unwrap()
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter_map(|op| match op {
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Local { offset: 0, .. }),
                        src,
                        ..
                    } => Some(src.clone()),
                    _ => None,
                })
                .last()
                .unwrap();
            let writes_of_last_value = |p: &MirProgram| {
                p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops).filter(|op| {
                    matches!(op, MirOp::Store { src, .. } | MirOp::StoreIndirect { src, .. } if src == &expected)
                }).count()
            };
            let before = writes_of_last_value(&program);
            let copies = program
                .routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .filter(|op| matches!(op, MirOp::CopyBytes { .. }))
                .count();
            let selected = select(&mut program).values().sum::<usize>();
            assert_eq!(selected, if size <= MAX_BYTES { copies } else { 0 });
            crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
            assert_eq!(writes_of_last_value(&program), before + selected);
        }
    }

    #[test]
    fn copy_values_do_not_forward_across_an_alternate_entry() {
        let mut program = lower(CHOICE, true);
        tiny_fills::select(&mut program);
        let r = program
            .routines
            .iter_mut()
            .find(|r| r.name == "Make")
            .unwrap();
        let id = r.id;
        let exit = r
            .blocks
            .iter()
            .find(|b| {
                b.ops.iter().any(|op| {
                    matches!(
                        op,
                        MirOp::Store {
                            src: MirValue::ConstU8(1),
                            ..
                        }
                    )
                }) && b.ops.iter().any(|op| matches!(op, MirOp::CopyBytes { .. }))
            })
            .unwrap()
            .id;
        r.blocks.push(MirBlock {
            id: MirBlockId(r.blocks.iter().map(|b| b.id.0).max().unwrap() + 1),
            label: "alternate-entry".into(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Jump(MirEdge::plain(exit)),
        });
        // FOUND remains eligible; EMPTY cannot inherit its payload bytes from
        // just one of the two possible predecessors.
        assert_eq!(select(&mut program).get(&id), Some(&1));
    }

    #[test]
    fn copy_values_save_emitted_bytes_without_growing_homes() {
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            for optimized in [false, true] {
                let input = lower(CHOICE, optimized);
                let on = Mir6502Config::default();
                let off = Mir6502Config {
                    enable_copy_value_forwarding: false,
                    ..on.clone()
                };
                let old = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input.clone(),
                    &off,
                    0x3000,
                    runtime,
                )
                .unwrap();
                let new = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input, &on, 0x3000, runtime,
                )
                .unwrap();
                let (before, after) = (
                    Cost::measure(&old, 0x3000).unwrap(),
                    Cost::measure(&new, 0x3000).unwrap(),
                );
                assert!(
                    after.bytes < before.bytes,
                    "{runtime:?}, optimized={optimized}: {} -> {}",
                    before.bytes,
                    after.bytes
                );
                for r in &new.routines {
                    assert!(after.does_not_grow(&before, r.id));
                }
                assert!(occupied_zero_page(&new).is_subset(&occupied_zero_page(&old)));
            }
        }
    }

    #[test]
    fn copy_values_keep_the_original_when_staging_is_already_free() {
        let input = lower(
            "TYPE One=[BYTE value] One result PROC Make() One piece piece.value=7 result=piece RETURN PROC Main() Make() RETURN",
            false,
        );
        assert!(!select(&mut input.clone()).is_empty());
        let on = Mir6502Config::default();
        let off = Mir6502Config {
            enable_copy_value_forwarding: false,
            ..on.clone()
        };
        let before = crate::mir6502::materialize_program(input.clone(), &off).unwrap();
        let after = crate::mir6502::materialize_program(input, &on).unwrap();
        assert_eq!(
            after, before,
            "no saving: retain the original allocation and operations"
        );
    }
}
