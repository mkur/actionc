use std::collections::{BTreeMap, BTreeSet};

use crate::ast::machine_address_symbolic_offset;
use crate::codegen::runtime_zp;
use crate::nir::{
    self, AddressValue, BlockId, ByteOffset, ByteSize, LocalId, NirBinaryOp, NirCompareOp,
    NirGlobalBacking, NirInlineAsm,
    NirInlineAsmTarget, NirLocalBacking, NirLocalPurpose, NirMachineAtom, NirMachineByteSelector,
    NirMachineEffects, NirMachineItem, NirMemoryAccess, NirMemoryRegionKind, NirOp as NirOpKind,
    NirPlace, NirPlaceKind, NirProgram, NirRealOp, NirRealSource, NirRoutine, NirStorageId,
    NirStorageInit, NirTerminator, NirType, NirTypeKind, NirUnaryOp, NirValue as NirValueKind,
    TempId,
};
use crate::resident::resident_variable;

use super::call_plan;
use super::classify::{
    MirAddressShape, MirPlaceShape, MirValueShape, classify_address, classify_place, classify_value,
};
use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirAddr, MirAtariFppService, MirBinaryOp, MirBlock, MirBlockId, MirBlockParam, MirCallAbi,
    MirCallTarget, MirCarryOut, MirCompareOp, MirCond, MirCondDest, MirDataBacking, MirDataImage,
    MirDataRelocation, MirDataRelocationKind, MirDataRelocationTarget, MirDef, MirEdge, MirEdgeArg,
    MirEffects, MirFixedZpSlot, MirFlagTest, MirFrame, MirGlobal, MirGlobalBacking, MirGlobalInit,
    MirInlineAsmTarget, MirMachineAtom, MirMachineBlock, MirMachineBlockId, MirMachineByteSelector,
    MirMachineItem, MirMem, MirMemoryEffect, MirMemoryRegionKind, MirOp, MirProgram,
    MirRegisterSet, MirRoutine, MirRoutineAbi, MirRuntimeHelper, MirRuntimeHelperDecl,
    MirRuntimeHelperTarget, MirStatic, MirStorageBacking, MirStorageBase, MirStorageClass,
    MirStorageId, MirStorageInit, MirStorageSlot, MirTemp, MirTempId, MirTerminator, MirUnaryOp,
    MirValue, MirWidth, RoutineId,
};

#[derive(Debug, Clone, Copy)]
enum LoweredRealBranch {
    Boolean,
    PackedFlags,
}

fn nir_size_u16(size: ByteSize) -> u16 {
    u16::try_from(size).expect("verified Atari NIR byte size must fit in 16 bits")
}

fn nir_offset_u16(offset: ByteOffset) -> u16 {
    u16::try_from(offset).expect("verified Atari NIR byte offset must fit in 16 bits")
}

fn nir_address_u16(address: AddressValue) -> u16 {
    u16::try_from(address.value).expect("verified Atari NIR address must fit in 16 bits")
}

fn direct_real_branch_result(block: &nir::NirBlock) -> Option<TempId> {
    let NirTerminator::Branch {
        condition: NirValueKind::Temp { id: condition, .. },
        then_edge,
        else_edge,
        ..
    } = &block.terminator
    else {
        return None;
    };
    let Some(NirOpKind::Real(NirRealOp::Compare { result, .. })) = block.ops.last() else {
        return None;
    };
    if *condition != *result
        || then_edge
            .args
            .iter()
            .chain(&else_edge.args)
            .any(|arg| matches!(arg, NirValueKind::Temp { id, .. } if id == result))
    {
        return None;
    }
    Some(*result)
}

#[derive(Debug, Default, Clone, Copy)]
struct RealLocalAccesses {
    reads: usize,
    writes: usize,
    other: usize,
}

#[derive(Debug, Default)]
struct AdjacentFppResultChains {
    left: BTreeSet<LocalId>,
    ordered_right: BTreeSet<LocalId>,
    commutative_right: BTreeSet<LocalId>,
}

#[derive(Debug, Clone, Copy)]
enum AdjacentFppResultChain {
    Left(LocalId),
    OrderedRight(LocalId),
    CommutativeRight(LocalId),
}

fn adjacent_fpp_result_chains(routine: &NirRoutine) -> AdjacentFppResultChains {
    let private_real_temps = routine
        .locals
        .iter()
        .filter(|local| matches!(local.purpose, NirLocalPurpose::RealTemporary))
        .map(|local| local.id)
        .collect::<BTreeSet<_>>();
    let accesses = real_local_accesses(routine);
    let mut chained = AdjacentFppResultChains::default();

    for block in &routine.blocks {
        for pair in block.ops.windows(2) {
            let Some(chain) = adjacent_fpp_result_chain(&pair[0], &pair[1]) else {
                continue;
            };
            let local = match chain {
                AdjacentFppResultChain::Left(local)
                | AdjacentFppResultChain::OrderedRight(local)
                | AdjacentFppResultChain::CommutativeRight(local) => local,
            };
            if private_real_temps.contains(&local)
                && accesses.get(&local).is_some_and(|access| {
                    access.reads == 1 && access.writes == 1 && access.other == 0
                })
            {
                match chain {
                    AdjacentFppResultChain::Left(local) => {
                        chained.left.insert(local);
                    }
                    AdjacentFppResultChain::OrderedRight(local) => {
                        chained.ordered_right.insert(local);
                    }
                    AdjacentFppResultChain::CommutativeRight(local) => {
                        chained.commutative_right.insert(local);
                    }
                }
            }
        }
    }
    chained
}

fn adjacent_real_copy_forwards(routine: &NirRoutine) -> BTreeSet<LocalId> {
    let private_real_temps = routine
        .locals
        .iter()
        .filter(|local| matches!(local.purpose, NirLocalPurpose::RealTemporary))
        .map(|local| local.id)
        .collect::<BTreeSet<_>>();
    let accesses = real_local_accesses(routine);
    let mut forwarded = BTreeSet::new();

    for block in &routine.blocks {
        for (index, producer) in block.ops.iter().enumerate() {
            let adjacent = block
                .ops
                .get(index + 1)
                .and_then(|consumer| adjacent_real_copy_forward_local(producer, consumer));
            let after_static_negation =
                block
                    .ops
                    .get(index + 1..=index + 2)
                    .and_then(|ops| match ops {
                        [intervening, consumer] if is_static_real_negation(intervening) => {
                            adjacent_real_copy_forward_local(producer, consumer)
                        }
                        _ => None,
                    });
            let Some(local) = adjacent.or(after_static_negation) else {
                continue;
            };
            if private_real_temps.contains(&local)
                && accesses.get(&local).is_some_and(|access| {
                    access.reads == 1 && access.writes == 1 && access.other == 0
                })
            {
                forwarded.insert(local);
            }
        }
    }
    forwarded
}

fn is_static_real_negation(op: &NirOpKind) -> bool {
    matches!(
        op,
        NirOpKind::Real(NirRealOp::Unary {
            operation: NirUnaryOp::Neg,
            operand: NirRealSource::Static { .. },
            ..
        })
    )
}

fn static_real_negation_forwards(routine: &NirRoutine) -> BTreeSet<LocalId> {
    let private_real_temps = routine
        .locals
        .iter()
        .filter(|local| matches!(local.purpose, NirLocalPurpose::RealTemporary))
        .map(|local| local.id)
        .collect::<BTreeSet<_>>();
    let accesses = real_local_accesses(routine);
    let mut forwarded = BTreeSet::new();

    for block in &routine.blocks {
        for (index, producer) in block.ops.iter().enumerate() {
            let NirOpKind::Real(NirRealOp::Unary {
                operation: NirUnaryOp::Neg,
                destination,
                operand: NirRealSource::Static { .. },
            }) = producer
            else {
                continue;
            };
            let Some(local) = direct_real_local(destination) else {
                continue;
            };
            let read_later_in_block = block.ops[index + 1..].iter().any(
                |op| matches!(op, NirOpKind::Real(real) if real_op_reads_direct_local(real, local)),
            );
            if read_later_in_block
                && private_real_temps.contains(&local)
                && accesses.get(&local).is_some_and(|access| {
                    access.reads == 1 && access.writes == 1 && access.other == 0
                })
            {
                forwarded.insert(local);
            }
        }
    }
    forwarded
}

fn adjacent_fpp_result_chain(
    producer: &NirOpKind,
    consumer: &NirOpKind,
) -> Option<AdjacentFppResultChain> {
    let destination = match producer {
        NirOpKind::Real(NirRealOp::Binary { destination, .. })
        | NirOpKind::Real(NirRealOp::IntegerToReal { destination, .. }) => destination,
        _ => return None,
    };
    let local = direct_real_local(destination)?;
    let (left, right, operation) = match consumer {
        NirOpKind::Real(NirRealOp::Binary {
            operation,
            left,
            right,
            ..
        }) => (left, right, Some(*operation)),
        NirOpKind::Real(NirRealOp::Compare { left, right, .. }) => (left, right, None),
        _ => return None,
    };
    if real_source_is_direct_local(left, local) {
        return Some(AdjacentFppResultChain::Left(local));
    }
    if !real_source_is_direct_local(right, local) {
        return None;
    }
    match operation {
        Some(NirBinaryOp::Add | NirBinaryOp::Mul) => {
            Some(AdjacentFppResultChain::CommutativeRight(local))
        }
        Some(NirBinaryOp::Sub | NirBinaryOp::Div) => {
            Some(AdjacentFppResultChain::OrderedRight(local))
        }
        Some(
            NirBinaryOp::Mod
            | NirBinaryOp::Lsh
            | NirBinaryOp::Rsh
            | NirBinaryOp::And
            | NirBinaryOp::Or
            | NirBinaryOp::Xor,
        )
        | None => None,
    }
}

fn real_source_is_direct_local(source: &NirRealSource, local: LocalId) -> bool {
    matches!(source, NirRealSource::Place(place) if direct_real_local(place) == Some(local))
}

fn adjacent_real_copy_forward_local(producer: &NirOpKind, consumer: &NirOpKind) -> Option<LocalId> {
    let NirOpKind::Real(NirRealOp::Copy { destination, .. }) = producer else {
        return None;
    };
    let local = direct_real_local(destination)?;
    match consumer {
        NirOpKind::Real(real) if real_op_reads_direct_local(real, local) => Some(local),
        _ => None,
    }
}

fn real_op_reads_direct_local(op: &NirRealOp, local: LocalId) -> bool {
    let reads = |source: &NirRealSource| matches!(source, NirRealSource::Place(place) if direct_real_local(place) == Some(local));
    match op {
        NirRealOp::Copy { source, .. } => reads(source),
        NirRealOp::Unary { operand, .. } => reads(operand),
        NirRealOp::Binary { left, right, .. } | NirRealOp::Compare { left, right, .. } => {
            reads(left) || reads(right)
        }
        NirRealOp::RealToInteger { source, .. } => direct_real_local(source) == Some(local),
        NirRealOp::IntegerToReal { .. } => false,
    }
}

fn direct_real_local(place: &NirPlace) -> Option<LocalId> {
    match place.kind {
        NirPlaceKind::Local { id, .. } => Some(id),
        _ => None,
    }
}

fn real_local_accesses(routine: &NirRoutine) -> BTreeMap<LocalId, RealLocalAccesses> {
    let mut accesses = BTreeMap::<LocalId, RealLocalAccesses>::new();
    for local in &routine.locals {
        if let NirLocalBacking::Alias { target, .. } = local.backing {
            accesses.entry(target).or_default().other += 1;
        }
        if let Some(init) = &local.init {
            record_storage_init_local_references(init, &mut accesses);
        }
    }
    for block in &routine.blocks {
        for op in &block.ops {
            match op {
                NirOpKind::Load { place, .. }
                | NirOpKind::VolatileLoad { place, .. }
                | NirOpKind::AddrOf { place, .. }
                | NirOpKind::Store { place, .. }
                | NirOpKind::VolatileStore { place, .. } => {
                    record_other_real_local(place, &mut accesses);
                }
                NirOpKind::CopyBytes {
                    destination,
                    source,
                    ..
                } => {
                    record_other_real_local(destination, &mut accesses);
                    record_other_real_local(source, &mut accesses);
                }
                NirOpKind::Real(real) => record_real_local_accesses(real, &mut accesses),
                NirOpKind::Call { effects, .. } => {
                    record_effect_local_references(&effects.memory, &mut accesses);
                }
                NirOpKind::MachineBlock { items, effects } => {
                    for item in items {
                        if let NirMachineItem::Relocation { target, .. } = item {
                            record_inline_target_local(*target, &mut accesses);
                        }
                    }
                    record_effect_local_references(&effects.memory, &mut accesses);
                }
                NirOpKind::InlineAsm { code, effects } => {
                    for relocation in &code.relocations {
                        record_inline_target_local(relocation.target, &mut accesses);
                    }
                    record_effect_local_references(&effects.memory, &mut accesses);
                }
                NirOpKind::RuntimeHelperOverride { .. }
                | NirOpKind::Unary { .. }
                | NirOpKind::Cast { .. }
                | NirOpKind::PointerOffset { .. }
                | NirOpKind::Binary { .. }
                | NirOpKind::Compare { .. }
                | NirOpKind::Unsupported { .. } => {}
            }
        }
    }
    accesses
}

fn record_real_local_accesses(op: &NirRealOp, accesses: &mut BTreeMap<LocalId, RealLocalAccesses>) {
    match op {
        NirRealOp::Copy {
            destination,
            source,
        } => {
            record_real_local_write(destination, accesses);
            record_real_local_read(source, accesses);
        }
        NirRealOp::Unary {
            destination,
            operand,
            ..
        } => {
            record_real_local_write(destination, accesses);
            record_real_local_read(operand, accesses);
        }
        NirRealOp::Binary {
            destination,
            left,
            right,
            ..
        } => {
            record_real_local_write(destination, accesses);
            record_real_local_read(left, accesses);
            record_real_local_read(right, accesses);
        }
        NirRealOp::Compare { left, right, .. } => {
            record_real_local_read(left, accesses);
            record_real_local_read(right, accesses);
        }
        NirRealOp::IntegerToReal { destination, .. } => {
            record_real_local_write(destination, accesses);
        }
        NirRealOp::RealToInteger { source, .. } => {
            if let Some(id) = root_real_local(source) {
                accesses.entry(id).or_default().reads += 1;
            }
        }
    }
}

fn record_real_local_read(
    source: &NirRealSource,
    accesses: &mut BTreeMap<LocalId, RealLocalAccesses>,
) {
    if let NirRealSource::Place(place) = source
        && let Some(id) = root_real_local(place)
    {
        accesses.entry(id).or_default().reads += 1;
    }
}

fn record_real_local_write(place: &NirPlace, accesses: &mut BTreeMap<LocalId, RealLocalAccesses>) {
    if let Some(id) = root_real_local(place) {
        accesses.entry(id).or_default().writes += 1;
    }
}

fn record_other_real_local(place: &NirPlace, accesses: &mut BTreeMap<LocalId, RealLocalAccesses>) {
    if let Some(id) = root_real_local(place) {
        accesses.entry(id).or_default().other += 1;
    }
}

fn root_real_local(place: &NirPlace) -> Option<LocalId> {
    match &place.kind {
        NirPlaceKind::Local { id, .. } => Some(*id),
        NirPlaceKind::Field { base, .. } => root_real_local(base),
        NirPlaceKind::Param { .. }
        | NirPlaceKind::Global { .. }
        | NirPlaceKind::Absolute(_)
        | NirPlaceKind::Deref { .. }
        | NirPlaceKind::Index { .. } => None,
    }
}

