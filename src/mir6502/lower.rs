use std::collections::{BTreeMap, BTreeSet};

use crate::ast::machine_address_symbolic_offset;
use crate::codegen::runtime_zp;
use crate::nir::{
    self, BlockId, LocalId, NirBinaryOp, NirCompareOp, NirGlobalBacking, NirLocalBacking,
    NirMachineAtom, NirMachineByteSelector, NirMachineEffects, NirMachineItem, NirOp as NirOpKind,
    NirPlace, NirPlaceKind, NirProgram, NirTerminator, NirType, NirTypeKind, NirUnaryOp,
    NirValue as NirValueKind, TempId,
};
use crate::resident::resident_variable;

use super::builtin::{MirBuiltinResolution, resolve_builtin_target};
use super::call_plan;
use super::classify::{
    MirAddressShape, MirPlaceShape, MirValueShape, classify_address, classify_place, classify_value,
};
use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirAddr, MirBinaryOp, MirBlock, MirBlockId, MirBlockParam, MirCarryOut, MirCompareOp, MirCond,
    MirCondDest, MirDataBacking, MirDef, MirEdge, MirEdgeArg, MirEffects, MirFixedZpSlot, MirFrame,
    MirGlobal, MirGlobalBacking, MirGlobalInit, MirMachineAtom, MirMachineBlock, MirMachineBlockId,
    MirMachineByteSelector, MirMachineItem, MirMem, MirOp, MirProgram, MirRoutine, MirRoutineAbi,
    MirRuntimeHelper, MirRuntimeHelperDecl, MirRuntimeHelperTarget, MirStatic, MirStorageBacking,
    MirStorageBase, MirStorageId, MirStorageInit, MirStorageSlot, MirTemp, MirTempId,
    MirTerminator, MirUnaryOp, MirValue, MirWidth, RoutineId,
};

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

    let mut diagnostics = Vec::new();
    let routine_ids = nir_program
        .routines
        .iter()
        .enumerate()
        .map(|(index, routine)| (routine.name.as_str(), RoutineId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let routine_system_addresses = nir_program
        .routines
        .iter()
        .filter_map(|routine| {
            routine_system_address(routine).map(|address| (routine.name.as_str(), address))
        })
        .collect::<BTreeMap<_, _>>();
    let public_action_abi_routines = nir_program
        .routines
        .iter()
        .filter(|routine| routine_has_current_location_address(routine))
        .map(|routine| routine.name.as_str())
        .collect::<BTreeSet<_>>();
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
    let global_ids_by_name = nir_program
        .globals
        .iter()
        .map(|global| (global.name.as_str(), global.id))
        .collect::<BTreeMap<_, _>>();
    let machine_numeric_defines = collect_machine_numeric_defines(nir_program);
    let mut machine_blocks = Vec::new();
    let routines = nir_program
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
                        Some((machine_name_key(&local.name), address))
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

            let blocks: Vec<MirBlock> = routine
                .blocks
                .iter()
                .enumerate()
                .map(|(block_index, block)| {
                    let mut ops = lower_ops(
                        &routine.name,
                        &block.label,
                        &block.ops,
                        &routine_ids,
                        &routine_system_addresses,
                        &public_action_abi_routines,
                        &global_array_pointer_backing,
                        &local_array_pointer_backing,
                        &local_absolute_addresses,
                        &machine_numeric_defines,
                        &mut machine_blocks,
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
                        terminator: lower_terminator(
                            &routine.name,
                            &block.label,
                            block.id,
                            &block.terminator,
                            &block_ids,
                            &mut diagnostics,
                        ),
                    }
                })
                .collect();

            MirRoutine {
                id: RoutineId(routine_index as u32),
                name: routine.name.clone(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    params: routine
                        .params
                        .iter()
                        .enumerate()
                        .map(|(index, param)| MirStorageSlot {
                            id: MirStorageId(index as u32),
                            name: Some(param.name.clone()),
                            width: mir_width(&param.ty).unwrap_or(MirWidth::Byte),
                            base: MirStorageBase::Param(param.id),
                            offset: 0,
                            mutable: true,
                            init: None,
                        })
                        .collect(),
                    locals: routine
                        .locals
                        .iter()
                        .filter(|local| {
                            matches!(
                                local.backing,
                                NirLocalBacking::Ordinary | NirLocalBacking::Alias { .. }
                            )
                        })
                        .enumerate()
                        .map(|(index, local)| MirStorageSlot {
                            id: MirStorageId(index as u32),
                            name: Some(local.name.clone()),
                            width: local_storage_width(local),
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
                                NirLocalBacking::Alias { offset, .. } => offset,
                                NirLocalBacking::Ordinary => 0,
                                NirLocalBacking::GlobalAlias { .. } => unreachable!(
                                    "global-alias locals are resolved directly to global places"
                                ),
                                NirLocalBacking::Absolute(_) => unreachable!(
                                    "absolute locals are filtered out of the routine frame"
                                ),
                            },
                            mutable: true,
                            init: lower_local_storage_init(local, &routine_ids),
                        })
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
    Ok(MirProgram {
        statics: nir_program
            .statics
            .iter()
            .map(|static_data| MirStatic {
                id: static_data.id,
                name: static_data.name.clone(),
                ty: static_data.ty.summary.clone(),
                bytes: static_data.bytes.clone(),
                display: static_data.display.clone(),
                alignment: static_data.alignment,
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
                        next_global_offset = next_global_offset.saturating_add(global.storage_size);
                    }
                    MirGlobal {
                        id: global.id,
                        name: global.name.clone(),
                        kind: global.kind.clone(),
                        width,
                        storage_size: global.storage_size,
                        backing: match global.backing {
                            NirGlobalBacking::Ordinary => MirGlobalBacking::Ordinary {
                                offset: ordinary_offset,
                            },
                            NirGlobalBacking::Absolute(address) => {
                                MirGlobalBacking::Absolute(address)
                            }
                            NirGlobalBacking::Alias { ref target, offset } => {
                                MirGlobalBacking::Alias {
                                    target: global_ids_by_name
                                        .get(target.as_str())
                                        .copied()
                                        .unwrap_or(crate::nir::SymbolId(u32::MAX)),
                                    offset,
                                }
                            }
                        },
                        init: global
                            .init
                            .as_ref()
                            .map(|init| lower_global_init(init, &routine_ids)),
                    }
                })
                .collect()
        },
        routines,
        machine_blocks,
        runtime_helpers,
    })
}

