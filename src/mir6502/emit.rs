use std::collections::{HashMap, HashSet};

use crate::codegen::native_emitter::NativeTrackedEmitter;
use crate::codegen::{
    Absolute, CodegenAddressSpace, CodegenMachineBlockAnalysis, CodegenRoutineEffect,
    CodegenRoutineParam, CodegenRoutineSignature, CodegenSourceRange, CodegenSourceRangeKind,
    CodegenStorageSymbol, CodegenSymbolKind, CodegenSymbolScope, IndirectIndexedY, RoutineAddress,
    RoutineRange, SkippedRange, ZeroPage, ZeroPageX, opcode,
};
use crate::nir::{LocalId, ParamId, SymbolId};
use crate::source::{Span, source_char_byte};

use super::builtin::{MirBuiltinResolution, resolve_builtin_target};
use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirAddr, MirAddressConsumer, MirBinaryOp, MirBlockId, MirCallTarget, MirCarryIn, MirCompareOp,
    MirCond, MirCondDest, MirDef, MirEffects, MirFixedZpSlot, MirFlagTest, MirGlobalBacking,
    MirGlobalInit, MirMachineAtom, MirMachineByteSelector, MirMachineItem, MirMem, MirOp, MirPhase,
    MirPointerPair, MirProgram, MirReg, MirRoutine, MirRuntimeHelperTarget, MirSpillId,
    MirStorageBase, MirStorageId, MirStorageInit, MirStorageSlot, MirTerminator, MirUnaryOp,
    MirUpdateOp, MirValue, MirWidth, MirZpSlot, RoutineId,
};
use super::verify;

const SYNTHETIC_SPAN: Span = Span { start: 0, end: 0 };
const ADDRESS_INDEX_SCRATCH_LO: u8 = 0xAE;
const ADDRESS_INDEX_SCRATCH_HI: u8 = 0xAF;

#[derive(Debug, Default, Clone)]
pub(super) struct MirEmissionSummary {
    pub routine_addresses: Vec<RoutineAddress>,
    pub routine_ranges: Vec<RoutineRange>,
    pub routine_signatures: Vec<CodegenRoutineSignature>,
    pub storage_symbols: Vec<CodegenStorageSymbol>,
    pub skipped_ranges: Vec<SkippedRange>,
    pub source_ranges: Vec<CodegenSourceRange>,
    pub routine_effects: Vec<CodegenRoutineEffect>,
    pub machine_blocks: Vec<CodegenMachineBlockAnalysis>,
}

pub(super) fn emit_program(
    mir: &MirProgram,
    origin: u16,
    emitter: &mut NativeTrackedEmitter,
) -> Result<MirEmissionSummary, Vec<MirDiagnostic>> {
    verify::verify_program(mir, MirPhase::PreEmission)?;
    let mut layout_diagnostics = Vec::new();
    let inline_probe = MirObjectLayout::new(mir, origin, Some(origin), &mut layout_diagnostics);
    let mut deferred_base = inline_probe.storage_end;
    let mut layout = inline_probe;
    let mut routine_code_size = 0;
    let mut branch_plan = MirBranchRelaxationPlan::default();
    let mut converged = false;
    for _ in 0..16 {
        let mut iteration_diagnostics = Vec::new();
        let candidate =
            MirObjectLayout::new(mir, origin, Some(deferred_base), &mut iteration_diagnostics);
        let (emitted_size, candidate_branch_plan) =
            emitted_size_for_layout(mir, origin, candidate.clone(), &mut iteration_diagnostics);
        let storage_size = candidate.storage_end.saturating_sub(origin);
        let candidate_code_size = emitted_size.saturating_sub(storage_size);
        let next_deferred_base = candidate.storage_end.saturating_add(candidate_code_size);
        layout = candidate;
        routine_code_size = candidate_code_size;
        branch_plan = candidate_branch_plan;
        if next_deferred_base <= deferred_base {
            layout_diagnostics.append(&mut iteration_diagnostics);
            converged = true;
            break;
        }
        deferred_base = next_deferred_base;
    }
    if !converged {
        layout_diagnostics.push(MirDiagnostic {
            routine: None,
            block: None,
            message: "MIR layout did not converge while placing deferred storage".to_string(),
        });
    }
    layout
        .plan
        .push(MirSegmentKind::Code, layout.storage_end, routine_code_size);
    let mut ctx = MirEmitContext::with_layout(mir, origin, layout, branch_plan);
    ctx.diagnostics.append(&mut layout_diagnostics);
    emit_storage(&mut ctx, emitter);
    for routine in &mir.routines {
        emit_routine(&mut ctx, routine, emitter);
    }
    if ctx.diagnostics.is_empty() {
        Ok(ctx.summary)
    } else {
        Err(ctx.diagnostics)
    }
}