fn record_effect_local_references(
    effects: &nir::NirMemoryEffects,
    accesses: &mut BTreeMap<LocalId, RealLocalAccesses>,
) {
    for access in [&effects.reads, &effects.writes] {
        let NirMemoryAccess::Regions(regions) = access else {
            continue;
        };
        for region in regions {
            if let NirMemoryRegionKind::Storage(NirStorageId::Local(id)) = region.kind {
                accesses.entry(id).or_default().other += 1;
            }
        }
    }
}

fn record_inline_target_local(
    target: NirInlineAsmTarget,
    accesses: &mut BTreeMap<LocalId, RealLocalAccesses>,
) {
    if let NirInlineAsmTarget::Storage(NirStorageId::Local(id)) = target {
        accesses.entry(id).or_default().other += 1;
    }
}

fn record_storage_init_local_references(
    init: &NirStorageInit,
    accesses: &mut BTreeMap<LocalId, RealLocalAccesses>,
) {
    let image = match init {
        NirStorageInit::Bytes { image, .. } => Some(image),
        NirStorageInit::Descriptor { backing, .. } => Some(&backing.image),
        NirStorageInit::ZeroFill { .. } => None,
    };
    let Some(image) = image else {
        return;
    };
    for fragment in &image.fragments {
        if let nir::NirDataFragment::Address {
            target: nir::NirDataAddressTarget::Storage(NirStorageId::Local(id)),
            ..
        } = fragment
        {
            accesses.entry(*id).or_default().other += 1;
        }
    }
}

