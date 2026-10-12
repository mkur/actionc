//! Full-pair address materialization and bounded final-image transactions.
use super::*;
use crate::mir6502::MirRoutine;
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::rewrite::context::PreHomeRewriteContext;
use crate::mir6502::rewrite::context::{MirExitStateChange, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::{
    MirChangeSet, MirEffectDelta, MirRemovedDefinition, MirRewritePlan,
};
use std::collections::{BTreeMap, BTreeSet};

/// A complete-pair constructor preserves Y. Keep a captured unsigned byte
/// index there when its private saved byte has no other reader. Final placement
/// also proves that the removed store cannot change a base input.
pub(in crate::mir6502) fn discover_saved_byte_indexes(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<crate::mir6502::rewrite::plan::MirPostHomeRewritePlan> {
    let physical = |mem: &MirMem| match mem {
        MirMem::FixedZeroPage(slot) => Some(u16::from(slot.0)),
        MirMem::ZeroPage(slot) => routine
            .frame
            .zero_page_allocations
            .iter()
            .find(|allocation| allocation.slot == *slot)
            .map(|allocation| u16::from(allocation.start.0)),
        _ => layout.mem_address(routine.id, mem),
    };
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (start, ops) in block.ops.windows(4).enumerate() {
            let [
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(home),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
                constructor @ MirOp::MaterializeIndexedAddress {
                    consumer,
                    base,
                    index: MirValue::ConstU8(_) | MirValue::ConstU16(_),
                    scale: 1 | 2,
                },
                MirOp::AdvanceAddress {
                    consumer: advance,
                    index: MirValue::Word { lo, hi },
                    scale,
                },
            ] = ops
            else {
                continue;
            };
            if consumer != advance
                || consumer.preserves_index_in_y()
                || !peepholes::mem_is_private_scratch(home)
                || !matches!(lo.as_ref(), MirValue::PointerCell(mem) if mem == home)
                || !matches!(hi.as_ref(), MirValue::ConstU8(0))
                || classify_op(constructor).may_clobber_reg_compat(MirReg::Y)
            {
                continue;
            }
            fn reads_a(value: &MirValue) -> bool {
                match value {
                    MirValue::Def(MirDef::Reg(MirReg::A)) => true,
                    MirValue::Word { lo, hi } => reads_a(lo) || reads_a(hi),
                    _ => false,
                }
            }
            let Some(address) = physical(home) else {
                continue;
            };
            if reads_a(base) || (0xAC..=0xAF).contains(&address) {
                continue;
            }
            let effects = classify_op(constructor);
            if effects.memory.direct_reads.iter().any(|range| {
                physical(&range.base).is_none_or(|base| address.wrapping_sub(base) < range.bytes)
            }) {
                continue;
            }
            // Logical home deadness does not recognize a separately named
            // absolute alias of a spill. Check all surviving direct reads too.
            if routine.blocks.iter().any(|other| {
                other.ops.iter().enumerate().any(|(i, op)| {
                    !(other.id == block.id && (start..start + 4).contains(&i))
                        && classify_op(op).memory.direct_reads.iter().any(|range| {
                            range.base != *home
                                && physical(&range.base)
                                    .is_none_or(|base| address.wrapping_sub(base) < range.bytes)
                        })
                })
            }) {
                continue;
            }
            if let Some(plan) = crate::mir6502::rewrite::posthome::structural_plan(
                routine,
                context,
                block.id,
                start..start + 4,
                vec![
                    constructor.clone(),
                    MirOp::AdvanceAddress {
                        consumer: *advance,
                        index: MirValue::Word {
                            lo: Box::new(MirValue::Def(MirDef::Reg(MirReg::Y))),
                            hi: Box::new(MirValue::ConstU8(0)),
                        },
                        scale: *scale,
                    },
                ],
                MirExitStateChange::default(),
                "address-value-index-y",
                0,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(super) fn forward_saved_byte_indexes(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
    known_callees: &MirKnownCalleeSummaries,
    stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let result = MirPostHomeRewriteDriver::default()
        .run_fixed_point_with_known_callees(routine, known_callees, |r, c| {
            discover_saved_byte_indexes(r, c, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("address index forwarding failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, stats);
    Ok(())
}

pub(in crate::mir6502) fn discover_address_stores(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (start, ops) in block.ops.windows(2).enumerate() {
            let [
                MirOp::ComputeAddress {
                    dst: MirDef::VTemp(temp),
                    base,
                    index: Some(index),
                    elem_size,
                    offset,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(dst),
                    src: MirValue::Def(MirDef::VTemp(source)),
                    width: MirWidth::Word,
                },
            ] = ops
            else {
                continue;
            };
            if temp != source || !(1..=255).contains(elem_size) {
                continue;
            }
            let site = MirSite::Op {
                block: block.id,
                op_index: start + 1,
            };
            if !context
                .exclusive_word_temp_definition(*temp, site)
                .is_proven()
                || !context.flags_dead_after(
                    classify_op(&ops[0]).machine.flag_writes,
                    MirSite::Op {
                        block: block.id,
                        op_index: start,
                    },
                )
                || !context
                    .pointer_pair_dead_after(DEFAULT_POINTER_PAIR, context.point(site))
                    .is_proven()
                || !context
                    .pointer_pair_dead_after(INDEX_POINTER_PAIR, context.point(site))
                    .is_proven()
            {
                continue;
            }
            let mut replacement = Vec::new();
            if *offset == 0 {
                indexes::materialize_indexed_address_to_consumer(
                    base.clone(),
                    index.clone(),
                    *elem_size,
                    DEFAULT_POINTER_PAIR,
                    layout,
                    &mut replacement,
                );
            } else {
                // Construct the full displaced base in one operation. A
                // constant index avoids a separate base copy and increment.
                replacement.push(MirOp::MaterializeIndexedAddress {
                    consumer: DEFAULT_POINTER_PAIR,
                    base: base.clone(),
                    index: MirValue::ConstU16(*offset),
                    scale: 1,
                });
                replacement.push(MirOp::AdvanceAddress {
                    consumer: DEFAULT_POINTER_PAIR,
                    index: index.clone(),
                    scale: *elem_size as u8,
                });
            }
            // This is the complete address pair, with no residual Y component.
            // Keep the source/public word store at its original semantic point.
            replacement.push(MirOp::Store {
                dst: MirAddr::Direct(dst.clone()),
                src: MirValue::Word {
                    lo: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
                        MirFixedZpSlot(POINTER_SCRATCH_LO),
                    ))),
                    hi: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
                        MirFixedZpSlot(POINTER_SCRATCH_HI),
                    ))),
                },
                width: MirWidth::Word,
            });
            let Some(definitions) =
                crate::mir6502::rewrite::pilots::prove_removed_window_definitions(
                    block.id,
                    &block.ops,
                    start,
                    start + 2,
                    &replacement,
                    context,
                )
            else {
                continue;
            };
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: start..start + 2,
                replacement,
                removed_defs: definitions
                    .into_iter()
                    .map(|definition| MirRemovedDefinition { definition })
                    .collect(),
                exit_effect_delta: MirEffectDelta::MaterializedIndexConsumer,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: "address-value-store",
                observations: vec![],
                family_priority: 10,
                estimated_byte_saving: 0,
                estimated_cycle_saving: 0,
            });
        }
    }
    plans
}