fn emitted_size_for_layout(
    mir: &MirProgram,
    origin: u16,
    layout: MirObjectLayout,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> (u16, MirBranchRelaxationPlan) {
    let mut branch_plan = MirBranchRelaxationPlan::default();
    let branch_count = mir
        .routines
        .iter()
        .flat_map(|routine| &routine.blocks)
        .filter(|block| matches!(block.terminator, MirTerminator::Branch { .. }))
        .count();

    for _ in 0..=branch_count.saturating_mul(2) {
        let (emitted_size, measurements, mut probe_diagnostics) =
            emission_probe_for_layout(mir, origin, layout.clone(), branch_plan.clone());
        if extend_branch_relaxation_plan(mir, &measurements, &mut branch_plan) {
            continue;
        }
        if let Some(target) =
            find_self_enabling_branch_relaxation(mir, origin, &layout, &measurements, &branch_plan)
        {
            branch_plan.direct_targets.insert(target);
            continue;
        }
        diagnostics.append(&mut probe_diagnostics);
        return (emitted_size, branch_plan);
    }

    diagnostics.push(MirDiagnostic {
        routine: None,
        block: None,
        message: "MIR forward-branch relaxation did not converge".to_string(),
    });
    let (emitted_size, _, mut probe_diagnostics) =
        emission_probe_for_layout(mir, origin, layout, branch_plan.clone());
    diagnostics.append(&mut probe_diagnostics);
    (emitted_size, branch_plan)
}

fn emission_probe_for_layout(
    mir: &MirProgram,
    origin: u16,
    layout: MirObjectLayout,
    branch_plan: MirBranchRelaxationPlan,
) -> (u16, MirEmissionMeasurements, Vec<MirDiagnostic>) {
    let mut ctx = MirEmitContext::with_layout(mir, origin, layout, branch_plan);
    let mut emitter = NativeTrackedEmitter::with_origin(origin);
    emit_storage(&mut ctx, &mut emitter);
    for routine in &mir.routines {
        emit_routine(&mut ctx, routine, &mut emitter);
    }
    (emitter.position() as u16, ctx.measurements, ctx.diagnostics)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct MirBranchRelaxationPlan {
    direct_targets: HashSet<(RoutineId, MirBlockId, MirBlockId)>,
}

impl MirBranchRelaxationPlan {
    fn contains(&self, routine: RoutineId, block: MirBlockId, target: MirBlockId) -> bool {
        self.direct_targets.contains(&(routine, block, target))
    }
}

#[derive(Debug, Default)]
struct MirEmissionMeasurements {
    block_positions: HashMap<(RoutineId, MirBlockId), usize>,
    branch_positions: HashMap<(RoutineId, MirBlockId), usize>,
}

fn extend_branch_relaxation_plan(
    mir: &MirProgram,
    measurements: &MirEmissionMeasurements,
    plan: &mut MirBranchRelaxationPlan,
) -> bool {
    let mut changed = false;
    for routine in &mir.routines {
        for block in &routine.blocks {
            let MirTerminator::Branch {
                cond,
                then_edge,
                else_edge,
            } = &block.terminator
            else {
                continue;
            };
            let Some(&branch_position) = measurements.branch_positions.get(&(routine.id, block.id))
            else {
                continue;
            };
            let then_block = then_edge.target;
            let else_block = else_edge.target;
            let Some(&then_position) = measurements.block_positions.get(&(routine.id, then_block))
            else {
                continue;
            };
            let Some(&else_position) = measurements.block_positions.get(&(routine.id, else_block))
            else {
                continue;
            };

            let then_fits = measured_branch_target_fits(
                cond,
                then_block,
                then_block,
                else_block,
                branch_position,
                then_position,
            );
            if then_fits {
                changed |= plan
                    .direct_targets
                    .insert((routine.id, block.id, then_block));
            }
            if else_block != then_block
                && measured_branch_target_fits(
                    cond,
                    else_block,
                    then_block,
                    else_block,
                    branch_position,
                    else_position,
                )
            {
                changed |= plan
                    .direct_targets
                    .insert((routine.id, block.id, else_block));
            }
        }
    }
    changed
}

fn find_self_enabling_branch_relaxation(
    mir: &MirProgram,
    origin: u16,
    layout: &MirObjectLayout,
    measurements: &MirEmissionMeasurements,
    plan: &MirBranchRelaxationPlan,
) -> Option<(RoutineId, MirBlockId, MirBlockId)> {
    const MAX_SELF_RELAXATION_SAVING: isize = 3;

    for routine in &mir.routines {
        for block in &routine.blocks {
            let MirTerminator::Branch {
                cond,
                then_edge,
                else_edge,
            } = &block.terminator
            else {
                continue;
            };
            let Some(&branch_position) = measurements.branch_positions.get(&(routine.id, block.id))
            else {
                continue;
            };
            let then_block = then_edge.target;
            let else_block = else_edge.target;
            for (is_then_target, target) in [(true, then_block), (false, else_block)] {
                let key = (routine.id, block.id, target);
                if plan.direct_targets.contains(&key) || !is_then_target && target == then_block {
                    continue;
                }
                let Some(&target_position) =
                    measurements.block_positions.get(&(routine.id, target))
                else {
                    continue;
                };
                let latest_branch_position =
                    if matches!(cond, MirCond::AnyFlagTest(_)) && target == else_block {
                        branch_position.saturating_add(2)
                    } else {
                        branch_position
                    };
                let offset = target_position as isize - (latest_branch_position as isize + 2);
                if !(128..=127 + MAX_SELF_RELAXATION_SAVING).contains(&offset) {
                    continue;
                }

                let mut trial_plan = plan.clone();
                trial_plan.direct_targets.insert(key);
                let (_, trial_measurements, _) =
                    emission_probe_for_layout(mir, origin, layout.clone(), trial_plan);
                let Some(&trial_branch_position) = trial_measurements
                    .branch_positions
                    .get(&(routine.id, block.id))
                else {
                    continue;
                };
                let Some(&trial_target_position) = trial_measurements
                    .block_positions
                    .get(&(routine.id, target))
                else {
                    continue;
                };
                if measured_branch_target_fits(
                    cond,
                    target,
                    then_block,
                    else_block,
                    trial_branch_position,
                    trial_target_position,
                ) {
                    return Some(key);
                }
            }
        }
    }
    None
}

fn measured_branch_target_fits(
    cond: &MirCond,
    target: MirBlockId,
    then_block: MirBlockId,
    else_block: MirBlockId,
    branch_position: usize,
    target_position: usize,
) -> bool {
    if matches!(cond, MirCond::AnyFlagTest(_)) && target == then_block {
        branch_offset_fits(branch_position, target_position)
            && branch_offset_fits(branch_position.saturating_add(2), target_position)
    } else {
        let branch_position = if matches!(cond, MirCond::AnyFlagTest(_)) && target == else_block {
            branch_position.saturating_add(2)
        } else {
            branch_position
        };
        branch_offset_fits(branch_position, target_position)
    }
}

struct MirEmitContext<'a> {
    origin: u16,
    layout: MirObjectLayout,
    diagnostics: Vec<MirDiagnostic>,
    summary: MirEmissionSummary,
    mir: &'a MirProgram,
    indirect_call_counter: u32,
    branch_plan: MirBranchRelaxationPlan,
    measurements: MirEmissionMeasurements,
}

impl<'a> MirEmitContext<'a> {
    fn with_layout(
        mir: &'a MirProgram,
        origin: u16,
        layout: MirObjectLayout,
        branch_plan: MirBranchRelaxationPlan,
    ) -> Self {
        let summary = MirEmissionSummary {
            storage_symbols: layout.storage_symbols(mir),
            skipped_ranges: layout.plan.skipped_ranges(),
            ..MirEmissionSummary::default()
        };
        Self {
            origin,
            layout,
            diagnostics: Vec::new(),
            summary,
            mir,
            indirect_call_counter: 0,
            branch_plan,
            measurements: MirEmissionMeasurements::default(),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct MirObjectLayout {
    globals: HashMap<SymbolId, MirStoragePlacement>,
    statics: HashMap<SymbolId, MirStoragePlacement>,
    routine_storage: HashMap<RoutineId, MirRoutineStorageLayout>,
    routine_labels: HashMap<RoutineId, String>,
    block_labels: HashMap<(RoutineId, MirBlockId), String>,
    storage_names: HashMap<String, u16>,
    storage_value_names: HashMap<String, u16>,
    routine_names: HashMap<String, RoutineId>,
    storage_items: Vec<MirStorageItem>,
    plan: MirLayoutPlan,
    storage_end: u16,
}

#[derive(Debug, Default, Clone)]
struct MirLayoutPlan {
    allocations: Vec<MirAllocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MirSegmentKind {
    LoadData,
    Code,
    DeferredData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MirAllocation {
    segment: MirSegmentKind,
    start: u16,
    len: u16,
}

impl MirLayoutPlan {
    fn push(&mut self, segment: MirSegmentKind, start: u16, len: u16) {
        if len == 0 {
            return;
        }
        self.allocations.push(MirAllocation {
            segment,
            start,
            len,
        });
    }

    fn skipped_ranges(&self) -> Vec<SkippedRange> {
        self.allocations
            .iter()
            .filter_map(|allocation| {
                (allocation.segment == MirSegmentKind::DeferredData).then_some(SkippedRange {
                    start: allocation.start,
                    len: allocation.len,
                })
            })
            .collect()
    }

    fn runtime_high_water(&self, fallback: u16) -> u16 {
        self.allocations
            .iter()
            .map(|allocation| allocation.start.saturating_add(allocation.len))
            .max()
            .unwrap_or(fallback)
    }
}

#[derive(Debug, Default, Clone)]
struct MirRoutineStorageLayout {
    params: HashMap<ParamId, MirStoragePlacement>,
    locals: HashMap<LocalId, MirStoragePlacement>,
    spills: HashMap<super::ir::MirSpillId, MirStoragePlacement>,
    zero_page: HashMap<MirZpSlot, u8>,
    machine_symbols: HashMap<String, MirMachineSymbol>,
}

#[derive(Debug, Clone, Copy)]
enum MirStoragePlacement {
    Absolute { address: u16, size: u16 },
    Unresolved,
}

#[derive(Debug, Clone)]
enum MirStorageItem {
    Global {
        id: SymbolId,
        address: u16,
    },
    Static {
        id: SymbolId,
        address: u16,
    },
    RoutineSlot {
        routine: RoutineId,
        slot: MirStorageSlot,
        address: u16,
    },
}

#[derive(Debug, Clone, Copy)]
enum ResolvedMem {
    Absolute(u16),
    ZeroPage(u8),
}

#[derive(Debug, Clone, Copy)]
enum ResolvedIndexedMem {
    AbsoluteX(u16),
    AbsoluteY(u16),
    ZeroPageX(u8),
    IndirectY(u8),
}

#[derive(Debug, Clone)]
enum MirMachineSymbol {
    Absolute(u16),
    Label(String),
}

impl MirObjectLayout {
    fn new(
        mir: &MirProgram,
        origin: u16,
        deferred_base: Option<u16>,
        diagnostics: &mut Vec<MirDiagnostic>,
    ) -> Self {
        let mut layout = Self::default();
        let mut cursor = origin;
        let mut deferred_cursor = deferred_base.unwrap_or(origin);
        for global in &mir.globals {
            match global.backing {
                MirGlobalBacking::Absolute(address) => {
                    layout.globals.insert(
                        global.id,
                        MirStoragePlacement::Absolute {
                            address,
                            size: global.storage_size,
                        },
                    );
                    layout
                        .storage_names
                        .insert(normalize_machine_name(&global.name), address);
                    if let Some(value) = machine_caret_global_value(global) {
                        layout
                            .storage_value_names
                            .insert(normalize_machine_name(&global.name), value);
                    }
                }
                MirGlobalBacking::Ordinary { .. } => {
                    let size = global_object_size(global.storage_size, global.init.as_ref());
                    let storage_address =
                        if deferred_base.is_some() && is_deferred_global_byte_array(global, size) {
                            let address = deferred_cursor;
                            deferred_cursor = deferred_cursor.saturating_add(size);
                            layout
                                .plan
                                .push(MirSegmentKind::DeferredData, address, size);
                            address
                        } else {
                            let address = cursor;
                            layout.storage_items.push(MirStorageItem::Global {
                                id: global.id,
                                address,
                            });
                            cursor = cursor.saturating_add(size);
                            layout.plan.push(MirSegmentKind::LoadData, address, size);
                            address
                        };
                    let address = logical_global_address(global, storage_address);
                    let logical_size = logical_global_size(global, size);
                    layout.globals.insert(
                        global.id,
                        MirStoragePlacement::Absolute {
                            address,
                            size: logical_size,
                        },
                    );
                    layout
                        .storage_names
                        .insert(normalize_machine_name(&global.name), address);
                    if let Some(value) = machine_caret_global_value(global) {
                        layout
                            .storage_value_names
                            .insert(normalize_machine_name(&global.name), value);
                    }
                }
                MirGlobalBacking::Alias { target, offset } => {
                    if let Some(address) = layout.global_address(target) {
                        let address = address.saturating_add(offset);
                        let size = global.storage_size;
                        layout
                            .globals
                            .insert(global.id, MirStoragePlacement::Absolute { address, size });
                        layout
                            .storage_names
                            .insert(normalize_machine_name(&global.name), address);
                        if let Some(value) = machine_caret_global_value(global) {
                            layout
                                .storage_value_names
                                .insert(normalize_machine_name(&global.name), value);
                        }
                    } else {
                        diagnostics.push(MirDiagnostic {
                            routine: None,
                            block: None,
                            message: format!(
                                "global alias `{}` target g{} is unresolved",
                                global.name, target.0
                            ),
                        });
                        layout
                            .globals
                            .insert(global.id, MirStoragePlacement::Unresolved);
                    }
                }
            }
        }
        for static_data in &mir.statics {
            let size = static_data.bytes.len() as u16;
            layout.storage_items.push(MirStorageItem::Static {
                id: static_data.id,
                address: cursor,
            });
            layout.plan.push(MirSegmentKind::LoadData, cursor, size);
            layout.statics.insert(
                static_data.id,
                MirStoragePlacement::Absolute {
                    address: cursor,
                    size,
                },
            );
            layout
                .storage_names
                .insert(normalize_machine_name(&static_data.name), cursor);
            cursor = cursor.saturating_add(size);
        }
        for routine in &mir.routines {
            layout
                .routine_names
                .insert(normalize_machine_name(&routine.name), routine.id);
            layout
                .routine_labels
                .insert(routine.id, routine_label(routine.id));
            for block in &routine.blocks {
                layout
                    .block_labels
                    .insert((routine.id, block.id), block_label(routine.id, block.id));
            }
            let mut routine_layout = MirRoutineStorageLayout::default();
            for allocation in &routine.frame.zero_page_allocations {
                routine_layout
                    .zero_page
                    .insert(allocation.slot, allocation.start.0);
            }
            for param in &routine.frame.params {
                place_routine_slot(
                    &mut routine_layout,
                    &mut layout.storage_items,
                    &mut layout.plan,
                    routine.id,
                    param,
                    &mut cursor,
                    &mut deferred_cursor,
                    deferred_base.is_some(),
                    diagnostics,
                );
            }
            for local in &routine.frame.locals {
                place_routine_slot(
                    &mut routine_layout,
                    &mut layout.storage_items,
                    &mut layout.plan,
                    routine.id,
                    local,
                    &mut cursor,
                    &mut deferred_cursor,
                    deferred_base.is_some(),
                    diagnostics,
                );
            }
            for spill in &routine.frame.spills {
                place_routine_slot(
                    &mut routine_layout,
                    &mut layout.storage_items,
                    &mut layout.plan,
                    routine.id,
                    &MirStorageSlot {
                        id: MirStorageId(spill.0),
                        name: None,
                        width: MirWidth::Byte,
                        base: MirStorageBase::Spill(*spill),
                        offset: 0,
                        mutable: true,
                        init: None,
                    },
                    &mut cursor,
                    &mut deferred_cursor,
                    deferred_base.is_some(),
                    diagnostics,
                );
            }
            layout.routine_storage.insert(routine.id, routine_layout);
        }
        layout.storage_end = cursor;
        layout
    }

    fn direct_mem(&self, routine: RoutineId, mem: &MirMem) -> Option<ResolvedMem> {
        match mem {
            MirMem::Absolute(address) => Some(ResolvedMem::Absolute(*address)),
            MirMem::Global { id, offset } => {
                self.resolve_offset(self.globals.get(id).copied(), *offset)
            }
            MirMem::Static { id, offset } => {
                self.resolve_offset(self.statics.get(id).copied(), *offset)
            }
            MirMem::Local { id, offset } => {
                let storage = self.routine_storage.get(&routine)?;
                self.resolve_offset(storage.locals.get(id).copied(), *offset)
            }
            MirMem::Param { id, offset } => {
                let storage = self.routine_storage.get(&routine)?;
                self.resolve_offset(storage.params.get(id).copied(), *offset)
            }
            MirMem::Spill { id, offset } => {
                let storage = self.routine_storage.get(&routine)?;
                self.resolve_offset(storage.spills.get(id).copied(), *offset)
            }
            MirMem::ZeroPage(slot) => {
                let storage = self.routine_storage.get(&routine)?;
                let address = *storage.zero_page.get(slot)?;
                Some(ResolvedMem::ZeroPage(address))
            }
            MirMem::FixedZeroPage(MirFixedZpSlot(address)) => Some(ResolvedMem::ZeroPage(*address)),
        }
    }

    fn zero_page_slot(&self, routine: RoutineId, slot: MirZpSlot) -> Option<u8> {
        self.routine_storage
            .get(&routine)?
            .zero_page
            .get(&slot)
            .copied()
    }

    fn indexed_addr(&self, routine: RoutineId, addr: &MirAddr) -> Option<ResolvedIndexedMem> {
        match addr {
            MirAddr::AbsoluteIndexedX { base } => match self.direct_mem(routine, base)? {
                ResolvedMem::Absolute(address) => Some(ResolvedIndexedMem::AbsoluteX(address)),
                ResolvedMem::ZeroPage(_) => None,
            },
            MirAddr::AbsoluteIndexedY { base } => match self.direct_mem(routine, base)? {
                ResolvedMem::Absolute(address) => Some(ResolvedIndexedMem::AbsoluteY(address)),
                ResolvedMem::ZeroPage(_) => None,
            },
            MirAddr::ZeroPageIndexedX { base } => Some(ResolvedIndexedMem::ZeroPageX(
                self.zero_page_slot(routine, *base)?,
            )),
            MirAddr::IndirectIndexedY { zp } => Some(ResolvedIndexedMem::IndirectY(
                self.zero_page_slot(routine, *zp)?,
            )),
            MirAddr::FixedIndirectIndexedY { zp } => Some(ResolvedIndexedMem::IndirectY(zp.0)),
            _ => None,
        }
    }

    fn resolve_offset(
        &self,
        placement: Option<MirStoragePlacement>,
        offset: u16,
    ) -> Option<ResolvedMem> {
        match placement? {
            MirStoragePlacement::Absolute { address, size } => {
                if offset < size {
                    Some(ResolvedMem::Absolute(address.saturating_add(offset)))
                } else {
                    None
                }
            }
            MirStoragePlacement::Unresolved => None,
        }
    }

    fn static_address(&self, id: crate::nir::SymbolId) -> Option<u16> {
        self.statics.get(&id).and_then(|placement| match placement {
            MirStoragePlacement::Absolute { address, size: _ } => Some(*address),
            MirStoragePlacement::Unresolved => None,
        })
    }

    fn global_address(&self, id: crate::nir::SymbolId) -> Option<u16> {
        self.globals.get(&id).and_then(|placement| match placement {
            MirStoragePlacement::Absolute { address, size: _ } => Some(*address),
            MirStoragePlacement::Unresolved => None,
        })
    }

    fn routine_label(&self, routine: RoutineId) -> String {
        self.routine_labels
            .get(&routine)
            .cloned()
            .unwrap_or_else(|| routine_label(routine))
    }

    fn block_label(&self, routine: RoutineId, block: MirBlockId) -> String {
        self.block_labels
            .get(&(routine, block))
            .cloned()
            .unwrap_or_else(|| block_label(routine, block))
    }

    fn machine_symbol(&self, routine: RoutineId, name: &str) -> Option<MirMachineSymbol> {
        let normalized = normalize_machine_name(name);
        if let Some(symbol) = self
            .routine_storage
            .get(&routine)
            .and_then(|storage| storage.machine_symbols.get(&normalized))
            .cloned()
        {
            return Some(symbol);
        }
        if let Some(address) = self.storage_names.get(&normalized).copied() {
            return Some(MirMachineSymbol::Absolute(address));
        }
        self.routine_names
            .get(&normalized)
            .copied()
            .map(|routine| MirMachineSymbol::Label(self.routine_label(routine)))
    }

    fn machine_caret_symbol(&self, name: &str) -> Option<u16> {
        self.storage_value_names
            .get(&normalize_machine_name(name))
            .copied()
    }

    fn storage_symbols(&self, mir: &MirProgram) -> Vec<CodegenStorageSymbol> {
        let mut symbols = Vec::new();
        for global in &mir.globals {
            if let Some(symbol) = storage_symbol(
                global.name.clone(),
                CodegenSymbolScope::Global,
                CodegenSymbolKind::Storage,
                self.globals.get(&global.id).copied(),
            ) {
                symbols.push(symbol);
            }
        }
        for static_data in &mir.statics {
            if let Some(symbol) = storage_symbol(
                static_data.name.clone(),
                CodegenSymbolScope::Global,
                CodegenSymbolKind::Storage,
                self.statics.get(&static_data.id).copied(),
            ) {
                symbols.push(symbol);
            }
        }
        for routine in &mir.routines {
            for slot in routine
                .frame
                .params
                .iter()
                .chain(routine.frame.locals.iter())
            {
                push_routine_storage_symbol(self, routine, slot, &mut symbols);
            }
            for spill in &routine.frame.spills {
                let slot = MirStorageSlot {
                    id: MirStorageId(spill.0),
                    name: None,
                    width: MirWidth::Byte,
                    base: MirStorageBase::Spill(*spill),
                    offset: 0,
                    mutable: true,
                    init: None,
                };
                push_routine_storage_symbol(self, routine, &slot, &mut symbols);
            }
        }
        symbols
    }
}

fn push_routine_storage_symbol(
    layout: &MirObjectLayout,
    routine: &MirRoutine,
    slot: &MirStorageSlot,
    symbols: &mut Vec<CodegenStorageSymbol>,
) {
    let name = routine_slot_name(slot);
    if let Some(symbol) = storage_symbol(
        name,
        CodegenSymbolScope::Routine(routine.name.clone()),
        match slot.base {
            MirStorageBase::Param(_) => CodegenSymbolKind::Parameter,
            _ => CodegenSymbolKind::Local,
        },
        routine_slot_placement(layout, routine.id, slot),
    ) {
        symbols.push(symbol);
    }
}

fn place_routine_slot(
    routine_layout: &mut MirRoutineStorageLayout,
    storage_items: &mut Vec<MirStorageItem>,
    plan: &mut MirLayoutPlan,
    routine: RoutineId,
    slot: &MirStorageSlot,
    cursor: &mut u16,
    deferred_cursor: &mut u16,
    defer_large_slots: bool,
    diagnostics: &mut Vec<MirDiagnostic>,
) {
    let placement = match slot.base {
        MirStorageBase::Absolute(address) => MirStoragePlacement::Absolute {
            address: address.saturating_add(slot.offset),
            size: slot_size(slot),
        },
        MirStorageBase::LocalAlias { target, .. } => {
            if let Some(target) = routine_layout.locals.get(&target).copied()
                && let Some(address) = placement_address(target)
            {
                MirStoragePlacement::Absolute {
                    address: address.saturating_add(slot.offset),
                    size: logical_slot_size(slot, width_size(slot.width)),
                }
            } else {
                MirStoragePlacement::Unresolved
            }
        }
        MirStorageBase::Param(_) | MirStorageBase::Local(_) => {
            let size = slot_size(slot);
            let storage_address = if defer_large_slots && is_deferred_byte_array_slot(slot, size) {
                let address = *deferred_cursor;
                *deferred_cursor = deferred_cursor.saturating_add(size);
                plan.push(MirSegmentKind::DeferredData, address, size);
                address
            } else {
                let address = *cursor;
                *cursor = cursor.saturating_add(size);
                storage_items.push(MirStorageItem::RoutineSlot {
                    routine,
                    slot: slot.clone(),
                    address,
                });
                plan.push(MirSegmentKind::LoadData, address, size);
                address
            };
            MirStoragePlacement::Absolute {
                address: logical_slot_address(slot, storage_address),
                size: logical_slot_size(slot, size),
            }
        }
        MirStorageBase::Spill(id) => {
            let address = *cursor;
            let size = slot_size(slot);
            *cursor = cursor.saturating_add(size);
            storage_items.push(MirStorageItem::RoutineSlot {
                routine,
                slot: slot.clone(),
                address,
            });
            plan.push(MirSegmentKind::LoadData, address, size);
            routine_layout
                .spills
                .insert(id, MirStoragePlacement::Absolute { address, size });
            MirStoragePlacement::Absolute { address, size }
        }
        MirStorageBase::Global(_) | MirStorageBase::Static(_) => {
            diagnostics.push(MirDiagnostic {
                routine: Some(format!("r{}", routine.0)),
                block: None,
                message: "routine slot aliases global/static storage; emission layout does not duplicate it".to_string(),
            });
            MirStoragePlacement::Unresolved
        }
    };
    match slot.base {
        MirStorageBase::Param(id) => {
            routine_layout.params.insert(id, placement);
        }
        MirStorageBase::Local(id) => {
            routine_layout.locals.insert(id, placement);
        }
        MirStorageBase::LocalAlias { id, .. } => {
            routine_layout.locals.insert(id, placement);
        }
        MirStorageBase::Spill(_) => {}
        _ => {}
    }
    if let Some(name) = &slot.name
        && let Some(symbol) = fixed_machine_alias(slot)
            .map(MirMachineSymbol::Absolute)
            .or_else(|| placement_address(placement).map(MirMachineSymbol::Absolute))
    {
        routine_layout
            .machine_symbols
            .insert(normalize_machine_name(name), symbol);
    }
}

fn placement_address(placement: MirStoragePlacement) -> Option<u16> {
    match placement {
        MirStoragePlacement::Absolute { address, .. } => Some(address),
        MirStoragePlacement::Unresolved => None,
    }
}

fn logical_global_address(global: &super::ir::MirGlobal, storage_address: u16) -> u16 {
    global
        .init
        .as_ref()
        .and_then(global_descriptor_backing_size)
        .map_or(storage_address, |backing_size| {
            storage_address.saturating_add(backing_size)
        })
}

fn logical_global_size(global: &super::ir::MirGlobal, object_size: u16) -> u16 {
    global
        .init
        .as_ref()
        .and_then(global_descriptor_size)
        .unwrap_or(object_size)
}

fn logical_slot_address(slot: &MirStorageSlot, storage_address: u16) -> u16 {
    slot.init
        .as_ref()
        .and_then(slot_descriptor_backing_size)
        .map_or(storage_address, |backing_size| {
            storage_address.saturating_add(backing_size)
        })
}

fn logical_slot_size(slot: &MirStorageSlot, object_size: u16) -> u16 {
    slot.init
        .as_ref()
        .and_then(slot_descriptor_size)
        .unwrap_or(object_size)
}

fn global_descriptor_backing_size(init: &MirGlobalInit) -> Option<u16> {
    match init {
        MirGlobalInit::Descriptor { backing, .. } => {
            Some((backing.bytes.len() as u16).saturating_add(backing.zero_fill))
        }
        _ => None,
    }
}

fn global_descriptor_size(init: &MirGlobalInit) -> Option<u16> {
    match init {
        MirGlobalInit::Descriptor {
            descriptor_size, ..
        }
        | MirGlobalInit::RoutineAddress {
            descriptor_size, ..
        } => Some(*descriptor_size),
        _ => None,
    }
}

fn slot_descriptor_backing_size(init: &MirStorageInit) -> Option<u16> {
    match init {
        MirStorageInit::Descriptor { backing, .. } => {
            Some((backing.bytes.len() as u16).saturating_add(backing.zero_fill))
        }
        _ => None,
    }
}

fn slot_descriptor_size(init: &MirStorageInit) -> Option<u16> {
    match init {
        MirStorageInit::Descriptor {
            descriptor_size, ..
        }
        | MirStorageInit::RoutineAddress {
            descriptor_size, ..
        } => Some(*descriptor_size),
        _ => None,
    }
}

fn fixed_machine_alias(slot: &MirStorageSlot) -> Option<u16> {
    if !matches!(
        slot.base,
        MirStorageBase::Local(_) | MirStorageBase::LocalAlias { .. }
    ) {
        return None;
    }
    let Some(MirStorageInit::Bytes {
        bytes, zero_fill, ..
    }) = &slot.init
    else {
        return None;
    };
    if *zero_fill != 0 {
        return None;
    }
    match slot.width {
        MirWidth::Byte => bytes.first().copied().map(u16::from),
        MirWidth::Word if bytes.len() >= 2 => Some(u16::from_le_bytes([bytes[0], bytes[1]])),
        MirWidth::Word => None,
    }
}

fn is_deferred_byte_array_slot(slot: &MirStorageSlot, size: u16) -> bool {
    if slot.width != MirWidth::Byte || size <= 0x0100 {
        return false;
    }
    match &slot.init {
        Some(MirStorageInit::ZeroFill { .. }) => true,
        Some(MirStorageInit::Bytes {
            bytes, zero_fill, ..
        }) => bytes.is_empty() && *zero_fill > 0,
        _ => false,
    }
}

fn is_deferred_global_byte_array(global: &super::ir::MirGlobal, size: u16) -> bool {
    if global.width != Some(MirWidth::Byte) || size <= 0x0100 {
        return false;
    }
    match &global.init {
        Some(MirGlobalInit::ZeroFill { .. }) => true,
        Some(MirGlobalInit::Bytes {
            bytes, zero_fill, ..
        }) => bytes.is_empty() && *zero_fill > 0,
        _ => false,
    }
}

fn machine_caret_global_value(global: &super::ir::MirGlobal) -> Option<u16> {
    let MirGlobalInit::Bytes { bytes, .. } = global.init.as_ref()? else {
        return None;
    };
    let [low, high, ..] = bytes.as_slice() else {
        return None;
    };
    Some(u16::from_le_bytes([*low, *high]))
}

fn emit_storage(ctx: &mut MirEmitContext<'_>, emitter: &mut NativeTrackedEmitter) {
    for item in ctx.layout.storage_items.clone() {
        match item {
            MirStorageItem::Global { id, address } => {
                bind_data_label(ctx, emitter, global_label(id), address);
                let Some(global) = ctx.mir.globals.iter().find(|global| global.id == id) else {
                    unsupported_message(
                        None,
                        None,
                        "global layout references missing global",
                        &mut ctx.diagnostics,
                    );
                    continue;
                };
                emit_global_storage(ctx, global, emitter);
            }
            MirStorageItem::Static { id, address } => {
                bind_data_label(ctx, emitter, static_label(id), address);
                let Some(static_data) = ctx
                    .mir
                    .statics
                    .iter()
                    .find(|static_data| static_data.id == id)
                else {
                    unsupported_message(
                        None,
                        None,
                        "static layout references missing static",
                        &mut ctx.diagnostics,
                    );
                    continue;
                };
                for byte in &static_data.bytes {
                    emitter.emit_u8(*byte);
                }
            }
            MirStorageItem::RoutineSlot {
                routine,
                slot,
                address,
            } => {
                bind_data_label(ctx, emitter, routine_slot_label(routine, &slot), address);
                emit_storage_init(ctx, slot.init.as_ref(), slot_size(&slot), emitter);
            }
        }
    }
}

fn bind_data_label(
    ctx: &mut MirEmitContext<'_>,
    emitter: &mut NativeTrackedEmitter,
    label: String,
    address: u16,
) {
    let expected = ctx.origin.saturating_add(emitter.position() as u16);
    if expected != address {
        unsupported_message(
            None,
            None,
            &format!(
                "object layout/data cursor mismatch for {label}: expected ${address:04X}, current ${expected:04X}"
            ),
            &mut ctx.diagnostics,
        );
    }
    if let Err(diagnostic) = emitter.bind_label(label, SYNTHETIC_SPAN) {
        ctx.diagnostics.push(MirDiagnostic {
            routine: None,
            block: None,
            message: diagnostic.message,
        });
    }
}

fn emit_global_storage(
    ctx: &mut MirEmitContext<'_>,
    global: &super::ir::MirGlobal,
    emitter: &mut NativeTrackedEmitter,
) {
    emit_storage_init(ctx, global.init.as_ref(), global.storage_size, emitter);
}

fn emit_storage_init(
    ctx: &mut MirEmitContext<'_>,
    init: Option<&impl StorageInitView>,
    storage_size: u16,
    emitter: &mut NativeTrackedEmitter,
) {
    match init {
        Some(init) => init.emit(ctx, emitter),
        None => emitter.emit_zeroes(storage_size),
    }
}

trait StorageInitView {
    fn emit(&self, ctx: &mut MirEmitContext<'_>, emitter: &mut NativeTrackedEmitter);
    fn object_size(&self, storage_size: u16) -> u16;
}

impl StorageInitView for MirGlobalInit {
    fn emit(&self, ctx: &mut MirEmitContext<'_>, emitter: &mut NativeTrackedEmitter) {
        match self {
            MirGlobalInit::Bytes {
                bytes, zero_fill, ..
            } => {
                for byte in bytes {
                    emitter.emit_u8(*byte);
                }
                emitter.emit_zeroes(*zero_fill);
            }
            MirGlobalInit::ZeroFill { bytes, .. } => emitter.emit_zeroes(*bytes),
            MirGlobalInit::ProgramEndWord { .. } => {
                emitter.emit_u16_le(ctx.layout.plan.runtime_high_water(ctx.origin));
            }
            MirGlobalInit::Descriptor {
                backing,
                descriptor_size,
                size_word,
                ..
            } => {
                for byte in &backing.bytes {
                    emitter.emit_u8(*byte);
                }
                emitter.emit_zeroes(backing.zero_fill);
                let backing_size = (backing.bytes.len() as u16).saturating_add(backing.zero_fill);
                let backing_address = ctx
                    .origin
                    .saturating_add(emitter.position() as u16)
                    .saturating_sub(backing_size);
                emitter.emit_u16_le(backing_address);
                if *descriptor_size >= 4 {
                    emitter.emit_u16_le(size_word.unwrap_or(backing_size));
                }
                if *descriptor_size > 4 {
                    emitter.emit_zeroes(descriptor_size.saturating_sub(4));
                }
            }
            MirGlobalInit::RoutineAddress {
                routine,
                descriptor_size,
                size_word,
                ..
            } => {
                emitter.emit_u16_label(ctx.layout.routine_label(*routine), SYNTHETIC_SPAN);
                if *descriptor_size >= 4 {
                    if let Some(size_word) = size_word {
                        emitter.emit_u16_le(*size_word);
                    } else {
                        emitter.emit_u16_label(ctx.layout.routine_label(*routine), SYNTHETIC_SPAN);
                    }
                }
                if *descriptor_size > 4 {
                    emitter.emit_zeroes(descriptor_size.saturating_sub(4));
                }
            }
        }
    }

    fn object_size(&self, storage_size: u16) -> u16 {
        match self {
            MirGlobalInit::Bytes {
                bytes, zero_fill, ..
            } => (bytes.len() as u16)
                .saturating_add(*zero_fill)
                .max(storage_size),
            MirGlobalInit::ZeroFill { bytes, .. } => (*bytes).max(storage_size),
            MirGlobalInit::ProgramEndWord { .. } => 2.max(storage_size),
            MirGlobalInit::Descriptor {
                backing,
                descriptor_size,
                ..
            } => (backing.bytes.len() as u16)
                .saturating_add(backing.zero_fill)
                .saturating_add(*descriptor_size)
                .max(storage_size),
            MirGlobalInit::RoutineAddress {
                descriptor_size, ..
            } => (*descriptor_size).max(storage_size),
        }
    }
}

impl StorageInitView for MirStorageInit {
    fn emit(&self, ctx: &mut MirEmitContext<'_>, emitter: &mut NativeTrackedEmitter) {
        match self {
            MirStorageInit::Bytes {
                bytes, zero_fill, ..
            } => {
                for byte in bytes {
                    emitter.emit_u8(*byte);
                }
                emitter.emit_zeroes(*zero_fill);
            }
            MirStorageInit::ZeroFill { bytes, .. } => emitter.emit_zeroes(*bytes),
            MirStorageInit::Descriptor {
                backing,
                descriptor_size,
                size_word,
                ..
            } => {
                for byte in &backing.bytes {
                    emitter.emit_u8(*byte);
                }
                emitter.emit_zeroes(backing.zero_fill);
                let backing_size = (backing.bytes.len() as u16).saturating_add(backing.zero_fill);
                let backing_address = ctx
                    .origin
                    .saturating_add(emitter.position() as u16)
                    .saturating_sub(backing_size);
                emitter.emit_u16_le(backing_address);
                if *descriptor_size >= 4 {
                    emitter.emit_u16_le(size_word.unwrap_or(backing_size));
                }
                if *descriptor_size > 4 {
                    emitter.emit_zeroes(descriptor_size.saturating_sub(4));
                }
            }
            MirStorageInit::RoutineAddress {
                routine,
                descriptor_size,
                size_word,
                ..
            } => {
                emitter.emit_u16_label(ctx.layout.routine_label(*routine), SYNTHETIC_SPAN);
                if *descriptor_size >= 4 {
                    emitter.emit_u16_le(size_word.unwrap_or(0));
                }
                if *descriptor_size > 4 {
                    emitter.emit_zeroes(descriptor_size.saturating_sub(4));
                }
            }
        }
    }

    fn object_size(&self, storage_size: u16) -> u16 {
        match self {
            MirStorageInit::Bytes {
                bytes, zero_fill, ..
            } => (bytes.len() as u16)
                .saturating_add(*zero_fill)
                .max(storage_size),
            MirStorageInit::ZeroFill { bytes, .. } => (*bytes).max(storage_size),
            MirStorageInit::Descriptor {
                backing,
                descriptor_size,
                ..
            } => (backing.bytes.len() as u16)
                .saturating_add(backing.zero_fill)
                .saturating_add(*descriptor_size)
                .max(storage_size),
            MirStorageInit::RoutineAddress {
                descriptor_size, ..
            } => (*descriptor_size).max(storage_size),
        }
    }
}

fn emit_routine(
    ctx: &mut MirEmitContext<'_>,
    routine: &MirRoutine,
    emitter: &mut NativeTrackedEmitter,
) {
    let range_start = current_address(ctx, emitter);
    let routine_label = ctx.layout.routine_label(routine.id);
    bind_label(ctx, emitter, routine.id, None, routine_label);
    let routine_start = current_address(ctx, emitter);
    ctx.summary.routine_addresses.push(RoutineAddress {
        name: routine.name.clone(),
        address: routine_start,
    });
    ctx.summary
        .routine_signatures
        .push(mir_routine_signature(routine));
    for (index, block) in routine.blocks.iter().enumerate() {
        ctx.measurements
            .block_positions
            .insert((routine.id, block.id), emitter.position());
        bind_label(
            ctx,
            emitter,
            routine.id,
            Some(block.id),
            ctx.layout.block_label(routine.id, block.id),
        );
        let tail_call = block.ops.last().is_some_and(is_tail_call_op)
            && matches!(
                block.terminator,
                MirTerminator::Return | MirTerminator::Exit
            );
        for (op_index, op) in block.ops.iter().enumerate() {
            if tail_call
                && op_index + 1 == block.ops.len()
                && emit_tail_call(ctx, routine.id, block.id, op, emitter)
            {
                continue;
            }
            emit_op(ctx, routine.id, block.id, op, emitter);
        }
        let next_block = routine.blocks.get(index + 1).map(|block| block.id);
        if !tail_call {
            emit_terminator(
                ctx,
                routine.id,
                block.id,
                next_block,
                &block.terminator,
                emitter,
            );
        }
    }
    ctx.summary.routine_ranges.push(RoutineRange {
        name: routine.name.clone(),
        start: range_start,
        end: current_address(ctx, emitter),
    });
    ctx.summary.source_ranges.push(CodegenSourceRange {
        kind: CodegenSourceRangeKind::Routine,
        name: Some(routine.name.clone()),
        source_span: SYNTHETIC_SPAN,
        start: range_start,
        end: current_address(ctx, emitter),
    });
    if let Some(summary) = effect_summary(&routine.effects) {
        ctx.summary.routine_effects.push(CodegenRoutineEffect {
            routine: routine.name.clone(),
            summary,
        });
    }
}

fn mir_routine_signature(routine: &MirRoutine) -> CodegenRoutineSignature {
    CodegenRoutineSignature {
        name: routine.name.clone(),
        kind: "PROC".to_string(),
        params: routine
            .frame
            .params
            .iter()
            .map(|param| CodegenRoutineParam {
                name: param
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("p{}", param.id.0)),
                type_name: mir_width_trace_name(param.width).to_string(),
                width: mir_width_bytes(param.width),
            })
            .collect(),
        return_type: None,
        return_width: None,
    }
}

fn mir_width_trace_name(width: MirWidth) -> &'static str {
    match width {
        MirWidth::Byte => "BYTE",
        MirWidth::Word => "WORD",
    }
}

fn mir_width_bytes(width: MirWidth) -> u16 {
    match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    }
}

fn bind_label(
    ctx: &mut MirEmitContext<'_>,
    emitter: &mut NativeTrackedEmitter,
    routine: RoutineId,
    block: Option<MirBlockId>,
    label: String,
) {
    if let Err(diagnostic) = emitter.bind_label(label, SYNTHETIC_SPAN) {
        ctx.diagnostics.push(MirDiagnostic {
            routine: Some(format!("r{}", routine.0)),
            block: block.map(|block| format!("b{}", block.0)),
            message: diagnostic.message,
        });
    }
}

fn emit_op(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    op: &MirOp,
    emitter: &mut NativeTrackedEmitter,
) {
    match op {
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value,
            width: MirWidth::Byte,
        } => emitter.emit_lda_imm((*value & 0x00FF) as u8),
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value,
            width: MirWidth::Byte,
        } => emitter.emit_ldx_imm((*value & 0x00FF) as u8),
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::Y),
            value,
            width: MirWidth::Byte,
        } => emitter.emit_ldy_imm((*value & 0x00FF) as u8),
        MirOp::LeaAddr {
            dst,
            target,
            width: MirWidth::Word,
        } => {
            let Some(address) = ctx
                .layout
                .direct_mem(routine, target)
                .map(resolved_mem_address)
            else {
                unsupported(ctx, routine, block, "lea target is not emit-ready");
                return;
            };
            emit_address_to_def(ctx, routine, block, dst, address, emitter);
        }
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(src),
            width: MirWidth::Byte,
        } => match ctx.layout.direct_mem(routine, src) {
            Some(mem) => emit_lda_mem(mem, emitter),
            None => unsupported(ctx, routine, block, "load source is not emit-ready"),
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::X),
            src: MirAddr::Direct(src),
            width: MirWidth::Byte,
        } => match ctx.layout.direct_mem(routine, src) {
            Some(mem) => emit_ldx_mem(mem, emitter),
            None => unsupported(ctx, routine, block, "load source is not emit-ready"),
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::Y),
            src: MirAddr::Direct(src),
            width: MirWidth::Byte,
        } => match ctx.layout.direct_mem(routine, src) {
            Some(mem) => emit_ldy_mem(mem, emitter),
            None => unsupported(ctx, routine, block, "load source is not emit-ready"),
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src,
            width: MirWidth::Byte,
        } => match ctx.layout.indexed_addr(routine, src) {
            Some(mem) => emit_lda_indexed(mem, emitter),
            None => unsupported(ctx, routine, block, "indexed load source is not emit-ready"),
        },
        MirOp::Store {
            dst: MirAddr::Direct(dst),
            src,
            width: MirWidth::Byte,
        } => {
            let Some(mem) = ctx.layout.direct_mem(routine, dst) else {
                unsupported(ctx, routine, block, "store destination is not emit-ready");
                return;
            };
            if matches!(src, MirValue::Def(MirDef::Reg(MirReg::X))) {
                emit_stx_mem(mem, emitter);
                return;
            }
            if matches!(src, MirValue::Def(MirDef::Reg(MirReg::Y))) {
                emit_sty_mem(mem, emitter);
                return;
            }
            if !emit_value_to_a(ctx, routine, block, src, emitter) {
                unsupported(ctx, routine, block, "store source is not emit-ready");
                return;
            }
            emit_sta_mem(mem, emitter);
        }
        MirOp::Store {
            dst,
            src,
            width: MirWidth::Byte,
        } => {
            let Some(mem) = ctx.layout.indexed_addr(routine, dst) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indexed store destination is not emit-ready",
                );
                return;
            };
            if !emit_value_to_a(ctx, routine, block, src, emitter) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indexed store source is not emit-ready",
                );
                return;
            }
            emit_sta_indexed(mem, emitter);
        }
        MirOp::UpdateMem {
            op,
            mem,
            width: MirWidth::Byte,
        } => match ctx.layout.direct_mem(routine, mem) {
            Some(mem) => emit_update_mem(*op, mem, emitter),
            None => unsupported(
                ctx,
                routine,
                block,
                "memory update target is not emit-ready",
            ),
        },
        MirOp::UpdateMem {
            op: MirUpdateOp::Inc,
            mem,
            width: MirWidth::Word,
        } => match ctx.layout.direct_mem(routine, mem) {
            Some(mem) => emit_word_inc_mem(ctx, routine, block, mem, emitter),
            None => unsupported(
                ctx,
                routine,
                block,
                "word memory update target is not emit-ready",
            ),
        },
        MirOp::UpdateMem {
            op: MirUpdateOp::Dec,
            mem,
            width: MirWidth::Word,
        } => match ctx.layout.direct_mem(routine, mem) {
            Some(mem) => emit_word_dec_mem(ctx, routine, block, mem, emitter),
            None => unsupported(
                ctx,
                routine,
                block,
                "word memory update target is not emit-ready",
            ),
        },
        MirOp::AddByteToWordMem { mem, value } => match ctx.layout.direct_mem(routine, mem) {
            Some(mem) => {
                emit_byte_to_word_mem(ctx, routine, block, MirBinaryOp::Add, mem, value, emitter)
            }
            None => unsupported(
                ctx,
                routine,
                block,
                "add-byte word target is not emit-ready",
            ),
        },
        MirOp::SubByteFromWordMem { mem, value } => match ctx.layout.direct_mem(routine, mem) {
            Some(mem) => {
                emit_byte_to_word_mem(ctx, routine, block, MirBinaryOp::Sub, mem, value, emitter)
            }
            None => unsupported(
                ctx,
                routine,
                block,
                "sub-byte word target is not emit-ready",
            ),
        },
        MirOp::MaterializeAddress { consumer, value } => {
            if consumer.uses_scaled_y() {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "plain address materialization cannot use scaled Y",
                );
                return;
            }
            let Some(pointer_slot) = resolve_pointer_consumer_slot(ctx, routine, consumer) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "materialize address consumer is not placed",
                );
                return;
            };
            let Some((lo, hi)) = split_value_as_word(ctx, value) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "materialize address value is not emit-ready",
                );
                return;
            };
            if !emit_value_to_a(ctx, routine, block, &lo, emitter) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "materialize address low byte is not emit-ready",
                );
                return;
            }
            emit_sta_mem(ResolvedMem::ZeroPage(pointer_slot), emitter);
            if !emit_value_to_a(ctx, routine, block, &hi, emitter) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "materialize address high byte is not emit-ready",
                );
                return;
            }
            emit_sta_mem(
                ResolvedMem::ZeroPage(pointer_slot.saturating_add(1)),
                emitter,
            );
        }
        MirOp::MaterializeIndexedAddress {
            consumer,
            base,
            index,
            scale,
        } => {
            let Some(pointer_slot) = resolve_pointer_consumer_slot(ctx, routine, consumer) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indexed address consumer is not placed",
                );
                return;
            };
            let Some((base_lo, base_hi)) = split_value_as_word(ctx, base) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indexed address base is not emit-ready",
                );
                return;
            };
            if !matches!(*scale, 1 | 2) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "only byte and word indexed address materialization is supported",
                );
                return;
            }
            if consumer.uses_scaled_y() {
                if *scale != 2
                    || !emit_scaled_y_index_plus_base_to_pointer(
                        ctx,
                        routine,
                        block,
                        index,
                        &base_lo,
                        &base_hi,
                        pointer_slot,
                        emitter,
                    )
                {
                    unsupported(
                        ctx,
                        routine,
                        block,
                        "scaled-Y word address is not emit-ready",
                    );
                }
                return;
            }
            if emit_scaled_index_plus_base_to_pointer(
                ctx,
                routine,
                block,
                index,
                *scale,
                &base_lo,
                &base_hi,
                pointer_slot,
                emitter,
            ) {
                return;
            }
            if !emit_scaled_index_to_scratch(ctx, routine, block, index, *scale, emitter) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indexed address index is not emit-ready",
                );
                return;
            }
            emitter.emit_clc();
            emit_lda_mem(ResolvedMem::ZeroPage(ADDRESS_INDEX_SCRATCH_LO), emitter);
            if !emit_adc_value_to_a(ctx, routine, block, &base_lo, emitter) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indexed address low base is not emit-ready",
                );
                return;
            }
            emit_sta_mem(ResolvedMem::ZeroPage(pointer_slot), emitter);
            emit_lda_mem(ResolvedMem::ZeroPage(ADDRESS_INDEX_SCRATCH_HI), emitter);
            if !emit_adc_value_to_a(ctx, routine, block, &base_hi, emitter) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indexed address high base is not emit-ready",
                );
                return;
            }
            emit_sta_mem(
                ResolvedMem::ZeroPage(pointer_slot.saturating_add(1)),
                emitter,
            );
        }
        MirOp::AdvanceAddress {
            consumer,
            index,
            scale,
        } => {
            if consumer.uses_scaled_y() {
                unsupported(ctx, routine, block, "address advance cannot use scaled Y");
                return;
            }
            let Some(pointer_slot) = resolve_pointer_consumer_slot(ctx, routine, consumer) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "address advance consumer is not placed",
                );
                return;
            };
            if !matches!(*scale, 1 | 2) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "only byte and word index address advance is supported",
                );
                return;
            }
            if emit_scaled_index_advance_pointer(
                ctx,
                routine,
                block,
                index,
                *scale,
                pointer_slot,
                emitter,
            ) {
                return;
            }
            if !emit_scaled_index_to_scratch(ctx, routine, block, index, *scale, emitter) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "address advance index is not emit-ready",
                );
                return;
            }

            emitter.emit_clc();
            emit_lda_mem(ResolvedMem::ZeroPage(pointer_slot), emitter);
            emitter.emit_adc_zero_page(ZeroPage::new(ADDRESS_INDEX_SCRATCH_LO));
            emit_sta_mem(ResolvedMem::ZeroPage(pointer_slot), emitter);
            emit_lda_mem(
                ResolvedMem::ZeroPage(pointer_slot.saturating_add(1)),
                emitter,
            );
            emitter.emit_adc_zero_page(ZeroPage::new(ADDRESS_INDEX_SCRATCH_HI));
            emit_sta_mem(
                ResolvedMem::ZeroPage(pointer_slot.saturating_add(1)),
                emitter,
            );
        }
        MirOp::LoadIndirect {
            consumer,
            dst: MirDef::Reg(MirReg::A),
            offset,
        } => {
            let Some(pointer_slot) = resolve_pointer_consumer_slot(ctx, routine, consumer) else {
                unsupported(ctx, routine, block, "indirect load consumer is not placed");
                return;
            };
            emit_lda_indirect(*consumer, pointer_slot, *offset, emitter);
        }
        MirOp::LoadIndirect {
            dst: _,
            consumer: _,
            offset: _,
        } => {
            unsupported(
                ctx,
                routine,
                block,
                "load destination must be A for indirect load",
            );
        }
        MirOp::StoreIndirect {
            consumer,
            src,
            offset,
        } => {
            let Some(pointer_slot) = resolve_pointer_consumer_slot(ctx, routine, consumer) else {
                unsupported(ctx, routine, block, "indirect store consumer is not placed");
                return;
            };
            if !emit_value_to_a(ctx, routine, block, src, emitter) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indirect store source is not emit-ready",
                );
                return;
            }
            emit_sta_indirect(*consumer, pointer_slot, *offset, emitter);
        }
        MirOp::IndirectByteCompound {
            op,
            target,
            source,
            offset,
        } => {
            if target.uses_scaled_y() || source.uses_scaled_y() {
                unsupported(ctx, routine, block, "indirect compound cannot use scaled Y");
                return;
            }
            let Some(target_slot) = resolve_pointer_consumer_slot(ctx, routine, target) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indirect compound target is not placed",
                );
                return;
            };
            let Some(source_slot) = resolve_pointer_consumer_slot(ctx, routine, source) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indirect compound source is not placed",
                );
                return;
            };
            emit_indirect_byte_compound(*op, target_slot, source_slot, *offset, emitter);
        }
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src,
            width: MirWidth::Byte,
        } => {
            if !emit_value_to_a(ctx, routine, block, src, emitter) {
                unsupported(ctx, routine, block, "move source is not emit-ready");
            }
        }
        MirOp::Move {
            dst: MirDef::Reg(MirReg::X),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } => emitter.emit_tax(),
        MirOp::Move {
            dst: MirDef::Reg(MirReg::Y),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } => emitter.emit_tay(),
        MirOp::Move {
            dst: MirDef::Reg(reg @ (MirReg::X | MirReg::Y)),
            src,
            width: MirWidth::Byte,
        } if const_u8(src).is_some() => {
            let value = const_u8(src).expect("checked byte constant");
            match reg {
                MirReg::X => emitter.emit_ldx_imm(value),
                MirReg::Y => emitter.emit_ldy_imm(value),
                MirReg::A => unreachable!("A is handled above"),
            }
        }
        MirOp::Move {
            dst: MirDef::Reg(reg @ (MirReg::X | MirReg::Y)),
            src: MirValue::StorageAddrByte { mem, byte },
            width: MirWidth::Byte,
        } if *byte <= 1 => {
            let Some(address) = ctx
                .layout
                .direct_mem(routine, mem)
                .map(resolved_mem_address)
            else {
                unsupported(ctx, routine, block, "storage address value is not placed");
                return;
            };
            let value = if *byte == 0 {
                (address & 0x00FF) as u8
            } else {
                (address >> 8) as u8
            };
            match reg {
                MirReg::X => emitter.emit_ldx_imm(value),
                MirReg::Y => emitter.emit_ldy_imm(value),
                MirReg::A => unreachable!("A is handled above"),
            }
        }
        MirOp::Move {
            dst: MirDef::Reg(reg @ (MirReg::X | MirReg::Y)),
            src: MirValue::RoutineAddrByte { id, byte },
            width: MirWidth::Byte,
        } if *byte <= 1 => {
            let label = ctx.layout.routine_label(*id);
            match reg {
                MirReg::X => emitter.emit_u8(opcode::LDX_IMM),
                MirReg::Y => emitter.emit_u8(opcode::LDY_IMM),
                MirReg::A => unreachable!("A is handled above"),
            }
            if *byte == 0 {
                emitter.emit_u8_label_low(label, SYNTHETIC_SPAN);
            } else {
                emitter.emit_u8_label_high(label, SYNTHETIC_SPAN);
            }
        }
        MirOp::Unary {
            op: MirUnaryOp::Neg,
            dst: MirDef::Reg(MirReg::A),
            src,
            width: MirWidth::Byte,
        } => {
            if !emit_value_to_a(ctx, routine, block, src, emitter) {
                unsupported(ctx, routine, block, "unary neg source is not emit-ready");
                return;
            }
            emitter.emit_eor_imm(0xFF);
            emitter.emit_clc();
            emitter.emit_adc_imm(0x01);
        }
        MirOp::Unary {
            op: MirUnaryOp::BitNot,
            dst: MirDef::Reg(MirReg::A),
            src,
            width: MirWidth::Byte,
        } => {
            if !emit_value_to_a(ctx, routine, block, src, emitter) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "unary bit-not source is not emit-ready",
                );
                return;
            }
            emitter.emit_eor_imm(0xFF);
        }
        MirOp::Binary {
            op,
            dst: MirDef::Reg(MirReg::A),
            left,
            right,
            width: MirWidth::Byte,
            carry_in,
            ..
        } => {
            if !emit_value_to_a(ctx, routine, block, left, emitter) {
                unsupported(ctx, routine, block, "binary left source is not emit-ready");
                return;
            }
            emit_carry(*carry_in, *op, emitter);
            if let Some(right) = const_u8(right) {
                emit_binary_imm(ctx, routine, block, *op, right, emitter);
            } else if let MirValue::PointerCell(mem) = right {
                match ctx.layout.direct_mem(routine, mem) {
                    Some(mem) => emit_binary_mem(ctx, routine, block, *op, mem, emitter),
                    None => unsupported(ctx, routine, block, "binary right source is not placed"),
                }
            } else {
                unsupported(ctx, routine, block, "binary right source is not emit-ready");
            }
        }
        MirOp::Compare {
            dst,
            op,
            left,
            right,
            width: MirWidth::Byte,
            signed,
        } => {
            emit_compare(ctx, routine, block, dst, *op, left, right, *signed, emitter);
        }
        MirOp::Call { target, .. } => emit_call(ctx, routine, block, target, emitter),
        MirOp::RuntimeHelper { helper, .. } => {
            let Some(decl) = ctx
                .mir
                .runtime_helpers
                .iter()
                .find(|decl| decl.helper == *helper)
            else {
                unsupported(ctx, routine, block, "runtime helper target is not declared");
                return;
            };
            match &decl.target {
                MirRuntimeHelperTarget::KnownAbsolute(address) => emitter.emit_jsr_abs(*address),
                MirRuntimeHelperTarget::RuntimeSymbol(symbol) => {
                    let normalized = normalize_machine_name(symbol);
                    let Some(target) = ctx.layout.routine_names.get(&normalized).copied() else {
                        unsupported(
                            ctx,
                            routine,
                            block,
                            &format!("runtime helper symbol `{symbol}` is unresolved"),
                        );
                        return;
                    };
                    emitter.emit_jsr_label(ctx.layout.routine_label(target), SYNTHETIC_SPAN);
                }
                MirRuntimeHelperTarget::Deferred => {
                    unsupported(ctx, routine, block, "runtime helper target is deferred")
                }
            }
        }
        MirOp::Barrier { .. } => {}
        MirOp::MachineBlock { id, .. } => {
            let Some(machine_block) = ctx.mir.machine_blocks.iter().find(|item| item.id == *id)
            else {
                unsupported(ctx, routine, block, "machine block payload is missing");
                return;
            };
            let address = current_address(ctx, emitter);
            for item in &machine_block.items {
                emit_machine_item(ctx, routine, block, item, emitter);
            }
            let end = current_address(ctx, emitter);
            ctx.summary
                .machine_blocks
                .push(CodegenMachineBlockAnalysis {
                    routine: ctx
                        .mir
                        .routines
                        .iter()
                        .find(|candidate| candidate.id == routine)
                        .map(|routine| routine.name.clone()),
                    source_span: SYNTHETIC_SPAN,
                    address,
                    trusted: true,
                    summary: format!("{} structured item(s)", machine_block.items.len()),
                });
            ctx.summary.source_ranges.push(CodegenSourceRange {
                kind: CodegenSourceRangeKind::MachineBlock,
                name: Some(format!("m{}", id.0)),
                source_span: SYNTHETIC_SPAN,
                start: address,
                end,
            });
        }
        _ => unsupported(
            ctx,
            routine,
            block,
            "op is not supported by MIR6502 emission",
        ),
    }
}