pub(super) fn lower_program(nir_program: &NirProgram) -> Result<MirProgram, Vec<MirDiagnostic>> {
    if let Err(diagnostics) = nir::verify_program(nir_program) {
        return Err(diagnostics
            .into_iter()
            .map(|diagnostic| MirDiagnostic {
                routine: diagnostic.routine,
                block: diagnostic.block,
                message: format!("NIR verification failed: {}", diagnostic.message),
            })
            .collect());
    }
    if nir_program.target_layout.target != crate::target::TargetId::Atari6502 {
        return Err(vec![MirDiagnostic {
            routine: None,
            block: None,
            message: format!(
                "MIR6502 cannot lower target `{}`",
                nir_program.target_layout.target
            ),
        }]);
    }
    let mut diagnostics = Vec::new();
    let routine_ids = nir_program
        .routines
        .iter()
        .enumerate()
        .map(|(index, routine)| (routine.name.as_str(), RoutineId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let routine_system_addresses_by_id = nir_program
        .routines
        .iter()
        .enumerate()
        .filter_map(|(index, routine)| {
            routine_system_address(routine).map(|address| (index as u32, address))
        })
        .collect::<BTreeMap<_, _>>();
    let routine_system_addresses = nir_program
        .routines
        .iter()
        .filter_map(|routine| {
            routine_system_address(routine).map(|address| (routine.name.as_str(), address))
        })
        .collect::<BTreeMap<_, _>>();
    let global_array_pointer_backing = nir_program
        .globals
        .iter()
        .filter_map(|global| {
            global.array.as_ref().map(|array| {
                (
                    global.id,
                    array.pointer_backed
                        || matches!(
                            global.init.as_ref(),
                            Some(nir::NirGlobalInit::Descriptor { .. })
                        ),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let machine_numeric_defines = collect_machine_numeric_defines(nir_program);
    let mut machine_blocks = Vec::new();
    let routines =
        nir_program
            .routines
            .iter()
            .enumerate()
            .map(|(routine_index, routine)| {
                let block_ids = routine
                    .blocks
                    .iter()
                    .enumerate()
                    .map(|(index, block)| (block.id, MirBlockId(index as u32)))
                    .collect::<BTreeMap<_, _>>();
                let local_absolute_addresses = routine
                    .locals
                    .iter()
                    .filter_map(|local| match local.backing {
                        NirLocalBacking::Absolute(address) => {
                            Some((machine_name_key(&local.name), nir_address_u16(address)))
                        }
                        NirLocalBacking::Ordinary
                        | NirLocalBacking::Alias { .. }
                        | NirLocalBacking::GlobalAlias { .. } => None,
                    })
                    .collect::<BTreeMap<_, _>>();
                let local_array_pointer_backing = routine
                    .locals
                    .iter()
                    .filter(|local| local_pointer_backed_array(local))
                    .map(|local| local.id)
                    .collect::<Vec<_>>();
                let mut next_generated_temp = routine
                    .temps
                    .iter()
                    .map(|temp| temp.id.0)
                    .max()
                    .map_or(0, |id| id.saturating_add(1));
                let mut generated_temps = Vec::new();
                let mut next_generated_local = routine
                    .locals
                    .iter()
                    .map(|local| local.id.0)
                    .max()
                    .map_or(0, |id| id.saturating_add(1));
                let mut generated_locals = Vec::new();
                let fpp_result_chains = adjacent_fpp_result_chains(routine);
                let real_copy_forwards = adjacent_real_copy_forwards(routine);
                let real_negation_forwards = static_real_negation_forwards(routine);

                let mut blocks: Vec<MirBlock> = routine
                    .blocks
                    .iter()
                    .enumerate()
                    .map(|(block_index, block)| {
                        let mut real_branch_values = BTreeMap::new();
                        let direct_real_branch_result = direct_real_branch_result(block);
                        let mut ops = lower_ops(
                            &routine.name,
                            &block.label,
                            &block.ops,
                            &routine_ids,
                            &routine_system_addresses_by_id,
                            &routine_system_addresses,
                            &global_array_pointer_backing,
                            &local_array_pointer_backing,
                            &local_absolute_addresses,
                            &machine_numeric_defines,
                            &mut machine_blocks,
                            &mut next_generated_temp,
                            &mut generated_temps,
                            &mut next_generated_local,
                            &mut generated_locals,
                            direct_real_branch_result,
                            &mut real_branch_values,
                            &mut diagnostics,
                        );
                        lower_return_value_ops(
                            &routine.name,
                            &block.label,
                            routine_return_width(routine),
                            &block.terminator,
                            &mut ops,
                            &mut diagnostics,
                        );
                        let mut terminator = lower_terminator(
                            &routine.name,
                            &block.label,
                            block.id,
                            &block.terminator,
                            &block_ids,
                            &mut diagnostics,
                        );
                        if let MirTerminator::Branch {
                            cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(result))),
                            ..
                        } = &mut terminator
                            && let Some(answer) = real_branch_values.get(&result.0).copied()
                        {
                            if let LoweredRealBranch::Boolean = answer {
                                ops.push(MirOp::Compare {
                                    dst: MirCondDest::Flags,
                                    op: MirCompareOp::Ne,
                                    left: temp_value(*result),
                                    right: MirValue::ConstU8(0),
                                    width: MirWidth::Byte,
                                    signed: false,
                                });
                            }
                            if let MirTerminator::Branch { cond, .. } = &mut terminator {
                                *cond = MirCond::FlagTest(MirFlagTest::ZClear);
                            }
                        }
                        MirBlock {
                            id: MirBlockId(block_index as u32),
                            label: block.label.clone(),
                            params: block
                                .params
                                .iter()
                                .filter_map(|param| {
                                    mir_width(&param.ty)
                                        .map(|width| MirBlockParam {
                                            dest: MirTempId(param.dest.0),
                                            width,
                                        })
                                        .or_else(|| {
                                            diagnostics.push(MirDiagnostic::block(
                                            &routine.name,
                                            &block.label,
                                            format!(
                                                "NIR block parameter `%t{}` has unsupported width",
                                                param.dest.0
                                            ),
                                        ));
                                            None
                                        })
                                })
                                .collect(),
                            ops,
                            terminator,
                        }
                    })
                    .collect();
                let mut elided_real_locals =
                    eliminate_adjacent_fpp_result_round_trips(&mut blocks, &fpp_result_chains.left);
                elided_real_locals.extend(eliminate_adjacent_fpp_right_result_round_trips(
                    &mut blocks,
                    &fpp_result_chains.ordered_right,
                    &fpp_result_chains.commutative_right,
                ));
                elided_real_locals.extend(forward_static_real_temp_negations(
                    &mut blocks,
                    &real_negation_forwards,
                ));
                elided_real_locals.extend(forward_adjacent_real_temp_copies(
                    &mut blocks,
                    &real_copy_forwards,
                ));
                let retained_source_local_count = routine
                    .locals
                    .iter()
                    .filter(|local| {
                        matches!(
                            local.backing,
                            NirLocalBacking::Ordinary | NirLocalBacking::Alias { .. }
                        ) && !elided_real_locals.contains(&local.id)
                    })
                    .count();

                MirRoutine {
                id: RoutineId(routine_index as u32),
                name: routine.name.clone(),
                abi: if routine_has_external_interface(routine) {
                    MirRoutineAbi::ExternalInterface
                } else if routine_is_program_entry(routine)
                    && routine_has_observable_action_entry(routine)
                {
                    MirRoutineAbi::ProgramEntryObservable
                } else if routine_is_program_entry(routine) {
                    MirRoutineAbi::ProgramEntry
                } else if routine_has_observable_action_entry(routine) {
                    MirRoutineAbi::ActionObservable
                } else {
                    MirRoutineAbi::Action
                },
                frame: MirFrame {
                    params: routine
                        .params
                        .iter()
                        .enumerate()
                        .map(|(index, param)| {
                            let scalar_width = mir_width(&param.ty);
                            MirStorageSlot {
                                id: MirStorageId(index as u32),
                                name: Some(param.name.clone()),
                                storage: lower_storage_class(param.storage),
                                storage_size: nir_size_u16(param
                                    .ty
                                    .width
                                    .unwrap_or_else(|| ByteSize::from(
                                        scalar_width.map(mir_width_bytes).unwrap_or(1),
                                    ))),
                                scalar_width,
                                base: MirStorageBase::Param(param.id),
                                offset: 0,
                                mutable: true,
                                init: None,
                            }
                        })
                        .collect(),
                    locals: routine
                        .locals
                        .iter()
                        .filter(|local| {
                            matches!(
                                local.backing,
                                NirLocalBacking::Ordinary | NirLocalBacking::Alias { .. }
                            ) && !elided_real_locals.contains(&local.id)
                        })
                        .enumerate()
                        .map(|(index, local)| {
                            let scalar_width = local_scalar_width(local);
                            let init = lower_local_storage_init(
                                local,
                                &routine_ids,
                                RoutineId(routine_index as u32),
                            );
                            MirStorageSlot {
                            id: MirStorageId(index as u32),
                            name: Some(local.name.clone()),
                            storage: lower_storage_class(local.storage),
                            storage_size: local_storage_size(local, scalar_width, init.as_ref()),
                            scalar_width,
                            base: match local.backing {
                                NirLocalBacking::Alias { target, .. } => {
                                    MirStorageBase::LocalAlias {
                                        id: local.id,
                                        target,
                                    }
                                }
                                NirLocalBacking::Ordinary => MirStorageBase::Local(local.id),
                                NirLocalBacking::GlobalAlias { .. } => unreachable!(
                                    "global-alias locals are resolved directly to global places"
                                ),
                                NirLocalBacking::Absolute(_) => unreachable!(
                                    "absolute locals are filtered out of the routine frame"
                                ),
                            },
                            offset: match local.backing {
                                NirLocalBacking::Alias { offset, .. } => nir_offset_u16(offset),
                                NirLocalBacking::Ordinary => 0,
                                NirLocalBacking::GlobalAlias { .. } => unreachable!(
                                    "global-alias locals are resolved directly to global places"
                                ),
                                NirLocalBacking::Absolute(_) => unreachable!(
                                    "absolute locals are filtered out of the routine frame"
                                ),
                            },
                            mutable: true,
                            init,
                        }
                        })
                        .chain(generated_locals.into_iter().enumerate().map(
                            |(generated_index, (id, name))| MirStorageSlot {
                                id: MirStorageId(
                                    retained_source_local_count as u32
                                        + generated_index as u32,
                                ),
                                name: Some(name),
                                storage: MirStorageClass::Scalar,
                                storage_size: 1,
                                scalar_width: Some(MirWidth::Byte),
                                base: MirStorageBase::Local(id),
                                offset: 0,
                                mutable: true,
                                init: None,
                            },
                        ))
                        .collect(),
                    spills: Vec::new(),
                    virtual_zero_page: Vec::new(),
                    fixed_zero_page: fixed_zero_page_slots(&blocks),
                    zero_page_allocations: Vec::new(),
                },
                temps: routine
                    .temps
                    .iter()
                    .map(|temp| MirTemp {
                        id: MirTempId(temp.id.0),
                    })
                    .chain(generated_temps)
                    .collect(),
                blocks,
                effects: MirEffects::default(),
            }
            })
            .collect();

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let runtime_helpers = runtime_helper_decls_from_sets(nir_program);
    let program = MirProgram {
        statics: nir_program
            .statics
            .iter()
            .map(|static_data| MirStatic {
                id: static_data.id,
                name: static_data.name.clone(),
                ty: static_data.ty.summary.clone(),
                image: lower_data_image(&static_data.image, None),
                display: static_data.display.clone(),
                alignment: nir_size_u16(static_data.alignment),
                mutable: static_data.mutable,
                section: static_data.section.clone(),
            })
            .collect(),
        globals: {
            let mut next_global_offset = 0u16;
            nir_program
                .globals
                .iter()
                .map(|global| {
                    let width = global.ty.as_ref().and_then(mir_width);
                    let ordinary_offset = next_global_offset;
                    if matches!(global.backing, NirGlobalBacking::Ordinary) {
                        next_global_offset = next_global_offset
                            .saturating_add(nir_size_u16(global.storage_size));
                    }
                    MirGlobal {
                        id: global.id,
                        name: global.name.clone(),
                        kind: global.kind.clone(),
                        width,
                        storage_size: nir_size_u16(global.storage_size),
                        backing: match global.backing {
                            NirGlobalBacking::Ordinary => MirGlobalBacking::Ordinary {
                                offset: ordinary_offset,
                            },
                            NirGlobalBacking::Absolute(address) => {
                                MirGlobalBacking::Absolute(nir_address_u16(address))
                            }
                            NirGlobalBacking::Alias { ref target, offset } => {
                                MirGlobalBacking::Alias {
                                    target: *target,
                                    offset: nir_offset_u16(offset),
                                }
                            }
                        },
                        init: global
                            .init
                            .as_ref()
                            .map(|init| lower_global_init(init, global.array.as_ref())),
                    }
                })
                .collect()
        },
        routines,
        machine_blocks,
        runtime_helpers,
    };
    Ok(program)
}

fn routine_system_address(routine: &nir::NirRoutine) -> Option<u16> {
    routine.notes.iter().find_map(|note| {
        let value = note.text.strip_prefix("system-address ")?;
        parse_system_address_note(value)
    })
}

fn routine_has_observable_action_entry(routine: &nir::NirRoutine) -> bool {
    routine
        .notes
        .iter()
        .any(|note| note.text.starts_with("system-address "))
}

fn routine_is_program_entry(routine: &nir::NirRoutine) -> bool {
    routine
        .notes
        .iter()
        .any(|note| note.kind == nir::NirRoutineNoteKind::ProgramEntry)
}

fn routine_has_external_interface(routine: &nir::NirRoutine) -> bool {
    routine
        .notes
        .iter()
        .any(|note| note.kind == nir::NirRoutineNoteKind::ExternalInterface)
}

fn parse_system_address_note(value: &str) -> Option<u16> {
    let value = value.trim();
    if value == "*" {
        return None;
    }
    if let Some(hex) = value.strip_prefix('$') {
        return u16::from_str_radix(hex, 16).ok();
    }
    value.parse::<u16>().ok()
}

fn runtime_helper_decls_from_sets(nir_program: &NirProgram) -> Vec<MirRuntimeHelperDecl> {
    let mut decls = Vec::<MirRuntimeHelperDecl>::new();
    for routine in &nir_program.routines {
        for block in &routine.blocks {
            for op in &block.ops {
                let NirOpKind::RuntimeHelperOverride { slot, target } = op else {
                    continue;
                };
                let Some(helper) = runtime_helper_from_slot(nir_address_u16(*slot)) else {
                    continue;
                };
                if decls.iter().any(|decl| decl.helper == helper) {
                    continue;
                }
                let target = match target {
                    crate::nir::NirRuntimeHelperTarget::Absolute(address) => {
                        MirRuntimeHelperTarget::KnownAbsolute(nir_address_u16(*address))
                    }
                    crate::nir::NirRuntimeHelperTarget::Routine(id) => {
                        MirRuntimeHelperTarget::Routine(RoutineId(*id))
                    }
                };
                decls.push(MirRuntimeHelperDecl {
                    effects: super::materialize::helper_effects(&helper),
                    helper,
                    target,
                    abi: super::materialize::helper_abi(),
                });
            }
        }
    }
    decls
}

fn runtime_helper_from_slot(slot: u16) -> Option<MirRuntimeHelper> {
    match slot {
        0x04E4 => Some(MirRuntimeHelper::Lsh),
        0x04E6 => Some(MirRuntimeHelper::Rsh),
        0x04E8 => Some(MirRuntimeHelper::Mul),
        0x04EA => Some(MirRuntimeHelper::Div),
        0x04EC => Some(MirRuntimeHelper::Mod),
        0x04EE => Some(MirRuntimeHelper::SArgs),
        _ => None,
    }
}

fn lower_global_init(
    init: &crate::nir::NirGlobalInit,
    array: Option<&crate::nir::NirArrayGlobalFact>,
) -> MirGlobalInit {
    let array = array.map(|array| super::ir::MirArrayGlobalFact {
        elem_size: nir_size_u16(array.elem_size),
        length: array.length,
        pointer_backed: array.pointer_backed,
        address_initializer: array.address_initializer.map(nir_address_u16),
    });
    match init {
        crate::nir::NirGlobalInit::Bytes {
            image,
            zero_fill,
            mutable,
            section,
        } => MirGlobalInit::Bytes {
            image: lower_data_image(image, None),
            zero_fill: nir_size_u16(*zero_fill),
            mutable: *mutable,
            section: section.clone(),
            array,
        },
        crate::nir::NirGlobalInit::Descriptor {
            backing,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => MirGlobalInit::Descriptor {
            backing: MirDataBacking {
                owner: backing.owner,
                image: lower_data_image(&backing.image, None),
                zero_fill: nir_size_u16(backing.zero_fill),
                section: backing.section.clone(),
            },
            descriptor_size: nir_size_u16(*descriptor_size),
            size_word: *size_word,
            mutable: *mutable,
            section: section.clone(),
        },
        crate::nir::NirGlobalInit::ZeroFill {
            bytes,
            mutable,
            section,
        } => MirGlobalInit::ZeroFill {
            bytes: nir_size_u16(*bytes),
            mutable: *mutable,
            section: section.clone(),
            array,
        },
        crate::nir::NirGlobalInit::LinkValue {
            value: crate::nir::NirLinkValue::ImageEndAddress,
            width,
            mutable,
            section,
        } => {
            assert_eq!(*width, ByteSize::new(2), "verified Atari image-end width");
            MirGlobalInit::ProgramEndWord {
                mutable: *mutable,
                section: section.clone(),
            }
        }
        crate::nir::NirGlobalInit::RoutineAddress {
            routine,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => MirGlobalInit::RoutineAddress {
            routine: RoutineId(*routine),
            descriptor_size: nir_size_u16(*descriptor_size),
            size_word: *size_word,
            mutable: *mutable,
            section: section.clone(),
        },
    }
}

fn lower_local_storage_init(
    local: &crate::nir::NirLocal,
    routine_ids: &BTreeMap<&str, RoutineId>,
    owner: RoutineId,
) -> Option<MirStorageInit> {
    if let Some(name) = local_pointer_init_symbol(local)
        && let Some(routine) = routine_ids.get(name.as_str()).copied()
    {
        return Some(MirStorageInit::RoutineAddress {
            routine,
            descriptor_size: 2,
            size_word: None,
            mutable: true,
            section: "local".to_string(),
        });
    }
    local
        .init
        .as_ref()
        .map(|init| lower_storage_init(init, owner))
}

fn lower_storage_init(init: &crate::nir::NirStorageInit, owner: RoutineId) -> MirStorageInit {
    match init {
        crate::nir::NirStorageInit::Bytes {
            image,
            zero_fill,
            mutable,
            section,
        } => MirStorageInit::Bytes {
            image: lower_data_image(image, Some(owner)),
            zero_fill: nir_size_u16(*zero_fill),
            mutable: *mutable,
            section: section.clone(),
        },
        crate::nir::NirStorageInit::Descriptor {
            backing,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => MirStorageInit::Descriptor {
            backing: MirStorageBacking {
                image: lower_data_image(&backing.image, Some(owner)),
                zero_fill: nir_size_u16(backing.zero_fill),
                section: backing.section.clone(),
            },
            descriptor_size: nir_size_u16(*descriptor_size),
            size_word: *size_word,
            mutable: *mutable,
            section: section.clone(),
        },
        crate::nir::NirStorageInit::ZeroFill {
            bytes,
            mutable,
            section,
        } => MirStorageInit::ZeroFill {
            bytes: nir_size_u16(*bytes),
            mutable: *mutable,
            section: section.clone(),
        },
    }
}

fn lower_data_image(image: &crate::nir::NirDataImage, owner: Option<RoutineId>) -> MirDataImage {
    MirDataImage {
        bytes: image
            .project_constants(crate::target::Endian::Little)
            .expect("verified NIR data fragments must project to Atari bytes"),
        relocations: image
            .fragments
            .iter()
            .filter_map(|fragment| {
                let crate::nir::NirDataFragment::Address {
                    offset,
                    encoding,
                    target,
                    addend,
                    span,
                } = fragment
                else {
                    return None;
                };
                Some(MirDataRelocation {
                offset: nir_offset_u16(*offset),
                kind: match encoding {
                    crate::nir::NirDataAddressEncoding::TargetByte {
                        target: crate::target::TargetId::Atari6502,
                        byte_index: 0,
                    } => MirDataRelocationKind::Low8,
                    crate::nir::NirDataAddressEncoding::TargetByte {
                        target: crate::target::TargetId::Atari6502,
                        byte_index: 1,
                    } => MirDataRelocationKind::High8,
                    crate::nir::NirDataAddressEncoding::Pointer { width, .. }
                        if *width == ByteSize::new(2) => MirDataRelocationKind::Word16,
                    _ => unreachable!("verified Atari address fragment encoding"),
                },
                target: match target {
                    crate::nir::NirDataAddressTarget::Storage(
                        crate::nir::NirStorageId::Global(id),
                    ) => MirDataRelocationTarget::Global(*id),
                    crate::nir::NirDataAddressTarget::Storage(
                        crate::nir::NirStorageId::Local(id),
                    ) => MirDataRelocationTarget::Local {
                        routine: owner.expect("verified local data relocation has an owner"),
                        id: *id,
                    },
                    crate::nir::NirDataAddressTarget::Storage(
                        crate::nir::NirStorageId::Param(id),
                    ) => MirDataRelocationTarget::Param {
                        routine: owner.expect("verified parameter data relocation has an owner"),
                        id: *id,
                    },
                    crate::nir::NirDataAddressTarget::Routine(id) => {
                        MirDataRelocationTarget::Routine(RoutineId(*id))
                    }
                    crate::nir::NirDataAddressTarget::Absolute(address) => {
                        MirDataRelocationTarget::Absolute(nir_address_u16(*address))
                    }
                },
                addend: i32::try_from(*addend).expect("verified Atari relocation addend"),
                span: *span,
            })
            })
            .collect(),
    }
}

#[derive(Debug, Clone)]
struct MirAddrDef {
    mem: MirMem,
    pointer_backed: bool,
}

fn mem_is_pointer_backed_array(
    mem: &MirMem,
    global_array_pointer_backing: &BTreeMap<crate::nir::SymbolId, bool>,
    local_array_pointer_backing: &[LocalId],
) -> bool {
    match mem {
        MirMem::Global { id, offset: 0 } => global_array_pointer_backing
            .get(id)
            .copied()
            .unwrap_or(false),
        MirMem::Local { id, offset: 0 } => local_array_pointer_backing.contains(id),
        _ => false,
    }
}

fn pointer_backed_direct_place_store_width(
    place: &NirPlace,
    global_array_pointer_backing: &BTreeMap<crate::nir::SymbolId, bool>,
    local_array_pointer_backing: &[LocalId],
    src_width: Option<MirWidth>,
) -> Option<MirWidth> {
    if src_width != Some(MirWidth::Word) {
        return None;
    }
    match &place.kind {
        NirPlaceKind::Global { id, .. } => global_array_pointer_backing
            .get(id)
            .copied()
            .unwrap_or(false)
            .then_some(MirWidth::Word),
        NirPlaceKind::Local { id, .. } => local_array_pointer_backing
            .contains(id)
            .then_some(MirWidth::Word),
        _ => None,
    }
}

fn lower_return_value_ops(
    routine: &str,
    block: &str,
    return_width: Option<MirWidth>,
    terminator: &NirTerminator,
    ops: &mut Vec<MirOp>,
    diagnostics: &mut Vec<MirDiagnostic>,
) {
    let NirTerminator::Return(Some(value)) = terminator else {
        return;
    };
    let Some(value_width) = value_width(value) else {
        diagnostics.push(MirDiagnostic::block(
            routine,
            block,
            "return value has unsupported MIR6502 width",
        ));
        return;
    };
    let width = return_width.unwrap_or(value_width);
    let Some(src) = lower_value(routine, block, value, diagnostics) else {
        return;
    };
    if width == MirWidth::Word && value_width == MirWidth::Byte {
        ops.push(MirOp::Store {
            dst: MirAddr::Direct(return_slot_mem(0)),
            src,
            width: MirWidth::Byte,
        });
        ops.push(MirOp::Store {
            dst: MirAddr::Direct(return_slot_mem(1)),
            src: MirValue::ConstU8(0),
            width: MirWidth::Byte,
        });
    } else {
        ops.push(MirOp::Store {
            dst: MirAddr::Direct(return_slot_mem(0)),
            src,
            width,
        });
    }
}

fn return_slot_mem(offset: u16) -> MirMem {
    MirMem::FixedZeroPage(MirFixedZpSlot(
        runtime_zp::ARGS.address().wrapping_add(offset as u8),
    ))
}

fn fixed_zero_page_slots(blocks: &[MirBlock]) -> Vec<MirFixedZpSlot> {
    let mut slots = Vec::new();
    for block in blocks {
        for op in &block.ops {
            collect_op_fixed_zero_page(op, &mut slots);
        }
    }
    slots
}

fn collect_op_fixed_zero_page(op: &MirOp, slots: &mut Vec<MirFixedZpSlot>) {
    match op {
        MirOp::Load {
            src: MirAddr::Direct(mem),
            ..
        } => collect_mem_fixed_zero_page(mem, slots),
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            ..
        } => collect_mem_fixed_zero_page(mem, slots),
        MirOp::PackedRealCompare { .. } => {
            for slot in 0xD4..=0xD9 {
                collect_mem_fixed_zero_page(&MirMem::FixedZeroPage(MirFixedZpSlot(slot)), slots);
            }
            for slot in 0xE0..=0xE5 {
                collect_mem_fixed_zero_page(&MirMem::FixedZeroPage(MirFixedZpSlot(slot)), slots);
            }
        }
        MirOp::PackedRealCopy {
            source,
            destination,
            source_offset,
            destination_offset,
            ..
        } => {
            collect_packed_real_fixed_zero_page(source, *source_offset, slots);
            collect_packed_real_fixed_zero_page(destination, *destination_offset, slots);
        }
        MirOp::Call { effects, .. }
        | MirOp::RuntimeHelper { effects, .. }
        | MirOp::Barrier { effects }
        | MirOp::MachineBlock { effects, .. } => {
            collect_effect_fixed_zero_page(&effects.memory_reads, slots);
            collect_effect_fixed_zero_page(&effects.memory_writes, slots);
        }
        _ => {}
    }
}

fn collect_effect_fixed_zero_page(effect: &MirMemoryEffect, slots: &mut Vec<MirFixedZpSlot>) {
    let MirMemoryEffect::Regions(regions) = effect else {
        return;
    };
    for region in regions
        .iter()
        .filter(|region| region.kind == MirMemoryRegionKind::ZeroPage)
    {
        let end = region.offset.saturating_add(region.size).min(0x100);
        for address in region.offset.min(0x100)..end {
            collect_mem_fixed_zero_page(
                &MirMem::FixedZeroPage(MirFixedZpSlot(address as u8)),
                slots,
            );
        }
    }
}

fn collect_packed_real_fixed_zero_page(
    addr: &MirAddr,
    base_offset: u16,
    slots: &mut Vec<MirFixedZpSlot>,
) {
    if let MirAddr::Direct(MirMem::FixedZeroPage(slot)) = addr {
        for lane in 0..ATARI_REAL_BYTES {
            collect_mem_fixed_zero_page(
                &MirMem::FixedZeroPage(MirFixedZpSlot(
                    slot.0
                        .saturating_add(base_offset.saturating_add(lane) as u8),
                )),
                slots,
            );
        }
    }
}

fn collect_mem_fixed_zero_page(mem: &MirMem, slots: &mut Vec<MirFixedZpSlot>) {
    if let MirMem::FixedZeroPage(slot) = mem
        && !slots.contains(slot)
    {
        slots.push(*slot);
    }
}

fn lower_ops(
    routine: &str,
    block: &str,
    ops: &[NirOpKind],
    routine_ids: &BTreeMap<&str, RoutineId>,
    routine_system_addresses_by_id: &BTreeMap<u32, u16>,
    routine_system_addresses: &BTreeMap<&str, u16>,
    global_array_pointer_backing: &BTreeMap<crate::nir::SymbolId, bool>,
    local_array_pointer_backing: &[LocalId],
    local_absolute_addresses: &BTreeMap<String, u16>,
    machine_numeric_defines: &BTreeMap<String, u16>,
    machine_blocks: &mut Vec<MirMachineBlock>,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    next_generated_local: &mut u32,
    generated_locals: &mut Vec<(LocalId, String)>,
    direct_real_branch_result: Option<TempId>,
    real_branch_values: &mut BTreeMap<u32, LoweredRealBranch>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Vec<MirOp> {
    let mut lowered = Vec::new();
    let mut addr_defs = BTreeMap::<TempId, MirAddrDef>::new();
    for op in ops {
        match op {
            NirOpKind::RuntimeHelperOverride { .. } => {}
            NirOpKind::Load { dest, ty, place } | NirOpKind::VolatileLoad { dest, ty, place } => {
                let is_volatile = matches!(op, NirOpKind::VolatileLoad { .. });
                let Some(width) = mir_width(ty) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        format!("unsupported load width `{}`", ty.summary),
                    ));
                    continue;
                };
                let Some(src) = lower_place_addr(routine, block, place, &addr_defs, diagnostics)
                else {
                    continue;
                };
                if is_volatile {
                    lowered.push(volatile_memory_barrier());
                }
                lowered.push(MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(dest.0)),
                    src,
                    width,
                });
                if is_volatile {
                    lowered.push(volatile_memory_barrier());
                }
            }
            NirOpKind::Store { place, src, ty } | NirOpKind::VolatileStore { place, src, ty } => {
                let is_volatile = matches!(op, NirOpKind::VolatileStore { .. });
                let src_width = value_width(src);
                let Some(declared_width) = mir_width(ty) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        format!("unsupported store width `{}`", ty.summary),
                    ));
                    continue;
                };
                let Some(dst) = lower_place_addr(routine, block, place, &addr_defs, diagnostics)
                else {
                    continue;
                };
                let Some(mut src_value) = lower_value(routine, block, src, diagnostics) else {
                    continue;
                };
                if let Some(addr_def) = addr_temp_def(src, &addr_defs)
                    && addr_def.pointer_backed
                {
                    src_value = MirValue::PointerCell(addr_def.mem.clone());
                }
                let width = pointer_backed_direct_place_store_width(
                    place,
                    global_array_pointer_backing,
                    local_array_pointer_backing,
                    src_width,
                )
                .unwrap_or(declared_width);
                if is_volatile {
                    lowered.push(volatile_memory_barrier());
                }
                lowered.push(MirOp::Store {
                    dst,
                    src: src_value,
                    width,
                });
                if is_volatile {
                    lowered.push(volatile_memory_barrier());
                }
            }
            NirOpKind::CopyBytes {
                destination,
                source,
                size,
                destination_volatile,
                source_volatile,
            } => {
                let Some(destination) =
                    lower_place_addr(routine, block, destination, &addr_defs, diagnostics)
                else {
                    continue;
                };
                let Some(source) =
                    lower_place_addr(routine, block, source, &addr_defs, diagnostics)
                else {
                    continue;
                };
                lowered.push(MirOp::CopyBytes {
                    destination,
                    source,
                    size: nir_size_u16(*size),
                    destination_volatile: *destination_volatile,
                    source_volatile: *source_volatile,
                });
            }
            NirOpKind::Cast {
                dest,
                src,
                from,
                to,
                ..
            } => {
                let Some(from_width) = mir_width(from) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        format!("unsupported cast source width `{}`", from.summary),
                    ));
                    continue;
                };
                let Some(to_width) = mir_width(to) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        format!("unsupported cast target width `{}`", to.summary),
                    ));
                    continue;
                };
                let Some(src) = lower_value(routine, block, src, diagnostics) else {
                    continue;
                };
                let dst = MirDef::VTemp(MirTempId(dest.0));
                if from_width == to_width {
                    lowered.push(MirOp::Move {
                        dst,
                        src,
                        width: to_width,
                    });
                } else if from_width == MirWidth::Byte && to_width == MirWidth::Word {
                    lowered.push(MirOp::Extend {
                        dst,
                        src,
                        from_width,
                        to_width,
                        signed: is_signed(from),
                    });
                } else if from_width == MirWidth::Word && to_width == MirWidth::Byte {
                    lowered.push(MirOp::Truncate {
                        dst,
                        src,
                        from_width,
                        to_width,
                    });
                } else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        "unsupported cast width transition",
                    ));
                }
            }
            NirOpKind::AddrOf { dest, ty, place } => {
                let Some(width) = mir_width(ty) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        format!("unsupported address width `{}`", ty.summary),
                    ));
                    continue;
                };
                let Some(target) = lower_place_mem(routine, block, place, diagnostics) else {
                    continue;
                };
                addr_defs.insert(
                    *dest,
                    MirAddrDef {
                        pointer_backed: mem_is_pointer_backed_array(
                            &target,
                            global_array_pointer_backing,
                            local_array_pointer_backing,
                        ),
                        mem: target.clone(),
                    },
                );
                lowered.push(MirOp::LeaAddr {
                    dst: MirDef::VTemp(MirTempId(dest.0)),
                    target,
                    width,
                });
            }
            NirOpKind::Unary { dest, ty, op, src } => {
                let Some(width) = mir_width(ty) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        format!("unsupported unary width `{}`", ty.summary),
                    ));
                    continue;
                };
                let Some(src) = lower_value(routine, block, src, diagnostics) else {
                    continue;
                };
                let dst = MirDef::VTemp(MirTempId(dest.0));
                match op {
                    NirUnaryOp::Plus => lowered.push(MirOp::Move { dst, src, width }),
                    NirUnaryOp::Neg => lowered.push(MirOp::Unary {
                        op: MirUnaryOp::Neg,
                        dst,
                        src,
                        width,
                    }),
                }
            }
            NirOpKind::Binary {
                dest,
                ty,
                op,
                left,
                right,
            } => {
                let Some(width) = mir_width(ty) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        format!("unsupported binary width `{}`", ty.summary),
                    ));
                    continue;
                };
                let Some(left) = lower_value(routine, block, left, diagnostics) else {
                    continue;
                };
                let Some(right) = lower_value(routine, block, right, diagnostics) else {
                    continue;
                };
                lowered.push(MirOp::Binary {
                    op: mir_binary_op(*op),
                    dst: MirDef::VTemp(MirTempId(dest.0)),
                    left,
                    right,
                    width,
                    carry_in: None,
                    carry_out: MirCarryOut::Ignore,
                });
            }
            NirOpKind::PointerOffset {
                dest,
                ty,
                base,
                offset,
                subtract,
            } => {
                let Some(width) = mir_width(ty) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        format!("unsupported pointer width `{}`", ty.summary),
                    ));
                    continue;
                };
                let Some(left) = lower_value(routine, block, base, diagnostics) else {
                    continue;
                };
                let Some(right) = lower_value(routine, block, offset, diagnostics) else {
                    continue;
                };
                lowered.push(MirOp::Binary {
                    op: if *subtract { MirBinaryOp::Sub } else { MirBinaryOp::Add },
                    dst: MirDef::VTemp(MirTempId(dest.0)),
                    left,
                    right,
                    width,
                    carry_in: None,
                    carry_out: MirCarryOut::Ignore,
                });
            }
            NirOpKind::Compare {
                dest,
                operand_ty,
                op,
                left,
                right,
                ..
            } => {
                let Some(width) = mir_width(operand_ty) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        "unsupported compare width",
                    ));
                    continue;
                };
                let signed = is_signed(operand_ty);
                let Some(left) = lower_compare_value(routine, block, left, width, diagnostics)
                else {
                    continue;
                };
                let Some(right) = lower_compare_value(routine, block, right, width, diagnostics)
                else {
                    continue;
                };
                lowered.push(MirOp::Compare {
                    dst: MirCondDest::Temp(MirTempId(dest.0)),
                    op: mir_compare_op(*op),
                    left,
                    right,
                    width,
                    signed,
                });
            }
            NirOpKind::Call {
                callee,
                args,
                result,
                signature,
                effects,
            } => {
                let Some(signature) = signature else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        "call is missing signature facts",
                    ));
                    continue;
                };
                if signature.variadic.is_none() && args.len() > signature.params.len() {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        "call argument count does not match signature",
                    ));
                    continue;
                }
                if signature.variadic.is_some() && args.len() < signature.params.len() {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        "call argument count does not match signature",
                    ));
                    continue;
                }
                let mut lowered_args = Vec::new();
                let mut args_ok = true;
                for (index, arg) in args.iter().enumerate() {
                    let Some(mut value) = lower_value(routine, block, arg, diagnostics) else {
                        args_ok = false;
                        continue;
                    };
                    if let Some(addr_def) = addr_temp_def(arg, &addr_defs)
                        && addr_def.pointer_backed
                    {
                        value = MirValue::PointerCell(addr_def.mem.clone());
                    }
                    let expected_ty = signature.params.get(index).or(signature.variadic.as_ref());
                    let Some(width) = expected_ty.and_then(mir_width).or_else(|| value_width(arg))
                    else {
                        diagnostics.push(MirDiagnostic::block(
                            routine,
                            block,
                            "call argument has unsupported MIR6502 width",
                        ));
                        args_ok = false;
                        continue;
                    };
                    if width == MirWidth::Word && value_width(arg) == Some(MirWidth::Byte) {
                        value = MirValue::Word {
                            lo: Box::new(value),
                            hi: Box::new(MirValue::ConstU8(0)),
                        };
                    }
                    lowered_args.push((value, width));
                }
                if !args_ok {
                    continue;
                }
                let lowered_result = match result {
                    Some(result) => {
                        let Some(width) = mir_width(&result.ty) else {
                            diagnostics.push(MirDiagnostic::block(
                                routine,
                                block,
                                "call result has unsupported MIR6502 width",
                            ));
                            continue;
                        };
                        Some((MirDef::VTemp(MirTempId(result.dest.0)), width))
                    }
                    None => None,
                };
                let indirect_target = match callee {
                    crate::nir::NirCallee::Indirect { target, ty } => {
                        let Some(value) = lower_value(routine, block, target, diagnostics) else {
                            continue;
                        };
                        let Some(width) = mir_width(ty).or_else(|| value_width(target)) else {
                            diagnostics.push(MirDiagnostic::block(
                                routine,
                                block,
                                "indirect call target has unsupported MIR6502 width",
                            ));
                            continue;
                        };
                        Some((value, width))
                    }
                    _ => None,
                };
                let Some(plan) = call_plan::plan_call(
                    routine,
                    block,
                    callee,
                    signature,
                    &lowered_args,
                    lowered_result,
                    indirect_target,
                    effects,
                    routine_ids,
                    routine_system_addresses_by_id,
                    diagnostics,
                ) else {
                    continue;
                };
                lowered.push(MirOp::Call {
                    target: plan.target,
                    abi: plan.abi,
                    args: plan.args,
                    result: plan.result,
                    effects: plan.effects,
                });
            }
            NirOpKind::MachineBlock { items, effects } => {
                let Some(items) = lower_machine_items(
                    routine,
                    block,
                    items,
                    local_absolute_addresses,
                    routine_system_addresses,
                    machine_numeric_defines,
                    diagnostics,
                ) else {
                    continue;
                };
                let id = MirMachineBlockId(machine_blocks.len() as u32);
                machine_blocks.push(MirMachineBlock { id, items });
                lowered.push(MirOp::MachineBlock {
                    id,
                    effects: lower_machine_effects(effects),
                });
            }
            NirOpKind::InlineAsm { code, effects } => {
                let items = lower_inline_asm(code);
                let id = MirMachineBlockId(machine_blocks.len() as u32);
                machine_blocks.push(MirMachineBlock { id, items });
                lowered.push(MirOp::MachineBlock {
                    id,
                    effects: lower_inline_asm_effects(code, effects),
                });
            }
            NirOpKind::Real(real) => lower_real_op(
                routine,
                block,
                real,
                &addr_defs,
                next_generated_temp,
                generated_temps,
                next_generated_local,
                generated_locals,
                direct_real_branch_result,
                real_branch_values,
                &mut lowered,
                diagnostics,
            ),
            _ => {}
        }
    }
    lowered
}