fn routine_system_address(routine: &nir::NirRoutine) -> Option<u16> {
    routine.notes.iter().find_map(|note| {
        let value = note.text.strip_prefix("system-address ")?;
        parse_system_address_note(value)
    })
}

fn routine_has_current_location_address(routine: &nir::NirRoutine) -> bool {
    routine
        .notes
        .iter()
        .any(|note| note.kind == nir::NirRoutineNoteKind::CurrentLocationEntry)
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
                let NirOpKind::Set { address, value } = op else {
                    continue;
                };
                let Some(helper) = runtime_helper_from_set_address(address) else {
                    continue;
                };
                if decls.iter().any(|decl| decl.helper == helper) {
                    continue;
                }
                let Some(target) = runtime_helper_set_target(value) else {
                    continue;
                };
                decls.push(MirRuntimeHelperDecl {
                    helper,
                    target,
                    abi: super::materialize::helper_abi(),
                    effects: super::materialize::helper_effects(),
                });
            }
        }
    }
    decls
}

fn runtime_helper_from_set_address(address: &crate::nir::NirOperand) -> Option<MirRuntimeHelper> {
    let crate::nir::NirOperandKind::Literal {
        value: Some(address),
        ..
    } = address.kind
    else {
        return None;
    };
    match address {
        0x04E4 => Some(MirRuntimeHelper::Lsh),
        0x04E6 => Some(MirRuntimeHelper::Rsh),
        0x04E8 => Some(MirRuntimeHelper::Mul),
        0x04EA => Some(MirRuntimeHelper::Div),
        0x04EC => Some(MirRuntimeHelper::Mod),
        0x04EE => Some(MirRuntimeHelper::SArgs),
        _ => None,
    }
}

fn runtime_helper_set_target(value: &crate::nir::NirOperand) -> Option<MirRuntimeHelperTarget> {
    match &value.kind {
        crate::nir::NirOperandKind::Literal {
            value: Some(address),
            ..
        } => Some(MirRuntimeHelperTarget::KnownAbsolute(*address)),
        crate::nir::NirOperandKind::Symbol(name)
        | crate::nir::NirOperandKind::AddressOfSymbol(name) => {
            Some(MirRuntimeHelperTarget::RuntimeSymbol(name.clone()))
        }
        _ => None,
    }
}