fn emit_machine_item(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    item: &MirMachineItem,
    emitter: &mut NativeTrackedEmitter,
) {
    match item {
        MirMachineItem::Byte(value) => emitter.emit_u8(*value),
        MirMachineItem::Word(value) => emitter.emit_u16_le(*value),
        MirMachineItem::StringLiteral(value) => {
            emit_machine_string_literal(ctx, routine, block, value, emitter);
        }
        MirMachineItem::CharLiteral(value) => {
            emit_machine_char_literal(ctx, routine, block, *value, emitter);
        }
        MirMachineItem::Name(name) => {
            emit_machine_name(ctx, routine, block, name, None, 0, name, emitter);
        }
        MirMachineItem::AddressExpr {
            selector,
            atom,
            offset,
            text,
            ..
        } => {
            emit_machine_address_expr(ctx, routine, block, *selector, atom, *offset, text, emitter)
        }
        MirMachineItem::AddressByte { high, name } => {
            match ctx.layout.machine_symbol(routine, name) {
                Some(MirMachineSymbol::Absolute(address)) => {
                    let byte = if *high {
                        (address >> 8) as u8
                    } else {
                        (address & 0x00FF) as u8
                    };
                    emitter.emit_u8(byte);
                }
                Some(MirMachineSymbol::Label(label)) => {
                    if *high {
                        emitter.emit_u8_label_high(label, SYNTHETIC_SPAN);
                    } else {
                        emitter.emit_u8_label_low(label, SYNTHETIC_SPAN);
                    }
                }
                None => unsupported(
                    ctx,
                    routine,
                    block,
                    &format!("machine block reference `{name}` is not emit-ready"),
                ),
            }
        }
    }
}