const ATARI_FPP_FR0: MirFixedZpSlot = MirFixedZpSlot(0xD4);
const ATARI_FPP_FR1: MirFixedZpSlot = MirFixedZpSlot(0xE0);
const ATARI_REAL_BYTES: u16 = 6;

fn eliminate_adjacent_fpp_result_round_trips(
    blocks: &mut [MirBlock],
    candidates: &BTreeSet<LocalId>,
) -> BTreeSet<LocalId> {
    let mut eliminated = BTreeSet::new();
    for block in blocks {
        let mut index = 0;
        while index + 1 < block.ops.len() {
            let Some(local) =
                adjacent_fpp_result_round_trip(&block.ops[index], &block.ops[index + 1])
            else {
                index += 1;
                continue;
            };
            if !candidates.contains(&local) {
                index += 1;
                continue;
            }
            block.ops.drain(index..index + 2);
            eliminated.insert(local);
        }
    }
    eliminated
}

fn adjacent_fpp_result_round_trip(first: &MirOp, second: &MirOp) -> Option<LocalId> {
    let MirOp::PackedRealCopy {
        source: MirAddr::Direct(MirMem::FixedZeroPage(first_source)),
        destination:
            MirAddr::Direct(MirMem::Local {
                id: local,
                offset: first_local_offset,
            }),
        source_offset: first_source_offset,
        destination_offset: first_destination_offset,
        negate: first_negate,
    } = first
    else {
        return None;
    };
    let MirOp::PackedRealCopy {
        source:
            MirAddr::Direct(MirMem::Local {
                id: second_local,
                offset: second_local_offset,
            }),
        destination: MirAddr::Direct(MirMem::FixedZeroPage(second_destination)),
        source_offset: second_source_offset,
        destination_offset: second_destination_offset,
        negate: second_negate,
    } = second
    else {
        return None;
    };
    (*first_source == ATARI_FPP_FR0
        && *second_destination == ATARI_FPP_FR0
        && local == second_local
        && *first_local_offset == 0
        && *second_local_offset == 0
        && *first_source_offset == 0
        && *first_destination_offset == 0
        && *second_source_offset == 0
        && *second_destination_offset == 0
        && !*first_negate
        && !*second_negate)
        .then_some(*local)
}

fn eliminate_adjacent_fpp_right_result_round_trips(
    blocks: &mut [MirBlock],
    ordered_candidates: &BTreeSet<LocalId>,
    commutative_candidates: &BTreeSet<LocalId>,
) -> BTreeSet<LocalId> {
    let mut eliminated = BTreeSet::new();
    for block in blocks {
        let mut index = 0;
        while index + 2 < block.ops.len() {
            let Some((local, mut left_staging)) = adjacent_fpp_right_result_round_trip(
                &block.ops[index],
                &block.ops[index + 1],
                &block.ops[index + 2],
            ) else {
                index += 1;
                continue;
            };
            let replacement = if commutative_candidates.contains(&local) {
                let MirOp::PackedRealCopy { destination, .. } = &mut left_staging else {
                    unreachable!("right-result matcher returns packed REAL staging");
                };
                *destination = MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR1));
                vec![left_staging]
            } else if ordered_candidates.contains(&local) {
                vec![
                    MirOp::PackedRealCopy {
                        source: MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
                        destination: MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR1)),
                        source_offset: 0,
                        destination_offset: 0,
                        negate: false,
                    },
                    left_staging,
                ]
            } else {
                index += 1;
                continue;
            };
            block.ops.splice(index..index + 3, replacement);
            eliminated.insert(local);
            index += 1;
        }
    }
    eliminated
}