pub(super) fn materialize_address_stores(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
    stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |r, c| discover_address_stores(r, c, layout),
            crate::mir6502::rewrite::address_values::address_value_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("address-store materialization failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, stats);
    Ok(())
}

struct Cost {
    bytes: usize,
    routine_bytes: BTreeMap<RoutineId, usize>,
    homes: BTreeMap<RoutineId, usize>,
}
impl Cost {
    fn measure(program: &MirProgram, origin: u16) -> Option<Self> {
        let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
        let summary = crate::mir6502::emit::emit_program(program, origin, &mut emitter).ok()?;
        let image = emitter.finish_with_relocations().ok()?;
        let mut routine_bytes = BTreeMap::new();
        for (routine, _, range) in summary.block_ranges {
            *routine_bytes.entry(routine).or_insert(0) += range.len();
        }
        let homes = program
            .routines
            .iter()
            .map(|r| {
                (
                    r.id,
                    r.frame.spills.len()
                        + r.frame
                            .params
                            .iter()
                            .chain(&r.frame.locals)
                            .filter(|s| {
                                matches!(
                                    s.base,
                                    MirStorageBase::Local(_) | MirStorageBase::Param(_)
                                )
                            })
                            .map(|s| usize::from(s.storage_size))
                            .sum::<usize>(),
                )
            })
            .collect();
        Some(Self {
            bytes: image.bytes.len(),
            routine_bytes,
            homes,
        })
    }
    fn routine(&self, id: RoutineId) -> Option<usize> {
        Some(*self.routine_bytes.get(&id)? + *self.homes.get(&id)?)
    }
}

/// Avoid introducing writes to source-owned physical scratch, including aliases
/// and absolute-backed local cells. Other inputs remain captured virtual values;
/// the allocator reserves all four carrier bytes in the materialized operation.
fn scratch_is_private(program: &MirProgram, origin: u16) -> bool {
    if origin < 0x100 {
        return false;
    }
    let layout = MaterializeLayout::new(program, origin);
    let overlaps = |address: u16, size: u16| {
        (0..size).any(|n| (0xAC..=0xAF).contains(&address.wrapping_add(n)))
    };
    for global in &program.globals {
        if layout
            .mem_address(
                RoutineId(0),
                &MirMem::Global {
                    id: global.id,
                    offset: 0,
                },
            )
            .is_none_or(|a| overlaps(a, global.storage_size))
        {
            return false;
        }
    }
    for r in &program.routines {
        for slot in r.frame.locals.iter().chain(&r.frame.params) {
            if let MirStorageBase::Absolute(a) = slot.base
                && overlaps(a.wrapping_add(slot.offset), slot.storage_size)
            {
                return false;
            }
        }
    }
    true
}

pub(in crate::mir6502) fn materialize_costed(
    original: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let mut baseline_config = config.clone();
    baseline_config.enable_indexed_address_values = false;
    if !scratch_is_private(&original, origin) {
        return crate::mir6502::materialize_resolved_program_without_address_costing(
            original,
            &baseline_config,
            origin,
            runtime,
            report,
        );
    }
    let mut selected = original.clone();
    let mut candidates = BTreeSet::new();
    for routine in &mut selected.routines {
        let mut driver = MirPreHomeRewriteDriver::default();
        let result = driver
            .run_fixed_point_by_key(
                routine,
                crate::mir6502::rewrite::address_values::discover_indexed_address_values,
                crate::mir6502::rewrite::address_values::address_value_rank,
            )
            .map_err(|error| {
                vec![MirDiagnostic::routine(
                    &routine.name,
                    format!("address-value provenance failed: {error:?}"),
                )]
            })?;
        if result.applied > 0
            || routine.blocks.iter().flat_map(|b| &b.ops).any(|op| {
                matches!(
                    op,
                    MirOp::ComputeAddress {
                        index: Some(_),
                        elem_size: 1..=255,
                        ..
                    }
                )
            })
        {
            candidates.insert(routine.id);
        }
    }
    if candidates.is_empty() {
        return crate::mir6502::materialize_resolved_program_without_address_costing(
            original,
            &baseline_config,
            origin,
            runtime,
            report,
        );
    }
    let baseline = crate::mir6502::materialize_resolved_program_without_address_costing(
        original.clone(),
        &baseline_config,
        origin,
        runtime,
        false,
    )?;
    let mut trial = crate::mir6502::materialize_resolved_program_without_address_costing(
        selected.clone(),
        config,
        origin,
        runtime,
        false,
    )?;
    let baseline_cost = Cost::measure(&baseline, origin);
    let trial_cost = Cost::measure(&trial, origin);
    let mut accepted = BTreeSet::new();
    for id in &candidates {
        if baseline_cost
            .as_ref()
            .and_then(|c| c.routine(*id))
            .zip(trial_cost.as_ref().and_then(|c| c.routine(*id)))
            .is_some_and(|(old, new)| new <= old)
        {
            accepted.insert(*id);
        }
    }
    if accepted != candidates && !accepted.is_empty() {
        for routine in &mut selected.routines {
            if candidates.contains(&routine.id) && !accepted.contains(&routine.id) {
                *routine = original
                    .routines
                    .iter()
                    .find(|r| r.id == routine.id)
                    .unwrap()
                    .clone();
            }
        }
        trial = crate::mir6502::materialize_resolved_program_without_address_costing(
            selected.clone(),
            config,
            origin,
            runtime,
            false,
        )?;
    }
    // Shared homes, runtime helpers and relaxed branches are charged again at
    // the whole-image boundary. A local saving is not a pooled-home saving.
    if accepted.is_empty()
        || !baseline_cost
            .as_ref()
            .zip(Cost::measure(&trial, origin).as_ref())
            .is_some_and(|(old, new)| new.bytes <= old.bytes)
    {
        accepted.clear();
    }
    let applied = !accepted.is_empty();
    if report {
        let mut stats = MirPeepholeStats::default();
        for id in &candidates {
            stats.record(
                *id,
                if accepted.contains(id) {
                    "address-value-applied"
                } else {
                    "address-value-declined-size"
                },
            );
        }
        for id in &candidates {
            if let Some((old, new)) = baseline_cost
                .as_ref()
                .and_then(|c| c.routine(*id))
                .zip(trial_cost.as_ref().and_then(|c| c.routine(*id)))
            {
                stats.record_dynamic(*id, format!("address-value-code-home-{old}-to-{new}"));
            }
        }
        maybe_report_peepholes(&original, &stats, config);
    }
    if report && stats::reporting_enabled(config) {
        return crate::mir6502::materialize_resolved_program_without_address_costing(
            if applied { selected } else { original },
            if applied { config } else { &baseline_config },
            origin,
            runtime,
            true,
        );
    }
    Ok(if applied { trial } else { baseline })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{MirBlock, MirEdge};
    pub(super) fn lower(source: &str) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        crate::mir6502::lower_program(&nir).unwrap()
    }
    #[test]
    fn saved_byte_index_requires_dead_home_and_disjoint_physical_inputs() {
        let mut p = lower("PROC Main() RETURN");
        let r = &mut p.routines[0];
        r.frame.spills = vec![MirSpillId(0)];
        let home = MirMem::Spill {
            id: MirSpillId(0),
            offset: 0,
        };
        let window = vec![
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(home.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::MaterializeIndexedAddress {
                consumer: DEFAULT_POINTER_PAIR,
                base: MirValue::ConstU16(0x5000),
                index: MirValue::ConstU16(38),
                scale: 1,
            },
            MirOp::AdvanceAddress {
                consumer: DEFAULT_POINTER_PAIR,
                index: MirValue::Word {
                    lo: Box::new(MirValue::PointerCell(home.clone())),
                    hi: Box::new(MirValue::ConstU8(0)),
                },
                scale: 20,
            },
        ];
        r.blocks[0].ops = window.clone();
        let layout = MaterializeLayout::new(&p, 0x3000);
        let address = layout.mem_address(p.routines[0].id, &home).unwrap();
        let run = |r: &mut MirRoutine| {
            MirPostHomeRewriteDriver::default()
                .run_fixed_point(r, |r, c| discover_saved_byte_indexes(r, c, &layout))
                .unwrap()
                .applied
        };
        let mut r = p.routines[0].clone();
        assert_eq!(run(&mut r), 1);
        assert_eq!(r.blocks[0].ops.len(), 2);
        // A was changed by the removed TYA; it cannot be an original base input.
        r.blocks[0].ops = window.clone();
        if let MirOp::MaterializeIndexedAddress { base, .. } = &mut r.blocks[0].ops[2] {
            *base = MirValue::Word {
                lo: Box::new(MirValue::Def(MirDef::Reg(MirReg::A))),
                hi: Box::new(MirValue::ConstU8(0x50)),
            };
        }
        assert_eq!(run(&mut r), 0);
        // Distinct symbolic names can refer to the exact same physical byte.
        r.blocks[0].ops = window.clone();
        if let MirOp::MaterializeIndexedAddress { base, .. } = &mut r.blocks[0].ops[2] {
            *base = MirValue::Word {
                lo: Box::new(MirValue::PointerCell(MirMem::Absolute(address))),
                hi: Box::new(MirValue::ConstU8(0x50)),
            };
        }
        assert_eq!(run(&mut r), 0);
        for mem in [home.clone(), MirMem::Absolute(address)] {
            r.blocks[0].ops = window.clone();
            r.blocks[0].ops.push(MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            });
            assert_eq!(run(&mut r), 0);
        }
        // A successor's use is included in the shared home-deadness proof.
        r.blocks[0].ops = window;
        r.blocks[0].terminator = MirTerminator::Jump(MirEdge {
            target: MirBlockId(1),
            args: vec![],
        });
        r.blocks.push(MirBlock {
            id: MirBlockId(1),
            label: "successor".into(),
            params: vec![],
            ops: vec![MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(home),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Return,
        });
        assert_eq!(run(&mut r), 0);
    }
    #[test]
    fn address_value_cost_includes_final_code_and_homes_for_two_layouts() {
        for (stride, offset) in [(2u16, 38u16), (20, 256), (255, 254), (256, 255)] {
            let pad = if stride == 2 {
                String::new()
            } else {
                format!("BYTE ARRAY padding({})", stride - 2)
            };
            let source = format!(
                "TYPE E=[CARD value {pad}] TYPE T=[BYTE ARRAY prefix({offset}) E ARRAY rows(2)] CARD result=$6000 CARD FUNC Address(T POINTER p CARD index) RETURN(@p.rows(index)) PROC Main() result=Address($5000,257) RETURN"
            );
            for runtime in [
                crate::runtime::Runtime::ActionCart,
                crate::runtime::Runtime::Standalone,
            ] {
                let input = lower(&source);
                let config = Mir6502Config::optimized();
                let mut off = config.clone();
                off.enable_indexed_address_values = false;
                let baseline = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input.clone(),
                    &off,
                    0x3000,
                    runtime,
                )
                .unwrap();
                let output = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input, &config, 0x3000, runtime,
                )
                .unwrap();
                assert!(
                    Cost::measure(&output, 0x3000).unwrap().bytes
                        <= Cost::measure(&baseline, 0x3000).unwrap().bytes,
                    "stride={stride}/offset={offset}/{runtime:?}"
                );
            }
        }
    }
    #[test]
    fn address_value_selection_rejects_source_scratch_aliases_and_absolute_locals() {
        for cell in [0xACu16, 0xAD, 0xAE, 0xAF] {
            let p = lower(&format!(
                "CARD input=${cell:04X},result=$6000 PROC Main() result=input*20+100 RETURN"
            ));
            assert!(!scratch_is_private(&p, 0x3000));
        }
        let mut p = lower(
            "TYPE E=[CARD value] E object CARD result PROC Main() E POINTER p p=@object result=p.value RETURN",
        );
        p.routines[0].frame.locals[0].base = MirStorageBase::Absolute(0xAE);
        assert!(!scratch_is_private(&p, 0x3000));
        assert!(!scratch_is_private(&p, 0x80));
    }
}