fn emit_machine_string_literal(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    text: &str,
    emitter: &mut NativeTrackedEmitter,
) {
    let literal_address = current_address(ctx, emitter);
    let char_count = text.chars().count();
    let Ok(length) = u8::try_from(char_count) else {
        unsupported(
            ctx,
            routine,
            block,
            &format!(
                "machine block string literal length {char_count} exceeds ACTION! string byte limit"
            ),
        );
        return;
    };
    let mut bytes = Vec::with_capacity(usize::from(length) + 1);
    bytes.push(length);
    for ch in text.chars() {
        let Some(byte) = source_char_byte(ch) else {
            unsupported(
                ctx,
                routine,
                block,
                &format!("machine block string literal contains non-encodable character {ch:?}"),
            );
            return;
        };
        bytes.push(byte);
    }
    for byte in bytes {
        emitter.emit_u8(byte);
    }
    emitter.emit_u16_le(literal_address);
}

fn emit_machine_char_literal(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    value: char,
    emitter: &mut NativeTrackedEmitter,
) {
    let Some(byte) = source_char_byte(value) else {
        unsupported(
            ctx,
            routine,
            block,
            &format!("machine block char literal {value:?} is not encodable"),
        );
        return;
    };
    emitter.emit_u8(byte);
    emitter.emit_u8(0x9A);
}