fn adjacent_fpp_right_result_round_trip(
    first: &MirOp,
    second: &MirOp,
    third: &MirOp,
) -> Option<(LocalId, MirOp)> {
    let MirOp::PackedRealCopy {
        source: MirAddr::Direct(MirMem::FixedZeroPage(first_source)),
        destination:
            MirAddr::Direct(MirMem::Local {
                id: local,
                offset: first_local_offset,
            }),
        source_offset: first_source_offset,
        destination_offset: first_destination_offset,
        negate: first_negate,
    } = first
    else {
        return None;
    };
    let MirOp::PackedRealCopy {
        destination: MirAddr::Direct(MirMem::FixedZeroPage(second_destination)),
        destination_offset: second_destination_offset,
        ..
    } = second
    else {
        return None;
    };
    let MirOp::PackedRealCopy {
        source:
            MirAddr::Direct(MirMem::Local {
                id: third_local,
                offset: third_local_offset,
            }),
        destination: MirAddr::Direct(MirMem::FixedZeroPage(third_destination)),
        source_offset: third_source_offset,
        destination_offset: third_destination_offset,
        negate: third_negate,
    } = third
    else {
        return None;
    };
    (*first_source == ATARI_FPP_FR0
        && *second_destination == ATARI_FPP_FR0
        && *third_destination == ATARI_FPP_FR1
        && local == third_local
        && *first_local_offset == 0
        && *third_local_offset == 0
        && *first_source_offset == 0
        && *first_destination_offset == 0
        && *second_destination_offset == 0
        && *third_source_offset == 0
        && *third_destination_offset == 0
        && !*first_negate
        && !*third_negate)
        .then(|| (*local, second.clone()))
}

fn forward_adjacent_real_temp_copies(
    blocks: &mut [MirBlock],
    candidates: &BTreeSet<LocalId>,
) -> BTreeSet<LocalId> {
    let mut eliminated = BTreeSet::new();
    for block in blocks {
        let mut index = 0;
        while index + 1 < block.ops.len() {
            if let Some((local, replacements)) = block.ops.get(index..index + 3).and_then(|ops| {
                let [producer, staging, consumer] = ops else {
                    return None;
                };
                real_temp_copy_around_static_fr0_stage(producer, staging, consumer)
            }) && candidates.contains(&local)
            {
                block.ops.splice(index..index + 3, replacements);
                eliminated.insert(local);
                index += 2;
                continue;
            }
            let Some((local, replacement)) =
                adjacent_real_temp_copy_forward(&block.ops[index], &block.ops[index + 1])
            else {
                index += 1;
                continue;
            };
            if !candidates.contains(&local) {
                index += 1;
                continue;
            }
            block.ops.splice(index..index + 2, [replacement]);
            eliminated.insert(local);
            index += 1;
        }
    }
    eliminated
}

fn real_temp_copy_around_static_fr0_stage(
    producer: &MirOp,
    staging: &MirOp,
    consumer: &MirOp,
) -> Option<(LocalId, [MirOp; 2])> {
    let (local, forwarded) = adjacent_real_temp_copy_forward(producer, consumer)?;
    let MirOp::PackedRealCopy {
        source: MirAddr::Direct(MirMem::Static { .. }),
        destination: MirAddr::Direct(MirMem::FixedZeroPage(staging_destination)),
        source_offset: staging_source_offset,
        destination_offset: staging_destination_offset,
        ..
    } = staging
    else {
        return None;
    };
    let MirOp::PackedRealCopy {
        destination: MirAddr::Direct(MirMem::FixedZeroPage(forwarded_destination)),
        destination_offset: forwarded_destination_offset,
        ..
    } = &forwarded
    else {
        return None;
    };
    if *staging_destination != ATARI_FPP_FR0
        || *forwarded_destination != ATARI_FPP_FR1
        || *staging_source_offset != 0
        || *staging_destination_offset != 0
        || *forwarded_destination_offset != 0
    {
        return None;
    }

    Some((local, [forwarded, staging.clone()]))
}

fn forward_static_real_temp_negations(
    blocks: &mut [MirBlock],
    candidates: &BTreeSet<LocalId>,
) -> BTreeSet<LocalId> {
    let mut eliminated = BTreeSet::new();
    for block in blocks {
        for local in candidates {
            let Some((producer_index, static_source)) =
                block.ops.iter().enumerate().find_map(|(index, op)| {
                    static_real_temp_negation(op, *local).map(|source| (index, source))
                })
            else {
                continue;
            };
            let Some((consumer_index, replacement)) = block
                .ops
                .iter()
                .enumerate()
                .skip(producer_index + 1)
                .find_map(|(index, op)| {
                    forwarded_static_real_negation(op, *local, &static_source)
                        .map(|replacement| (index, replacement))
                })
            else {
                continue;
            };
            block.ops[consumer_index] = replacement;
            block.ops.remove(producer_index);
            eliminated.insert(*local);
        }
    }
    eliminated
}

fn static_real_temp_negation(op: &MirOp, candidate: LocalId) -> Option<MirAddr> {
    let MirOp::PackedRealCopy {
        source: source @ MirAddr::Direct(MirMem::Static { .. }),
        destination:
            MirAddr::Direct(MirMem::Local {
                id,
                offset: local_offset,
            }),
        source_offset,
        destination_offset,
        negate,
    } = op
    else {
        return None;
    };
    (*id == candidate
        && *local_offset == 0
        && *source_offset == 0
        && *destination_offset == 0
        && *negate)
        .then(|| source.clone())
}

fn forwarded_static_real_negation(
    op: &MirOp,
    candidate: LocalId,
    static_source: &MirAddr,
) -> Option<MirOp> {
    let MirOp::PackedRealCopy {
        source:
            MirAddr::Direct(MirMem::Local {
                id,
                offset: local_offset,
            }),
        destination,
        source_offset,
        destination_offset,
        negate,
    } = op
    else {
        return None;
    };
    (*id == candidate && *local_offset == 0 && *source_offset == 0 && !*negate).then(|| {
        MirOp::PackedRealCopy {
            source: static_source.clone(),
            destination: destination.clone(),
            source_offset: 0,
            destination_offset: *destination_offset,
            negate: true,
        }
    })
}

fn adjacent_real_temp_copy_forward(first: &MirOp, second: &MirOp) -> Option<(LocalId, MirOp)> {
    let MirOp::PackedRealCopy {
        source,
        destination:
            MirAddr::Direct(MirMem::Local {
                id: local,
                offset: first_local_offset,
            }),
        source_offset,
        destination_offset: first_destination_offset,
        negate: first_negate,
    } = first
    else {
        return None;
    };
    let MirOp::PackedRealCopy {
        source:
            MirAddr::Direct(MirMem::Local {
                id: second_local,
                offset: second_local_offset,
            }),
        destination,
        source_offset: second_source_offset,
        destination_offset,
        negate,
    } = second
    else {
        return None;
    };
    if local != second_local
        || *first_local_offset != 0
        || *second_local_offset != 0
        || *first_destination_offset != 0
        || *second_source_offset != 0
        || *first_negate
    {
        return None;
    }
    Some((
        *local,
        MirOp::PackedRealCopy {
            source: source.clone(),
            destination: destination.clone(),
            source_offset: *source_offset,
            destination_offset: *destination_offset,
            negate: *negate,
        },
    ))
}

fn lower_real_op(
    routine: &str,
    block: &str,
    op: &NirRealOp,
    addr_defs: &BTreeMap<TempId, MirAddrDef>,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    next_generated_local: &mut u32,
    generated_locals: &mut Vec<(LocalId, String)>,
    direct_real_branch_result: Option<TempId>,
    real_branch_values: &mut BTreeMap<u32, LoweredRealBranch>,
    lowered: &mut Vec<MirOp>,
    diagnostics: &mut Vec<MirDiagnostic>,
) {
    match op {
        NirRealOp::Copy {
            destination,
            source,
        } => {
            let Some(destination) =
                lower_place_addr(routine, block, destination, addr_defs, diagnostics)
            else {
                return;
            };
            let source = lower_real_source_addr(routine, block, source, addr_defs, diagnostics);
            let Some(source) = source else {
                return;
            };
            push_staged_real_copy(&source, &destination, lowered);
        }
        NirRealOp::Binary {
            operation,
            destination,
            left,
            right,
        } => {
            let Some(service) = real_binary_service(*operation) else {
                diagnostics.push(MirDiagnostic::block(
                    routine,
                    block,
                    "unsupported native REAL arithmetic operation",
                ));
                return;
            };
            let Some(destination) =
                lower_place_addr(routine, block, destination, addr_defs, diagnostics)
            else {
                return;
            };
            let Some(left) = lower_real_source_addr(routine, block, left, addr_defs, diagnostics)
            else {
                return;
            };
            let Some(right) = lower_real_source_addr(routine, block, right, addr_defs, diagnostics)
            else {
                return;
            };
            push_staged_real_copy(
                &left,
                &MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
                lowered,
            );
            push_staged_real_copy(
                &right,
                &MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR1)),
                lowered,
            );
            lowered.push(atari_fpp_call(service));
            push_staged_real_copy(
                &MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
                &destination,
                lowered,
            );
        }
        NirRealOp::IntegerToReal {
            destination,
            source,
            source_type,
        } => {
            let Some(destination) =
                lower_place_addr(routine, block, destination, addr_defs, diagnostics)
            else {
                return;
            };
            if let Some(bytes) = constant_integer_real_bytes(source, source_type) {
                for (offset, byte) in bytes.into_iter().enumerate() {
                    lowered.push(MirOp::Store {
                        dst: offset_addr(&destination, offset as u16),
                        src: MirValue::ConstU8(byte),
                        width: MirWidth::Byte,
                    });
                }
                return;
            }
            let Some(source) = lower_value(routine, block, source, diagnostics) else {
                return;
            };
            push_integer_to_real(
                source,
                source_type,
                &destination,
                next_generated_temp,
                generated_temps,
                next_generated_local,
                generated_locals,
                lowered,
            );
        }
        NirRealOp::Unary {
            operation,
            destination,
            operand,
        } => {
            let Some(destination) =
                lower_place_addr(routine, block, destination, addr_defs, diagnostics)
            else {
                return;
            };
            let Some(operand) =
                lower_real_source_addr(routine, block, operand, addr_defs, diagnostics)
            else {
                return;
            };
            match operation {
                NirUnaryOp::Plus => push_staged_real_copy(&operand, &destination, lowered),
                NirUnaryOp::Neg => push_packed_real_copy(&operand, &destination, true, lowered),
            }
        }
        NirRealOp::Compare {
            predicate,
            result,
            left,
            right,
            ..
        } => {
            let Some(left) = lower_real_source_addr(routine, block, left, addr_defs, diagnostics)
            else {
                return;
            };
            let Some(right) = lower_real_source_addr(routine, block, right, addr_defs, diagnostics)
            else {
                return;
            };
            if direct_real_branch_result == Some(*result) {
                push_staged_real_copy(
                    &left,
                    &MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
                    lowered,
                );
                push_staged_real_copy(
                    &right,
                    &MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR1)),
                    lowered,
                );
                lowered.push(MirOp::PackedRealCompare {
                    op: mir_compare_op(*predicate),
                });
                real_branch_values.insert(result.0, LoweredRealBranch::PackedFlags);
                return;
            }
            let answer = push_real_compare(
                *predicate,
                MirTempId(result.0),
                &left,
                &right,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let _ = answer;
            real_branch_values.insert(result.0, LoweredRealBranch::Boolean);
        }
        NirRealOp::RealToInteger {
            result,
            result_type,
            source,
        } => {
            let Some(source) = lower_place_addr(routine, block, source, addr_defs, diagnostics)
            else {
                return;
            };
            push_real_to_integer(
                &source,
                MirTempId(result.0),
                result_type,
                next_generated_temp,
                generated_temps,
                next_generated_local,
                generated_locals,
                lowered,
            );
        }
    }
}

fn generated_temp(next_generated_temp: &mut u32, generated_temps: &mut Vec<MirTemp>) -> MirTempId {
    let id = MirTempId(*next_generated_temp);
    *next_generated_temp = next_generated_temp.saturating_add(1);
    generated_temps.push(MirTemp { id });
    id
}

fn generated_scratch_local(
    next_generated_local: &mut u32,
    generated_locals: &mut Vec<(LocalId, String)>,
) -> MirMem {
    let id = LocalId(*next_generated_local);
    *next_generated_local = next_generated_local.saturating_add(1);
    let name = format!("__mir_real_scratch_{}", id.0);
    generated_locals.push((id, name));
    MirMem::Local { id, offset: 0 }
}

fn temp_value(id: MirTempId) -> MirValue {
    MirValue::Def(MirDef::VTemp(id))
}

fn push_generated_binary(
    operation: MirBinaryOp,
    left: MirValue,
    right: MirValue,
    width: MirWidth,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    lowered: &mut Vec<MirOp>,
) -> MirTempId {
    let result = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Binary {
        op: operation,
        dst: MirDef::VTemp(result),
        left,
        right,
        width,
        carry_in: None,
        carry_out: MirCarryOut::Ignore,
    });
    result
}

fn push_generated_compare(
    operation: MirCompareOp,
    left: MirValue,
    right: MirValue,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    lowered: &mut Vec<MirOp>,
) -> MirTempId {
    let result = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Compare {
        dst: MirCondDest::Temp(result),
        op: operation,
        left,
        right,
        width: MirWidth::Byte,
        signed: false,
    });
    result
}

fn push_bool_not(
    value: MirTempId,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    lowered: &mut Vec<MirOp>,
) -> MirTempId {
    push_generated_compare(
        MirCompareOp::Eq,
        temp_value(value),
        MirValue::ConstU8(0),
        next_generated_temp,
        generated_temps,
        lowered,
    )
}

fn push_bool_binary(
    operation: MirBinaryOp,
    left: MirTempId,
    right: MirTempId,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    lowered: &mut Vec<MirOp>,
) -> MirTempId {
    push_generated_binary(
        operation,
        temp_value(left),
        temp_value(right),
        MirWidth::Byte,
        next_generated_temp,
        generated_temps,
        lowered,
    )
}

fn push_real_byte_compare(
    operation: MirCompareOp,
    left: &MirAddr,
    right: &MirAddr,
    offset: u16,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    lowered: &mut Vec<MirOp>,
) -> MirTempId {
    let left_byte = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Load {
        dst: MirDef::VTemp(left_byte),
        src: offset_addr(left, offset),
        width: MirWidth::Byte,
    });
    let right_byte = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Load {
        dst: MirDef::VTemp(right_byte),
        src: offset_addr(right, offset),
        width: MirWidth::Byte,
    });
    push_generated_compare(
        operation,
        temp_value(left_byte),
        temp_value(right_byte),
        next_generated_temp,
        generated_temps,
        lowered,
    )
}

fn push_real_byte_const_compare(
    operation: MirCompareOp,
    source: &MirAddr,
    offset: u16,
    constant: u8,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    lowered: &mut Vec<MirOp>,
) -> MirTempId {
    let byte = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Load {
        dst: MirDef::VTemp(byte),
        src: offset_addr(source, offset),
        width: MirWidth::Byte,
    });
    push_generated_compare(
        operation,
        temp_value(byte),
        MirValue::ConstU8(constant),
        next_generated_temp,
        generated_temps,
        lowered,
    )
}