#[cfg(test)]
mod execution_tests {
    use super::tests::lower;
    use super::*;
    #[test]
    fn indexed_address_store_trials_preserve_complete_values_and_return_lanes() {
        for ty in ["BYTE", "CARD"] {
            for (stride, offset) in [(1u16, 38u16), (2, 256), (20, 38), (255, 255), (256, 254)] {
                let element = if stride == 1 {
                    "BYTE value".to_string()
                } else {
                    format!(
                        "CARD value {}",
                        if stride == 2 {
                            String::new()
                        } else {
                            format!("BYTE ARRAY pad({})", stride - 2)
                        }
                    )
                };
                let source = format!(
                    "TYPE E=[{element}] TYPE T=[BYTE ARRAY prefix({offset}) E ARRAY rows(2)] T POINTER base=$6000 {ty} indexValue=$6002 CARD result=$6004 CARD FUNC Address(T POINTER p {ty} index) RETURN(@p.rows(index)) PROC Main() result=Address(base,indexValue) RETURN"
                );
                for runtime in [
                    crate::runtime::Runtime::ActionCart,
                    crate::runtime::Runtime::Standalone,
                ] {
                    let mut selected = lower(&source);
                    for r in &mut selected.routines {
                        MirPreHomeRewriteDriver::default().run_fixed_point_by_key(r,crate::mir6502::rewrite::address_values::discover_indexed_address_values,crate::mir6502::rewrite::address_values::address_value_rank).unwrap();
                    }
                    let mir = crate::mir6502::materialize_resolved_program_without_address_costing(
                        selected,
                        &Mir6502Config::optimized(),
                        0x3000,
                        runtime,
                        false,
                    )
                    .unwrap();
                    if ty == "BYTE"
                        && stride == 20
                        && runtime == crate::runtime::Runtime::ActionCart
                    {
                        if let Ok(directory) = std::env::var("ACTIONC_ADDRESS_TRIAL_DUMP") {
                            std::fs::create_dir_all(&directory).unwrap();
                            std::fs::write(
                                std::path::Path::new(&directory).join("stride20-trial.mir"),
                                crate::mir6502::format_program(&mir),
                            )
                            .unwrap();
                        }
                    }
                    let mut emitter =
                        crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
                    let summary =
                        crate::mir6502::emit::emit_program(&mir, 0x3000, &mut emitter).unwrap();
                    let emission = emitter.finish_with_relocations().unwrap();
                    let output =
                        crate::mir6502::codegen_output(emission, 0x3000, summary, runtime, &mir);
                    for base in [0x5000u16, 0x50F1, 0xFFF0] {
                        for index in [0u16, 1, 127, 255, 256, 257, 65535] {
                            if ty == "BYTE" && index > 255 {
                                continue;
                            }
                            let mut ram = [0u8; 65536];
                            if runtime == crate::runtime::Runtime::ActionCart {
                                let cart = include_bytes!("../../../roms/action.rom");
                                ram[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
                                ram[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
                            }
                            ram[0x3000..0x3000 + output.bytes.len()].copy_from_slice(&output.bytes);
                            ram[0x6000..0x6002].copy_from_slice(&base.to_le_bytes());
                            ram[0x6002..0x6004].copy_from_slice(&index.to_le_bytes());
                            super::super::leaf_test_cpu::run_memory(
                                &mut ram,
                                usize::from(output.run_address),
                            );
                            let expected = base
                                .wrapping_add(offset)
                                .wrapping_add(index.wrapping_mul(stride));
                            assert_eq!(
                                u16::from_le_bytes([ram[0x6004], ram[0x6005]]),
                                expected,
                                "{ty}/stride={stride}/offset={offset}/base={base:04X}/index={index}/{runtime:?}"
                            );
                            assert_eq!([ram[0xA0], ram[0xA1]], expected.to_le_bytes());
                        }
                    }
                }
            }
        }
    }
}