fn emit_machine_address_expr(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    selector: Option<MirMachineByteSelector>,
    atom: &MirMachineAtom,
    offset: i32,
    text: &str,
    emitter: &mut NativeTrackedEmitter,
) {
    match atom {
        MirMachineAtom::Number(value) => {
            let value = apply_machine_offset(*value, offset);
            emit_machine_resolved_value(emitter, value, selector);
        }
        MirMachineAtom::Name(name) => {
            emit_machine_name(ctx, routine, block, name, selector, offset, text, emitter)
        }
        MirMachineAtom::Current => {
            let value = apply_machine_offset(current_address(ctx, emitter), offset);
            emit_machine_resolved_value(emitter, value, selector);
        }
    }
}

fn emit_machine_name(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    name: &str,
    selector: Option<MirMachineByteSelector>,
    offset: i32,
    text: &str,
    emitter: &mut NativeTrackedEmitter,
) {
    if machine_text_uses_caret(text) {
        if let Some(address) = ctx.layout.machine_caret_symbol(name) {
            let address = apply_machine_offset(address, offset);
            emit_machine_resolved_value(emitter, address, selector);
        } else {
            unsupported(
                ctx,
                routine,
                block,
                &format!(
                    "machine block item `{text}` cannot be resolved to a compile-time pointer value"
                ),
            );
        }
        return;
    }
    match ctx.layout.machine_symbol(routine, name) {
        Some(MirMachineSymbol::Absolute(address)) => {
            let address = apply_machine_offset(address, offset);
            emit_machine_resolved_value(emitter, address, selector);
        }
        Some(MirMachineSymbol::Label(label)) => {
            emit_machine_label_value(emitter, label, selector, offset);
        }
        None if text == name => unsupported(
            ctx,
            routine,
            block,
            &format!("machine block reference `{name}` is not emit-ready"),
        ),
        None => unsupported(
            ctx,
            routine,
            block,
            &format!("machine block item `{text}` cannot be resolved to a compile-time address"),
        ),
    }
}

fn machine_text_uses_caret(text: &str) -> bool {
    text.contains('^')
}

fn apply_machine_offset(base: u16, offset: i32) -> u16 {
    base.wrapping_add(offset as u16)
}

fn emit_machine_resolved_value(
    emitter: &mut NativeTrackedEmitter,
    value: u16,
    selector: Option<MirMachineByteSelector>,
) {
    match selector {
        Some(MirMachineByteSelector::Low) => emitter.emit_u8((value & 0x00FF) as u8),
        Some(MirMachineByteSelector::High) => emitter.emit_u8((value >> 8) as u8),
        None if value <= 0x00FF => emitter.emit_u8(value as u8),
        None => emitter.emit_u16_le(value),
    }
}

fn emit_machine_label_value(
    emitter: &mut NativeTrackedEmitter,
    label: String,
    selector: Option<MirMachineByteSelector>,
    offset: i32,
) {
    match selector {
        Some(MirMachineByteSelector::Low) => {
            emitter.emit_u8_label_low_offset(label, offset, SYNTHETIC_SPAN)
        }
        Some(MirMachineByteSelector::High) => {
            emitter.emit_u8_label_high_offset(label, offset, SYNTHETIC_SPAN)
        }
        None => emitter.emit_u16_label_offset(label, offset, SYNTHETIC_SPAN),
    }
}

fn effect_summary(effects: &MirEffects) -> Option<String> {
    let mut parts = Vec::new();
    if effects.clobbers.a {
        parts.push("clobbers A".to_string());
    }
    if effects.clobbers.x {
        parts.push("clobbers X".to_string());
    }
    if effects.clobbers.y {
        parts.push("clobbers Y".to_string());
    }
    if effects.clobbers.flags {
        parts.push("clobbers flags".to_string());
    }
    if effects.clobbers.sp {
        parts.push("clobbers SP".to_string());
    }
    if effects.may_call_os {
        parts.push("may call OS".to_string());
    }
    if effects.opaque {
        parts.push("opaque".to_string());
    }
    if effects.stack_depth_delta.is_some() {
        parts.push("changes stack depth".to_string());
    }
    match (&effects.memory_reads, &effects.memory_writes) {
        (super::ir::MirMemoryEffect::None, super::ir::MirMemoryEffect::None) => {}
        (reads, writes) => parts.push(format!(
            "memory reads {}; writes {}",
            memory_effect_summary(reads),
            memory_effect_summary(writes)
        )),
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn memory_effect_summary(effect: &super::ir::MirMemoryEffect) -> &'static str {
    match effect {
        super::ir::MirMemoryEffect::None => "none",
        super::ir::MirMemoryEffect::Regions(_) => "regions",
        super::ir::MirMemoryEffect::Unknown => "unknown",
        super::ir::MirMemoryEffect::All => "all",
    }
}

fn emit_call(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    target: &MirCallTarget,
    emitter: &mut NativeTrackedEmitter,
) {
    match target {
        MirCallTarget::Routine(target) => {
            emitter.emit_jsr_label(ctx.layout.routine_label(*target), SYNTHETIC_SPAN);
        }
        MirCallTarget::Builtin {
            address: Some(address),
            ..
        }
        | MirCallTarget::Runtime {
            address: Some(address),
            ..
        } => emitter.emit_jsr_abs(*address),
        MirCallTarget::Builtin {
            name,
            address: None,
        } => emit_resolved_or_diagnose_builtin_call(ctx, routine, block, name, emitter),
        MirCallTarget::Runtime {
            name,
            address: None,
        } => unsupported(
            ctx,
            routine,
            block,
            &format!("runtime call target `{name}` is unresolved"),
        ),
        MirCallTarget::Indirect { target, width } => {
            if !matches!(width, MirWidth::Word) {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indirect call target must be word-width",
                );
                return;
            }
            let Some(address) = indirect_call_target_address(ctx, routine, target) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "indirect call target home is not emit-ready",
                );
                return;
            };
            emit_indirect_call(ctx, address, emitter);
        }
    }
}

fn is_tail_call_op(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::Call {
            target: MirCallTarget::Routine(_)
                | MirCallTarget::Builtin { .. }
                | MirCallTarget::Runtime {
                    address: Some(_),
                    ..
                },
            ..
        } | MirOp::RuntimeHelper { .. }
    )
}

fn emit_tail_call(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    op: &MirOp,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    match op {
        MirOp::Call { target, .. } => emit_tail_call_target(ctx, routine, block, target, emitter),
        MirOp::RuntimeHelper { helper, .. } => {
            let Some(decl) = ctx
                .mir
                .runtime_helpers
                .iter()
                .find(|decl| decl.helper == *helper)
            else {
                unsupported(ctx, routine, block, "runtime helper target is not declared");
                return true;
            };
            match &decl.target {
                MirRuntimeHelperTarget::KnownAbsolute(address) => emitter.emit_jmp_abs(*address),
                MirRuntimeHelperTarget::RuntimeSymbol(symbol) => {
                    let normalized = normalize_machine_name(symbol);
                    let Some(target) = ctx.layout.routine_names.get(&normalized).copied() else {
                        unsupported(
                            ctx,
                            routine,
                            block,
                            &format!("runtime helper symbol `{symbol}` is unresolved"),
                        );
                        return true;
                    };
                    emitter.emit_jmp_label(ctx.layout.routine_label(target), SYNTHETIC_SPAN);
                }
                MirRuntimeHelperTarget::Deferred => {
                    unsupported(ctx, routine, block, "runtime helper target is deferred")
                }
            }
            true
        }
        _ => false,
    }
}

fn emit_tail_call_target(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    target: &MirCallTarget,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    match target {
        MirCallTarget::Routine(target) => {
            emitter.emit_jmp_label(ctx.layout.routine_label(*target), SYNTHETIC_SPAN);
            true
        }
        MirCallTarget::Builtin {
            address: Some(address),
            ..
        }
        | MirCallTarget::Runtime {
            address: Some(address),
            ..
        } => {
            emitter.emit_jmp_abs(*address);
            true
        }
        MirCallTarget::Builtin {
            name,
            address: None,
        } => match resolve_builtin_target(name) {
            MirBuiltinResolution::Resolved { address } => {
                emitter.emit_jmp_abs(address);
                true
            }
            MirBuiltinResolution::Deferred { reason } => {
                unsupported(
                    ctx,
                    routine,
                    block,
                    &format!("builtin call target `{name}` is deferred: {reason}"),
                );
                true
            }
            MirBuiltinResolution::Unsupported { reason } => {
                unsupported(
                    ctx,
                    routine,
                    block,
                    &format!("builtin call target `{name}` is unsupported by MIR6502: {reason}"),
                );
                true
            }
            MirBuiltinResolution::Unknown => {
                unsupported(
                    ctx,
                    routine,
                    block,
                    &format!("builtin call target `{name}` is not modeled by MIR6502"),
                );
                true
            }
        },
        MirCallTarget::Runtime {
            name,
            address: None,
        } => {
            unsupported(
                ctx,
                routine,
                block,
                &format!("runtime call target `{name}` is unresolved"),
            );
            true
        }
        MirCallTarget::Indirect { .. } => false,
    }
}

fn emit_resolved_or_diagnose_builtin_call(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    name: &str,
    emitter: &mut NativeTrackedEmitter,
) {
    let message = match resolve_builtin_target(name) {
        MirBuiltinResolution::Resolved { address } => {
            emitter.emit_jsr_abs(address);
            return;
        }
        MirBuiltinResolution::Deferred { reason } => {
            format!("builtin call target `{name}` is deferred: {reason}")
        }
        MirBuiltinResolution::Unsupported { reason } => {
            format!("builtin call target `{name}` is unsupported by MIR6502: {reason}")
        }
        MirBuiltinResolution::Unknown => {
            format!("builtin call target `{name}` is not modeled by MIR6502")
        }
    };
    unsupported(ctx, routine, block, &message);
}

fn emit_indirect_call(
    ctx: &mut MirEmitContext<'_>,
    address: u16,
    emitter: &mut NativeTrackedEmitter,
) {
    let return_minus_one = format!(
        "__mir6502_indirect_return_minus_one_{}",
        ctx.indirect_call_counter
    );
    ctx.indirect_call_counter = ctx.indirect_call_counter.saturating_add(1);
    emitter.emit_u8(opcode::LDA_IMM);
    emitter.emit_u8_label_high(return_minus_one.clone(), SYNTHETIC_SPAN);
    emitter.emit_pha();
    emitter.emit_u8(opcode::LDA_IMM);
    emitter.emit_u8_label_low(return_minus_one.clone(), SYNTHETIC_SPAN);
    emitter.emit_pha();
    emitter.emit_jmp_indirect(address);
    if let Err(diagnostic) = emitter.bind_label(return_minus_one, SYNTHETIC_SPAN) {
        ctx.diagnostics.push(MirDiagnostic {
            routine: None,
            block: None,
            message: diagnostic.message,
        });
    }
    emitter.emit_u8(0xEA);
}

fn indirect_call_target_address(
    ctx: &MirEmitContext<'_>,
    routine: RoutineId,
    target: &MirValue,
) -> Option<u16> {
    match target {
        MirValue::Word { lo, .. } => indirect_call_target_byte_address(ctx, routine, lo),
        _ => None,
    }
}

fn indirect_call_target_byte_address(
    ctx: &MirEmitContext<'_>,
    routine: RoutineId,
    value: &MirValue,
) -> Option<u16> {
    let MirValue::PointerCell(mem) = value else {
        return None;
    };
    match ctx.layout.direct_mem(routine, mem)? {
        ResolvedMem::Absolute(address) => Some(address),
        ResolvedMem::ZeroPage(address) => Some(address as u16),
    }
}