fn push_real_compare(
    predicate: NirCompareOp,
    result: MirTempId,
    left: &MirAddr,
    right: &MirAddr,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    lowered: &mut Vec<MirOp>,
) -> MirTempId {
    let equality_parts = (0..ATARI_REAL_BYTES)
        .map(|offset| {
            push_real_byte_compare(
                MirCompareOp::Eq,
                left,
                right,
                offset,
                next_generated_temp,
                generated_temps,
                lowered,
            )
        })
        .collect::<Vec<_>>();
    let equal = equality_parts
        .iter()
        .copied()
        .reduce(|left, right| {
            push_bool_binary(
                MirBinaryOp::And,
                left,
                right,
                next_generated_temp,
                generated_temps,
                lowered,
            )
        })
        .expect("REAL has at least one byte");

    let answer = match predicate {
        NirCompareOp::Eq => equal,
        NirCompareOp::Ne => push_bool_not(equal, next_generated_temp, generated_temps, lowered),
        NirCompareOp::Lt | NirCompareOp::Le | NirCompareOp::Gt | NirCompareOp::Ge => {
            let left_negative = push_real_byte_const_compare(
                MirCompareOp::Ge,
                left,
                0,
                0x80,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let right_negative = push_real_byte_const_compare(
                MirCompareOp::Ge,
                right,
                0,
                0x80,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let left_nonnegative =
                push_bool_not(left_negative, next_generated_temp, generated_temps, lowered);
            let right_nonnegative = push_bool_not(
                right_negative,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let negative_before_nonnegative = push_bool_binary(
                MirBinaryOp::And,
                left_negative,
                right_nonnegative,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let both_nonnegative = push_bool_binary(
                MirBinaryOp::And,
                left_nonnegative,
                right_nonnegative,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let both_negative = push_bool_binary(
                MirBinaryOp::And,
                left_negative,
                right_negative,
                next_generated_temp,
                generated_temps,
                lowered,
            );

            let mut prefix_equal = equality_parts[0];
            let mut lex_less = push_real_byte_compare(
                MirCompareOp::Lt,
                left,
                right,
                0,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let mut lex_greater = push_real_byte_compare(
                MirCompareOp::Lt,
                right,
                left,
                0,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            for index in 1..ATARI_REAL_BYTES as usize {
                let byte_less = push_real_byte_compare(
                    MirCompareOp::Lt,
                    left,
                    right,
                    index as u16,
                    next_generated_temp,
                    generated_temps,
                    lowered,
                );
                let byte_greater = push_real_byte_compare(
                    MirCompareOp::Lt,
                    right,
                    left,
                    index as u16,
                    next_generated_temp,
                    generated_temps,
                    lowered,
                );
                let first_less = push_bool_binary(
                    MirBinaryOp::And,
                    prefix_equal,
                    byte_less,
                    next_generated_temp,
                    generated_temps,
                    lowered,
                );
                let first_greater = push_bool_binary(
                    MirBinaryOp::And,
                    prefix_equal,
                    byte_greater,
                    next_generated_temp,
                    generated_temps,
                    lowered,
                );
                lex_less = push_bool_binary(
                    MirBinaryOp::Or,
                    lex_less,
                    first_less,
                    next_generated_temp,
                    generated_temps,
                    lowered,
                );
                lex_greater = push_bool_binary(
                    MirBinaryOp::Or,
                    lex_greater,
                    first_greater,
                    next_generated_temp,
                    generated_temps,
                    lowered,
                );
                prefix_equal = push_bool_binary(
                    MirBinaryOp::And,
                    prefix_equal,
                    equality_parts[index],
                    next_generated_temp,
                    generated_temps,
                    lowered,
                );
            }
            let positive_less = push_bool_binary(
                MirBinaryOp::And,
                both_nonnegative,
                lex_less,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let negative_less = push_bool_binary(
                MirBinaryOp::And,
                both_negative,
                lex_greater,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let same_sign_less = push_bool_binary(
                MirBinaryOp::Or,
                positive_less,
                negative_less,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            let less = push_bool_binary(
                MirBinaryOp::Or,
                negative_before_nonnegative,
                same_sign_less,
                next_generated_temp,
                generated_temps,
                lowered,
            );
            match predicate {
                NirCompareOp::Lt => less,
                NirCompareOp::Le => push_bool_binary(
                    MirBinaryOp::Or,
                    less,
                    equal,
                    next_generated_temp,
                    generated_temps,
                    lowered,
                ),
                NirCompareOp::Gt => {
                    let less_or_equal = push_bool_binary(
                        MirBinaryOp::Or,
                        less,
                        equal,
                        next_generated_temp,
                        generated_temps,
                        lowered,
                    );
                    push_bool_not(less_or_equal, next_generated_temp, generated_temps, lowered)
                }
                NirCompareOp::Ge => {
                    push_bool_not(less, next_generated_temp, generated_temps, lowered)
                }
                NirCompareOp::Eq | NirCompareOp::Ne => unreachable!(),
            }
        }
    };
    lowered.push(MirOp::Move {
        dst: MirDef::VTemp(result),
        src: temp_value(answer),
        width: MirWidth::Byte,
    });
    answer
}

fn push_integer_to_real(
    source: MirValue,
    source_type: &NirType,
    destination: &MirAddr,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    next_generated_local: &mut u32,
    generated_locals: &mut Vec<(LocalId, String)>,
    lowered: &mut Vec<MirOp>,
) {
    let byte_source = matches!(source_type.kind, NirTypeKind::U8 | NirTypeKind::I8);
    let signed = matches!(source_type.kind, NirTypeKind::I8 | NirTypeKind::I16);
    let source_word = if byte_source {
        let extended = generated_temp(next_generated_temp, generated_temps);
        lowered.push(MirOp::Extend {
            dst: MirDef::VTemp(extended),
            src: source,
            from_width: MirWidth::Byte,
            to_width: MirWidth::Word,
            signed,
        });
        temp_value(extended)
    } else {
        source
    };

    let (magnitude, sign_scratch) = if signed {
        let sign = generated_temp(next_generated_temp, generated_temps);
        lowered.push(MirOp::Compare {
            dst: MirCondDest::Temp(sign),
            op: MirCompareOp::Lt,
            left: source_word.clone(),
            right: MirValue::ConstU16(0),
            width: MirWidth::Word,
            signed: true,
        });
        let sign_word = generated_temp(next_generated_temp, generated_temps);
        lowered.push(MirOp::Extend {
            dst: MirDef::VTemp(sign_word),
            src: temp_value(sign),
            from_width: MirWidth::Byte,
            to_width: MirWidth::Word,
            signed: false,
        });
        let mask = generated_temp(next_generated_temp, generated_temps);
        lowered.push(MirOp::Unary {
            op: MirUnaryOp::Neg,
            dst: MirDef::VTemp(mask),
            src: temp_value(sign_word),
            width: MirWidth::Word,
        });
        let complemented = push_generated_binary(
            MirBinaryOp::Xor,
            source_word,
            temp_value(mask),
            MirWidth::Word,
            next_generated_temp,
            generated_temps,
            lowered,
        );
        let magnitude = push_generated_binary(
            MirBinaryOp::Add,
            temp_value(complemented),
            temp_value(sign_word),
            MirWidth::Word,
            next_generated_temp,
            generated_temps,
            lowered,
        );
        let sign_mask = generated_temp(next_generated_temp, generated_temps);
        lowered.push(MirOp::Unary {
            op: MirUnaryOp::Neg,
            dst: MirDef::VTemp(sign_mask),
            src: temp_value(sign),
            width: MirWidth::Byte,
        });
        let sign_bit = push_generated_binary(
            MirBinaryOp::And,
            temp_value(sign_mask),
            MirValue::ConstU8(0x80),
            MirWidth::Byte,
            next_generated_temp,
            generated_temps,
            lowered,
        );
        let sign_scratch = generated_scratch_local(next_generated_local, generated_locals);
        lowered.push(MirOp::Store {
            dst: MirAddr::Direct(sign_scratch.clone()),
            src: temp_value(sign_bit),
            width: MirWidth::Byte,
        });
        (temp_value(magnitude), Some(sign_scratch))
    } else {
        (source_word, None)
    };

    lowered.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
        src: magnitude,
        width: MirWidth::Word,
    });
    lowered.push(atari_fpp_call(MirAtariFppService::IntegerToFloat));
    if let Some(sign_scratch) = sign_scratch {
        let sign_bit = generated_temp(next_generated_temp, generated_temps);
        lowered.push(MirOp::Load {
            dst: MirDef::VTemp(sign_bit),
            src: MirAddr::Direct(sign_scratch),
            width: MirWidth::Byte,
        });
        let exponent = generated_temp(next_generated_temp, generated_temps);
        lowered.push(MirOp::Load {
            dst: MirDef::VTemp(exponent),
            src: MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
            width: MirWidth::Byte,
        });
        let signed_exponent = push_generated_binary(
            MirBinaryOp::Or,
            temp_value(exponent),
            temp_value(sign_bit),
            MirWidth::Byte,
            next_generated_temp,
            generated_temps,
            lowered,
        );
        lowered.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
            src: temp_value(signed_exponent),
            width: MirWidth::Byte,
        });
    }
    push_staged_real_copy(
        &MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
        destination,
        lowered,
    );
}

fn push_real_to_integer(
    source: &MirAddr,
    result: MirTempId,
    result_type: &NirType,
    next_generated_temp: &mut u32,
    generated_temps: &mut Vec<MirTemp>,
    next_generated_local: &mut u32,
    generated_locals: &mut Vec<(LocalId, String)>,
    lowered: &mut Vec<MirOp>,
) {
    push_staged_real_copy(
        source,
        &MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
        lowered,
    );
    let exponent = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Load {
        dst: MirDef::VTemp(exponent),
        src: MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
        width: MirWidth::Byte,
    });
    let sign = push_generated_compare(
        MirCompareOp::Ge,
        temp_value(exponent),
        MirValue::ConstU8(0x80),
        next_generated_temp,
        generated_temps,
        lowered,
    );
    let magnitude_exponent = push_generated_binary(
        MirBinaryOp::And,
        temp_value(exponent),
        MirValue::ConstU8(0x7F),
        MirWidth::Byte,
        next_generated_temp,
        generated_temps,
        lowered,
    );
    lowered.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
        src: temp_value(magnitude_exponent),
        width: MirWidth::Byte,
    });
    let sign_scratch = generated_scratch_local(next_generated_local, generated_locals);
    lowered.push(MirOp::Store {
        dst: MirAddr::Direct(sign_scratch.clone()),
        src: temp_value(sign),
        width: MirWidth::Byte,
    });
    lowered.push(atari_fpp_call(MirAtariFppService::FloatToInteger));
    let sign_after_call = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Load {
        dst: MirDef::VTemp(sign_after_call),
        src: MirAddr::Direct(sign_scratch),
        width: MirWidth::Byte,
    });
    let sign_word = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Extend {
        dst: MirDef::VTemp(sign_word),
        src: temp_value(sign_after_call),
        from_width: MirWidth::Byte,
        to_width: MirWidth::Word,
        signed: false,
    });
    let magnitude = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Load {
        dst: MirDef::VTemp(magnitude),
        src: MirAddr::Direct(MirMem::FixedZeroPage(ATARI_FPP_FR0)),
        width: MirWidth::Word,
    });
    let mask = generated_temp(next_generated_temp, generated_temps);
    lowered.push(MirOp::Unary {
        op: MirUnaryOp::Neg,
        dst: MirDef::VTemp(mask),
        src: temp_value(sign_word),
        width: MirWidth::Word,
    });
    let complemented = push_generated_binary(
        MirBinaryOp::Xor,
        temp_value(magnitude),
        temp_value(mask),
        MirWidth::Word,
        next_generated_temp,
        generated_temps,
        lowered,
    );
    let signed_value = push_generated_binary(
        MirBinaryOp::Add,
        temp_value(complemented),
        temp_value(sign_word),
        MirWidth::Word,
        next_generated_temp,
        generated_temps,
        lowered,
    );
    match result_type.width.map(ByteSize::get) {
        Some(1) => lowered.push(MirOp::Move {
            dst: MirDef::VTemp(result),
            src: MirValue::Def(MirDef::VTempByte {
                id: signed_value,
                byte: 0,
            }),
            width: MirWidth::Byte,
        }),
        Some(2) => lowered.push(MirOp::Move {
            dst: MirDef::VTemp(result),
            src: temp_value(signed_value),
            width: MirWidth::Word,
        }),
        _ => unreachable!("NIR verifier accepts only byte/word REAL conversion results"),
    }
}

fn real_binary_service(operation: NirBinaryOp) -> Option<MirAtariFppService> {
    match operation {
        NirBinaryOp::Add => Some(MirAtariFppService::Add),
        NirBinaryOp::Sub => Some(MirAtariFppService::Subtract),
        NirBinaryOp::Mul => Some(MirAtariFppService::Multiply),
        NirBinaryOp::Div => Some(MirAtariFppService::Divide),
        NirBinaryOp::Mod
        | NirBinaryOp::Lsh
        | NirBinaryOp::Rsh
        | NirBinaryOp::And
        | NirBinaryOp::Or
        | NirBinaryOp::Xor => None,
    }
}