fn lower_global_init(
    init: &crate::nir::NirGlobalInit,
    routine_ids: &BTreeMap<&str, RoutineId>,
) -> MirGlobalInit {
    match init {
        crate::nir::NirGlobalInit::Bytes {
            bytes,
            zero_fill,
            mutable,
            section,
        } => MirGlobalInit::Bytes {
            bytes: bytes.clone(),
            zero_fill: *zero_fill,
            mutable: *mutable,
            section: section.clone(),
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
                bytes: backing.bytes.clone(),
                zero_fill: backing.zero_fill,
                section: backing.section.clone(),
            },
            descriptor_size: *descriptor_size,
            size_word: *size_word,
            mutable: *mutable,
            section: section.clone(),
        },
        crate::nir::NirGlobalInit::ZeroFill {
            bytes,
            mutable,
            section,
        } => MirGlobalInit::ZeroFill {
            bytes: *bytes,
            mutable: *mutable,
            section: section.clone(),
        },
        crate::nir::NirGlobalInit::ProgramEndWord { mutable, section } => {
            MirGlobalInit::ProgramEndWord {
                mutable: *mutable,
                section: section.clone(),
            }
        }
        crate::nir::NirGlobalInit::RoutineAddress {
            name,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => MirGlobalInit::RoutineAddress {
            routine: routine_ids
                .get(name.as_str())
                .copied()
                .unwrap_or(RoutineId(u32::MAX)),
            descriptor_size: *descriptor_size,
            size_word: *size_word,
            mutable: *mutable,
            section: section.clone(),
        },
    }
}

fn lower_local_storage_init(
    local: &crate::nir::NirLocal,
    routine_ids: &BTreeMap<&str, RoutineId>,
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
    local.init.as_ref().map(lower_storage_init)
}