fn emit_terminator(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    next_block: Option<MirBlockId>,
    terminator: &MirTerminator,
    emitter: &mut NativeTrackedEmitter,
) {
    match terminator {
        MirTerminator::Return | MirTerminator::Exit => emitter.emit_rts(),
        MirTerminator::Jump(target) => {
            if Some(target.target) != next_block {
                emitter.emit_jmp_label(
                    ctx.layout.block_label(routine, target.target),
                    SYNTHETIC_SPAN,
                );
            }
        }
        MirTerminator::Branch {
            cond,
            then_edge,
            else_edge,
        } => {
            let then_block = then_edge.target;
            let else_block = else_edge.target;
            ctx.measurements
                .branch_positions
                .insert((routine, block), emitter.position());
            if emit_direct_branch_if_in_range(
                ctx, routine, block, cond, then_block, else_block, next_block, emitter,
            ) {
                return;
            }
            let Some(opcode) = inverted_branch_opcode(cond) else {
                unsupported(ctx, routine, block, "branch condition is not emit-ready");
                return;
            };
            let else_jump_label = format!("__mir6502_branch_else_{}_{}", routine.0, block.0);
            emitter.emit_branch_label(opcode, else_jump_label.clone(), SYNTHETIC_SPAN);
            emitter.emit_jmp_label(ctx.layout.block_label(routine, then_block), SYNTHETIC_SPAN);
            bind_label(ctx, emitter, routine, Some(block), else_jump_label);
            emitter.emit_jmp_label(ctx.layout.block_label(routine, else_block), SYNTHETIC_SPAN);
        }
        MirTerminator::Unreachable => {}
    }
}

fn emit_direct_branch_if_in_range(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    cond: &MirCond,
    then_block: MirBlockId,
    else_block: MirBlockId,
    next_block: Option<MirBlockId>,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    if let MirCond::AnyFlagTest(tests) = cond {
        let Some(first_opcode) = branch_flag_opcode(tests[0].clone()) else {
            return false;
        };
        let Some(second_opcode) = branch_flag_opcode(tests[1].clone()) else {
            return false;
        };
        let then_is_in_range = branch_target_is_in_range(
            ctx,
            emitter,
            routine,
            block,
            then_block,
            next_block,
            emitter.position(),
        ) && branch_target_is_in_range(
            ctx,
            emitter,
            routine,
            block,
            then_block,
            next_block,
            emitter.position().saturating_add(2),
        );
        let else_is_in_range = branch_target_is_in_range(
            ctx,
            emitter,
            routine,
            block,
            else_block,
            next_block,
            emitter.position().saturating_add(2),
        );
        if Some(else_block) == next_block && then_is_in_range {
            let then_label = ctx.layout.block_label(routine, then_block);
            emitter.emit_branch_label(first_opcode, then_label.clone(), SYNTHETIC_SPAN);
            emitter.emit_branch_label(second_opcode, then_label, SYNTHETIC_SPAN);
        } else if Some(then_block) == next_block && else_is_in_range {
            let Some(inverted_second_opcode) = invert_branch_opcode(second_opcode) else {
                return false;
            };
            emitter.emit_branch_label(
                first_opcode,
                ctx.layout.block_label(routine, then_block),
                SYNTHETIC_SPAN,
            );
            emitter.emit_branch_label(
                inverted_second_opcode,
                ctx.layout.block_label(routine, else_block),
                SYNTHETIC_SPAN,
            );
        } else if then_is_in_range {
            let then_label = ctx.layout.block_label(routine, then_block);
            emitter.emit_branch_label(first_opcode, then_label.clone(), SYNTHETIC_SPAN);
            emitter.emit_branch_label(second_opcode, then_label, SYNTHETIC_SPAN);
            if Some(else_block) != next_block {
                emitter.emit_jmp_label(ctx.layout.block_label(routine, else_block), SYNTHETIC_SPAN);
            }
        } else if else_is_in_range {
            let Some(inverted_second_opcode) = invert_branch_opcode(second_opcode) else {
                return false;
            };
            let then_jump_label = format!(
                "__mir6502_branch_then_{}_{}_{}",
                routine.0,
                then_block.0,
                emitter.position()
            );
            emitter.emit_branch_label(first_opcode, then_jump_label.clone(), SYNTHETIC_SPAN);
            emitter.emit_branch_label(
                inverted_second_opcode,
                ctx.layout.block_label(routine, else_block),
                SYNTHETIC_SPAN,
            );
            bind_label(ctx, emitter, routine, None, then_jump_label);
            if Some(then_block) != next_block {
                emitter.emit_jmp_label(ctx.layout.block_label(routine, then_block), SYNTHETIC_SPAN);
            }
        } else {
            let then_jump_label = format!(
                "__mir6502_branch_then_{}_{}_{}",
                routine.0,
                then_block.0,
                emitter.position()
            );
            emitter.emit_branch_label(first_opcode, then_jump_label.clone(), SYNTHETIC_SPAN);
            emitter.emit_branch_label(second_opcode, then_jump_label.clone(), SYNTHETIC_SPAN);
            emitter.emit_jmp_label(ctx.layout.block_label(routine, else_block), SYNTHETIC_SPAN);
            bind_label(ctx, emitter, routine, None, then_jump_label);
            emitter.emit_jmp_label(ctx.layout.block_label(routine, then_block), SYNTHETIC_SPAN);
        }
        return true;
    }
    let then_is_in_range = branch_target_is_in_range(
        ctx,
        emitter,
        routine,
        block,
        then_block,
        next_block,
        emitter.position(),
    );
    let else_is_in_range = branch_target_is_in_range(
        ctx,
        emitter,
        routine,
        block,
        else_block,
        next_block,
        emitter.position(),
    );
    if Some(else_block) == next_block
        && then_is_in_range
        && let Some(opcode) = branch_cond_opcode(cond)
    {
        emitter.emit_branch_label(
            opcode,
            ctx.layout.block_label(routine, then_block),
            SYNTHETIC_SPAN,
        );
        return true;
    }
    if Some(then_block) == next_block
        && else_is_in_range
        && let Some(opcode) = inverted_branch_opcode(cond)
    {
        emitter.emit_branch_label(
            opcode,
            ctx.layout.block_label(routine, else_block),
            SYNTHETIC_SPAN,
        );
        return true;
    }
    if then_is_in_range && let Some(opcode) = branch_cond_opcode(cond) {
        emitter.emit_branch_label(
            opcode,
            ctx.layout.block_label(routine, then_block),
            SYNTHETIC_SPAN,
        );
        if Some(else_block) != next_block {
            emitter.emit_jmp_label(ctx.layout.block_label(routine, else_block), SYNTHETIC_SPAN);
        }
        return true;
    }
    if else_is_in_range && let Some(opcode) = inverted_branch_opcode(cond) {
        emitter.emit_branch_label(
            opcode,
            ctx.layout.block_label(routine, else_block),
            SYNTHETIC_SPAN,
        );
        if Some(then_block) != next_block {
            emitter.emit_jmp_label(ctx.layout.block_label(routine, then_block), SYNTHETIC_SPAN);
        }
        return true;
    }
    false
}

fn branch_target_is_in_range(
    ctx: &MirEmitContext<'_>,
    emitter: &NativeTrackedEmitter,
    routine: RoutineId,
    block: MirBlockId,
    target: MirBlockId,
    next_block: Option<MirBlockId>,
    branch_position: usize,
) -> bool {
    if Some(target) == next_block {
        return true;
    }
    let label = ctx.layout.block_label(routine, target);
    emitter
        .label_position(&label)
        .is_some_and(|position| branch_offset_fits(branch_position, position))
        || ctx.branch_plan.contains(routine, block, target)
}

fn invert_branch_opcode(opcode: u8) -> Option<u8> {
    match opcode {
        opcode::BEQ_REL => Some(opcode::BNE_REL),
        opcode::BNE_REL => Some(opcode::BEQ_REL),
        opcode::BCS_REL => Some(opcode::BCC_REL),
        opcode::BCC_REL => Some(opcode::BCS_REL),
        opcode::BMI_REL => Some(opcode::BPL_REL),
        opcode::BPL_REL => Some(opcode::BMI_REL),
        opcode::BVS_REL => Some(opcode::BVC_REL),
        opcode::BVC_REL => Some(opcode::BVS_REL),
        _ => None,
    }
}

fn branch_offset_fits(branch_position: usize, target_position: usize) -> bool {
    let base = branch_position as isize + 2;
    let offset = target_position as isize - base;
    (-128..=127).contains(&offset)
}

fn emit_value_to_a(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    value: &MirValue,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    match value {
        MirValue::ConstU8(value) => {
            emitter.emit_lda_imm(*value);
            true
        }
        MirValue::ConstU16(value) if *value <= 0x00FF => {
            emitter.emit_lda_imm(*value as u8);
            true
        }
        MirValue::RoutineAddrByte { id, byte } if *byte <= 1 => {
            let label = ctx.layout.routine_label(*id);
            emitter.emit_u8(opcode::LDA_IMM);
            if *byte == 0 {
                emitter.emit_u8_label_low(label, SYNTHETIC_SPAN);
            } else {
                emitter.emit_u8_label_high(label, SYNTHETIC_SPAN);
            }
            true
        }
        MirValue::StorageAddrByte { mem, byte } if *byte <= 1 => {
            let Some(address) = ctx
                .layout
                .direct_mem(routine, mem)
                .map(resolved_mem_address)
            else {
                unsupported(ctx, routine, block, "storage address value is not placed");
                return false;
            };
            let value = if *byte == 0 {
                (address & 0x00FF) as u8
            } else {
                (address >> 8) as u8
            };
            emitter.emit_lda_imm(value);
            true
        }
        MirValue::Def(MirDef::Reg(MirReg::A)) => true,
        MirValue::Def(MirDef::Reg(MirReg::X)) => {
            emitter.emit_txa();
            true
        }
        MirValue::Def(MirDef::Reg(MirReg::Y)) => {
            emitter.emit_tya();
            true
        }
        MirValue::PointerCell(mem) => match ctx.layout.direct_mem(routine, mem) {
            Some(mem) => {
                emit_lda_mem(mem, emitter);
                true
            }
            None => {
                unsupported(ctx, routine, block, "value memory source is not placed");
                false
            }
        },
        _ => false,
    }
}

fn emit_carry(carry: Option<MirCarryIn>, op: MirBinaryOp, emitter: &mut NativeTrackedEmitter) {
    match (carry, op) {
        (Some(MirCarryIn::Clear), MirBinaryOp::Add) => emitter.emit_clc(),
        (Some(MirCarryIn::Set), MirBinaryOp::Sub) => emitter.emit_sec(),
        _ => {}
    }
}

fn emit_lda_mem(mem: ResolvedMem, emitter: &mut NativeTrackedEmitter) {
    match mem {
        ResolvedMem::Absolute(address) => emitter.emit_lda_abs(address),
        ResolvedMem::ZeroPage(address) => emitter.emit_lda_zero_page(ZeroPage::new(address)),
    }
}

fn emit_adc_value_to_a(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    value: &MirValue,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    match value {
        MirValue::ConstU8(value) => {
            emitter.emit_adc_imm(*value);
            true
        }
        MirValue::ConstU16(value) if *value <= 0x00FF => {
            emitter.emit_adc_imm(*value as u8);
            true
        }
        MirValue::StorageAddrByte { mem, byte } if *byte <= 1 => {
            let Some(address) = ctx
                .layout
                .direct_mem(routine, mem)
                .map(resolved_mem_address)
            else {
                unsupported(ctx, routine, block, "storage address value is not placed");
                return false;
            };
            let value = if *byte == 0 {
                (address & 0x00FF) as u8
            } else {
                (address >> 8) as u8
            };
            emitter.emit_adc_imm(value);
            true
        }
        MirValue::PointerCell(mem) => match ctx.layout.direct_mem(routine, mem) {
            Some(mem) => {
                emit_binary_mem(ctx, routine, block, MirBinaryOp::Add, mem, emitter);
                true
            }
            None => {
                unsupported(ctx, routine, block, "value memory source is not placed");
                false
            }
        },
        _ => false,
    }
}

fn emit_scaled_index_to_scratch(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    index: &MirValue,
    scale: u8,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    let Some((index_lo, index_hi)) = split_index_value_as_word(ctx, index) else {
        return false;
    };
    if !emit_value_to_a(ctx, routine, block, &index_lo, emitter) {
        return false;
    }
    if scale == 2 {
        emitter.emit_asl_a();
    }
    emit_sta_mem(ResolvedMem::ZeroPage(ADDRESS_INDEX_SCRATCH_LO), emitter);
    if !emit_value_to_a(ctx, routine, block, &index_hi, emitter) {
        return false;
    }
    if scale == 2 {
        emitter.emit_rol_a();
    }
    emit_sta_mem(ResolvedMem::ZeroPage(ADDRESS_INDEX_SCRATCH_HI), emitter);
    true
}

fn emit_scaled_index_plus_base_to_pointer(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    index: &MirValue,
    scale: u8,
    base_lo: &MirValue,
    base_hi: &MirValue,
    pointer_slot: u8,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    let Some((index_lo, index_hi)) = split_index_value_as_word(ctx, index) else {
        return false;
    };
    if !can_emit_value_to_a(ctx, routine, &index_lo)
        || !can_emit_value_to_a(ctx, routine, &index_hi)
        || !can_emit_adc_value_to_a(ctx, routine, base_lo)
        || !can_emit_adc_value_to_a(ctx, routine, base_hi)
    {
        return false;
    }

    emit_value_to_a(ctx, routine, block, &index_lo, emitter);
    if scale == 2 {
        emitter.emit_asl_a();
        emitter.emit_php();
    }
    emitter.emit_clc();
    emit_adc_value_to_a(ctx, routine, block, base_lo, emitter);
    emit_sta_mem(ResolvedMem::ZeroPage(pointer_slot), emitter);
    emit_value_to_a(ctx, routine, block, &index_hi, emitter);
    if scale == 2 {
        emitter.emit_rol_a();
        emitter.emit_plp();
    }
    emit_adc_value_to_a(ctx, routine, block, base_hi, emitter);
    emit_sta_mem(
        ResolvedMem::ZeroPage(pointer_slot.saturating_add(1)),
        emitter,
    );
    true
}

fn emit_scaled_y_index_plus_base_to_pointer(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    index: &MirValue,
    base_lo: &MirValue,
    base_hi: &MirValue,
    pointer_slot: u8,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    let Some((index_lo, index_hi)) = split_index_value_as_word(ctx, index) else {
        return false;
    };
    if !matches!(index_hi, MirValue::ConstU8(0) | MirValue::ConstU16(0))
        || !can_emit_value_to_a(ctx, routine, &index_lo)
        || !can_emit_scaled_y_base_byte(ctx, routine, base_lo)
        || !can_emit_scaled_y_base_byte(ctx, routine, base_hi)
    {
        return false;
    }

    emit_value_to_a(ctx, routine, block, &index_lo, emitter);
    emitter.emit_asl_a();
    emitter.emit_tay();
    emit_value_to_a(ctx, routine, block, base_lo, emitter);
    emit_sta_mem(ResolvedMem::ZeroPage(pointer_slot), emitter);
    emit_value_to_a(ctx, routine, block, base_hi, emitter);
    emitter.emit_adc_imm(0);
    emit_sta_mem(
        ResolvedMem::ZeroPage(pointer_slot.saturating_add(1)),
        emitter,
    );
    true
}

fn can_emit_scaled_y_base_byte(
    ctx: &MirEmitContext<'_>,
    routine: RoutineId,
    value: &MirValue,
) -> bool {
    !matches!(value, MirValue::Def(MirDef::Reg(MirReg::A | MirReg::Y)))
        && can_emit_value_to_a(ctx, routine, value)
}