fn lower_real_source_addr(
    routine: &str,
    block: &str,
    source: &NirRealSource,
    addr_defs: &BTreeMap<TempId, MirAddrDef>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirAddr> {
    match source {
        NirRealSource::Place(source) => {
            lower_place_addr(routine, block, source, addr_defs, diagnostics)
        }
        NirRealSource::Static { id, .. } => {
            Some(MirAddr::Direct(MirMem::Static { id: *id, offset: 0 }))
        }
    }
}

fn push_staged_real_copy(source: &MirAddr, destination: &MirAddr, lowered: &mut Vec<MirOp>) {
    push_packed_real_copy(source, destination, false, lowered);
}

fn push_packed_real_copy(
    source: &MirAddr,
    destination: &MirAddr,
    negate: bool,
    lowered: &mut Vec<MirOp>,
) {
    lowered.push(MirOp::PackedRealCopy {
        source: source.clone(),
        destination: destination.clone(),
        source_offset: 0,
        destination_offset: 0,
        negate,
    });
}

fn constant_integer_real_bytes(value: &NirValueKind, ty: &NirType) -> Option<[u8; 6]> {
    let decimal = match (&ty.kind, value) {
        (NirTypeKind::U8, NirValueKind::ConstU8(value)) => value.to_string(),
        (NirTypeKind::I8, NirValueKind::ConstU8(value)) => (*value as i8).to_string(),
        (NirTypeKind::U16, NirValueKind::ConstU16(value)) => value.to_string(),
        (NirTypeKind::I16, NirValueKind::ConstU16(value)) => (*value as i16).to_string(),
        _ => return None,
    };
    crate::atari_real::AtariReal::from_decimal(&decimal)
        .ok()
        .map(|value| value.to_bytes())
}

fn atari_fpp_call(service: MirAtariFppService) -> MirOp {
    let effects = service.effects();
    let clobbers = effects.clobbers;
    MirOp::Call {
        target: MirCallTarget::AtariFpp(service),
        abi: MirCallAbi {
            params: Vec::new(),
            result: None,
            clobbers,
            preserves: MirRegisterSet::default(),
        },
        args: Vec::new(),
        result: None,
        effects,
    }
}

fn volatile_memory_barrier() -> MirOp {
    MirOp::Barrier {
        effects: MirEffects {
            memory_reads: MirMemoryEffect::All,
            memory_writes: MirMemoryEffect::All,
            opaque: true,
            ..MirEffects::default()
        },
    }
}

fn lower_machine_items(
    routine: &str,
    block: &str,
    items: &[NirMachineItem],
    local_absolute_addresses: &BTreeMap<String, u16>,
    routine_system_addresses: &BTreeMap<&str, u16>,
    machine_numeric_defines: &BTreeMap<String, u16>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<Vec<MirMachineItem>> {
    let mut lowered = Vec::new();
    for item in items {
        match item {
            NirMachineItem::Byte(value) => lowered.push(MirMachineItem::Byte(*value)),
            NirMachineItem::Word(value) => lowered.push(MirMachineItem::Word(*value)),
            NirMachineItem::StringLiteral(value) => {
                lowered.push(MirMachineItem::StringLiteral(value.clone()))
            }
            NirMachineItem::CharLiteral(value) => lowered.push(MirMachineItem::CharLiteral(*value)),
            NirMachineItem::Name(name) => {
                if let Some(address) = fixed_machine_symbol_address(
                    name,
                    local_absolute_addresses,
                    routine_system_addresses,
                    machine_numeric_defines,
                ) {
                    lowered.push(machine_value_item(address));
                } else {
                    lowered.push(MirMachineItem::Name(name.clone()));
                }
            }
            NirMachineItem::AddressExpr {
                selector,
                explicit_address,
                atom,
                offset,
                text,
            } => {
                let Some(offset) = lower_machine_address_offset(
                    routine,
                    block,
                    *offset,
                    text,
                    machine_numeric_defines,
                    diagnostics,
                ) else {
                    return None;
                };
                let atom = lower_machine_atom_with_fixed_symbols(
                    atom,
                    local_absolute_addresses,
                    routine_system_addresses,
                    machine_numeric_defines,
                );
                lowered.push(MirMachineItem::AddressExpr {
                    selector: selector.map(lower_machine_byte_selector),
                    explicit_address: *explicit_address,
                    atom,
                    offset,
                    text: text.clone(),
                });
            }
            NirMachineItem::AddressByte { high, name } => {
                if let Some(address) = fixed_machine_symbol_address(
                    name,
                    local_absolute_addresses,
                    routine_system_addresses,
                    machine_numeric_defines,
                ) {
                    let byte = if *high {
                        (address >> 8) as u8
                    } else {
                        (address & 0x00FF) as u8
                    };
                    lowered.push(MirMachineItem::Byte(byte));
                } else {
                    lowered.push(MirMachineItem::AddressByte {
                        high: *high,
                        name: name.clone(),
                    });
                }
            }
            NirMachineItem::Relocation {
                kind,
                target,
                addend,
                requires_zero_page,
                span,
            } => lowered.push(MirMachineItem::Relocation {
                kind: *kind,
                target: lower_inline_asm_target(*target),
                addend: *addend,
                requires_zero_page: *requires_zero_page,
                span: *span,
            }),
        }
    }
    Some(lowered)
}

fn lower_inline_asm(code: &NirInlineAsm) -> Vec<MirMachineItem> {
    let mut relocations = code.relocations.iter().collect::<Vec<_>>();
    relocations.sort_by_key(|relocation| relocation.offset);
    let mut items = Vec::new();
    let mut cursor = 0usize;
    for relocation in relocations {
        let offset = usize::from(relocation.offset);
        items.extend(
            code.bytes[cursor..offset]
                .iter()
                .copied()
                .map(MirMachineItem::Byte),
        );
        let target = lower_inline_asm_target(relocation.target);
        items.push(MirMachineItem::Relocation {
            kind: relocation.kind,
            target,
            addend: relocation.addend,
            requires_zero_page: relocation.requires_zero_page,
            span: relocation.span,
        });
        cursor = offset
            + match relocation.kind {
                crate::asm6502::InlineAsmRelocationKind::Absolute16 => 2,
                crate::asm6502::InlineAsmRelocationKind::Byte8
                | crate::asm6502::InlineAsmRelocationKind::Low8
                | crate::asm6502::InlineAsmRelocationKind::High8 => 1,
            };
    }
    items.extend(
        code.bytes[cursor..]
            .iter()
            .copied()
            .map(MirMachineItem::Byte),
    );
    items
}

fn lower_inline_asm_target(target: NirInlineAsmTarget) -> MirInlineAsmTarget {
    match target {
        NirInlineAsmTarget::Storage(crate::nir::NirStorageId::Local(id)) => {
            MirInlineAsmTarget::Memory(MirMem::Local { id, offset: 0 })
        }
        NirInlineAsmTarget::Storage(crate::nir::NirStorageId::Param(id)) => {
            MirInlineAsmTarget::Memory(MirMem::Param { id, offset: 0 })
        }
        NirInlineAsmTarget::Storage(crate::nir::NirStorageId::Global(id)) => {
            MirInlineAsmTarget::Memory(MirMem::Global { id, offset: 0 })
        }
        NirInlineAsmTarget::Routine(id) => MirInlineAsmTarget::Routine(RoutineId(id)),
        NirInlineAsmTarget::Absolute(address) => {
            MirInlineAsmTarget::Absolute(nir_address_u16(address))
        }
        NirInlineAsmTarget::InlineOffset(offset) => {
            MirInlineAsmTarget::InlineOffset(nir_offset_u16(offset))
        }
    }
}

fn lower_machine_address_offset(
    routine: &str,
    block: &str,
    offset: i32,
    text: &str,
    machine_numeric_defines: &BTreeMap<String, u16>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<i32> {
    let Some((negative, name)) = machine_address_symbolic_offset(text) else {
        return Some(offset);
    };
    let Some(value) = machine_numeric_defines.get(&machine_name_key(name)) else {
        diagnostics.push(MirDiagnostic::block(
            routine,
            block,
            format!("machine block item `{text}` references unknown numeric define `{name}`"),
        ));
        return None;
    };
    let value = i32::from(*value);
    Some(offset.wrapping_add(if negative { -value } else { value }))
}

fn lower_machine_atom_with_fixed_symbols(
    atom: &NirMachineAtom,
    local_absolute_addresses: &BTreeMap<String, u16>,
    routine_system_addresses: &BTreeMap<&str, u16>,
    machine_numeric_defines: &BTreeMap<String, u16>,
) -> MirMachineAtom {
    match atom {
        NirMachineAtom::Name(name) => fixed_machine_symbol_address(
            name,
            local_absolute_addresses,
            routine_system_addresses,
            machine_numeric_defines,
        )
        .map(MirMachineAtom::Number)
        .unwrap_or_else(|| MirMachineAtom::Name(name.clone())),
        NirMachineAtom::Number(value) => MirMachineAtom::Number(*value),
        NirMachineAtom::Current => MirMachineAtom::Current,
    }
}

fn collect_machine_numeric_defines(nir_program: &NirProgram) -> BTreeMap<String, u16> {
    let mut defines = BTreeMap::new();
    for global in &nir_program.globals {
        if let Some(value) = global.kind.strip_prefix("define ")
            && let Some(value) = parse_machine_numeric_define_value(value)
        {
            defines.insert(machine_name_key(&global.name), value);
        }
    }
    defines
}

fn parse_machine_numeric_define_value(value: &str) -> Option<u16> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(hex) = value.strip_prefix('$') {
        return u16::from_str_radix(hex, 16).ok();
    }
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return u16::from_str_radix(hex, 16).ok();
    }
    if let Some(rest) = value.strip_prefix('-') {
        return parse_machine_numeric_define_value(rest).map(|value| 0u16.wrapping_sub(value));
    }
    if let Some(rest) = value.strip_prefix('+') {
        return parse_machine_numeric_define_value(rest);
    }
    value.parse::<u16>().ok()
}

fn local_absolute_address(
    local_absolute_addresses: &BTreeMap<String, u16>,
    name: &str,
) -> Option<u16> {
    local_absolute_addresses
        .get(&machine_name_key(name))
        .copied()
}

fn fixed_machine_symbol_address(
    name: &str,
    local_absolute_addresses: &BTreeMap<String, u16>,
    routine_system_addresses: &BTreeMap<&str, u16>,
    machine_numeric_defines: &BTreeMap<String, u16>,
) -> Option<u16> {
    local_absolute_address(local_absolute_addresses, name)
        .or_else(|| {
            machine_numeric_defines
                .get(&machine_name_key(name))
                .copied()
        })
        .or_else(|| machine_named_constant(name))
        .or_else(|| resident_variable(name).map(|variable| variable.address))
        .or_else(|| machine_routine_system_address(routine_system_addresses, name))
}

fn machine_routine_system_address(
    routine_system_addresses: &BTreeMap<&str, u16>,
    name: &str,
) -> Option<u16> {
    routine_system_addresses
        .iter()
        .find_map(|(candidate, address)| candidate.eq_ignore_ascii_case(name).then_some(*address))
}

fn machine_named_constant(name: &str) -> Option<u16> {
    match machine_name_key(name).as_str() {
        "eol" | "cr" | "return" => Some(0x9B),
        "esc" | "escape" => Some(0x1B),
        "clear" | "cls" => Some(0x7D),
        _ => None,
    }
}

fn machine_value_item(value: u16) -> MirMachineItem {
    if let Ok(value) = u8::try_from(value) {
        MirMachineItem::Byte(value)
    } else {
        MirMachineItem::Word(value)
    }
}

fn machine_name_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn lower_machine_byte_selector(selector: NirMachineByteSelector) -> MirMachineByteSelector {
    match selector {
        NirMachineByteSelector::Low => MirMachineByteSelector::Low,
        NirMachineByteSelector::High => MirMachineByteSelector::High,
    }
}

fn lower_machine_effects(effects: &NirMachineEffects) -> MirEffects {
    MirEffects {
        memory_reads: super::abi::mir_memory_effect(&effects.memory.reads),
        memory_writes: super::abi::mir_memory_effect(&effects.memory.writes),
        reads: Default::default(),
        clobbers: super::abi::opaque_machine_clobbers(),
        preserves: Default::default(),
        stack_depth_delta: None,
        may_call_os: effects.may_call_os,
        opaque: effects.opaque,
    }
}

fn lower_inline_asm_effects(code: &NirInlineAsm, effects: &NirMachineEffects) -> MirEffects {
    if effects.opaque {
        return lower_machine_effects(effects);
    }
    let local_control_targets = code
        .relocations
        .iter()
        .filter_map(|relocation| {
            let NirInlineAsmTarget::InlineOffset(target) = &relocation.target else {
                return None;
            };
            (relocation.symbol_use == crate::asm6502::InlineAsmSymbolUse::Control)
                .then_some((
                    nir_offset_u16(relocation.offset),
                    nir_offset_u16(*target),
                ))
        })
        .collect::<Vec<_>>();
    let machine = crate::asm6502::analyze_machine_state(&code.bytes, &local_control_targets);
    MirEffects {
        memory_reads: super::abi::mir_memory_effect(&effects.memory.reads),
        memory_writes: super::abi::mir_memory_effect(&effects.memory.writes),
        reads: inline_register_set(machine.reads),
        clobbers: inline_register_set(machine.clobbers),
        preserves: Default::default(),
        stack_depth_delta: machine.stack_depth_delta,
        may_call_os: effects.may_call_os,
        opaque: false,
    }
}

fn inline_register_set(registers: crate::asm6502::InlineAsmRegisterSet) -> MirRegisterSet {
    MirRegisterSet {
        a: registers.a,
        x: registers.x,
        y: registers.y,
        flags: registers.flags,
        sp: registers.sp,
    }
}

fn lower_place_addr(
    routine: &str,
    block: &str,
    place: &NirPlace,
    addr_defs: &BTreeMap<TempId, MirAddrDef>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirAddr> {
    match classify_place(place) {
        MirPlaceShape::DirectMemory(mem) => Some(MirAddr::Direct(mem)),
        MirPlaceShape::AbsoluteMemory(address) => Some(MirAddr::Direct(MirMem::Absolute(address))),
        MirPlaceShape::PointerDeref { addr, offset } => {
            let ptr = lower_value(routine, block, &addr, diagnostics)?;
            Some(MirAddr::Deref { ptr, offset })
        }
        MirPlaceShape::IndexedElement {
            base_addr,
            index,
            elem_size,
        } => lower_index_addr(
            routine,
            block,
            &base_addr,
            &index,
            elem_size,
            addr_defs,
            diagnostics,
        ),
        MirPlaceShape::RecordField { base, offset } => {
            lower_field_addr(routine, block, &base, offset, addr_defs, diagnostics)
        }
    }
}

fn lower_index_addr(
    routine: &str,
    block: &str,
    base_addr: &NirValueKind,
    index: &NirValueKind,
    elem_size: u16,
    addr_defs: &BTreeMap<TempId, MirAddrDef>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirAddr> {
    if let Some(base_def) = addr_temp_def(base_addr, addr_defs) {
        if let Some(offset) = const_index_offset(index, elem_size) {
            return if base_def.pointer_backed {
                Some(MirAddr::PointerCell {
                    ptr: base_def.mem.clone(),
                    offset,
                })
            } else {
                Some(MirAddr::Direct(offset_mem(&base_def.mem, offset)))
            };
        }
        if base_def.pointer_backed {
            let index = lower_value(routine, block, index, diagnostics)?;
            return Some(MirAddr::PointerIndex {
                ptr: base_def.mem.clone(),
                index,
                elem_size,
                offset: 0,
            });
        }
        if (elem_size == 1 || !matches!(base_def.mem, MirMem::Local { .. }))
            && let Some((base, offset)) = direct_mem_base_value(&base_def.mem)
        {
            let index = lower_value(routine, block, index, diagnostics)?;
            return Some(MirAddr::ComputedIndex {
                base,
                index,
                elem_size,
                offset,
            });
        }
    }
    let base = lower_value(routine, block, base_addr, diagnostics)?;
    let index = lower_value(routine, block, index, diagnostics)?;
    Some(MirAddr::ComputedIndex {
        base,
        index,
        elem_size,
        offset: 0,
    })
}

fn lower_field_addr(
    routine: &str,
    block: &str,
    base: &NirPlace,
    offset: u16,
    addr_defs: &BTreeMap<TempId, MirAddrDef>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirAddr> {
    if base.ty.as_ref().is_some_and(|ty| ty.pointer) {
        let ptr = lower_place_mem(routine, block, base, diagnostics)?;
        return Some(MirAddr::PointerCell { ptr, offset });
    }
    match lower_place_addr(routine, block, base, addr_defs, diagnostics)? {
        MirAddr::Direct(mem) => Some(MirAddr::Direct(offset_mem(&mem, offset))),
        MirAddr::Deref {
            ptr,
            offset: base_offset,
        } => Some(MirAddr::Deref {
            ptr,
            offset: base_offset.saturating_add(offset),
        }),
        MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset: base_offset,
        } => Some(MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset: base_offset.saturating_add(offset),
        }),
        MirAddr::PointerCell {
            ptr,
            offset: base_offset,
        } => Some(MirAddr::PointerCell {
            ptr,
            offset: base_offset.saturating_add(offset),
        }),
        MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset: base_offset,
        } => Some(MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset: base_offset.saturating_add(offset),
        }),
        other => Some(other),
    }
}

fn addr_temp_def<'a>(
    value: &NirValueKind,
    addr_defs: &'a BTreeMap<TempId, MirAddrDef>,
) -> Option<&'a MirAddrDef> {
    match value {
        NirValueKind::Temp { id, .. } => addr_defs.get(id),
        _ => None,
    }
}

fn const_index_offset(value: &NirValueKind, elem_size: u16) -> Option<u16> {
    let index = match value {
        NirValueKind::ConstU8(value) => u16::from(*value),
        NirValueKind::ConstU16(value) => *value,
        _ => return None,
    };
    Some(index.saturating_mul(elem_size))
}

fn direct_mem_base_value(mem: &MirMem) -> Option<(MirValue, u16)> {
    match mem {
        MirMem::Absolute(address) => Some((MirValue::ConstU16(*address), 0)),
        MirMem::Static { id, offset } => Some((MirValue::StaticAddr(*id), *offset)),
        MirMem::Global { id, offset } => Some((MirValue::GlobalAddr(*id), *offset)),
        MirMem::Local { id, offset } => {
            let base = MirMem::Local { id: *id, offset: 0 };
            Some((
                MirValue::Word {
                    lo: Box::new(MirValue::StorageAddrByte {
                        mem: base.clone(),
                        byte: 0,
                    }),
                    hi: Box::new(MirValue::StorageAddrByte { mem: base, byte: 1 }),
                },
                *offset,
            ))
        }
        _ => None,
    }
}

