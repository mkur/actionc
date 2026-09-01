use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::sync::OnceLock;

use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirAddr, MirByteIndexedSource, MirCallTarget, MirCond, MirDataImage, MirDataRelocationTarget,
    MirEffects, MirGlobalBacking, MirGlobalInit, MirInlineAsmTarget, MirMachineBlock,
    MirMachineBlockId, MirMachineItem, MirMem, MirMemoryEffect, MirMemoryRegionKind, MirOp,
    MirProgram, MirRuntimeHelper, MirRuntimeHelperTarget, MirStorageBase, MirStorageInit,
    MirTerminator, MirValue, RoutineId,
};
use crate::linker::{LinkGraph, LinkReason};
use crate::nir::SymbolId;
#[cfg(test)]
use crate::runtime_link_manifest::embedded_sys_link_manifest;
use crate::runtime_link_manifest::{RuntimeLinkManifest, RuntimeLinkNode};
#[cfg(test)]
static SYSLIB_MIR: OnceLock<Result<MirProgram, Vec<MirDiagnostic>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MirLinkNode {
    Routine(RoutineId),
    Global(SymbolId),
    Static(SymbolId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeSelection {
    pub(super) routines: BTreeSet<RoutineId>,
    globals: BTreeSet<SymbolId>,
    statics: BTreeSet<SymbolId>,
}

pub(super) fn bind_runtime_selection(
    program: &MirProgram,
    selected: &crate::runtime_link_manifest::RuntimeLinkSelection,
    link_module: &str,
) -> Result<RuntimeSelection, Vec<MirDiagnostic>> {
    let routines = program
        .routines
        .iter()
        .filter(|routine| {
            selected
                .routines
                .contains(&runtime_routine_name(&routine.name, link_module).to_ascii_uppercase())
        })
        .map(|routine| routine.id)
        .collect::<BTreeSet<_>>();
    let globals = program
        .globals
        .iter()
        .filter(|global| {
            selected
                .globals
                .contains(&runtime_storage_name(&global.name, link_module).to_ascii_uppercase())
        })
        .map(|global| global.id)
        .collect::<BTreeSet<_>>();
    let bound_routines = program
        .routines
        .iter()
        .filter(|routine| routines.contains(&routine.id))
        .map(|routine| runtime_routine_name(&routine.name, link_module).to_ascii_uppercase())
        .collect::<BTreeSet<_>>();
    let bound_globals = program
        .globals
        .iter()
        .filter(|global| globals.contains(&global.id))
        .map(|global| runtime_storage_name(&global.name, link_module).to_ascii_uppercase())
        .collect::<BTreeSet<_>>();
    if bound_routines != selected.routines || bound_globals != selected.globals {
        return Err(diagnostic(format!(
            "selected runtime SemIR did not lower every manifest node; missing routines [{}], globals [{}]",
            selected
                .routines
                .difference(&bound_routines)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            selected
                .globals
                .difference(&bound_globals)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(RuntimeSelection {
        routines,
        globals,
        // Provider statics are owned by already-selected source routines and
        // do not have independent SemIR identities.
        statics: program
            .statics
            .iter()
            .map(|static_data| static_data.id)
            .collect(),
    })
}

#[cfg(test)]
pub(super) fn syslib_mir() -> Result<MirProgram, Vec<MirDiagnostic>> {
    SYSLIB_MIR.get_or_init(compile_syslib).clone()
}

pub(super) fn link_helpers(program: &mut MirProgram) -> Result<(), Vec<MirDiagnostic>> {
    for declaration in &program.runtime_helpers {
        match declaration.target {
            MirRuntimeHelperTarget::KnownAbsolute(address) => {
                return Err(diagnostic(format!(
                    "standalone runtime rejects absolute override ${address:04X} for `{}`",
                    super::runtime::helper_name(declaration.helper)
                )));
            }
            MirRuntimeHelperTarget::Routine(id) => {
                if !program.routines.iter().any(|routine| routine.id == id) {
                    return Err(diagnostic(format!(
                        "local override for `{}` refers to missing routine r{}",
                        super::runtime::helper_name(declaration.helper),
                        id.0
                    )));
                }
            }
            MirRuntimeHelperTarget::Deferred => {}
        }
    }

    let required = program
        .runtime_helpers
        .iter()
        .filter(|declaration| matches!(declaration.target, MirRuntimeHelperTarget::Deferred))
        .map(|declaration| declaration.helper)
        .collect::<BTreeSet<_>>();
    if required.is_empty() {
        return Ok(());
    }

    let helper_roots = required
        .iter()
        .map(|helper| super::runtime::helper_name(*helper).to_string())
        .collect::<BTreeSet<_>>();
    let selected_runtime = crate::runtime_source::select_runtime_unit(
        "syslib.act",
        "ACTION.RUNTIME.SYSLIB",
        &helper_roots,
    )
    .map_err(|diagnostics| frontend_diagnostics("syslib.act", diagnostics))?;
    let runtime = lower_runtime_semir(&selected_runtime.semir, "syslib.act")?;
    let roots = required
        .iter()
        .map(|helper| {
            validate_helper_contract(program, &runtime, *helper)?;
            find_runtime_helper(&runtime, *helper).map(|routine| (*helper, routine))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let selected = bind_runtime_selection(
        &runtime,
        &selected_runtime.selection,
        "ACTION_RUNTIME_SYSLIB",
    )?;
    let routine_rebase = append_runtime_selection(
        program,
        &runtime,
        &selected,
        "ACTION.RUNTIME.SYSLIB",
        "ACTION_RUNTIME_SYSLIB",
    )?;

    for declaration in &mut program.runtime_helpers {
        if matches!(declaration.target, MirRuntimeHelperTarget::Deferred) {
            declaration.target =
                MirRuntimeHelperTarget::Routine(routine_rebase[&roots[&declaration.helper]]);
        }
    }
    Ok(())
}

pub(super) fn append_runtime_selection(
    program: &mut MirProgram,
    runtime: &MirProgram,
    selected: &RuntimeSelection,
    display_module: &str,
    link_module: &str,
) -> Result<BTreeMap<RoutineId, RoutineId>, Vec<MirDiagnostic>> {
    let routine_rebase = selected
        .routines
        .iter()
        .copied()
        .enumerate()
        .map(|(index, old)| {
            (
                old,
                RoutineId(next_routine_id(program).wrapping_add(index as u32)),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let selected_machine_ids = selected_machine_ids(runtime, &selected.routines);
    let machine_base = next_machine_id(program);
    let machine_rebase = selected_machine_ids
        .iter()
        .copied()
        .enumerate()
        .map(|(index, old)| {
            (
                old,
                MirMachineBlockId(machine_base.wrapping_add(index as u32)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let global_base = next_global_id(program);
    let global_rebase = selected
        .globals
        .iter()
        .copied()
        .enumerate()
        .map(|(index, old)| (old, SymbolId(global_base.wrapping_add(index as u32))))
        .collect::<BTreeMap<_, _>>();
    let static_base = next_static_id(program);
    let static_rebase = selected
        .statics
        .iter()
        .copied()
        .enumerate()
        .map(|(index, old)| (old, SymbolId(static_base.wrapping_add(index as u32))))
        .collect::<BTreeMap<_, _>>();

    for old_id in &selected.routines {
        let mut routine = runtime
            .routines
            .iter()
            .find(|routine| routine.id == *old_id)
            .expect("selected runtime routine exists")
            .clone();
        routine.id = routine_rebase[old_id];
        routine.name = format!(
            "{display_module}::{}",
            runtime_routine_name(&routine.name, link_module)
        );
        for slot in routine
            .frame
            .params
            .iter_mut()
            .chain(&mut routine.frame.locals)
        {
            rebase_storage_base(&mut slot.base, &global_rebase, &static_rebase)?;
            if let Some(init) = &mut slot.init {
                rebase_storage_init(init, &routine_rebase, &global_rebase)?;
            }
        }
        rebase_effects(&mut routine.effects, &global_rebase, &static_rebase)?;
        for block in &mut routine.blocks {
            for op in &mut block.ops {
                if let MirOp::MachineBlock { id, .. } = op {
                    *id = *machine_rebase.get(id).ok_or_else(|| {
                        diagnostic(format!(
                            "runtime routine `{}` references unselected machine block m{}",
                            routine.name, id.0
                        ))
                    })?;
                }
                rebase_op(op, &routine_rebase, &global_rebase, &static_rebase)?;
            }
            rebase_terminator(
                &mut block.terminator,
                &routine_rebase,
                &global_rebase,
                &static_rebase,
            )?;
        }
        program.routines.push(routine);
    }

    for old_id in &selected_machine_ids {
        let mut machine = runtime
            .machine_blocks
            .iter()
            .find(|machine| machine.id == *old_id)
            .expect("selected runtime machine block exists")
            .clone();
        machine.id = machine_rebase[old_id];
        for item in &mut machine.items {
            if let MirMachineItem::Relocation { target, .. } = item {
                rebase_inline_target(target, &routine_rebase, &global_rebase, &static_rebase)?;
            }
        }
        program.machine_blocks.push(machine);
    }

    let global_offset_base = next_global_offset(program);
    for old_id in &selected.globals {
        let mut global = runtime
            .globals
            .iter()
            .find(|global| global.id == *old_id)
            .expect("selected runtime global exists")
            .clone();
        global.id = global_rebase[old_id];
        match &mut global.backing {
            MirGlobalBacking::Ordinary { offset } => {
                *offset = offset.checked_add(global_offset_base).ok_or_else(|| {
                    diagnostic("standalone runtime global storage exceeds 64 KiB")
                })?;
            }
            MirGlobalBacking::Alias { target, .. } => {
                *target = rebased_global(*target, &global_rebase)?;
            }
            MirGlobalBacking::Absolute(_) => {}
        }
        if let Some(init) = &mut global.init {
            rebase_global_init(init, &routine_rebase, &global_rebase)?;
        }
        program.globals.push(global);
    }
    for old_id in &selected.statics {
        let mut static_data = runtime
            .statics
            .iter()
            .find(|static_data| static_data.id == *old_id)
            .expect("selected runtime static exists")
            .clone();
        static_data.id = static_rebase[old_id];
        rebase_data_image(&mut static_data.image, &routine_rebase, &global_rebase)?;
        program.statics.push(static_data);
    }

    Ok(routine_rebase)
}

fn selected_machine_ids(
    runtime: &MirProgram,
    selected: &BTreeSet<RoutineId>,
) -> BTreeSet<MirMachineBlockId> {
    runtime
        .routines
        .iter()
        .filter(|routine| selected.contains(&routine.id))
        .flat_map(|routine| routine.blocks.iter())
        .flat_map(|block| block.ops.iter())
        .filter_map(|op| match op {
            MirOp::MachineBlock { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

fn visit_op_storage(
    op: &MirOp,
    globals: &mut BTreeSet<SymbolId>,
    statics: &mut BTreeSet<SymbolId>,
) {
    match op {
        MirOp::Load { src, .. } => visit_addr_storage(src, globals, statics),
        MirOp::Store { dst, src, .. } => {
            visit_addr_storage(dst, globals, statics);
            visit_value_storage(src, globals, statics);
        }
        MirOp::PackedRealCopy {
            source,
            destination,
            ..
        } => {
            visit_addr_storage(source, globals, statics);
            visit_addr_storage(destination, globals, statics);
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => visit_value_storage(src, globals, statics),
        MirOp::LeaAddr { target, .. }
        | MirOp::UpdateMem { mem: target, .. }
        | MirOp::UpdateIndexedMem { base: target, .. } => {
            record_mem_storage(target, globals, statics)
        }
        MirOp::BinaryDirectIndexedByte {
            source: MirByteIndexedSource::Absolute { base, .. },
            ..
        } => record_mem_storage(base, globals, statics),
        MirOp::BinaryDirectIndexedByte {
            source: MirByteIndexedSource::FixedIndirectY { .. },
            ..
        } => {}
        MirOp::AddByteToWordMem { mem: target, value }
        | MirOp::SubByteFromWordMem { mem: target, value } => {
            record_mem_storage(target, globals, statics);
            visit_value_storage(value, globals, statics);
        }
        MirOp::OffsetPointerByIndirectByte { dst, .. }
        | MirOp::CopyDirectWordToIndirect { source: dst, .. } => {
            record_mem_storage(dst, globals, statics)
        }
        MirOp::AbsoluteWordSubToIndirect { source, rhs, .. } => {
            record_mem_storage(source, globals, statics);
            record_mem_storage(rhs, globals, statics);
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            visit_value_storage(left, globals, statics);
            visit_value_storage(right, globals, statics);
        }
        MirOp::CompareDirectIndexedBytes { left, right, .. } => {
            record_mem_storage(left, globals, statics);
            record_mem_storage(right, globals, statics);
        }
        MirOp::Call {
            target,
            args,
            effects,
            ..
        } => {
            if let MirCallTarget::Indirect { target, .. } = target {
                visit_value_storage(target, globals, statics);
            }
            for arg in args {
                visit_value_storage(&arg.value, globals, statics);
            }
            visit_effect_storage(effects, globals, statics);
        }
        MirOp::RuntimeHelper { effects, .. }
        | MirOp::Barrier { effects }
        | MirOp::MachineBlock { effects, .. } => visit_effect_storage(effects, globals, statics),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            visit_value_storage(base, globals, statics);
            visit_value_storage(index, globals, statics);
        }
        MirOp::LoadImm { .. }
        | MirOp::UpdateReg { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. } => {}
    }
}

fn visit_addr_storage(
    addr: &MirAddr,
    globals: &mut BTreeSet<SymbolId>,
    statics: &mut BTreeSet<SymbolId>,
) {
    match addr {
        MirAddr::Direct(target)
        | MirAddr::AbsoluteIndexedX { base: target }
        | MirAddr::AbsoluteIndexedY { base: target }
        | MirAddr::PointerCell { ptr: target, .. } => record_mem_storage(target, globals, statics),
        MirAddr::ComputedIndex { base, index, .. } => {
            visit_value_storage(base, globals, statics);
            visit_value_storage(index, globals, statics);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            record_mem_storage(ptr, globals, statics);
            visit_value_storage(index, globals, statics);
        }
        MirAddr::Deref { ptr, .. } => visit_value_storage(ptr, globals, statics),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
    }
}

fn visit_value_storage(
    value: &MirValue,
    globals: &mut BTreeSet<SymbolId>,
    statics: &mut BTreeSet<SymbolId>,
) {
    match value {
        MirValue::StaticAddr(id) => {
            statics.insert(*id);
        }
        MirValue::GlobalAddr(id) => {
            globals.insert(*id);
        }
        MirValue::Word { lo, hi } => {
            visit_value_storage(lo, globals, statics);
            visit_value_storage(hi, globals, statics);
        }
        MirValue::StorageAddrByte { mem: target, .. } | MirValue::PointerCell(target) => {
            record_mem_storage(target, globals, statics)
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => {}
    }
}

fn visit_terminator_storage(
    terminator: &MirTerminator,
    globals: &mut BTreeSet<SymbolId>,
    statics: &mut BTreeSet<SymbolId>,
) {
    let edges = match terminator {
        MirTerminator::Jump(edge) => std::slice::from_ref(edge),
        MirTerminator::Branch {
            cond,
            then_edge,
            else_edge,
        } => {
            if let MirCond::BoolValue(value) = cond {
                visit_value_storage(value, globals, statics);
            }
            for arg in then_edge.args.iter().chain(&else_edge.args) {
                visit_value_storage(&arg.value, globals, statics);
            }
            return;
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => return,
    };
    for edge in edges {
        for arg in &edge.args {
            visit_value_storage(&arg.value, globals, statics);
        }
    }
}

fn record_mem_storage(
    mem: &MirMem,
    globals: &mut BTreeSet<SymbolId>,
    statics: &mut BTreeSet<SymbolId>,
) {
    match mem {
        MirMem::Global { id, .. } => {
            globals.insert(*id);
        }
        MirMem::Static { id, .. } => {
            statics.insert(*id);
        }
        _ => {}
    }
}

fn visit_effect_storage(
    effects: &MirEffects,
    globals: &mut BTreeSet<SymbolId>,
    statics: &mut BTreeSet<SymbolId>,
) {
    for effect in [&effects.memory_reads, &effects.memory_writes] {
        let MirMemoryEffect::Regions(regions) = effect else {
            continue;
        };
        for region in regions {
            match region.kind {
                MirMemoryRegionKind::Global(id) => {
                    globals.insert(id);
                }
                MirMemoryRegionKind::Static(id) => {
                    statics.insert(id);
                }
                _ => {}
            }
        }
    }
}

fn visit_storage_init_storage(init: &MirStorageInit, globals: &mut BTreeSet<SymbolId>) {
    match init {
        MirStorageInit::Bytes { image, .. }
        | MirStorageInit::Descriptor {
            backing: super::ir::MirStorageBacking { image, .. },
            ..
        } => visit_data_image_storage(image, globals),
        MirStorageInit::RoutineAddress { .. } | MirStorageInit::ZeroFill { .. } => {}
    }
}

fn visit_global_init_storage(init: &MirGlobalInit, globals: &mut BTreeSet<SymbolId>) {
    match init {
        MirGlobalInit::Bytes { image, .. } => visit_data_image_storage(image, globals),
        MirGlobalInit::Descriptor { backing, .. } => {
            globals.insert(backing.owner);
            visit_data_image_storage(&backing.image, globals);
        }
        MirGlobalInit::ZeroFill { .. }
        | MirGlobalInit::ProgramEndWord { .. }
        | MirGlobalInit::RoutineAddress { .. } => {}
    }
}

fn visit_data_image_storage(image: &MirDataImage, globals: &mut BTreeSet<SymbolId>) {
    for relocation in &image.relocations {
        if let MirDataRelocationTarget::Global(id) = relocation.target {
            globals.insert(id);
        }
    }
}

fn rebase_op(
    op: &mut MirOp,
    routines: &BTreeMap<RoutineId, RoutineId>,
    globals: &BTreeMap<SymbolId, SymbolId>,
    statics: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    match op {
        MirOp::Load { src, .. } => rebase_addr(src, routines, globals, statics)?,
        MirOp::Store { dst, src, .. } => {
            rebase_addr(dst, routines, globals, statics)?;
            rebase_value(src, routines, globals, statics)?;
        }
        MirOp::PackedRealCopy {
            source,
            destination,
            ..
        } => {
            rebase_addr(source, routines, globals, statics)?;
            rebase_addr(destination, routines, globals, statics)?;
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => rebase_value(src, routines, globals, statics)?,
        MirOp::LeaAddr { target, .. }
        | MirOp::UpdateMem { mem: target, .. }
        | MirOp::UpdateIndexedMem { base: target, .. } => rebase_mem(target, globals, statics)?,
        MirOp::BinaryDirectIndexedByte {
            source: MirByteIndexedSource::Absolute { base, .. },
            ..
        } => rebase_mem(base, globals, statics)?,
        MirOp::BinaryDirectIndexedByte {
            source: MirByteIndexedSource::FixedIndirectY { .. },
            ..
        } => {}
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            rebase_mem(mem, globals, statics)?;
            rebase_value(value, routines, globals, statics)?;
        }
        MirOp::OffsetPointerByIndirectByte { dst, .. }
        | MirOp::CopyDirectWordToIndirect { source: dst, .. } => rebase_mem(dst, globals, statics)?,
        MirOp::AbsoluteWordSubToIndirect { source, rhs, .. } => {
            rebase_mem(source, globals, statics)?;
            rebase_mem(rhs, globals, statics)?;
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            rebase_value(left, routines, globals, statics)?;
            rebase_value(right, routines, globals, statics)?;
        }
        MirOp::CompareDirectIndexedBytes { left, right, .. } => {
            rebase_mem(left, globals, statics)?;
            rebase_mem(right, globals, statics)?;
        }
        MirOp::Call {
            target,
            args,
            effects,
            ..
        } => {
            match target {
                MirCallTarget::Routine(id) => *id = rebased_routine(*id, routines)?,
                MirCallTarget::Indirect { target, .. } => {
                    rebase_value(target, routines, globals, statics)?
                }
                MirCallTarget::Builtin { .. }
                | MirCallTarget::Runtime { .. }
                | MirCallTarget::AtariFpp(_) => {}
            }
            for arg in args {
                rebase_value(&mut arg.value, routines, globals, statics)?;
            }
            rebase_effects(effects, globals, statics)?;
        }
        MirOp::RuntimeHelper { effects, .. }
        | MirOp::Barrier { effects }
        | MirOp::MachineBlock { effects, .. } => rebase_effects(effects, globals, statics)?,
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            rebase_value(base, routines, globals, statics)?;
            rebase_value(index, routines, globals, statics)?;
        }
        MirOp::LoadImm { .. }
        | MirOp::UpdateReg { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. } => {}
    }
    Ok(())
}

fn rebase_addr(
    addr: &mut MirAddr,
    routines: &BTreeMap<RoutineId, RoutineId>,
    globals: &BTreeMap<SymbolId, SymbolId>,
    statics: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    match addr {
        MirAddr::Direct(mem)
        | MirAddr::AbsoluteIndexedX { base: mem }
        | MirAddr::AbsoluteIndexedY { base: mem }
        | MirAddr::PointerCell { ptr: mem, .. } => rebase_mem(mem, globals, statics)?,
        MirAddr::ComputedIndex { base, index, .. } => {
            rebase_value(base, routines, globals, statics)?;
            rebase_value(index, routines, globals, statics)?;
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            rebase_mem(ptr, globals, statics)?;
            rebase_value(index, routines, globals, statics)?;
        }
        MirAddr::Deref { ptr, .. } => rebase_value(ptr, routines, globals, statics)?,
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
    }
    Ok(())
}

fn rebase_value(
    value: &mut MirValue,
    routines: &BTreeMap<RoutineId, RoutineId>,
    globals: &BTreeMap<SymbolId, SymbolId>,
    statics: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    match value {
        MirValue::StaticAddr(id) => *id = rebased_static(*id, statics)?,
        MirValue::GlobalAddr(id) => *id = rebased_global(*id, globals)?,
        MirValue::RoutineAddr(id) | MirValue::RoutineAddrByte { id, .. } => {
            *id = rebased_routine(*id, routines)?
        }
        MirValue::Word { lo, hi } => {
            rebase_value(lo, routines, globals, statics)?;
            rebase_value(hi, routines, globals, statics)?;
        }
        MirValue::StorageAddrByte { mem, .. } | MirValue::PointerCell(mem) => {
            rebase_mem(mem, globals, statics)?
        }
        MirValue::ConstU8(_) | MirValue::ConstU16(_) | MirValue::Def(_) => {}
    }
    Ok(())
}

fn rebase_mem(
    mem: &mut MirMem,
    globals: &BTreeMap<SymbolId, SymbolId>,
    statics: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    match mem {
        MirMem::Global { id, .. } => *id = rebased_global(*id, globals)?,
        MirMem::Static { id, .. } => *id = rebased_static(*id, statics)?,
        MirMem::Absolute(_)
        | MirMem::Local { .. }
        | MirMem::Param { .. }
        | MirMem::Spill { .. }
        | MirMem::ZeroPage(_)
        | MirMem::FixedZeroPage(_) => {}
    }
    Ok(())
}

fn rebase_terminator(
    terminator: &mut MirTerminator,
    routines: &BTreeMap<RoutineId, RoutineId>,
    globals: &BTreeMap<SymbolId, SymbolId>,
    statics: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    let rebase_edge = |edge: &mut super::ir::MirEdge| -> Result<(), Vec<MirDiagnostic>> {
        for arg in &mut edge.args {
            rebase_value(&mut arg.value, routines, globals, statics)?;
        }
        Ok(())
    };
    match terminator {
        MirTerminator::Jump(edge) => rebase_edge(edge)?,
        MirTerminator::Branch {
            cond,
            then_edge,
            else_edge,
        } => {
            if let MirCond::BoolValue(value) = cond {
                rebase_value(value, routines, globals, statics)?;
            }
            rebase_edge(then_edge)?;
            rebase_edge(else_edge)?;
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
    }
    Ok(())
}

pub(super) fn append_runtime_helper_requirements(
    program: &mut MirProgram,
    runtime: &MirProgram,
    selected: &BTreeSet<RoutineId>,
) -> Result<(), Vec<MirDiagnostic>> {
    let required = runtime
        .routines
        .iter()
        .filter(|routine| selected.contains(&routine.id))
        .flat_map(|routine| &routine.blocks)
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            MirOp::RuntimeHelper { helper, .. } => Some(*helper),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for helper in required {
        let declarations = runtime
            .runtime_helpers
            .iter()
            .filter(|declaration| declaration.helper == helper)
            .collect::<Vec<_>>();
        let [implementation] = declarations.as_slice() else {
            return Err(diagnostic(format!(
                "embedded runtime has {} declarations for helper `{}`",
                declarations.len(),
                super::runtime::helper_name(helper)
            )));
        };
        if let Some(existing) = program
            .runtime_helpers
            .iter()
            .find(|declaration| declaration.helper == helper)
        {
            if existing.abi != implementation.abi || existing.effects != implementation.effects {
                return Err(diagnostic(format!(
                    "runtime helper contract mismatch for `{}`",
                    super::runtime::helper_name(helper)
                )));
            }
        } else {
            let mut declaration = (*implementation).clone();
            declaration.target = MirRuntimeHelperTarget::Deferred;
            program.runtime_helpers.push(declaration);
        }
    }
    Ok(())
}

fn rebase_effects(
    effects: &mut MirEffects,
    globals: &BTreeMap<SymbolId, SymbolId>,
    statics: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    for effect in [&mut effects.memory_reads, &mut effects.memory_writes] {
        let MirMemoryEffect::Regions(regions) = effect else {
            continue;
        };
        for region in regions {
            match &mut region.kind {
                MirMemoryRegionKind::Global(id) => *id = rebased_global(*id, globals)?,
                MirMemoryRegionKind::Static(id) => *id = rebased_static(*id, statics)?,
                _ => {}
            }
        }
    }
    Ok(())
}

fn rebase_storage_base(
    base: &mut MirStorageBase,
    globals: &BTreeMap<SymbolId, SymbolId>,
    statics: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    match base {
        MirStorageBase::Global(id) => *id = rebased_global(*id, globals)?,
        MirStorageBase::Static(id) => *id = rebased_static(*id, statics)?,
        _ => {}
    }
    Ok(())
}

fn rebase_storage_init(
    init: &mut MirStorageInit,
    routines: &BTreeMap<RoutineId, RoutineId>,
    globals: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    match init {
        MirStorageInit::Bytes { image, .. }
        | MirStorageInit::Descriptor {
            backing: super::ir::MirStorageBacking { image, .. },
            ..
        } => rebase_data_image(image, routines, globals)?,
        MirStorageInit::RoutineAddress { routine, .. } => {
            *routine = rebased_routine(*routine, routines)?
        }
        MirStorageInit::ZeroFill { .. } => {}
    }
    Ok(())
}

fn rebase_global_init(
    init: &mut MirGlobalInit,
    routines: &BTreeMap<RoutineId, RoutineId>,
    globals: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    match init {
        MirGlobalInit::Bytes { image, .. } => rebase_data_image(image, routines, globals)?,
        MirGlobalInit::Descriptor { backing, .. } => {
            backing.owner = rebased_global(backing.owner, globals)?;
            rebase_data_image(&mut backing.image, routines, globals)?;
        }
        MirGlobalInit::RoutineAddress { routine, .. } => {
            *routine = rebased_routine(*routine, routines)?
        }
        MirGlobalInit::ZeroFill { .. } | MirGlobalInit::ProgramEndWord { .. } => {}
    }
    Ok(())
}

fn rebase_data_image(
    image: &mut MirDataImage,
    routines: &BTreeMap<RoutineId, RoutineId>,
    globals: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    for relocation in &mut image.relocations {
        match &mut relocation.target {
            MirDataRelocationTarget::Global(id) => *id = rebased_global(*id, globals)?,
            MirDataRelocationTarget::Routine(id) => *id = rebased_routine(*id, routines)?,
            MirDataRelocationTarget::Local { routine, .. }
            | MirDataRelocationTarget::Param { routine, .. } => {
                *routine = rebased_routine(*routine, routines)?
            }
            MirDataRelocationTarget::Absolute(_) => {}
        }
    }
    Ok(())
}

fn rebase_inline_target(
    target: &mut MirInlineAsmTarget,
    routines: &BTreeMap<RoutineId, RoutineId>,
    globals: &BTreeMap<SymbolId, SymbolId>,
    statics: &BTreeMap<SymbolId, SymbolId>,
) -> Result<(), Vec<MirDiagnostic>> {
    match target {
        MirInlineAsmTarget::Memory(mem) => rebase_mem(mem, globals, statics)?,
        MirInlineAsmTarget::Routine(id) => *id = rebased_routine(*id, routines)?,
        MirInlineAsmTarget::Absolute(_) | MirInlineAsmTarget::InlineOffset(_) => {}
    }
    Ok(())
}

fn validate_helper_contract(
    application: &MirProgram,
    runtime: &MirProgram,
    helper: MirRuntimeHelper,
) -> Result<(), Vec<MirDiagnostic>> {
    let application = application
        .runtime_helpers
        .iter()
        .find(|declaration| declaration.helper == helper)
        .ok_or_else(|| {
            diagnostic(format!(
                "application has no logical declaration for runtime helper `{}`",
                super::runtime::helper_name(helper)
            ))
        })?;
    let implementation = runtime
        .runtime_helpers
        .iter()
        .find(|declaration| declaration.helper == helper)
        .ok_or_else(|| {
            diagnostic(format!(
                "embedded SYSLIB has no contract for runtime helper `{}`",
                super::runtime::helper_name(helper)
            ))
        })?;
    if application.abi != implementation.abi {
        return Err(diagnostic(format!(
            "ABI mismatch for standalone runtime helper `{}`",
            super::runtime::helper_name(helper)
        )));
    }
    if application.effects != implementation.effects {
        return Err(diagnostic(format!(
            "effect mismatch for standalone runtime helper `{}`",
            super::runtime::helper_name(helper)
        )));
    }
    Ok(())
}

fn find_runtime_helper(
    program: &MirProgram,
    helper: MirRuntimeHelper,
) -> Result<RoutineId, Vec<MirDiagnostic>> {
    let matches = program
        .runtime_helpers
        .iter()
        .filter(|declaration| declaration.helper == helper)
        .filter_map(|declaration| match declaration.target {
            MirRuntimeHelperTarget::Routine(id) => Some(id),
            _ => None,
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(diagnostic(format!(
            "embedded SYSLIB has no implementation for `{}`",
            super::runtime::helper_name(helper)
        ))),
        _ => Err(diagnostic(format!(
            "embedded SYSLIB has more than one implementation for `{}`",
            super::runtime::helper_name(helper)
        ))),
    }
}

pub(super) fn runtime_routine_name<'a>(name: &'a str, link_module: &str) -> &'a str {
    let prefix = format!("M_{link_module}_");
    let key = name
        .strip_prefix(&prefix)
        .and_then(|name| name.rsplit_once('_').map(|(name, _)| name))
        .unwrap_or(name);
    match key {
        "ERROR" => "Error",
        "BREAK" => "Break",
        "LSHIFT" => "LShift",
        "RSHIFT" => "RShift",
        "SETSIGN" => "SetSign",
        "SS1" => "SS1",
        "SMOPS" => "SMOps",
        "MULTB" => "MultB",
        "MULTI" => "MultI",
        "DIVI" => "DivI",
        "REMI" => "RemI",
        "SARGS" => "SArgs",
        "ZERO" => "Zero",
        "SETBLOCK" => "SetBlock",
        "MOVEBLOCK" => "MoveBlock",
        _ => key,
    }
}

#[cfg(test)]
pub(super) fn dependency_closure(
    program: &MirProgram,
    roots: BTreeSet<RoutineId>,
) -> Result<BTreeSet<RoutineId>, Vec<MirDiagnostic>> {
    let selection = dynamic_runtime_selection(program, roots)?;
    Ok(selection.routines)
}

#[cfg(test)]
fn dynamic_runtime_selection(
    program: &MirProgram,
    roots: BTreeSet<RoutineId>,
) -> Result<RuntimeSelection, Vec<MirDiagnostic>> {
    note_runtime_graph_discovery();
    let graph = discover_runtime_dependency_graph(program)?;
    let selection = graph
        .closure(roots.into_iter().map(MirLinkNode::Routine))
        .map_err(|node| diagnostic(format!("embedded runtime dependency {node:?} is missing")))?;
    Ok(runtime_selection_from_nodes(&selection.retained))
}

fn discover_runtime_dependency_graph(
    program: &MirProgram,
) -> Result<LinkGraph<MirLinkNode>, Vec<MirDiagnostic>> {
    let machine_blocks = program
        .machine_blocks
        .iter()
        .map(|machine| (machine.id, machine))
        .collect::<BTreeMap<_, _>>();
    let mut graph = LinkGraph::default();
    for routine in &program.routines {
        graph.add_node(MirLinkNode::Routine(routine.id));
    }
    for global in &program.globals {
        graph.add_node(MirLinkNode::Global(global.id));
    }
    for static_data in &program.statics {
        graph.add_node(MirLinkNode::Static(static_data.id));
    }

    for routine in &program.routines {
        let source = MirLinkNode::Routine(routine.id);
        // Adjacent declarations with no body are entry aliases for the next
        // resident routine. Their empty, unreachable MIR block is as much a
        // fallthrough edge as a machine block whose final instruction is not
        // RTS/JMP. This is used by families such as InputB/C/I and ValB/C/I.
        let entry_alias = routine.blocks.len() == 1
            && routine.blocks[0].ops.is_empty()
            && matches!(routine.blocks[0].terminator, MirTerminator::Unreachable);
        let mut machine_fallthrough_reason = None;
        let mut globals = BTreeSet::new();
        let mut statics = BTreeSet::new();
        let mut routine_addresses = BTreeSet::new();

        for slot in routine.frame.params.iter().chain(&routine.frame.locals) {
            match slot.base {
                MirStorageBase::Global(id) => {
                    globals.insert(id);
                }
                MirStorageBase::Static(id) => {
                    statics.insert(id);
                }
                _ => {}
            }
            if let Some(init) = &slot.init {
                visit_storage_init_storage(init, &mut globals);
                visit_storage_init_routines(init, &mut routine_addresses);
            }
        }
        visit_effect_storage(&routine.effects, &mut globals, &mut statics);
        for block in &routine.blocks {
            for (op_index, op) in block.ops.iter().enumerate() {
                visit_op_storage(op, &mut globals, &mut statics);
                visit_op_routines(op, &mut routine_addresses);
                match op {
                    MirOp::Call {
                        target: MirCallTarget::Routine(target),
                        ..
                    } => {
                        add_checked_runtime_edge(
                            &mut graph,
                            source,
                            MirLinkNode::Routine(*target),
                            LinkReason::DirectCall,
                        )?;
                    }
                    MirOp::MachineBlock { id, .. } => {
                        let machine = machine_blocks.get(id).ok_or_else(|| {
                            diagnostic(format!(
                                "runtime routine `{}` refers to missing machine block m{}",
                                routine.name, id.0
                            ))
                        })?;
                        for item in &machine.items {
                            if let MirMachineItem::Relocation {
                                target: MirInlineAsmTarget::Routine(target),
                                ..
                            } = item
                            {
                                add_checked_runtime_edge(
                                    &mut graph,
                                    source,
                                    MirLinkNode::Routine(*target),
                                    LinkReason::MachineRelocation,
                                )?;
                            }
                            if let MirMachineItem::Relocation {
                                target: MirInlineAsmTarget::Memory(mem),
                                ..
                            } = item
                            {
                                record_mem_storage(mem, &mut globals, &mut statics);
                            }
                        }
                        if routine.blocks.len() == 1 {
                            let required_prefix = super::analysis::known_callees::machine_block_backward_prefix_bytes(
                                machine, routine, program,
                            )
                            .unwrap_or(0);
                            if required_prefix > 0 {
                                for preceding in preceding_runtime_bytes(
                                    program,
                                    &machine_blocks,
                                    routine.id,
                                    required_prefix,
                                ) {
                                    add_checked_runtime_edge(
                                        &mut graph,
                                        source,
                                        MirLinkNode::Routine(preceding),
                                        LinkReason::RequiredPrefix,
                                    )?;
                                }
                            }
                        }
                        if routine.blocks.len() == 1
                            && op_index + 1 == block.ops.len()
                            && matches!(block.terminator, MirTerminator::Unreachable)
                        {
                            use super::analysis::known_callees::MachineBlockFallthrough;
                            machine_fallthrough_reason =
                                match super::analysis::known_callees::machine_block_fallthrough(
                                    machine,
                                ) {
                                    MachineBlockFallthrough::Stops => None,
                                    MachineBlockFallthrough::FallsThrough => {
                                        Some(LinkReason::MachineFallthrough)
                                    }
                                    MachineBlockFallthrough::Conservative => {
                                        Some(LinkReason::ConservativeLayout)
                                    }
                                };
                        }
                    }
                    _ => {}
                }
            }
            visit_terminator_storage(&block.terminator, &mut globals, &mut statics);
            visit_terminator_routines(&block.terminator, &mut routine_addresses);
        }

        for target in routine_addresses {
            add_checked_runtime_edge(
                &mut graph,
                source,
                MirLinkNode::Routine(target),
                LinkReason::RoutineAddress,
            )?;
        }
        for global in globals {
            add_checked_runtime_edge(
                &mut graph,
                source,
                MirLinkNode::Global(global),
                LinkReason::StorageReference,
            )?;
        }
        for static_data in statics {
            add_checked_runtime_edge(
                &mut graph,
                source,
                MirLinkNode::Static(static_data),
                LinkReason::StorageReference,
            )?;
        }
        if entry_alias || machine_fallthrough_reason.is_some() {
            if let Some(index) = program
                .routines
                .iter()
                .position(|candidate| candidate.id == routine.id)
            {
                if let Some(next) = program.routines.get(index + 1) {
                    if entry_alias {
                        add_checked_runtime_edge(
                            &mut graph,
                            source,
                            MirLinkNode::Routine(next.id),
                            LinkReason::EntryAlias,
                        )?;
                    }
                    if let Some(reason) = machine_fallthrough_reason {
                        add_checked_runtime_edge(
                            &mut graph,
                            source,
                            MirLinkNode::Routine(next.id),
                            reason,
                        )?;
                    }
                }
            }
        }
    }

    for global in &program.globals {
        let source = MirLinkNode::Global(global.id);
        let mut globals = BTreeSet::new();
        let mut routines = BTreeSet::new();
        if let MirGlobalBacking::Alias { target, .. } = global.backing {
            add_checked_runtime_edge(
                &mut graph,
                source,
                MirLinkNode::Global(target),
                LinkReason::AliasBacking,
            )?;
        }
        if let Some(init) = &global.init {
            visit_global_init_storage(init, &mut globals);
            visit_global_init_routines(init, &mut routines);
        }
        for target in globals {
            add_checked_runtime_edge(
                &mut graph,
                source,
                MirLinkNode::Global(target),
                LinkReason::InitializerRelocation,
            )?;
        }
        for target in routines {
            add_checked_runtime_edge(
                &mut graph,
                source,
                MirLinkNode::Routine(target),
                LinkReason::InitializerRelocation,
            )?;
        }
    }
    for static_data in &program.statics {
        let source = MirLinkNode::Static(static_data.id);
        let mut globals = BTreeSet::new();
        let mut routines = BTreeSet::new();
        visit_data_image_storage(&static_data.image, &mut globals);
        visit_data_image_routines(&static_data.image, &mut routines);
        for target in globals {
            add_checked_runtime_edge(
                &mut graph,
                source,
                MirLinkNode::Global(target),
                LinkReason::InitializerRelocation,
            )?;
        }
        for target in routines {
            add_checked_runtime_edge(
                &mut graph,
                source,
                MirLinkNode::Routine(target),
                LinkReason::InitializerRelocation,
            )?;
        }
    }
    Ok(graph)
}

fn add_checked_runtime_edge(
    graph: &mut LinkGraph<MirLinkNode>,
    source: MirLinkNode,
    target: MirLinkNode,
    reason: LinkReason,
) -> Result<(), Vec<MirDiagnostic>> {
    if !graph.nodes().contains(&target) {
        return Err(diagnostic(format!(
            "embedded runtime dependency {target:?} is missing"
        )));
    }
    graph.add_edge(source, target, reason);
    Ok(())
}

fn preceding_runtime_bytes(
    program: &MirProgram,
    machine_blocks: &BTreeMap<MirMachineBlockId, &MirMachineBlock>,
    routine_id: RoutineId,
    required: usize,
) -> BTreeSet<RoutineId> {
    let mut selected = BTreeSet::new();
    let Some(index) = program
        .routines
        .iter()
        .position(|routine| routine.id == routine_id)
    else {
        return selected;
    };
    let mut retained = 0usize;
    for routine in program.routines[..index].iter().rev() {
        selected.insert(routine.id);
        for block in &routine.blocks {
            for op in &block.ops {
                let MirOp::MachineBlock { id, .. } = op else {
                    continue;
                };
                let Some(machine) = machine_blocks.get(id) else {
                    continue;
                };
                retained = retained.saturating_add(
                    super::analysis::known_callees::machine_block_byte_len(
                        machine, routine, program,
                    )
                    .unwrap_or(0),
                );
            }
        }
        if retained >= required {
            break;
        }
    }
    selected
}

#[cfg(test)]
fn runtime_selection_from_nodes(nodes: &BTreeSet<MirLinkNode>) -> RuntimeSelection {
    let mut selection = RuntimeSelection {
        routines: BTreeSet::new(),
        globals: BTreeSet::new(),
        statics: BTreeSet::new(),
    };
    for node in nodes {
        match node {
            MirLinkNode::Routine(id) => {
                selection.routines.insert(*id);
            }
            MirLinkNode::Global(id) => {
                selection.globals.insert(*id);
            }
            MirLinkNode::Static(id) => {
                selection.statics.insert(*id);
            }
        }
    }
    selection
}

#[cfg(test)]
pub(super) fn embedded_resident_selection(
    program: &MirProgram,
    roots: BTreeSet<RoutineId>,
) -> Result<RuntimeSelection, Vec<MirDiagnostic>> {
    let manifest = embedded_sys_link_manifest()
        .map_err(|message| diagnostic(format!("embedded SYS link manifest: {message}")))?;
    let (stable_to_mir, mir_to_stable) = resident_node_bindings(program)?;
    let stable_roots = roots
        .into_iter()
        .map(|id| {
            mir_to_stable
                .get(&MirLinkNode::Routine(id))
                .cloned()
                .ok_or_else(|| {
                    diagnostic(format!(
                        "embedded SYS link manifest has no binding for routine r{}",
                        id.0
                    ))
                })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let selected = manifest.graph.closure(stable_roots).map_err(|node| {
        diagnostic(format!(
            "embedded SYS link manifest dependency {node:?} is missing"
        ))
    })?;
    let selected = selected
        .retained
        .iter()
        .map(|node| {
            stable_to_mir.get(node).copied().ok_or_else(|| {
                diagnostic(format!(
                    "embedded SYS link manifest node {node:?} does not bind to the runtime image"
                ))
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(runtime_selection_from_nodes(&selected))
}

#[cfg(test)]
fn resident_node_bindings(
    program: &MirProgram,
) -> Result<
    (
        BTreeMap<RuntimeLinkNode, MirLinkNode>,
        BTreeMap<MirLinkNode, RuntimeLinkNode>,
    ),
    Vec<MirDiagnostic>,
> {
    let (stable_to_mir, mir_to_stable) = resident_node_bindings_without_manifest(program)?;

    let manifest = embedded_sys_link_manifest()
        .map_err(|message| diagnostic(format!("embedded SYS link manifest: {message}")))?;
    let actual = stable_to_mir.keys().cloned().collect::<BTreeSet<_>>();
    if actual != *manifest.graph.nodes() {
        let missing = manifest
            .graph
            .nodes()
            .difference(&actual)
            .map(|node| format!("{node:?}"))
            .collect::<Vec<_>>();
        let unexpected = actual
            .difference(manifest.graph.nodes())
            .map(|node| format!("{node:?}"))
            .collect::<Vec<_>>();
        return Err(diagnostic(format!(
            "embedded SYS link manifest node mismatch; missing [{}], unexpected [{}]",
            missing.join(", "),
            unexpected.join(", ")
        )));
    }
    Ok((stable_to_mir, mir_to_stable))
}

fn runtime_storage_name<'a>(name: &'a str, link_module: &str) -> &'a str {
    let prefix = format!("M_{link_module}_");
    name.strip_prefix(&prefix)
        .and_then(|name| name.rsplit_once('_').map(|(name, _)| name))
        .unwrap_or(name)
}

pub(crate) fn generate_resident_link_manifest() -> Result<String, Vec<MirDiagnostic>> {
    let (_, program) = compile_runtime_image_with_semir()?;
    let graph = discover_runtime_dependency_graph(&program)?;
    let (_, mir_to_stable) = resident_node_bindings_without_manifest(&program)?;
    let mut stable = LinkGraph::default();
    for node in graph.nodes() {
        stable.add_node(mir_to_stable[node].clone());
    }
    for (source, edge) in graph.edges() {
        stable.add_edge(
            mir_to_stable[source].clone(),
            mir_to_stable[&edge.target].clone(),
            edge.reason,
        );
    }
    Ok(RuntimeLinkManifest::new(stable).render())
}

fn resident_node_bindings_without_manifest(
    program: &MirProgram,
) -> Result<
    (
        BTreeMap<RuntimeLinkNode, MirLinkNode>,
        BTreeMap<MirLinkNode, RuntimeLinkNode>,
    ),
    Vec<MirDiagnostic>,
> {
    let mut stable_to_mir = BTreeMap::new();
    let mut mir_to_stable = BTreeMap::new();
    let mut insert = |stable: RuntimeLinkNode, node: MirLinkNode| {
        if let Some(previous) = stable_to_mir.insert(stable.clone(), node) {
            return Err(diagnostic(format!(
                "embedded runtime nodes {previous:?} and {node:?} share manifest identity {stable:?}"
            )));
        }
        mir_to_stable.insert(node, stable);
        Ok(())
    };
    for routine in &program.routines {
        insert(
            RuntimeLinkNode::routine(runtime_routine_name(
                &routine.name,
                "ACTION_RUNTIME_RESIDENT",
            )),
            MirLinkNode::Routine(routine.id),
        )?;
    }
    for global in &program.globals {
        insert(
            RuntimeLinkNode::global(runtime_storage_name(
                &global.name,
                "ACTION_RUNTIME_RESIDENT",
            )),
            MirLinkNode::Global(global.id),
        )?;
    }
    for static_data in &program.statics {
        insert(
            RuntimeLinkNode::static_data(runtime_storage_name(
                &static_data.name,
                "ACTION_RUNTIME_RESIDENT",
            )),
            MirLinkNode::Static(static_data.id),
        )?;
    }
    Ok((stable_to_mir, mir_to_stable))
}

#[cfg(test)]
std::thread_local! {
    static RUNTIME_GRAPH_DISCOVERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_runtime_graph_discovery() {
    RUNTIME_GRAPH_DISCOVERIES.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn runtime_graph_discoveries() -> usize {
    RUNTIME_GRAPH_DISCOVERIES.with(std::cell::Cell::get)
}

fn visit_op_routines(op: &MirOp, routines: &mut BTreeSet<RoutineId>) {
    match op {
        MirOp::Load { src, .. } => visit_addr_routines(src, routines),
        MirOp::Store { dst, src, .. } => {
            visit_addr_routines(dst, routines);
            visit_value_routines(src, routines);
        }
        MirOp::PackedRealCopy {
            source,
            destination,
            ..
        } => {
            visit_addr_routines(source, routines);
            visit_addr_routines(destination, routines);
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => visit_value_routines(src, routines),
        MirOp::AddByteToWordMem { value, .. } | MirOp::SubByteFromWordMem { value, .. } => {
            visit_value_routines(value, routines)
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            visit_value_routines(left, routines);
            visit_value_routines(right, routines);
        }
        MirOp::Call { target, args, .. } => {
            if let MirCallTarget::Indirect { target, .. } = target {
                visit_value_routines(target, routines);
            }
            for argument in args {
                visit_value_routines(&argument.value, routines);
            }
        }
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            visit_value_routines(base, routines);
            visit_value_routines(index, routines);
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::UpdateReg { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::BinaryDirectIndexedByte { .. }
        | MirOp::OffsetPointerByIndirectByte { .. }
        | MirOp::CopyDirectWordToIndirect { .. }
        | MirOp::AbsoluteWordSubToIndirect { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::CompareDirectIndexedBytes { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. } => {}
    }
}

fn visit_addr_routines(addr: &MirAddr, routines: &mut BTreeSet<RoutineId>) {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            visit_value_routines(base, routines);
            visit_value_routines(index, routines);
        }
        MirAddr::PointerIndex { index, .. } => visit_value_routines(index, routines),
        MirAddr::Deref { ptr, .. } => visit_value_routines(ptr, routines),
        MirAddr::Direct(_)
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::PointerCell { .. }
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::Label(_) => {}
    }
}

fn visit_value_routines(value: &MirValue, routines: &mut BTreeSet<RoutineId>) {
    match value {
        MirValue::RoutineAddr(id) | MirValue::RoutineAddrByte { id, .. } => {
            routines.insert(*id);
        }
        MirValue::Word { lo, hi } => {
            visit_value_routines(lo, routines);
            visit_value_routines(hi, routines);
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::StorageAddrByte { .. }
        | MirValue::PointerCell(_) => {}
    }
}

fn visit_terminator_routines(terminator: &MirTerminator, routines: &mut BTreeSet<RoutineId>) {
    let edges = match terminator {
        MirTerminator::Jump(edge) => std::slice::from_ref(edge),
        MirTerminator::Branch {
            cond,
            then_edge,
            else_edge,
        } => {
            if let MirCond::BoolValue(value) = cond {
                visit_value_routines(value, routines);
            }
            for argument in then_edge.args.iter().chain(&else_edge.args) {
                visit_value_routines(&argument.value, routines);
            }
            return;
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => return,
    };
    for edge in edges {
        for argument in &edge.args {
            visit_value_routines(&argument.value, routines);
        }
    }
}

fn visit_storage_init_routines(init: &MirStorageInit, routines: &mut BTreeSet<RoutineId>) {
    match init {
        MirStorageInit::Bytes { image, .. }
        | MirStorageInit::Descriptor {
            backing: super::ir::MirStorageBacking { image, .. },
            ..
        } => visit_data_image_routines(image, routines),
        MirStorageInit::RoutineAddress { routine, .. } => {
            routines.insert(*routine);
        }
        MirStorageInit::ZeroFill { .. } => {}
    }
}

fn visit_global_init_routines(init: &MirGlobalInit, routines: &mut BTreeSet<RoutineId>) {
    match init {
        MirGlobalInit::Bytes { image, .. } => visit_data_image_routines(image, routines),
        MirGlobalInit::Descriptor { backing, .. } => {
            visit_data_image_routines(&backing.image, routines)
        }
        MirGlobalInit::RoutineAddress { routine, .. } => {
            routines.insert(*routine);
        }
        MirGlobalInit::ZeroFill { .. } | MirGlobalInit::ProgramEndWord { .. } => {}
    }
}

fn visit_data_image_routines(image: &MirDataImage, routines: &mut BTreeSet<RoutineId>) {
    for relocation in &image.relocations {
        match relocation.target {
            MirDataRelocationTarget::Routine(id)
            | MirDataRelocationTarget::Local { routine: id, .. }
            | MirDataRelocationTarget::Param { routine: id, .. } => {
                routines.insert(id);
            }
            MirDataRelocationTarget::Global(_) | MirDataRelocationTarget::Absolute(_) => {}
        }
    }
}

fn rebased_routine(
    old: RoutineId,
    rebase: &BTreeMap<RoutineId, RoutineId>,
) -> Result<RoutineId, Vec<MirDiagnostic>> {
    rebase.get(&old).copied().ok_or_else(|| {
        diagnostic(format!(
            "runtime dependency r{} was not included in the selected closure",
            old.0
        ))
    })
}

fn rebased_global(
    old: SymbolId,
    rebase: &BTreeMap<SymbolId, SymbolId>,
) -> Result<SymbolId, Vec<MirDiagnostic>> {
    rebase.get(&old).copied().ok_or_else(|| {
        diagnostic(format!(
            "runtime global dependency g{} was not included in the selected closure",
            old.0
        ))
    })
}

fn rebased_static(
    old: SymbolId,
    rebase: &BTreeMap<SymbolId, SymbolId>,
) -> Result<SymbolId, Vec<MirDiagnostic>> {
    rebase.get(&old).copied().ok_or_else(|| {
        diagnostic(format!(
            "runtime static dependency s{} was not included in the selected closure",
            old.0
        ))
    })
}

fn next_routine_id(program: &MirProgram) -> u32 {
    program
        .routines
        .iter()
        .map(|routine| routine.id.0)
        .max()
        .map_or(0, |id| id.wrapping_add(1))
}

fn next_machine_id(program: &MirProgram) -> u32 {
    program
        .machine_blocks
        .iter()
        .map(|machine| machine.id.0)
        .max()
        .map_or(0, |id| id.wrapping_add(1))
}

fn next_global_id(program: &MirProgram) -> u32 {
    program
        .globals
        .iter()
        .map(|global| global.id.0)
        .max()
        .map_or(0, |id| id.wrapping_add(1))
}

fn next_static_id(program: &MirProgram) -> u32 {
    program
        .statics
        .iter()
        .map(|static_data| static_data.id.0)
        .max()
        .map_or(0, |id| id.wrapping_add(1))
}

fn next_global_offset(program: &MirProgram) -> u16 {
    program
        .globals
        .iter()
        .filter_map(|global| match global.backing {
            MirGlobalBacking::Ordinary { offset } => {
                Some(offset.saturating_add(global.storage_size))
            }
            MirGlobalBacking::Absolute(_) | MirGlobalBacking::Alias { .. } => None,
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
fn compile_syslib() -> Result<MirProgram, Vec<MirDiagnostic>> {
    compile_runtime_unit("syslib.act", "ACTION.RUNTIME.SYSLIB")
}

#[cfg(test)]
pub(super) fn compile_runtime_unit(
    file_name: &str,
    module_name: &str,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    compile_runtime_unit_with_semir(file_name, module_name).map(|(_, mir)| mir)
}

#[cfg(test)]
pub(super) fn compile_runtime_unit_with_semir(
    file_name: &str,
    module_name: &str,
) -> Result<(crate::semantic::ir::SemProgram, MirProgram), Vec<MirDiagnostic>> {
    let semir = crate::runtime_source::compile_runtime_unit(file_name, module_name)
        .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    let mir = lower_runtime_semir(&semir, file_name)?;
    Ok((semir, mir))
}

fn lower_runtime_semir(
    semir: &crate::semantic::ir::SemProgram,
    file_name: &str,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let nir = lower_runtime_nir(semir);
    crate::nir::verify_program(&nir).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| MirDiagnostic {
                routine: diagnostic.routine,
                block: diagnostic.block,
                message: format!("embedded {file_name} NIR: {}", diagnostic.message),
            })
            .collect::<Vec<_>>()
    })?;
    let mir = super::lower_program(&nir).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|mut diagnostic| {
                diagnostic.message = format!("embedded {file_name} MIR: {}", diagnostic.message);
                diagnostic
            })
            .collect::<Vec<_>>()
    })?;
    Ok(mir)
}

pub(super) fn compile_runtime_image_with_semir()
-> Result<(crate::runtime_source::RuntimeImage, MirProgram), Vec<MirDiagnostic>> {
    let image = crate::runtime_source::compile_runtime_image()
        .map_err(|diagnostics| frontend_diagnostics("sysall.act", diagnostics))?;
    let mir = lower_runtime_image(&image)?;
    Ok((image, mir))
}

pub(super) fn lower_runtime_image(
    image: &crate::runtime_source::RuntimeImage,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    lower_runtime_semir(&image.semir, "sysall.act")
}

fn lower_runtime_nir(semir: &crate::semantic::ir::SemProgram) -> crate::nir::NirProgram {
    let mut nir = crate::nir::lower_program(semir);
    for routine in &mut nir.routines {
        routine
            .notes
            .retain(|note| note.kind != crate::nir::NirRoutineNoteKind::ProgramEntry);
    }
    nir
}

fn frontend_diagnostics(
    file_name: &str,
    diagnostics: Vec<crate::diagnostic::Diagnostic>,
) -> Vec<MirDiagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| MirDiagnostic {
            routine: None,
            block: None,
            message: format!("embedded {file_name} frontend: {}", diagnostic.message),
        })
        .collect()
}

fn diagnostic(message: impl Into<String>) -> Vec<MirDiagnostic> {
    vec![MirDiagnostic {
        routine: None,
        block: None,
        message: message.into(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm6502::InlineAsmRelocationKind;
    use crate::mir6502::MirOp;
    use crate::mir6502::ir::{MirInlineAsmTarget, MirMachineItem, MirRuntimeHelperDecl};
    use crate::runtime::Runtime;
    use crate::runtime_bindings::{BindingTarget, parse_bindings};
    use crate::source::Span;

    fn resident_routine_id(program: &MirProgram, expected: &str) -> RoutineId {
        let matches = program
            .routines
            .iter()
            .filter(|routine| {
                runtime_routine_name(&routine.name, "ACTION_RUNTIME_RESIDENT")
                    .eq_ignore_ascii_case(expected)
            })
            .map(|routine| routine.id)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [id] => *id,
            [] => panic!("resident runtime has no routine named {expected}"),
            _ => panic!("resident runtime has multiple routines named {expected}"),
        }
    }

    fn standalone_sys_closure_inventory() -> String {
        let bindings = parse_bindings(Runtime::Standalone).expect("standalone SYS bindings");
        let (image, program) = compile_runtime_image_with_semir().expect("resident runtime image");
        let mut inventory = String::new();

        for (external, target) in bindings {
            let BindingTarget::RuntimeRoutine { unit, routine } = target else {
                panic!("standalone binding {external} must target a runtime routine");
            };
            let expected_unit = crate::runtime_source::resolve_runtime_unit(&unit)
                .expect("standalone binding unit");
            assert_eq!(
                image.routine_units.get(&routine.to_ascii_uppercase()),
                Some(&expected_unit),
                "standalone binding {external} points at the wrong physical runtime unit"
            );

            let root = resident_routine_id(&program, &routine);
            let selected = dependency_closure(&program, BTreeSet::from([root]))
                .unwrap_or_else(|diagnostics| panic!("select {external}: {diagnostics:?}"));
            let names = program
                .routines
                .iter()
                .filter(|candidate| selected.contains(&candidate.id))
                .map(|candidate| {
                    runtime_routine_name(&candidate.name, "ACTION_RUNTIME_RESIDENT")
                        .to_ascii_uppercase()
                })
                .collect::<BTreeSet<_>>();
            assert!(
                names.contains(&routine.to_ascii_uppercase()),
                "{external} closure omitted its root {routine}"
            );

            inventory.push_str(&external);
            inventory.push_str(" = ");
            inventory.push_str(&names.into_iter().collect::<Vec<_>>().join(", "));
            inventory.push('\n');
        }
        inventory
    }

    #[test]
    fn every_standalone_sys_entry_point_has_an_audited_minimal_routine_closure() {
        let actual = standalone_sys_closure_inventory();
        let expected = include_str!("../../fixtures/runtime/standalone_sys_link_closures.txt")
            .replace("\r\n", "\n");
        assert_eq!(
            actual, expected,
            "a SYS dependency closure changed; audit the added/removed resident routines before updating the inventory"
        );
    }

    #[test]
    fn generated_sys_link_manifest_matches_the_embedded_artifact() {
        let generated = generate_resident_link_manifest().expect("generate SYS link manifest");
        let embedded =
            include_str!("../../embedded/manifests/sys-link-v1.txt").replace("\r\n", "\n");
        assert_eq!(
            generated, embedded,
            "embedded SYS sources or graph extraction changed; audit the graph and regenerate with `cargo run --bin actionc-runtime-link-manifest -- embedded/manifests/sys-link-v1.txt`"
        );
    }

    #[test]
    fn embedded_sys_graph_matches_dynamic_analysis_for_every_routine_root() {
        let (_, program) = compile_runtime_image_with_semir().expect("resident runtime image");
        let dynamic = discover_runtime_dependency_graph(&program).expect("discover runtime graph");

        for routine in &program.routines {
            let expected = dynamic
                .closure([MirLinkNode::Routine(routine.id)])
                .unwrap_or_else(|node| panic!("dynamic root {} misses {node:?}", routine.name));
            let expected = runtime_selection_from_nodes(&expected.retained);
            let actual = embedded_resident_selection(&program, BTreeSet::from([routine.id]))
                .unwrap_or_else(|diagnostics| {
                    panic!("embedded root {}: {diagnostics:?}", routine.name)
                });
            assert_eq!(actual, expected, "runtime root {}", routine.name);
        }
    }

    #[test]
    fn embedded_sys_manifest_records_legacy_layout_edges_explicitly() {
        let manifest = embedded_sys_link_manifest().expect("embedded SYS link manifest");
        let has_edge = |source: &str, target: &str, reason: LinkReason| {
            manifest
                .graph
                .edges_from(&RuntimeLinkNode::routine(source))
                .any(|edge| {
                    edge.target == RuntimeLinkNode::routine(target) && edge.reason == reason
                })
        };

        assert!(has_edge("InputB", "InputC", LinkReason::EntryAlias));
        assert!(has_edge("InputC", "InputI", LinkReason::EntryAlias));
        assert!(has_edge(
            "InputI",
            "InputBD",
            LinkReason::MachineFallthrough
        ));
        assert!(has_edge("ValB", "ValC", LinkReason::EntryAlias));
        assert!(has_edge("ValC", "ValI", LinkReason::EntryAlias));
        assert!(has_edge("Zero", "SetBlock", LinkReason::MachineFallthrough));
        assert!(has_edge("PutDE", "PutD1", LinkReason::RequiredPrefix));
        assert!(has_edge("InputID", "InputD", LinkReason::MachineRelocation));

        assert!(
            manifest
                .graph
                .edges_from(&RuntimeLinkNode::routine("SetBlock"))
                .all(|edge| !matches!(
                    edge.reason,
                    LinkReason::EntryAlias
                        | LinkReason::MachineFallthrough
                        | LinkReason::ConservativeLayout
                )),
            "terminal SetBlock must not retain the following routine"
        );
    }

    #[test]
    fn production_resident_selection_never_discovers_the_sys_graph() {
        RUNTIME_GRAPH_DISCOVERIES.with(|count| count.set(0));
        let unit = crate::runtime_source::resolve_runtime_unit("SYSBLK").expect("SYSBLK unit");
        let selection = crate::runtime_source::select_runtime_image(&BTreeMap::from([(
            unit,
            BTreeSet::from(["Zero".to_string()]),
        )]))
        .expect("select embedded Zero graph");

        assert!(selection.selection.routines.contains("ZERO"));
        assert!(selection.selection.routines.contains("SETBLOCK"));
        assert_eq!(runtime_graph_discoveries(), 0);
    }

    #[test]
    fn production_helper_selection_never_discovers_the_syslib_graph() {
        let runtime = syslib_mir().expect("compile embedded SYSLIB");
        let mut declaration = runtime
            .runtime_helpers
            .iter()
            .find(|declaration| declaration.helper == MirRuntimeHelper::Mod)
            .expect("RemI helper declaration")
            .clone();
        declaration.target = MirRuntimeHelperTarget::Deferred;
        let mut application = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: vec![declaration],
        };

        RUNTIME_GRAPH_DISCOVERIES.with(|count| count.set(0));
        link_helpers(&mut application).expect("link selected RemI package");

        let routines = application
            .routines
            .iter()
            .map(|routine| routine.name.as_str())
            .collect::<BTreeSet<_>>();
        for expected in ["RemI", "DivI", "SMOps", "SetSign", "SS1"] {
            assert!(
                routines.contains(format!("ACTION.RUNTIME.SYSLIB::{expected}").as_str()),
                "missing {expected}: {routines:?}"
            );
        }
        assert!(!routines.contains("ACTION.RUNTIME.SYSLIB::MultI"));
        assert_eq!(runtime_graph_discoveries(), 0);
    }

    #[test]
    fn embedded_syslib_is_lowered_with_resolved_local_machine_references() {
        let program = syslib_mir().expect("compile embedded SYSLIB");
        let sargs = program
            .routines
            .iter()
            .find(|routine| routine.name.to_ascii_uppercase().contains("SARGS"))
            .unwrap_or_else(|| {
                panic!(
                    "SArgs routine; found {:?}",
                    program
                        .routines
                        .iter()
                        .map(|routine| &routine.name)
                        .collect::<Vec<_>>()
                )
            });
        let machine_id = sargs.blocks.iter().find_map(|block| {
            block.ops.iter().find_map(|op| match op {
                MirOp::MachineBlock { id, .. } => Some(*id),
                _ => None,
            })
        });
        let machine = program
            .machine_blocks
            .iter()
            .find(|machine| Some(machine.id) == machine_id)
            .expect("SArgs machine block");
        assert!(machine.items.iter().any(|item| {
            matches!(
                item,
                MirMachineItem::Relocation {
                    target: MirInlineAsmTarget::Routine(_),
                    ..
                }
            )
        }));
    }

    #[test]
    fn dependency_closure_terminates_and_is_stable_for_a_recursive_group() {
        let mut program = syslib_mir().expect("compile embedded SYSLIB");
        let multi = find_runtime_helper(&program, MirRuntimeHelper::Mul).expect("MultI root");
        let set_sign = program
            .routines
            .iter()
            .find(|routine| {
                runtime_routine_name(&routine.name, "ACTION_RUNTIME_SYSLIB") == "SetSign"
            })
            .expect("SetSign dependency");
        let machine_id = set_sign
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .find_map(|op| match op {
                MirOp::MachineBlock { id, .. } => Some(*id),
                _ => None,
            })
            .expect("SetSign machine block");
        program
            .machine_blocks
            .iter_mut()
            .find(|machine| machine.id == machine_id)
            .expect("SetSign machine payload")
            .items
            .push(MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Absolute16,
                target: MirInlineAsmTarget::Routine(multi),
                addend: 0,
                requires_zero_page: false,
                span: Span::new(0, 0),
            });

        let first = dependency_closure(&program, BTreeSet::from([multi])).unwrap();
        let second = dependency_closure(&program, BTreeSet::from([multi])).unwrap();
        assert_eq!(first, second);
        assert!(first.contains(&multi));
        assert!(first.contains(&set_sign.id));
    }

    #[test]
    fn unified_runtime_mir_retains_cross_unit_dependencies() {
        let (image, program) = compile_runtime_image_with_semir().expect("compile SYSALL image");
        eprintln!(
            "statics={:?}\nglobals={:?}",
            program
                .statics
                .iter()
                .map(|item| (&item.name, item.image.bytes.len()))
                .collect::<Vec<_>>(),
            program
                .globals
                .iter()
                .filter(|item| item.storage_size != 0 || item.init.is_some())
                .map(|item| (&item.name, item.storage_size, &item.backing))
                .collect::<Vec<_>>()
        );
        let graphics = program
            .routines
            .iter()
            .find(|routine| {
                runtime_routine_name(&routine.name, "ACTION_RUNTIME_RESIDENT")
                    .eq_ignore_ascii_case("Graphics")
            })
            .unwrap_or_else(|| {
                panic!(
                    "Graphics routine; found {:?}",
                    program
                        .routines
                        .iter()
                        .map(|routine| &routine.name)
                        .collect::<Vec<_>>()
                )
            });
        let selected = dependency_closure(&program, BTreeSet::from([graphics.id])).unwrap();
        let open = program
            .routines
            .iter()
            .find(|routine| {
                runtime_routine_name(&routine.name, "ACTION_RUNTIME_RESIDENT")
                    .eq_ignore_ascii_case("Open")
            })
            .expect("Open routine");
        assert!(selected.contains(&open.id));
        assert_eq!(image.routine_units["GRAPHICS"].name, "SYSGR");
        assert_eq!(image.routine_units["OPEN"].name, "SYSIO");
    }

    #[test]
    fn resident_selection_closes_over_aliases_and_backward_branch_targets() {
        let unit = crate::runtime_source::resolve_runtime_unit("SYSIO").expect("SYSIO unit");
        let input = crate::runtime_source::select_runtime_image(&BTreeMap::from([(
            unit.clone(),
            BTreeSet::from(["InputB".to_string()]),
        )]))
        .expect("select InputB closure");

        for expected in [
            "InputB", "InputC", "InputI", "InputBD", "InputCD", "InputID", "ValB", "ValC", "ValI",
        ] {
            assert!(
                input
                    .selection
                    .routines
                    .contains(&expected.to_ascii_uppercase()),
                "missing {expected}: {:?}",
                input.selection.routines
            );
        }

        let put_de = crate::runtime_source::select_runtime_image(&BTreeMap::from([(
            unit,
            BTreeSet::from(["PutDE".to_string()]),
        )]))
        .expect("select PutDE closure");
        assert!(
            put_de.selection.routines.contains("PUTD1"),
            "missing backward branch target: {:?}",
            put_de.selection.routines
        );
    }

    #[test]
    fn resident_selection_keeps_cross_unit_code_and_only_referenced_data() {
        let unit = crate::runtime_source::resolve_runtime_unit("SYSGR").expect("SYSGR unit");
        let selection = crate::runtime_source::select_runtime_image(&BTreeMap::from([(
            unit,
            BTreeSet::from(["Graphics".to_string()]),
        )]))
        .expect("select Graphics closure");

        for expected in ["Graphics", "Close", "Open", "ChkErr", "Error"] {
            assert!(
                selection
                    .selection
                    .routines
                    .contains(&expected.to_ascii_uppercase()),
                "missing {expected}: {:?}",
                selection.selection.routines
            );
        }
        assert!(
            selection
                .selection
                .globals
                .iter()
                .any(|name| name.to_ascii_uppercase().contains("DEV_S"))
        );
        assert!(
            selection
                .selection
                .globals
                .iter()
                .any(|name| name.to_ascii_uppercase().contains("DEV_E"))
        );
        assert!(
            selection
                .selection
                .globals
                .iter()
                .all(|name| !name.to_ascii_uppercase().contains("COPY_RIGHT"))
        );
    }

    #[test]
    fn resident_printh_selection_keeps_its_machine_code_output_chain() {
        let unit = crate::runtime_source::resolve_runtime_unit("SYSIO").expect("SYSIO unit");
        let selection = crate::runtime_source::select_runtime_image(&BTreeMap::from([(
            unit,
            BTreeSet::from(["PrintH".to_string()]),
        )]))
        .expect("select PrintH closure");

        for expected in ["PrintH", "Put", "PutD", "PutD1", "CCIO", "ChkErr"] {
            assert!(
                selection
                    .selection
                    .routines
                    .contains(&expected.to_ascii_uppercase()),
                "missing {expected}: {:?}",
                selection.selection.routines
            );
        }
        for unrelated in ["PrintF", "PrintC", "InputD"] {
            assert!(
                selection
                    .selection
                    .routines
                    .iter()
                    .all(|name| !name.eq_ignore_ascii_case(unrelated)),
                "unexpected {unrelated}: {:?}",
                selection.selection.routines
            );
        }
    }

    #[test]
    fn standalone_linking_rejects_a_logical_helper_contract_mismatch() {
        let runtime = syslib_mir().expect("compile embedded SYSLIB");
        let mut declaration = runtime
            .runtime_helpers
            .iter()
            .find(|declaration| declaration.helper == MirRuntimeHelper::Mul)
            .expect("MultI declaration")
            .clone();
        declaration.target = MirRuntimeHelperTarget::Deferred;
        declaration.effects.opaque = !declaration.effects.opaque;
        let mut application = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: vec![MirRuntimeHelperDecl { ..declaration }],
        };

        let diagnostics = link_helpers(&mut application).expect_err("reject effect mismatch");
        assert!(diagnostics[0].message.contains("effect mismatch"));
    }
}