fn lower_storage_init(init: &crate::nir::NirStorageInit) -> MirStorageInit {
    match init {
        crate::nir::NirStorageInit::Bytes {
            bytes,
            zero_fill,
            mutable,
            section,
        } => MirStorageInit::Bytes {
            bytes: bytes.clone(),
            zero_fill: *zero_fill,
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
                bytes: backing.bytes.clone(),
                zero_fill: backing.zero_fill,
                section: backing.section.clone(),
            },
            descriptor_size: *descriptor_size,
            size_word: *size_word,
            mutable: *mutable,
            section: section.clone(),
        },
        crate::nir::NirStorageInit::ZeroFill {
            bytes,
            mutable,
            section,
        } => MirStorageInit::ZeroFill {
            bytes: *bytes,
            mutable: *mutable,
            section: section.clone(),
        },
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
        _ => {}
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
    routine_system_addresses: &BTreeMap<&str, u16>,
    public_action_abi_routines: &BTreeSet<&str>,
    global_array_pointer_backing: &BTreeMap<crate::nir::SymbolId, bool>,
    local_array_pointer_backing: &[LocalId],
    local_absolute_addresses: &BTreeMap<String, u16>,
    machine_numeric_defines: &BTreeMap<String, u16>,
    machine_blocks: &mut Vec<MirMachineBlock>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Vec<MirOp> {
    let mut lowered = Vec::new();
    let mut addr_defs = BTreeMap::<TempId, MirAddrDef>::new();
    for op in ops {
        match op {
            NirOpKind::Set { .. } => {}
            NirOpKind::Load { dest, ty, place } => {
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
                lowered.push(MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(dest.0)),
                    src,
                    width,
                });
            }
            NirOpKind::Store { place, src, ty } => {
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
                lowered.push(MirOp::Store {
                    dst,
                    src: src_value,
                    width,
                });
            }
            NirOpKind::Cast {
                dest,
                src,
                from,
                to,
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
                if let Some(routine_id) = routine_address_place(place, routine_ids) {
                    lowered.push(MirOp::Move {
                        dst: MirDef::VTemp(MirTempId(dest.0)),
                        src: MirValue::RoutineAddr(routine_id),
                        width,
                    });
                    continue;
                }
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
            NirOpKind::Compare {
                dest,
                ty,
                op,
                left,
                right,
            } => {
                let Some(width) = compare_width(left, right) else {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        "unsupported compare width",
                    ));
                    continue;
                };
                let signed = is_signed_compare(ty, left, right);
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
                    routine_system_addresses,
                    public_action_abi_routines,
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
            _ => {}
        }
    }
    lowered
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
    let mut ok = true;
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
                    ok = false;
                    continue;
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
            NirMachineItem::Raw(raw) => {
                diagnostics.push(MirDiagnostic::block(
                    routine,
                    block,
                    machine_raw_item_diagnostic(raw),
                ));
                ok = false;
            }
        }
    }
    ok.then_some(lowered)
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
    for routine in &nir_program.routines {
        for block in &routine.blocks {
            for op in &block.ops {
                if let NirOpKind::Define { name, value } = op
                    && let Some(value) = parse_machine_numeric_define_value(value)
                {
                    defines.insert(machine_name_key(name), value);
                }
            }
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
        .or_else(|| match resolve_builtin_target(name) {
            MirBuiltinResolution::Resolved { address } => Some(address),
            MirBuiltinResolution::Deferred { .. }
            | MirBuiltinResolution::Unsupported { .. }
            | MirBuiltinResolution::Unknown => None,
        })
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

fn machine_raw_item_diagnostic(raw: &str) -> String {
    match raw {
        "+" | "-" => {
            format!(
                "machine block item `{raw}` is not a byte-stream item; use it only inside an address expression"
            )
        }
        _ if raw.starts_with('$') || raw.chars().next().is_some_and(|ch| ch.is_ascii_digit()) => {
            format!("machine block item `{raw}` does not fit in 16 bits")
        }
        _ => format!("unsupported raw machine block item `{raw}`"),
    }
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
        clobbers: super::abi::opaque_machine_clobbers(),
        preserves: Default::default(),
        stack_depth_delta: None,
        may_call_os: effects.may_call_os,
        opaque: effects.opaque,
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
        MirPlaceShape::Unsupported(reason) => {
            unsupported_place(routine, block, reason, diagnostics);
            None
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
            .map(|mem| offset_mem(&mem, *offset));
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

fn routine_address_place(
    place: &NirPlace,
    routine_ids: &BTreeMap<&str, RoutineId>,
) -> Option<RoutineId> {
    match &place.kind {
        NirPlaceKind::Symbol(name) | NirPlaceKind::Global { name, .. } => {
            routine_ids.get(name.as_str()).copied()
        }
        _ => None,
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

fn is_signed_compare(ty: &NirType, left: &NirValueKind, right: &NirValueKind) -> bool {
    is_signed(ty) || value_is_signed(left) || value_is_signed(right)
}

fn compare_width(left: &NirValueKind, right: &NirValueKind) -> Option<MirWidth> {
    let left_width = compare_operand_width(left)?;
    let right_width = compare_operand_width(right)?;
    match (left_width, right_width) {
        (MirWidth::Word, MirWidth::Byte) if value_is_signed(right) => Some(MirWidth::Byte),
        (MirWidth::Byte, MirWidth::Word) if value_is_signed(left) => Some(MirWidth::Byte),
        (MirWidth::Word, _) | (_, MirWidth::Word) => Some(MirWidth::Word),
        (MirWidth::Byte, MirWidth::Byte) => Some(MirWidth::Byte),
    }
}

fn compare_operand_width(value: &NirValueKind) -> Option<MirWidth> {
    match value {
        NirValueKind::ConstU16(value) if *value <= u16::from(u8::MAX) => Some(MirWidth::Byte),
        _ => value_width(value),
    }
}

fn value_is_signed(value: &NirValueKind) -> bool {
    match value {
        NirValueKind::Temp { ty, .. } | NirValueKind::StaticAddr { ty, .. } => is_signed(ty),
        NirValueKind::ConstU8(_)
        | NirValueKind::ConstU16(_)
        | NirValueKind::Param(_)
        | NirValueKind::GlobalAddr(_) => false,
    }
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
        NirValueKind::Temp { ty, .. } | NirValueKind::StaticAddr { ty, .. } => mir_width(ty),
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
    match ty.width {
        Some(1) => Some(MirWidth::Byte),
        Some(2) => Some(MirWidth::Word),
        _ => None,
    }
}

fn local_storage_width(local: &nir::NirLocal) -> MirWidth {
    if local_pointer_backed_array(local) {
        MirWidth::Word
    } else {
        mir_width(&local.ty).unwrap_or(MirWidth::Byte)
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
        NirTerminator::Open | NirTerminator::Unknown(_) => block_ids
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