fn offset_mem(mem: &MirMem, offset: u16) -> MirMem {
    match mem {
        MirMem::Absolute(address) => MirMem::Absolute(address.saturating_add(offset)),
        MirMem::Static { id, offset: base } => MirMem::Static {
            id: *id,
            offset: base.saturating_add(offset),
        },
        MirMem::Global { id, offset: base } => MirMem::Global {
            id: *id,
            offset: base.saturating_add(offset),
        },
        MirMem::Local { id, offset: base } => MirMem::Local {
            id: *id,
            offset: base.saturating_add(offset),
        },
        MirMem::Param { id, offset: base } => MirMem::Param {
            id: *id,
            offset: base.saturating_add(offset),
        },
        MirMem::Spill { id, offset: base } => MirMem::Spill {
            id: *id,
            offset: base.saturating_add(offset),
        },
        MirMem::ZeroPage(id) => MirMem::ZeroPage(*id),
        MirMem::FixedZeroPage(id) => {
            MirMem::FixedZeroPage(MirFixedZpSlot(id.0.saturating_add(offset as u8)))
        }
    }
}

fn offset_addr(addr: &MirAddr, offset: u16) -> MirAddr {
    match addr {
        MirAddr::Direct(mem) => MirAddr::Direct(offset_mem(mem, offset)),
        MirAddr::AbsoluteIndexedX { base } => MirAddr::AbsoluteIndexedX {
            base: offset_mem(base, offset),
        },
        MirAddr::AbsoluteIndexedY { base } => MirAddr::AbsoluteIndexedY {
            base: offset_mem(base, offset),
        },
        MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset: base_offset,
        } => MirAddr::ComputedIndex {
            base: base.clone(),
            index: index.clone(),
            elem_size: *elem_size,
            offset: base_offset.saturating_add(offset),
        },
        MirAddr::PointerCell {
            ptr,
            offset: base_offset,
        } => MirAddr::PointerCell {
            ptr: ptr.clone(),
            offset: base_offset.saturating_add(offset),
        },
        MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset: base_offset,
        } => MirAddr::PointerIndex {
            ptr: ptr.clone(),
            index: index.clone(),
            elem_size: *elem_size,
            offset: base_offset.saturating_add(offset),
        },
        MirAddr::Deref {
            ptr,
            offset: base_offset,
        } => MirAddr::Deref {
            ptr: ptr.clone(),
            offset: base_offset.saturating_add(offset),
        },
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {
            unreachable!("pre-materialization REAL address has an unsupported indexed form")
        }
    }
}

fn lower_place_mem(
    routine: &str,
    block: &str,
    place: &NirPlace,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirMem> {
    if let NirPlaceKind::Field { base, offset, .. } = &place.kind
        && !base.ty.as_ref().is_some_and(|ty| ty.pointer)
    {
        return lower_place_mem(routine, block, base, diagnostics)
            .map(|mem| offset_mem(&mem, nir_offset_u16(*offset)));
    }
    match classify_address(place) {
        MirAddressShape::Direct(mem) => Some(mem),
        MirAddressShape::Absolute(address) => Some(MirMem::Absolute(address)),
        MirAddressShape::Static(id) => Some(MirMem::Static { id, offset: 0 }),
        MirAddressShape::Global(id) => Some(MirMem::Global { id, offset: 0 }),
        MirAddressShape::Unsupported(reason) => {
            unsupported_place(routine, block, reason, diagnostics);
            None
        }
    }
}

fn unsupported_place(
    routine: &str,
    block: &str,
    place: &str,
    diagnostics: &mut Vec<MirDiagnostic>,
) {
    diagnostics.push(MirDiagnostic::block(
        routine,
        block,
        format!("unsupported MIR6502 direct scalar place: {place}"),
    ));
}

fn is_signed(ty: &NirType) -> bool {
    matches!(ty.kind, NirTypeKind::I8 | NirTypeKind::I16)
}

fn lower_compare_value(
    routine: &str,
    block: &str,
    value: &NirValueKind,
    width: MirWidth,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirValue> {
    let source_width = value_width(value)?;
    let value = lower_value(routine, block, value, diagnostics)?;
    if width == MirWidth::Word && source_width == MirWidth::Byte {
        Some(MirValue::Word {
            lo: Box::new(value),
            hi: Box::new(MirValue::ConstU8(0)),
        })
    } else {
        Some(value)
    }
}

fn mir_binary_op(op: NirBinaryOp) -> MirBinaryOp {
    match op {
        NirBinaryOp::Add => MirBinaryOp::Add,
        NirBinaryOp::Sub => MirBinaryOp::Sub,
        NirBinaryOp::Mul => MirBinaryOp::Mul,
        NirBinaryOp::Div => MirBinaryOp::Div,
        NirBinaryOp::Mod => MirBinaryOp::Mod,
        NirBinaryOp::Lsh => MirBinaryOp::Lsh,
        NirBinaryOp::Rsh => MirBinaryOp::Rsh,
        NirBinaryOp::And => MirBinaryOp::And,
        NirBinaryOp::Or => MirBinaryOp::Or,
        NirBinaryOp::Xor => MirBinaryOp::Xor,
    }
}

fn mir_compare_op(op: NirCompareOp) -> MirCompareOp {
    match op {
        NirCompareOp::Eq => MirCompareOp::Eq,
        NirCompareOp::Ne => MirCompareOp::Ne,
        NirCompareOp::Lt => MirCompareOp::Lt,
        NirCompareOp::Le => MirCompareOp::Le,
        NirCompareOp::Gt => MirCompareOp::Gt,
        NirCompareOp::Ge => MirCompareOp::Ge,
    }
}

fn routine_return_width(routine: &nir::NirRoutine) -> Option<MirWidth> {
    routine.notes.iter().find_map(|note| {
        note.text
            .strip_prefix("return-width ")
            .and_then(|width| width.parse::<u16>().ok())
            .and_then(|width| match width {
                1 => Some(MirWidth::Byte),
                2 => Some(MirWidth::Word),
                _ => None,
            })
    })
}

fn value_width(value: &NirValueKind) -> Option<MirWidth> {
    match value {
        NirValueKind::ConstU8(_) => Some(MirWidth::Byte),
        NirValueKind::ConstU16(_) => Some(MirWidth::Word),
        NirValueKind::Null { ty }
        | NirValueKind::AddressConst { ty, .. }
        | NirValueKind::Temp { ty, .. }
        | NirValueKind::StaticAddr { ty, .. }
        | NirValueKind::RoutineAddr { ty, .. } => mir_width(ty),
        NirValueKind::Param(_) | NirValueKind::GlobalAddr(_) => None,
    }
}

fn lower_value(
    routine: &str,
    block: &str,
    value: &NirValueKind,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirValue> {
    match classify_value(value) {
        MirValueShape::ConstByte(value) => Some(MirValue::ConstU8(value)),
        MirValueShape::ConstWord(value) => Some(MirValue::ConstU16(value)),
        MirValueShape::Temp(id) => Some(MirValue::Def(MirDef::VTemp(MirTempId(id.0)))),
        MirValueShape::StaticAddress(id) => Some(MirValue::StaticAddr(id)),
        MirValueShape::GlobalAddress(id) => Some(MirValue::GlobalAddr(id)),
        MirValueShape::RoutineAddress(id) => Some(MirValue::RoutineAddr(RoutineId(id))),
        MirValueShape::ParamValue(id) => {
            diagnostics.push(MirDiagnostic::block(
                routine,
                block,
                format!(
                    "param value `p{}` needs an explicit load before MIR6502",
                    id.0
                ),
            ));
            None
        }
    }
}

fn mir_width(ty: &nir::NirType) -> Option<MirWidth> {
    match ty.width.map(ByteSize::get) {
        Some(1) => Some(MirWidth::Byte),
        Some(2) => Some(MirWidth::Word),
        _ => None,
    }
}

fn lower_storage_class(storage: nir::NirStorageClass) -> MirStorageClass {
    match storage {
        nir::NirStorageClass::Scalar => MirStorageClass::Scalar,
        nir::NirStorageClass::Array => MirStorageClass::Array,
        nir::NirStorageClass::Record => MirStorageClass::Record,
        nir::NirStorageClass::Type => MirStorageClass::Type,
    }
}

fn local_scalar_width(local: &nir::NirLocal) -> Option<MirWidth> {
    if local_pointer_backed_array(local) {
        Some(MirWidth::Word)
    } else if local.storage == nir::NirStorageClass::Scalar {
        mir_width(&local.ty)
    } else {
        None
    }
}

fn local_storage_size(
    local: &nir::NirLocal,
    scalar_width: Option<MirWidth>,
    init: Option<&MirStorageInit>,
) -> u16 {
    let declared_size = if local_pointer_backed_array(local) {
        MirWidth::Word
    } else {
        scalar_width.unwrap_or(MirWidth::Byte)
    };
    let declared_size = local
        .ty
        .width
        .filter(|_| !local_pointer_backed_array(local))
        .map(nir_size_u16)
        .unwrap_or_else(|| mir_width_bytes(declared_size));
    init.map_or(declared_size, |init| {
        mir_storage_init_object_size(init, declared_size)
    })
}

fn mir_storage_init_object_size(init: &MirStorageInit, storage_size: u16) -> u16 {
    match init {
        MirStorageInit::Bytes {
            image, zero_fill, ..
        } => (image.bytes.len() as u16)
            .saturating_add(*zero_fill)
            .max(storage_size),
        MirStorageInit::ZeroFill { bytes, .. } => (*bytes).max(storage_size),
        MirStorageInit::Descriptor {
            backing,
            descriptor_size,
            ..
        } => (backing.image.bytes.len() as u16)
            .saturating_add(backing.zero_fill)
            .saturating_add(*descriptor_size)
            .max(storage_size),
        MirStorageInit::RoutineAddress {
            descriptor_size, ..
        } => (*descriptor_size).max(storage_size),
    }
}

fn mir_width_bytes(width: MirWidth) -> u16 {
    match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    }
}

fn local_pointer_backed_array(local: &nir::NirLocal) -> bool {
    matches!(
        local.init.as_ref(),
        Some(nir::NirStorageInit::Descriptor { .. })
    ) || (local.init.is_none() && local.storage == nir::NirStorageClass::Array)
        || local_pointer_init_symbol(local).is_some()
}

fn local_pointer_init_symbol(local: &nir::NirLocal) -> Option<String> {
    local
        .kind
        .split_whitespace()
        .find_map(|part| part.strip_prefix("pointer_init=").map(str::to_string))
}

fn lower_terminator(
    routine: &str,
    block: &str,
    block_id: BlockId,
    terminator: &NirTerminator,
    block_ids: &BTreeMap<BlockId, MirBlockId>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> MirTerminator {
    match terminator {
        NirTerminator::Fallthrough => MirTerminator::Unreachable,
        NirTerminator::Goto(edge) => lower_edge(routine, block, edge, block_ids, diagnostics)
            .map(MirTerminator::Jump)
            .unwrap_or(MirTerminator::Unreachable),
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
            ..
        } => {
            let then_edge = lower_edge(routine, block, then_edge, block_ids, diagnostics);
            let else_edge = lower_edge(routine, block, else_edge, block_ids, diagnostics);
            match (then_edge, else_edge) {
                (Some(then_edge), Some(else_edge)) => MirTerminator::Branch {
                    cond: lower_value(routine, block, condition, diagnostics)
                        .map(MirCond::BoolValue)
                        .unwrap_or(MirCond::Deferred),
                    then_edge,
                    else_edge,
                },
                _ => MirTerminator::Unreachable,
            }
        }
        NirTerminator::Return(_) => MirTerminator::Return,
        NirTerminator::Exit => MirTerminator::Exit,
        NirTerminator::Open => block_ids
            .get(&block_id)
            .copied()
            .map(|target| MirTerminator::Jump(MirEdge::plain(target)))
            .unwrap_or(MirTerminator::Unreachable),
    }
}

fn lower_edge(
    routine: &str,
    block: &str,
    edge: &nir::NirEdge,
    block_ids: &BTreeMap<BlockId, MirBlockId>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirEdge> {
    let target = block_ids.get(&edge.target).copied()?;
    let mut args = Vec::with_capacity(edge.args.len());
    for arg in &edge.args {
        let Some(width) = value_width(arg) else {
            diagnostics.push(MirDiagnostic::block(
                routine,
                block,
                "NIR edge argument has unsupported width",
            ));
            continue;
        };
        let Some(value) = lower_value(routine, block, arg, diagnostics) else {
            continue;
        };
        args.push(MirEdgeArg { value, width });
    }
    Some(MirEdge { target, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lower_modern_source(source: &str) -> NirProgram {
        let tokens = crate::lexer::tokenize(source).expect("tokenize modern source");
        let program = crate::parser::parse(&tokens).expect("parse modern source");
        let model = crate::semantic::analyze_with_options(
            &program,
            crate::semantic::SemanticOptions::modern(),
        )
        .expect("analyze modern source");
        let semir = crate::semantic::ir::lower_program(&program, &model);
        crate::nir::lower_program(&semir)
    }

    #[test]
    fn retains_record_copy_until_target_selection() {
        let nir = lower_modern_source(
            "TYPE Pair=[BYTE tag CARD word] Pair ARRAY table(2) Pair current PROC Main() current=table(1) RETURN",
        );
        crate::nir::verify_program(&nir).expect("record-copy NIR verifies");

        let mir = lower_program(&nir).expect("record copy lowers to MIR");
        crate::mir6502::verify_program(&mir, crate::mir6502::MirPhase::PreMaterialization)
            .expect("aggregate record-copy MIR verifies");

        let ops = &mir.routines[0].blocks[0].ops;
        let copies = ops
            .iter()
            .filter(|op| matches!(op, MirOp::CopyBytes { .. }))
            .collect::<Vec<_>>();
        assert_eq!(copies.len(), 1);
        assert!(matches!(
            copies[0],
            MirOp::CopyBytes {
                size: 3,
                destination_volatile: false,
                source_volatile: false,
                ..
            }
        ));
        assert!(
            !ops.iter()
                .any(|op| matches!(op, MirOp::Load { .. } | MirOp::Store { .. }))
        );
    }

    #[test]
    fn lowers_six_byte_real_local_as_address_only_storage() {
        let program = nir::NirProgram {
            target_layout: crate::target::TargetLayout::atari_6502(),
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![nir::NirRoutine {
                name: "Main".to_string(),
                params: Vec::new(),
                locals: vec![nir::NirLocal {
                    id: LocalId(0),
                    name: "value".to_string(),
                    kind: "REAL".to_string(),
                    purpose: nir::NirLocalPurpose::Storage,
                    storage: nir::NirStorageClass::Scalar,
                    ty: NirType {
                        kind: NirTypeKind::Real,
                        summary: "REAL".to_string(),
                        width: Some(ByteSize::new(6)),
                        pointer: false,
                    },
                    backing: NirLocalBacking::Ordinary,
                    init: None,
                }],
                temps: Vec::new(),
                notes: Vec::new(),
                blocks: vec![nir::NirBlock {
                    id: BlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(None),
                }],
            }],
        };

        let mir = lower_program(&program).expect("address-only REAL local lowers to MIR");
        let slot = &mir.routines[0].frame.locals[0];
        assert_eq!(slot.storage_size, 6);
        assert_eq!(slot.scalar_width, None);
    }
}