fn emit_scaled_index_advance_pointer(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    index: &MirValue,
    scale: u8,
    pointer_slot: u8,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    let Some((index_lo, index_hi)) = split_index_value_as_word(ctx, index) else {
        return false;
    };
    if !can_emit_value_to_a(ctx, routine, &index_lo)
        || !can_emit_value_to_a(ctx, routine, &index_hi)
    {
        return false;
    }

    emit_value_to_a(ctx, routine, block, &index_lo, emitter);
    if scale == 2 {
        emitter.emit_asl_a();
        emitter.emit_php();
    }
    emitter.emit_clc();
    emitter.emit_adc_zero_page(ZeroPage::new(pointer_slot));
    emit_sta_mem(ResolvedMem::ZeroPage(pointer_slot), emitter);
    emit_value_to_a(ctx, routine, block, &index_hi, emitter);
    if scale == 2 {
        emitter.emit_rol_a();
        emitter.emit_plp();
    }
    emitter.emit_adc_zero_page(ZeroPage::new(pointer_slot.saturating_add(1)));
    emit_sta_mem(
        ResolvedMem::ZeroPage(pointer_slot.saturating_add(1)),
        emitter,
    );
    true
}

fn can_emit_value_to_a(ctx: &MirEmitContext<'_>, routine: RoutineId, value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(_)
        | MirValue::ConstU16(0..=0x00FF)
        | MirValue::Def(MirDef::Reg(MirReg::A))
        | MirValue::Def(MirDef::Reg(MirReg::X))
        | MirValue::Def(MirDef::Reg(MirReg::Y)) => true,
        MirValue::RoutineAddrByte { byte, .. } => *byte <= 1,
        MirValue::StorageAddrByte { mem, byte } => {
            *byte <= 1 && ctx.layout.direct_mem(routine, mem).is_some()
        }
        MirValue::PointerCell(mem) => ctx.layout.direct_mem(routine, mem).is_some(),
        _ => false,
    }
}

fn can_emit_adc_value_to_a(ctx: &MirEmitContext<'_>, routine: RoutineId, value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(_) | MirValue::ConstU16(0..=0x00FF) => true,
        MirValue::StorageAddrByte { mem, byte } => {
            *byte <= 1 && ctx.layout.direct_mem(routine, mem).is_some()
        }
        MirValue::PointerCell(mem) => ctx.layout.direct_mem(routine, mem).is_some(),
        _ => false,
    }
}

fn split_index_value_as_word(
    ctx: &MirEmitContext<'_>,
    value: &MirValue,
) -> Option<(MirValue, MirValue)> {
    split_value_as_word(ctx, value).or_else(|| match value {
        MirValue::PointerCell(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => Some((value.clone(), MirValue::ConstU8(0))),
        _ => None,
    })
}

fn resolved_mem_address(mem: ResolvedMem) -> u16 {
    match mem {
        ResolvedMem::Absolute(address) => address,
        ResolvedMem::ZeroPage(address) => u16::from(address),
    }
}

fn emit_address_to_def(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    dst: &MirDef,
    address: u16,
    emitter: &mut NativeTrackedEmitter,
) {
    let lo = (address & 0x00FF) as u8;
    let hi = (address >> 8) as u8;
    match dst {
        MirDef::VTemp(temp) => {
            emit_address_byte_to_spill(
                ctx,
                routine,
                block,
                MirSpillId(temp.0.saturating_mul(2)),
                lo,
                emitter,
            );
            emit_address_byte_to_spill(
                ctx,
                routine,
                block,
                MirSpillId(temp.0.saturating_mul(2).saturating_add(1)),
                hi,
                emitter,
            );
        }
        MirDef::VTempByte { id, byte } => {
            let value = if *byte == 0 { lo } else { hi };
            emit_address_byte_to_spill(
                ctx,
                routine,
                block,
                MirSpillId(id.0.saturating_mul(2).saturating_add(u32::from(*byte))),
                value,
                emitter,
            );
        }
        MirDef::Reg(MirReg::A) => emitter.emit_lda_imm(lo),
        MirDef::Reg(MirReg::X) => emitter.emit_ldx_imm(lo),
        MirDef::Reg(MirReg::Y) => emitter.emit_ldy_imm(lo),
    }
}

fn emit_address_byte_to_spill(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    spill: MirSpillId,
    value: u8,
    emitter: &mut NativeTrackedEmitter,
) {
    let Some(mem) = ctx.layout.direct_mem(
        routine,
        &MirMem::Spill {
            id: spill,
            offset: 0,
        },
    ) else {
        unsupported(ctx, routine, block, "lea spill destination is not placed");
        return;
    };
    emitter.emit_lda_imm(value);
    emit_sta_mem(mem, emitter);
}

fn emit_ldx_mem(mem: ResolvedMem, emitter: &mut NativeTrackedEmitter) {
    match mem {
        ResolvedMem::Absolute(address) => emitter.emit_ldx_abs(address),
        ResolvedMem::ZeroPage(address) => emitter.emit_ldx_zero_page(ZeroPage::new(address)),
    }
}

fn emit_ldy_mem(mem: ResolvedMem, emitter: &mut NativeTrackedEmitter) {
    match mem {
        ResolvedMem::Absolute(address) => emitter.emit_ldy_abs(address),
        ResolvedMem::ZeroPage(address) => emitter.emit_ldy_zero_page(ZeroPage::new(address)),
    }
}

fn emit_sta_mem(mem: ResolvedMem, emitter: &mut NativeTrackedEmitter) {
    match mem {
        ResolvedMem::Absolute(address) => emitter.emit_sta_absolute(Absolute::new(address)),
        ResolvedMem::ZeroPage(address) => emitter.emit_sta_zero_page(ZeroPage::new(address)),
    }
}

fn emit_update_mem(op: MirUpdateOp, mem: ResolvedMem, emitter: &mut NativeTrackedEmitter) {
    match (op, mem) {
        (MirUpdateOp::Inc, ResolvedMem::Absolute(address)) => {
            emitter.emit_inc_absolute(Absolute::new(address));
        }
        (MirUpdateOp::Inc, ResolvedMem::ZeroPage(address)) => {
            emitter.emit_inc_zero_page(ZeroPage::new(address));
        }
        (MirUpdateOp::Dec, ResolvedMem::Absolute(address)) => {
            emitter.emit_dec_absolute(Absolute::new(address));
        }
        (MirUpdateOp::Dec, ResolvedMem::ZeroPage(address)) => {
            emitter.emit_dec_zero_page(ZeroPage::new(address));
        }
    }
}

fn emit_word_inc_mem(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    mem: ResolvedMem,
    emitter: &mut NativeTrackedEmitter,
) {
    let done = format!(
        "__mir6502_word_inc_done_{}_{}_{}",
        routine.0,
        block.0,
        emitter.position()
    );
    emit_update_mem(MirUpdateOp::Inc, mem, emitter);
    emitter.emit_branch_label(opcode::BNE_REL, done.clone(), SYNTHETIC_SPAN);
    emit_update_mem(MirUpdateOp::Inc, offset_resolved_mem(mem, 1), emitter);
    bind_label(ctx, emitter, routine, Some(block), done);
}

fn emit_word_dec_mem(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    mem: ResolvedMem,
    emitter: &mut NativeTrackedEmitter,
) {
    let dec_low = format!(
        "__mir6502_word_dec_low_{}_{}_{}",
        routine.0,
        block.0,
        emitter.position()
    );
    emit_lda_mem(mem, emitter);
    emitter.emit_branch_label(opcode::BNE_REL, dec_low.clone(), SYNTHETIC_SPAN);
    emit_update_mem(MirUpdateOp::Dec, offset_resolved_mem(mem, 1), emitter);
    bind_label(ctx, emitter, routine, Some(block), dec_low);
    emit_update_mem(MirUpdateOp::Dec, mem, emitter);
}

fn emit_byte_to_word_mem(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    op: MirBinaryOp,
    mem: ResolvedMem,
    value: &MirValue,
    emitter: &mut NativeTrackedEmitter,
) {
    emit_lda_mem(mem, emitter);
    emit_carry(
        Some(match op {
            MirBinaryOp::Add => MirCarryIn::Clear,
            MirBinaryOp::Sub => MirCarryIn::Set,
            _ => unreachable!(),
        }),
        op,
        emitter,
    );
    if let Some(value) = const_u8(value) {
        emit_binary_imm(ctx, routine, block, op, value, emitter);
    } else if let MirValue::PointerCell(value_mem) = value {
        match ctx.layout.direct_mem(routine, value_mem) {
            Some(value_mem) => emit_binary_mem(ctx, routine, block, op, value_mem, emitter),
            None => {
                unsupported(ctx, routine, block, "byte word update source is not placed");
                return;
            }
        }
    } else {
        unsupported(
            ctx,
            routine,
            block,
            "byte word update source is not emit-ready",
        );
        return;
    }
    emit_sta_mem(mem, emitter);

    let done = format!(
        "__mir6502_word_{}_byte_done_{}_{}_{}",
        match op {
            MirBinaryOp::Add => "add",
            MirBinaryOp::Sub => "sub",
            _ => unreachable!(),
        },
        routine.0,
        block.0,
        emitter.position()
    );
    match op {
        MirBinaryOp::Add => {
            emitter.emit_branch_label(opcode::BCC_REL, done.clone(), SYNTHETIC_SPAN);
            emit_update_mem(MirUpdateOp::Inc, offset_resolved_mem(mem, 1), emitter);
        }
        MirBinaryOp::Sub => {
            emitter.emit_branch_label(opcode::BCS_REL, done.clone(), SYNTHETIC_SPAN);
            emit_update_mem(MirUpdateOp::Dec, offset_resolved_mem(mem, 1), emitter);
        }
        _ => unreachable!(),
    }
    bind_label(ctx, emitter, routine, Some(block), done);
}

fn offset_resolved_mem(mem: ResolvedMem, offset: u16) -> ResolvedMem {
    match mem {
        ResolvedMem::Absolute(address) => ResolvedMem::Absolute(address.wrapping_add(offset)),
        ResolvedMem::ZeroPage(address) => ResolvedMem::ZeroPage(address.wrapping_add(offset as u8)),
    }
}

fn emit_stx_mem(mem: ResolvedMem, emitter: &mut NativeTrackedEmitter) {
    match mem {
        ResolvedMem::Absolute(address) => emitter.emit_stx_absolute(Absolute::new(address)),
        ResolvedMem::ZeroPage(address) => emitter.emit_stx_zero_page(ZeroPage::new(address)),
    }
}

fn emit_sty_mem(mem: ResolvedMem, emitter: &mut NativeTrackedEmitter) {
    match mem {
        ResolvedMem::Absolute(address) => emitter.emit_sty_absolute(Absolute::new(address)),
        ResolvedMem::ZeroPage(address) => emitter.emit_sty_zero_page(ZeroPage::new(address)),
    }
}

fn emit_lda_indexed(mem: ResolvedIndexedMem, emitter: &mut NativeTrackedEmitter) {
    match mem {
        ResolvedIndexedMem::AbsoluteX(address) => emitter.emit_lda_abs_x(address),
        ResolvedIndexedMem::AbsoluteY(address) => emitter.emit_lda_abs_y(address),
        ResolvedIndexedMem::ZeroPageX(address) => {
            emitter.emit_lda_zero_page_x(ZeroPageX::new(address))
        }
        ResolvedIndexedMem::IndirectY(address) => {
            emitter.emit_lda_indirect_indexed_y(IndirectIndexedY::new(ZeroPage::new(address)))
        }
    }
}

fn emit_sta_indexed(mem: ResolvedIndexedMem, emitter: &mut NativeTrackedEmitter) {
    match mem {
        ResolvedIndexedMem::AbsoluteX(address) => emitter.emit_sta_abs_x(address),
        ResolvedIndexedMem::AbsoluteY(address) => emitter.emit_sta_abs_y(address),
        ResolvedIndexedMem::ZeroPageX(address) => {
            emitter.emit_sta_zero_page_x(ZeroPageX::new(address))
        }
        ResolvedIndexedMem::IndirectY(address) => {
            emitter.emit_sta_indirect_indexed_y(IndirectIndexedY::new(ZeroPage::new(address)))
        }
    }
}

fn resolve_pointer_consumer_slot(
    ctx: &MirEmitContext<'_>,
    routine: RoutineId,
    consumer: &MirAddressConsumer,
) -> Option<u8> {
    match consumer.pointer_pair() {
        MirPointerPair::Fixed { lo } => Some(lo.0),
        MirPointerPair::Virtual(slot) => ctx.layout.zero_page_slot(routine, slot),
    }
}

fn emit_lda_indirect(
    consumer: MirAddressConsumer,
    pointer_slot: u8,
    offset: u16,
    emitter: &mut NativeTrackedEmitter,
) {
    prepare_indirect_y(consumer, offset, emitter);
    emitter.emit_lda_indirect_indexed_y(IndirectIndexedY::new(ZeroPage::new(pointer_slot)));
}

fn emit_sta_indirect(
    consumer: MirAddressConsumer,
    pointer_slot: u8,
    offset: u16,
    emitter: &mut NativeTrackedEmitter,
) {
    prepare_indirect_y(consumer, offset, emitter);
    emitter.emit_sta_indirect_indexed_y(IndirectIndexedY::new(ZeroPage::new(pointer_slot)));
}

fn prepare_indirect_y(
    consumer: MirAddressConsumer,
    offset: u16,
    emitter: &mut NativeTrackedEmitter,
) {
    if consumer.uses_scaled_y() {
        if offset == 1 {
            emitter.emit_iny();
        }
    } else {
        emitter.emit_ldy_imm(offset as u8);
    }
}

fn emit_indirect_byte_compound(
    op: MirBinaryOp,
    target_slot: u8,
    source_slot: u8,
    offset: u16,
    emitter: &mut NativeTrackedEmitter,
) {
    emitter.emit_ldy_imm(offset as u8);
    emitter.emit_lda_indirect_indexed_y(IndirectIndexedY::new(ZeroPage::new(target_slot)));
    match op {
        MirBinaryOp::Add => {
            emitter.emit_clc();
            emitter.emit_adc_indirect_indexed_y(IndirectIndexedY::new(ZeroPage::new(source_slot)));
        }
        MirBinaryOp::Sub => {
            emitter.emit_sec();
            emitter.emit_sbc_indirect_indexed_y(IndirectIndexedY::new(ZeroPage::new(source_slot)));
        }
        _ => unreachable!("indirect byte compound only supports add and subtract"),
    }
    emitter.emit_sta_indirect_indexed_y(IndirectIndexedY::new(ZeroPage::new(target_slot)));
}

fn split_value_as_word(ctx: &MirEmitContext<'_>, value: &MirValue) -> Option<(MirValue, MirValue)> {
    match value {
        MirValue::Word { lo, hi } => Some((lo.as_ref().clone(), hi.as_ref().clone())),
        MirValue::ConstU8(value) => Some((MirValue::ConstU8(*value), MirValue::ConstU8(0))),
        MirValue::ConstU16(value) => Some((
            MirValue::ConstU8((value & 0x00FF) as u8),
            MirValue::ConstU8((value >> 8) as u8),
        )),
        MirValue::StaticAddr(id) => ctx.layout.static_address(*id).map(split_address),
        MirValue::GlobalAddr(id) => ctx.layout.global_address(*id).map(split_address),
        MirValue::RoutineAddr(id) => Some(split_routine_address(*id)),
        MirValue::Def(def) => Some((MirValue::Def(def.clone()), MirValue::ConstU8(0))),
        _ => None,
    }
}

fn split_address(address: u16) -> (MirValue, MirValue) {
    (
        MirValue::ConstU8((address & 0x00FF) as u8),
        MirValue::ConstU8((address >> 8) as u8),
    )
}

fn split_routine_address(id: RoutineId) -> (MirValue, MirValue) {
    (
        MirValue::RoutineAddrByte { id, byte: 0 },
        MirValue::RoutineAddrByte { id, byte: 1 },
    )
}

fn emit_binary_imm(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    op: MirBinaryOp,
    right: u8,
    emitter: &mut NativeTrackedEmitter,
) {
    match op {
        MirBinaryOp::Add => emitter.emit_adc_imm(right),
        MirBinaryOp::Sub => emitter.emit_sbc_imm(right),
        MirBinaryOp::And => emitter.emit_and_imm(right),
        MirBinaryOp::Or => emitter.emit_ora_imm(right),
        MirBinaryOp::Xor => emitter.emit_eor_imm(right),
        MirBinaryOp::Lsh => {
            for _ in 0..right {
                emitter.emit_asl_a();
            }
        }
        MirBinaryOp::Rsh => {
            for _ in 0..right {
                emitter.emit_lsr_a();
            }
        }
        _ => unsupported(ctx, routine, block, "binary op is not emit-ready"),
    }
}

fn emit_binary_mem(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    op: MirBinaryOp,
    mem: ResolvedMem,
    emitter: &mut NativeTrackedEmitter,
) {
    match (op, mem) {
        (MirBinaryOp::Add, ResolvedMem::Absolute(address)) => emitter.emit_adc_abs(address),
        (MirBinaryOp::Add, ResolvedMem::ZeroPage(address)) => {
            emitter.emit_adc_zero_page(crate::codegen::ZeroPage::new(address))
        }
        (MirBinaryOp::Sub, ResolvedMem::Absolute(address)) => emitter.emit_sbc_abs(address),
        (MirBinaryOp::Sub, ResolvedMem::ZeroPage(address)) => {
            emitter.emit_sbc_zero_page(crate::codegen::ZeroPage::new(address))
        }
        (MirBinaryOp::And, ResolvedMem::Absolute(address)) => emitter.emit_and_abs(address),
        (MirBinaryOp::And, ResolvedMem::ZeroPage(address)) => {
            emitter.emit_and_zero_page(crate::codegen::ZeroPage::new(address))
        }
        (MirBinaryOp::Or, ResolvedMem::Absolute(address)) => emitter.emit_ora_abs(address),
        (MirBinaryOp::Or, ResolvedMem::ZeroPage(address)) => {
            emitter.emit_ora_zero_page(crate::codegen::ZeroPage::new(address))
        }
        (MirBinaryOp::Xor, ResolvedMem::Absolute(address)) => emitter.emit_eor_abs(address),
        (MirBinaryOp::Xor, ResolvedMem::ZeroPage(address)) => {
            emitter.emit_eor_zero_page(crate::codegen::ZeroPage::new(address))
        }
        _ => unsupported(ctx, routine, block, "binary op is not emit-ready"),
    }
}

fn emit_cmp_mem(mem: ResolvedMem, emitter: &mut NativeTrackedEmitter) {
    match mem {
        ResolvedMem::Absolute(address) => emitter.emit_cmp_abs(address),
        ResolvedMem::ZeroPage(address) => {
            emitter.emit_cmp_zero_page(crate::codegen::ZeroPage::new(address))
        }
    }
}

fn emit_compare(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    dst: &MirCondDest,
    op: MirCompareOp,
    left: &MirValue,
    right: &MirValue,
    signed: bool,
    emitter: &mut NativeTrackedEmitter,
) {
    match dst {
        MirCondDest::Flags => {
            let _ = emit_compare_flags(ctx, routine, block, op, left, right, emitter);
        }
        MirCondDest::Temp(id) => {
            if signed {
                unsupported(ctx, routine, block, "signed compare temp is not emit-ready");
                return;
            }
            let dst_mem = MirMem::Spill {
                id: MirSpillId(id.0.saturating_mul(2)),
                offset: 0,
            };
            let Some(dst_mem) = ctx.layout.direct_mem(routine, &dst_mem) else {
                unsupported(
                    ctx,
                    routine,
                    block,
                    "compare temp destination is not placed",
                );
                return;
            };
            match compare_temp_plan(op, right) {
                Some(CompareTempPlan::Known(value)) => {
                    emitter.emit_lda_imm(u8::from(value));
                    emit_sta_mem(dst_mem, emitter);
                }
                Some(CompareTempPlan::Flags { test, right }) => {
                    let compare_op = compare_op_for_flag_test(&test);
                    if !emit_compare_flags(ctx, routine, block, compare_op, left, &right, emitter) {
                        return;
                    }
                    let Some(opcode) = branch_flag_opcode(test) else {
                        unsupported(ctx, routine, block, "compare flag test is not branchable");
                        return;
                    };
                    let base = format!(
                        "__mir6502_cmp_bool_{}_{}_{}",
                        routine.0,
                        block.0,
                        emitter.position()
                    );
                    let true_label = format!("{base}_true");
                    let done_label = format!("{base}_done");
                    emitter.emit_branch_label(opcode, true_label.clone(), SYNTHETIC_SPAN);
                    emitter.emit_lda_imm(0);
                    emitter.emit_jmp_label(done_label.clone(), SYNTHETIC_SPAN);
                    let _ = emitter.bind_label(true_label, SYNTHETIC_SPAN);
                    emitter.emit_lda_imm(1);
                    let _ = emitter.bind_label(done_label, SYNTHETIC_SPAN);
                    emit_sta_mem(dst_mem, emitter);
                }
                Some(CompareTempPlan::AnyFlags { tests, right }) => {
                    if !emit_compare_flags(
                        ctx,
                        routine,
                        block,
                        MirCompareOp::Eq,
                        left,
                        &right,
                        emitter,
                    ) {
                        return;
                    }
                    let Some(first_opcode) = branch_flag_opcode(tests[0].clone()) else {
                        unsupported(ctx, routine, block, "compare flag test is not branchable");
                        return;
                    };
                    let Some(second_opcode) = branch_flag_opcode(tests[1].clone()) else {
                        unsupported(ctx, routine, block, "compare flag test is not branchable");
                        return;
                    };
                    let base = format!(
                        "__mir6502_cmp_bool_{}_{}_{}",
                        routine.0,
                        block.0,
                        emitter.position()
                    );
                    let true_label = format!("{base}_true");
                    let done_label = format!("{base}_done");
                    emitter.emit_branch_label(first_opcode, true_label.clone(), SYNTHETIC_SPAN);
                    emitter.emit_branch_label(second_opcode, true_label.clone(), SYNTHETIC_SPAN);
                    emitter.emit_lda_imm(0);
                    emitter.emit_jmp_label(done_label.clone(), SYNTHETIC_SPAN);
                    let _ = emitter.bind_label(true_label, SYNTHETIC_SPAN);
                    emitter.emit_lda_imm(1);
                    let _ = emitter.bind_label(done_label, SYNTHETIC_SPAN);
                    emit_sta_mem(dst_mem, emitter);
                }
                None => unsupported(ctx, routine, block, "compare op is not emit-ready"),
            }
        }
    }
}

fn emit_compare_flags(
    ctx: &mut MirEmitContext<'_>,
    routine: RoutineId,
    block: MirBlockId,
    op: MirCompareOp,
    left: &MirValue,
    right: &MirValue,
    emitter: &mut NativeTrackedEmitter,
) -> bool {
    if !emit_value_to_a(ctx, routine, block, left, emitter) {
        unsupported(ctx, routine, block, "compare left source is not emit-ready");
        return false;
    }
    if let Some(right) = const_u8(right) {
        if matches!(op, MirCompareOp::Eq | MirCompareOp::Ne) {
            emitter.emit_cmp_imm_for_z_branch(right);
        } else {
            emitter.emit_cmp_imm(right);
        }
        true
    } else if let MirValue::PointerCell(mem) = right {
        match ctx.layout.direct_mem(routine, mem) {
            Some(mem) => {
                emit_cmp_mem(mem, emitter);
                true
            }
            None => {
                unsupported(ctx, routine, block, "compare right source is not placed");
                false
            }
        }
    } else {
        unsupported(
            ctx,
            routine,
            block,
            "compare right source is not emit-ready",
        );
        false
    }
}

fn compare_op_for_flag_test(test: &MirFlagTest) -> MirCompareOp {
    match test {
        MirFlagTest::ZSet => MirCompareOp::Eq,
        MirFlagTest::ZClear => MirCompareOp::Ne,
        MirFlagTest::CSet => MirCompareOp::Ge,
        MirFlagTest::CClear => MirCompareOp::Lt,
        _ => MirCompareOp::Eq,
    }
}

enum CompareTempPlan {
    Flags {
        test: MirFlagTest,
        right: MirValue,
    },
    AnyFlags {
        tests: [MirFlagTest; 2],
        right: MirValue,
    },
    Known(bool),
}

fn compare_temp_plan(op: MirCompareOp, right: &MirValue) -> Option<CompareTempPlan> {
    match op {
        MirCompareOp::Eq => Some(CompareTempPlan::Flags {
            test: MirFlagTest::ZSet,
            right: right.clone(),
        }),
        MirCompareOp::Ne => Some(CompareTempPlan::Flags {
            test: MirFlagTest::ZClear,
            right: right.clone(),
        }),
        MirCompareOp::Lt => Some(CompareTempPlan::Flags {
            test: MirFlagTest::CClear,
            right: right.clone(),
        }),
        MirCompareOp::Ge => Some(CompareTempPlan::Flags {
            test: MirFlagTest::CSet,
            right: right.clone(),
        }),
        MirCompareOp::Le => {
            if let Some(value) = const_u8(right) {
                value
                    .checked_add(1)
                    .map_or(Some(CompareTempPlan::Known(true)), |next| {
                        Some(CompareTempPlan::Flags {
                            test: MirFlagTest::CClear,
                            right: MirValue::ConstU8(next),
                        })
                    })
            } else {
                Some(CompareTempPlan::AnyFlags {
                    tests: [MirFlagTest::CClear, MirFlagTest::ZSet],
                    right: right.clone(),
                })
            }
        }
        MirCompareOp::Gt => {
            let value = const_u8(right)?;
            value
                .checked_add(1)
                .map_or(Some(CompareTempPlan::Known(false)), |next| {
                    Some(CompareTempPlan::Flags {
                        test: MirFlagTest::CSet,
                        right: MirValue::ConstU8(next),
                    })
                })
        }
    }
}

fn inverted_branch_opcode(cond: &MirCond) -> Option<u8> {
    branch_flag_test(cond)
        .and_then(invert_branch_flag_test)
        .and_then(branch_flag_opcode)
}

fn branch_cond_opcode(cond: &MirCond) -> Option<u8> {
    branch_flag_test(cond).and_then(branch_flag_opcode)
}

fn branch_flag_test(cond: &MirCond) -> Option<MirFlagTest> {
    match cond {
        MirCond::FlagTest(test)
        | MirCond::FusedCompare {
            flag_test: test, ..
        } => Some(test.clone()),
        MirCond::Deferred | MirCond::BoolValue(_) | MirCond::AnyFlagTest(_) => None,
    }
}

fn branch_flag_opcode(test: MirFlagTest) -> Option<u8> {
    match test {
        MirFlagTest::ZSet => Some(opcode::BEQ_REL),
        MirFlagTest::ZClear => Some(opcode::BNE_REL),
        MirFlagTest::CSet => Some(opcode::BCS_REL),
        MirFlagTest::CClear => Some(opcode::BCC_REL),
        MirFlagTest::NSet => Some(opcode::BMI_REL),
        MirFlagTest::NClear => Some(opcode::BPL_REL),
        MirFlagTest::VSet => Some(opcode::BVS_REL),
        MirFlagTest::VClear => Some(opcode::BVC_REL),
    }
}

fn invert_branch_flag_test(test: MirFlagTest) -> Option<MirFlagTest> {
    match test {
        MirFlagTest::ZSet => Some(MirFlagTest::ZClear),
        MirFlagTest::ZClear => Some(MirFlagTest::ZSet),
        MirFlagTest::CSet => Some(MirFlagTest::CClear),
        MirFlagTest::CClear => Some(MirFlagTest::CSet),
        MirFlagTest::NSet => Some(MirFlagTest::NClear),
        MirFlagTest::NClear => Some(MirFlagTest::NSet),
        MirFlagTest::VSet => Some(MirFlagTest::VClear),
        MirFlagTest::VClear => Some(MirFlagTest::VSet),
    }
}

fn const_u8(value: &MirValue) -> Option<u8> {
    match value {
        MirValue::ConstU8(value) => Some(*value),
        MirValue::ConstU16(value) if *value <= 0x00FF => Some(*value as u8),
        _ => None,
    }
}

fn global_object_size(storage_size: u16, init: Option<&MirGlobalInit>) -> u16 {
    init.map_or(storage_size, |init| init.object_size(storage_size))
}

fn slot_size(slot: &MirStorageSlot) -> u16 {
    slot.init.as_ref().map_or(width_size(slot.width), |init| {
        init.object_size(width_size(slot.width))
    })
}

fn width_size(width: MirWidth) -> u16 {
    match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    }
}

fn current_address(ctx: &MirEmitContext<'_>, emitter: &NativeTrackedEmitter) -> u16 {
    ctx.origin.saturating_add(emitter.position() as u16)
}

fn storage_symbol(
    name: String,
    scope: CodegenSymbolScope,
    kind: CodegenSymbolKind,
    placement: Option<MirStoragePlacement>,
) -> Option<CodegenStorageSymbol> {
    match placement? {
        MirStoragePlacement::Absolute { address, size } => Some(CodegenStorageSymbol {
            name,
            scope,
            kind,
            address,
            size,
            address_space: CodegenAddressSpace::Absolute,
            pointee_size: None,
            array: None,
            signed: false,
        }),
        MirStoragePlacement::Unresolved => None,
    }
}

fn routine_slot_placement(
    layout: &MirObjectLayout,
    routine: RoutineId,
    slot: &MirStorageSlot,
) -> Option<MirStoragePlacement> {
    let storage = layout.routine_storage.get(&routine)?;
    match slot.base {
        MirStorageBase::Param(id) => storage.params.get(&id).copied(),
        MirStorageBase::Local(id) => storage.locals.get(&id).copied(),
        MirStorageBase::LocalAlias { id, .. } => storage.locals.get(&id).copied(),
        MirStorageBase::Spill(id) => storage.spills.get(&id).copied(),
        MirStorageBase::Absolute(address) => Some(MirStoragePlacement::Absolute {
            address: address.saturating_add(slot.offset),
            size: slot_size(slot),
        }),
        MirStorageBase::Global(id) => layout.globals.get(&id).copied(),
        MirStorageBase::Static(id) => layout.statics.get(&id).copied(),
    }
}

fn routine_slot_name(slot: &MirStorageSlot) -> String {
    if let Some(name) = &slot.name {
        return name.clone();
    }
    match slot.base {
        MirStorageBase::Param(id) => format!("p{}", id.0),
        MirStorageBase::Local(id) => format!("l{}", id.0),
        MirStorageBase::LocalAlias { id, .. } => format!("l{}", id.0),
        MirStorageBase::Spill(id) => format!("spill{}", id.0),
        MirStorageBase::Global(id) => format!("g{}", id.0),
        MirStorageBase::Static(id) => format!("s{}", id.0),
        MirStorageBase::Absolute(address) => format!("abs_{address:04X}"),
    }
}

fn global_label(id: SymbolId) -> String {
    format!("mir6502:global:{}", id.0)
}

fn static_label(id: SymbolId) -> String {
    format!("mir6502:static:{}", id.0)
}

fn routine_slot_label(routine: RoutineId, slot: &MirStorageSlot) -> String {
    format!("mir6502:r{}:slot:{}", routine.0, routine_slot_name(slot))
}

fn routine_label(routine: RoutineId) -> String {
    format!("mir6502:r{}:entry", routine.0)
}

fn block_label(routine: RoutineId, block: MirBlockId) -> String {
    format!("mir6502:r{}:b{}", routine.0, block.0)
}

fn normalize_machine_name(name: &str) -> String {
    name.chars()
        .filter(|ch| !matches!(ch, '_' | '-' | ' '))
        .flat_map(char::to_uppercase)
        .collect()
}

fn unsupported(ctx: &mut MirEmitContext<'_>, routine: RoutineId, block: MirBlockId, message: &str) {
    unsupported_message(
        Some(format!("r{}", routine.0)),
        Some(format!("b{}", block.0)),
        message,
        &mut ctx.diagnostics,
    );
}

fn unsupported_message(
    routine: Option<String>,
    block: Option<String>,
    message: &str,
    diagnostics: &mut Vec<MirDiagnostic>,
) {
    diagnostics.push(MirDiagnostic {
        routine,
        block,
        message: message.to_string(),
    });
}
