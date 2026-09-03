use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use actionc::includes::load_program_with_expanded_source;
use actionc::nir::{
    self, BlockId, NirActivationModel, NirBinaryOp, NirBlock, NirBlockParam, NirCallConvention,
    NirCallEffects, NirCallableSignature, NirCallee, NirCastKind, NirCompareOp,
    NirDataAddressEncoding, NirDataAddressTarget, NirDataFragment, NirDataImage, NirEdge,
    NirForeignCodeKind, NirForeignCodePayload, NirForeignCodeTarget, NirGlobalBacking,
    NirGlobalInit, NirIntegerType, NirLinkValue, NirLocalBacking, NirLocalPurpose, NirMachineAtom,
    NirMachineByteSelector, NirMachineItem, NirMemoryAccess, NirMemoryEffects, NirMemoryRegion,
    NirMemoryRegionKind, NirOp, NirPlace, NirPlaceKind, NirProgram, NirRealOp, NirRealSource,
    NirRoutinePlacement, NirRuntimeBinding, NirRuntimeTarget, NirStorageClass, NirStorageDuration,
    NirStorageId, NirStorageInit, NirTemp, NirTempDef, NirTerminator, NirType, NirTypeKind,
    NirUnaryOp, NirValue, TempId, runtime_symbol_id,
};
use actionc::semantic::{SemanticOptions, analyze_with_options, ir};
use actionc::target::{AddressValue, ByteOffset, ByteSize, TargetId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirFixtureStage {
    Lowered,
    Optimized,
}

#[derive(Debug, Clone, Copy)]
pub struct NirFixtureCase {
    pub name: &'static str,
    pub source: &'static str,
    pub snapshot: &'static str,
    pub target: TargetId,
    pub stage: NirFixtureStage,
}

macro_rules! lowered_atari_case {
    ($name:literal) => {
        NirFixtureCase {
            name: $name,
            source: concat!("fixtures/nir/", $name, ".act"),
            snapshot: concat!("fixtures/nir/", $name, ".nir"),
            target: TargetId::Atari6502,
            stage: NirFixtureStage::Lowered,
        }
    };
}

macro_rules! lowered_target_case {
    ($name:literal, $source:literal, $snapshot:literal, $target:expr) => {
        NirFixtureCase {
            name: $name,
            source: concat!("fixtures/nir/", $source, ".act"),
            snapshot: concat!("fixtures/nir/", $snapshot, ".nir"),
            target: $target,
            stage: NirFixtureStage::Lowered,
        }
    };
}

macro_rules! optimized_atari_case {
    ($name:literal) => {
        NirFixtureCase {
            name: concat!($name, ".optimized"),
            source: concat!("fixtures/nir/", $name, ".act"),
            snapshot: concat!("fixtures/nir/", $name, ".optimized.nir"),
            target: TargetId::Atari6502,
            stage: NirFixtureStage::Optimized,
        }
    };
}

pub const NIR_FIXTURE_CASES: &[NirFixtureCase] = &[
    lowered_atari_case!("activation_storage"),
    lowered_atari_case!("aggregate_static_initializer"),
    lowered_atari_case!("bare_do"),
    lowered_atari_case!("calls_returns"),
    lowered_atari_case!("conditions"),
    lowered_atari_case!("control_flow"),
    lowered_atari_case!("call_forms"),
    lowered_atari_case!("data_relocations"),
    lowered_atari_case!("for_condition"),
    lowered_atari_case!("inline_asm_fixed_array"),
    lowered_atari_case!("integer_operations"),
    lowered_atari_case!("layout_queries"),
    lowered_atari_case!("lexical_blocks"),
    lowered_atari_case!("lexical_type_scopes"),
    lowered_atari_case!("local_aggregate_declarations"),
    lowered_atari_case!("local_storage_views"),
    NirFixtureCase {
        name: "link_value_initializer",
        source: "fixtures/mir6502/set_symbol_current_location.act",
        snapshot: "fixtures/nir/link_value_initializer.nir",
        target: TargetId::Atari6502,
        stage: NirFixtureStage::Lowered,
    },
    lowered_atari_case!("lvalues"),
    lowered_atari_case!("machine_blocks"),
    lowered_atari_case!("native_real"),
    lowered_atari_case!("native_real_storage"),
    lowered_atari_case!("optimizer_local_promotion"),
    lowered_atari_case!("pointer_operations"),
    lowered_atari_case!("record_copy"),
    lowered_atari_case!("records_fields"),
    lowered_atari_case!("scalar_assignments"),
    lowered_atari_case!("static_ids_and_routine_addr"),
    lowered_atari_case!("string_static"),
    lowered_atari_case!("target_layout_matrix"),
    lowered_atari_case!("unary_cast"),
    lowered_atari_case!("volatile_record_copy"),
    lowered_atari_case!("word_addition"),
    lowered_target_case!(
        "activation_storage.wdc-65816-native",
        "activation_storage",
        "activation_storage.wdc-65816-native",
        TargetId::Wdc65816Native
    ),
    lowered_target_case!(
        "activation_storage.wdc-65816-small",
        "activation_storage",
        "activation_storage.wdc-65816-small",
        TargetId::Wdc65816Small
    ),
    lowered_target_case!(
        "activation_storage.motorola-68000",
        "activation_storage",
        "activation_storage.motorola-68000",
        TargetId::Motorola68000
    ),
    lowered_target_case!(
        "native_entry_initialization.wdc-65816-native",
        "native_entry_initialization",
        "native_entry_initialization.wdc-65816-native",
        TargetId::Wdc65816Native
    ),
    lowered_target_case!(
        "native_entry_initialization.wdc-65816-small",
        "native_entry_initialization",
        "native_entry_initialization.wdc-65816-small",
        TargetId::Wdc65816Small
    ),
    lowered_target_case!(
        "native_entry_initialization.motorola-68000",
        "native_entry_initialization",
        "native_entry_initialization.motorola-68000",
        TargetId::Motorola68000
    ),
    lowered_target_case!(
        "target_layout_matrix.wdc-65816-native",
        "target_layout_matrix",
        "target_layout_matrix.wdc-65816-native",
        TargetId::Wdc65816Native
    ),
    lowered_target_case!(
        "target_layout_matrix.wdc-65816-small",
        "target_layout_matrix",
        "target_layout_matrix.wdc-65816-small",
        TargetId::Wdc65816Small
    ),
    lowered_target_case!(
        "target_layout_matrix.motorola-68000",
        "target_layout_matrix",
        "target_layout_matrix.motorola-68000",
        TargetId::Motorola68000
    ),
    lowered_target_case!(
        "target_data_layout.atari-6502",
        "target_data_layout",
        "target_data_layout.atari-6502",
        TargetId::Atari6502
    ),
    lowered_target_case!(
        "target_data_layout.wdc-65816-native",
        "target_data_layout",
        "target_data_layout.wdc-65816-native",
        TargetId::Wdc65816Native
    ),
    lowered_target_case!(
        "target_data_layout.wdc-65816-small",
        "target_data_layout",
        "target_data_layout.wdc-65816-small",
        TargetId::Wdc65816Small
    ),
    lowered_target_case!(
        "target_data_layout.motorola-68000",
        "target_data_layout",
        "target_data_layout.motorola-68000",
        TargetId::Motorola68000
    ),
    optimized_atari_case!("control_flow"),
    optimized_atari_case!("local_storage_views"),
    optimized_atari_case!("optimizer_local_promotion"),
    optimized_atari_case!("scalar_assignments"),
    optimized_atari_case!("unary_cast"),
];

pub fn lower_case(repo_root: &Path, case: NirFixtureCase) -> NirProgram {
    let source = repo_root.join(case.source);
    let loaded = load_program_with_expanded_source(&source)
        .unwrap_or_else(|error| panic!("load {}: {error:?}", source.display()));
    let model = analyze_with_options(
        &loaded.program,
        SemanticOptions::modern().with_target(case.target),
    )
    .unwrap_or_else(|error| panic!("analyze {}: {error:?}", source.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let lowered = nir::lower_program(&semir);
    nir::verify_program(&lowered)
        .unwrap_or_else(|error| panic!("verify lowered NIR for {}: {error:?}", source.display()));

    match case.stage {
        NirFixtureStage::Lowered => lowered,
        NirFixtureStage::Optimized => nir::optimize_program(&lowered).unwrap_or_else(|error| {
            panic!(
                "optimize and verify NIR for {}: {error:?}",
                source.display()
            )
        }),
    }
}

pub fn snapshot_path(repo_root: &Path, case: NirFixtureCase) -> PathBuf {
    repo_root.join(case.snapshot)
}

/// Valid NIR shapes that currently have no source-language producer. Keep
/// these probes small and migrate them to ordinary source cases if lowering
/// later gains a direct producer.
pub fn structural_coverage_programs(repo_root: &Path) -> Vec<NirProgram> {
    let mut exit = lower_case(repo_root, lowered_atari_case!("scalar_assignments"));
    exit.routines
        .iter_mut()
        .find(|routine| routine.name == "Main")
        .and_then(|routine| routine.blocks.last_mut())
        .expect("scalar assignment Main block")
        .terminator = NirTerminator::Exit;
    nir::verify_program(&exit).expect("construction-only Exit terminator must verify");

    let mut calls = lower_case(repo_root, lowered_atari_case!("scalar_assignments"));
    let runtime = runtime_symbol_id("CoverageRuntime");
    let bound_runtime = runtime_symbol_id("CoverageBoundRuntime");
    let routine_runtime = runtime_symbol_id("CoverageRoutineRuntime");
    calls.runtime_bindings.extend([
        NirRuntimeBinding {
            symbol: runtime,
            name: "CoverageRuntime".to_string(),
            target: None,
        },
        NirRuntimeBinding {
            symbol: bound_runtime,
            name: "CoverageBoundRuntime".to_string(),
            target: Some(NirRuntimeTarget::Absolute(AddressValue::code(0xE456))),
        },
        NirRuntimeBinding {
            symbol: routine_runtime,
            name: "CoverageRoutineRuntime".to_string(),
            target: Some(NirRuntimeTarget::Routine(calls.routines[0].id)),
        },
    ]);
    let global = calls.globals[0].id;
    let global_name = calls.globals[0].name.clone();
    let global_type = calls.globals[0].ty.clone().expect("typed scalar global");
    let main = calls
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Main")
        .expect("scalar assignment Main routine");
    main.blocks[0].ops.extend([
        NirOp::Store {
            place: NirPlace {
                kind: NirPlaceKind::Absolute(AddressValue::data(0xD000)),
                ty: Some(global_type.clone()),
            },
            src: NirValue::ConstU8(1),
            ty: global_type.clone(),
        },
        NirOp::Call {
            callee: NirCallee::Builtin("CoverageBuiltin".to_string()),
            args: Vec::new(),
            result: None,
            signature: Some(NirCallableSignature::empty_proc(NirCallConvention::Runtime)),
            effects: NirCallEffects {
                memory: NirMemoryEffects {
                    reads: NirMemoryAccess::All,
                    writes: NirMemoryAccess::Regions(vec![NirMemoryRegion {
                        kind: NirMemoryRegionKind::Storage(NirStorageId::Global(global)),
                        offset: ByteOffset::ZERO,
                        size: ByteSize::ONE,
                    }]),
                },
                may_call_external: false,
                opaque: false,
            },
        },
        NirOp::Call {
            callee: NirCallee::Runtime {
                symbol: runtime,
                name: "CoverageRuntime".to_string(),
            },
            args: Vec::new(),
            result: None,
            signature: Some(NirCallableSignature::empty_proc(NirCallConvention::Runtime)),
            effects: NirCallEffects {
                memory: NirMemoryEffects {
                    reads: NirMemoryAccess::None,
                    writes: NirMemoryAccess::None,
                },
                may_call_external: true,
                opaque: true,
            },
        },
    ]);
    nir::verify_program(&calls).expect("construction-only call forms must verify");

    let mut block_parameter = lower_case(repo_root, lowered_atari_case!("scalar_assignments"));
    let main = block_parameter
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Main")
        .expect("scalar assignment Main routine");
    main.temps.push(NirTemp {
        id: TempId(1),
        ty: global_type.clone(),
        def: NirTempDef {
            block: BlockId(1),
            op_index: None,
        },
    });
    main.blocks[0].terminator = NirTerminator::Goto(NirEdge {
        target: BlockId(1),
        args: vec![NirValue::ConstU8(9)],
    });
    main.blocks.push(NirBlock {
        id: BlockId(1),
        label: "block_parameter".to_string(),
        params: vec![NirBlockParam {
            dest: TempId(1),
            ty: global_type.clone(),
        }],
        ops: vec![NirOp::Store {
            place: NirPlace {
                kind: NirPlaceKind::Global {
                    id: global,
                    name: global_name,
                },
                ty: Some(global_type.clone()),
            },
            src: NirValue::Temp {
                id: TempId(1),
                ty: global_type.clone(),
            },
            ty: global_type,
        }],
        terminator: NirTerminator::Return(None),
    });
    nir::verify_program(&block_parameter).expect("construction-only block parameter must verify");

    let mut data_shapes = lower_case(repo_root, lowered_atari_case!("data_relocations"));
    let routine = data_shapes.routines[0].id;
    let alias_target = data_shapes.globals[0].id;
    data_shapes.globals[1].backing = NirGlobalBacking::Alias {
        target: alias_target,
        offset: ByteOffset::ZERO,
    };
    if let Some(NirGlobalInit::Bytes { image, .. }) = &mut data_shapes.globals[1].init
        && let Some(NirDataFragment::Address { target, .. }) = image.fragments.first_mut()
    {
        *target = NirDataAddressTarget::Absolute(AddressValue::data(0x2000));
    }
    let descriptor_size = data_shapes.globals[2].storage_size;
    data_shapes.globals[2].init = Some(NirGlobalInit::RoutineAddress {
        routine,
        descriptor_size,
        size_word: None,
        mutable: true,
        section: "global".to_string(),
    });
    nir::verify_program(&data_shapes).expect("construction-only data forms must verify");

    vec![exit, calls, block_parameter, data_shapes]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NirFeature {
    TargetAtari6502,
    TargetWdc65816Native,
    TargetWdc65816Small,
    TargetMotorola68000,
    RuntimeBindingUnbound,
    RuntimeBindingAbsolute,
    RuntimeBindingRoutine,
    TypeVoid,
    TypeBool,
    TypeU8,
    TypeI8,
    TypeU16,
    TypeI16,
    TypeReal,
    TypePointer,
    TypeRecord,
    TypeCallable,
    TypeError,
    GlobalBackingOrdinary,
    GlobalBackingAbsolute,
    GlobalBackingAlias,
    GlobalInitBytes,
    GlobalInitDescriptor,
    GlobalInitZeroFill,
    GlobalInitLinkValue,
    GlobalInitRoutineAddress,
    StorageInitBytes,
    StorageInitDescriptor,
    StorageInitZeroFill,
    DataFragmentInteger,
    DataFragmentAddress,
    DataAddressEncodingPointer,
    DataAddressEncodingTargetByte,
    DataAddressTargetStorage,
    DataAddressTargetRoutine,
    DataAddressTargetAbsolute,
    LocalPurposeStorage,
    LocalPurposeAggregateBacking,
    LocalPurposeRealTemporary,
    StorageClassScalar,
    StorageClassArray,
    StorageClassRecord,
    StorageClassType,
    StorageDurationAutomatic,
    StorageDurationRoutineStatic,
    StorageDurationExternal,
    LocalBackingOrdinary,
    LocalBackingAbsolute,
    LocalBackingAlias,
    LocalBackingGlobalAlias,
    ActivationClassicStatic,
    ActivationNativeReentrant,
    ConventionTargetInternal,
    ConventionTargetPublic,
    ConventionRuntime,
    ConventionExternal,
    PlacementRelocatable,
    PlacementCurrentLocation,
    PlacementAbsolute,
    BlockParameter,
    OpLoad,
    OpVolatileLoad,
    OpAddrOf,
    OpStore,
    OpVolatileStore,
    OpCopyBytes,
    OpCopyBytesSourceVolatile,
    OpCopyBytesDestinationVolatile,
    OpUnaryPlus,
    OpUnaryNeg,
    OpCastInteger,
    OpCastPointer,
    OpCastIntegerToPointer,
    OpCastPointerToInteger,
    OpPointerOffsetAdd,
    OpPointerOffsetSubtract,
    OpBinaryAdd,
    OpBinarySub,
    OpBinaryMul,
    OpBinaryDiv,
    OpBinaryMod,
    OpBinaryLsh,
    OpBinaryRsh,
    OpBinaryAnd,
    OpBinaryOr,
    OpBinaryXor,
    OpCompareEq,
    OpCompareNe,
    OpCompareLt,
    OpCompareLe,
    OpCompareGt,
    OpCompareGe,
    OpRealCopy,
    OpRealUnary,
    OpRealBinary,
    OpRealCompare,
    OpIntegerToReal,
    OpRealToInteger,
    OpCall,
    OpForeignCode,
    OpUnsupported,
    CalleeUser,
    CalleeBuiltin,
    CalleeIndirect,
    CalleeRuntime,
    ForeignLegacyMachineBlock,
    ForeignInlineAssembly,
    ForeignPayloadStructured,
    ForeignPayloadBytes,
    ForeignTargetStorage,
    ForeignTargetRoutine,
    ForeignTargetAbsolute,
    ForeignTargetInlineOffset,
    MachineItemByte,
    MachineItemWord,
    MachineItemStringLiteral,
    MachineItemCharLiteral,
    MachineItemName,
    MachineItemAddressExpr,
    MachineItemAddressByte,
    MachineItemRelocation,
    MachineAtomNumber,
    MachineAtomName,
    MachineAtomCurrent,
    MachineSelectorNone,
    MachineSelectorLow,
    MachineSelectorHigh,
    EffectsPure,
    EffectsRegions,
    EffectsUnknown,
    EffectsAll,
    EffectsMayCallExternal,
    EffectsOpaque,
    RegionStorageLocal,
    RegionStorageParam,
    RegionStorageGlobal,
    RegionStatic,
    RegionAbsolute,
    PlaceParam,
    PlaceLocal,
    PlaceGlobal,
    PlaceAbsolute,
    PlaceDeref,
    PlaceIndex,
    PlaceField,
    ValueConstU8,
    ValueConstU16,
    ValueNull,
    ValueAddressConst,
    ValueStaticAddr,
    ValueTemp,
    ValueParam,
    ValueGlobalAddr,
    ValueRoutineAddr,
    TerminatorOpen,
    TerminatorFallthrough,
    TerminatorGoto,
    TerminatorBranch,
    TerminatorReturnVoid,
    TerminatorReturnValue,
    TerminatorExit,
}

/// The positive suite's executable-shape floor. `Unsupported`, `Open`, and the
/// legacy untyped `Param`/`GlobalAddr` values are deliberately absent: clean
/// NIR must reject or eliminate those shapes instead of treating them as
/// executable contracts.
pub const REQUIRED_EXECUTABLE_FEATURES: &[NirFeature] = &[
    NirFeature::BlockParameter,
    NirFeature::OpLoad,
    NirFeature::OpVolatileLoad,
    NirFeature::OpAddrOf,
    NirFeature::OpStore,
    NirFeature::OpVolatileStore,
    NirFeature::OpCopyBytes,
    NirFeature::OpCopyBytesSourceVolatile,
    NirFeature::OpCopyBytesDestinationVolatile,
    NirFeature::OpUnaryPlus,
    NirFeature::OpUnaryNeg,
    NirFeature::OpCastInteger,
    NirFeature::OpCastPointer,
    NirFeature::OpCastIntegerToPointer,
    NirFeature::OpCastPointerToInteger,
    NirFeature::OpPointerOffsetAdd,
    NirFeature::OpPointerOffsetSubtract,
    NirFeature::OpBinaryAdd,
    NirFeature::OpBinarySub,
    NirFeature::OpBinaryMul,
    NirFeature::OpBinaryDiv,
    NirFeature::OpBinaryMod,
    NirFeature::OpBinaryLsh,
    NirFeature::OpBinaryRsh,
    NirFeature::OpBinaryAnd,
    NirFeature::OpBinaryOr,
    NirFeature::OpBinaryXor,
    NirFeature::OpCompareEq,
    NirFeature::OpCompareNe,
    NirFeature::OpCompareLt,
    NirFeature::OpCompareLe,
    NirFeature::OpCompareGt,
    NirFeature::OpCompareGe,
    NirFeature::OpRealCopy,
    NirFeature::OpRealUnary,
    NirFeature::OpRealBinary,
    NirFeature::OpRealCompare,
    NirFeature::OpIntegerToReal,
    NirFeature::OpRealToInteger,
    NirFeature::OpCall,
    NirFeature::OpForeignCode,
    NirFeature::CalleeUser,
    NirFeature::CalleeBuiltin,
    NirFeature::CalleeIndirect,
    NirFeature::CalleeRuntime,
    NirFeature::PlaceParam,
    NirFeature::PlaceLocal,
    NirFeature::PlaceGlobal,
    NirFeature::PlaceAbsolute,
    NirFeature::PlaceDeref,
    NirFeature::PlaceIndex,
    NirFeature::PlaceField,
    NirFeature::ValueConstU8,
    NirFeature::ValueConstU16,
    NirFeature::ValueNull,
    NirFeature::ValueAddressConst,
    NirFeature::ValueStaticAddr,
    NirFeature::ValueTemp,
    NirFeature::ValueRoutineAddr,
    NirFeature::TerminatorFallthrough,
    NirFeature::TerminatorGoto,
    NirFeature::TerminatorBranch,
    NirFeature::TerminatorReturnVoid,
    NirFeature::TerminatorReturnValue,
    NirFeature::TerminatorExit,
];

pub const REQUIRED_TARGET_FEATURES: &[NirFeature] = &[
    NirFeature::TargetAtari6502,
    NirFeature::TargetWdc65816Native,
    NirFeature::TargetWdc65816Small,
    NirFeature::TargetMotorola68000,
    NirFeature::StorageDurationAutomatic,
    NirFeature::StorageDurationRoutineStatic,
    NirFeature::ActivationClassicStatic,
    NirFeature::ActivationNativeReentrant,
];

pub fn collect_features(program: &NirProgram) -> BTreeSet<NirFeature> {
    let mut features = BTreeSet::new();
    features.insert(match program.target_layout.target {
        TargetId::Atari6502 => NirFeature::TargetAtari6502,
        TargetId::Wdc65816Native => NirFeature::TargetWdc65816Native,
        TargetId::Wdc65816Small => NirFeature::TargetWdc65816Small,
        TargetId::Motorola68000 => NirFeature::TargetMotorola68000,
    });

    for binding in &program.runtime_bindings {
        features.insert(match binding.target {
            None => NirFeature::RuntimeBindingUnbound,
            Some(NirRuntimeTarget::Absolute(_)) => NirFeature::RuntimeBindingAbsolute,
            Some(NirRuntimeTarget::Routine(_)) => NirFeature::RuntimeBindingRoutine,
        });
    }
    for global in &program.globals {
        features.insert(match global.backing {
            NirGlobalBacking::Ordinary => NirFeature::GlobalBackingOrdinary,
            NirGlobalBacking::Absolute(_) => NirFeature::GlobalBackingAbsolute,
            NirGlobalBacking::Alias { .. } => NirFeature::GlobalBackingAlias,
        });
        if let Some(ty) = &global.ty {
            visit_type(ty, &mut features);
        }
        if let Some(init) = &global.init {
            visit_global_init(init, &mut features);
        }
    }
    for static_data in &program.statics {
        visit_type(&static_data.ty, &mut features);
        visit_data_image(&static_data.image, &mut features);
    }
    for routine in &program.routines {
        visit_convention(routine.convention, &mut features);
        features.insert(match routine.activation {
            NirActivationModel::ClassicStatic => NirFeature::ActivationClassicStatic,
            NirActivationModel::NativeReentrant => NirFeature::ActivationNativeReentrant,
        });
        features.insert(match routine.entry.placement {
            NirRoutinePlacement::Relocatable => NirFeature::PlacementRelocatable,
            NirRoutinePlacement::CurrentLocation => NirFeature::PlacementCurrentLocation,
            NirRoutinePlacement::Absolute(_) => NirFeature::PlacementAbsolute,
        });
        visit_signature(&routine.signature, &mut features);
        for param in &routine.params {
            visit_storage_class(param.storage, &mut features);
            visit_duration(param.duration, &mut features);
            visit_type(&param.ty, &mut features);
        }
        for local in &routine.locals {
            features.insert(match local.purpose {
                NirLocalPurpose::Storage => NirFeature::LocalPurposeStorage,
                NirLocalPurpose::AggregateBacking { .. } => {
                    NirFeature::LocalPurposeAggregateBacking
                }
                NirLocalPurpose::RealTemporary => NirFeature::LocalPurposeRealTemporary,
            });
            visit_storage_class(local.storage, &mut features);
            visit_duration(local.duration, &mut features);
            visit_type(&local.ty, &mut features);
            features.insert(match local.backing {
                NirLocalBacking::Ordinary => NirFeature::LocalBackingOrdinary,
                NirLocalBacking::Absolute(_) => NirFeature::LocalBackingAbsolute,
                NirLocalBacking::Alias { .. } => NirFeature::LocalBackingAlias,
                NirLocalBacking::GlobalAlias { .. } => NirFeature::LocalBackingGlobalAlias,
            });
            if let Some(init) = &local.init {
                visit_storage_init(init, &mut features);
            }
        }
        for temp in &routine.temps {
            visit_type(&temp.ty, &mut features);
        }
        for block in &routine.blocks {
            if !block.params.is_empty() {
                features.insert(NirFeature::BlockParameter);
            }
            for param in &block.params {
                visit_type(&param.ty, &mut features);
            }
            for op in &block.ops {
                visit_op(op, &mut features);
            }
            visit_terminator(&block.terminator, &mut features);
        }
    }
    features
}

pub fn format_feature_inventory(features: &BTreeSet<NirFeature>) -> String {
    let mut output = String::from("# Generated NIR fixture feature inventory.\n");
    output.push_str("# Update deliberately when fixture coverage changes.\n");
    for feature in features {
        output.push_str(&format!("{feature:?}\n"));
    }
    output
}

fn visit_type(ty: &NirType, features: &mut BTreeSet<NirFeature>) {
    match &ty.kind {
        NirTypeKind::Void => features.insert(NirFeature::TypeVoid),
        NirTypeKind::Bool => features.insert(NirFeature::TypeBool),
        NirTypeKind::Integer(integer) if *integer == NirIntegerType::U8 => {
            features.insert(NirFeature::TypeU8)
        }
        NirTypeKind::Integer(integer) if *integer == NirIntegerType::I8 => {
            features.insert(NirFeature::TypeI8)
        }
        NirTypeKind::Integer(integer) if *integer == NirIntegerType::U16 => {
            features.insert(NirFeature::TypeU16)
        }
        NirTypeKind::Integer(integer) if *integer == NirIntegerType::I16 => {
            features.insert(NirFeature::TypeI16)
        }
        NirTypeKind::Integer(_) => false,
        NirTypeKind::Real => features.insert(NirFeature::TypeReal),
        NirTypeKind::Pointer { pointee, .. } => {
            features.insert(NirFeature::TypePointer);
            if let Some(pointee) = pointee {
                visit_type_kind(pointee, features);
            }
            true
        }
        NirTypeKind::Record { .. } => features.insert(NirFeature::TypeRecord),
        NirTypeKind::Callable { convention, .. } => {
            features.insert(NirFeature::TypeCallable);
            visit_convention(*convention, features);
            true
        }
        NirTypeKind::Error => features.insert(NirFeature::TypeError),
    };
}

fn visit_type_kind(kind: &NirTypeKind, features: &mut BTreeSet<NirFeature>) {
    let ty = NirType {
        kind: kind.clone(),
        summary: String::new(),
        width: None,
        pointer: matches!(kind, NirTypeKind::Pointer { .. }),
    };
    visit_type(&ty, features);
}

fn visit_signature(
    signature: &actionc::nir::NirCallableSignature,
    features: &mut BTreeSet<NirFeature>,
) {
    visit_convention(signature.convention, features);
    for param in &signature.params {
        visit_type(param, features);
    }
    if let Some(variadic) = &signature.variadic {
        visit_type(variadic, features);
    }
    if let Some(result) = &signature.result {
        visit_type(result, features);
    }
}

fn visit_convention(convention: NirCallConvention, features: &mut BTreeSet<NirFeature>) {
    features.insert(match convention {
        NirCallConvention::TargetInternal => NirFeature::ConventionTargetInternal,
        NirCallConvention::TargetPublic => NirFeature::ConventionTargetPublic,
        NirCallConvention::Runtime => NirFeature::ConventionRuntime,
        NirCallConvention::External(_) => NirFeature::ConventionExternal,
    });
}

fn visit_storage_class(class: NirStorageClass, features: &mut BTreeSet<NirFeature>) {
    features.insert(match class {
        NirStorageClass::Scalar => NirFeature::StorageClassScalar,
        NirStorageClass::Array => NirFeature::StorageClassArray,
        NirStorageClass::Record => NirFeature::StorageClassRecord,
        NirStorageClass::Type => NirFeature::StorageClassType,
    });
}

fn visit_duration(duration: NirStorageDuration, features: &mut BTreeSet<NirFeature>) {
    features.insert(match duration {
        NirStorageDuration::Automatic => NirFeature::StorageDurationAutomatic,
        NirStorageDuration::RoutineStatic => NirFeature::StorageDurationRoutineStatic,
        NirStorageDuration::External => NirFeature::StorageDurationExternal,
    });
}

fn visit_global_init(init: &NirGlobalInit, features: &mut BTreeSet<NirFeature>) {
    match init {
        NirGlobalInit::Bytes { image, .. } => {
            features.insert(NirFeature::GlobalInitBytes);
            visit_data_image(image, features);
        }
        NirGlobalInit::Descriptor { backing, .. } => {
            features.insert(NirFeature::GlobalInitDescriptor);
            visit_data_image(&backing.image, features);
        }
        NirGlobalInit::ZeroFill { .. } => {
            features.insert(NirFeature::GlobalInitZeroFill);
        }
        NirGlobalInit::LinkValue { value, .. } => {
            match value {
                NirLinkValue::ImageEndAddress => {}
            }
            features.insert(NirFeature::GlobalInitLinkValue);
        }
        NirGlobalInit::RoutineAddress { .. } => {
            features.insert(NirFeature::GlobalInitRoutineAddress);
        }
    }
}

fn visit_storage_init(init: &NirStorageInit, features: &mut BTreeSet<NirFeature>) {
    match init {
        NirStorageInit::Bytes { image, .. } => {
            features.insert(NirFeature::StorageInitBytes);
            visit_data_image(image, features);
        }
        NirStorageInit::Descriptor { backing, .. } => {
            features.insert(NirFeature::StorageInitDescriptor);
            visit_data_image(&backing.image, features);
        }
        NirStorageInit::ZeroFill { .. } => {
            features.insert(NirFeature::StorageInitZeroFill);
        }
    }
}

fn visit_data_image(image: &NirDataImage, features: &mut BTreeSet<NirFeature>) {
    for fragment in &image.fragments {
        match fragment {
            NirDataFragment::Integer { .. } => {
                features.insert(NirFeature::DataFragmentInteger);
            }
            NirDataFragment::Address {
                encoding, target, ..
            } => {
                features.insert(NirFeature::DataFragmentAddress);
                features.insert(match encoding {
                    NirDataAddressEncoding::Pointer { .. } => {
                        NirFeature::DataAddressEncodingPointer
                    }
                    NirDataAddressEncoding::TargetByte { .. } => {
                        NirFeature::DataAddressEncodingTargetByte
                    }
                });
                features.insert(match target {
                    NirDataAddressTarget::Storage(_) => NirFeature::DataAddressTargetStorage,
                    NirDataAddressTarget::Routine(_) => NirFeature::DataAddressTargetRoutine,
                    NirDataAddressTarget::Absolute(_) => NirFeature::DataAddressTargetAbsolute,
                });
            }
        }
    }
}

fn visit_op(op: &NirOp, features: &mut BTreeSet<NirFeature>) {
    match op {
        NirOp::Load { ty, place, .. } => {
            features.insert(NirFeature::OpLoad);
            visit_type(ty, features);
            visit_place(place, features);
        }
        NirOp::VolatileLoad { ty, place, .. } => {
            features.insert(NirFeature::OpVolatileLoad);
            visit_type(ty, features);
            visit_place(place, features);
        }
        NirOp::AddrOf { ty, place, .. } => {
            features.insert(NirFeature::OpAddrOf);
            visit_type(ty, features);
            visit_place(place, features);
        }
        NirOp::Store { place, src, ty } => {
            features.insert(NirFeature::OpStore);
            visit_place(place, features);
            visit_value(src, features);
            visit_type(ty, features);
        }
        NirOp::VolatileStore { place, src, ty } => {
            features.insert(NirFeature::OpVolatileStore);
            visit_place(place, features);
            visit_value(src, features);
            visit_type(ty, features);
        }
        NirOp::CopyBytes {
            destination,
            source,
            destination_volatile,
            source_volatile,
            ..
        } => {
            features.insert(NirFeature::OpCopyBytes);
            if *source_volatile {
                features.insert(NirFeature::OpCopyBytesSourceVolatile);
            }
            if *destination_volatile {
                features.insert(NirFeature::OpCopyBytesDestinationVolatile);
            }
            visit_place(destination, features);
            visit_place(source, features);
        }
        NirOp::Unary { ty, op, src, .. } => {
            features.insert(match op {
                NirUnaryOp::Plus => NirFeature::OpUnaryPlus,
                NirUnaryOp::Neg => NirFeature::OpUnaryNeg,
            });
            visit_type(ty, features);
            visit_value(src, features);
        }
        NirOp::Cast {
            src,
            from,
            to,
            kind,
            ..
        } => {
            features.insert(match kind {
                NirCastKind::Integer => NirFeature::OpCastInteger,
                NirCastKind::Pointer => NirFeature::OpCastPointer,
                NirCastKind::IntegerToPointer => NirFeature::OpCastIntegerToPointer,
                NirCastKind::PointerToInteger => NirFeature::OpCastPointerToInteger,
            });
            visit_value(src, features);
            visit_type(from, features);
            visit_type(to, features);
        }
        NirOp::PointerOffset {
            ty,
            base,
            offset,
            subtract,
            ..
        } => {
            features.insert(if *subtract {
                NirFeature::OpPointerOffsetSubtract
            } else {
                NirFeature::OpPointerOffsetAdd
            });
            visit_type(ty, features);
            visit_value(base, features);
            visit_value(offset, features);
        }
        NirOp::Binary {
            ty,
            op,
            left,
            right,
            ..
        } => {
            features.insert(binary_feature(*op));
            visit_type(ty, features);
            visit_value(left, features);
            visit_value(right, features);
        }
        NirOp::Compare {
            ty,
            operand_ty,
            op,
            left,
            right,
            ..
        } => {
            features.insert(compare_feature(*op));
            visit_type(ty, features);
            visit_type(operand_ty, features);
            visit_value(left, features);
            visit_value(right, features);
        }
        NirOp::Real(op) => visit_real_op(op, features),
        NirOp::Call {
            callee,
            args,
            result,
            signature,
            effects,
        } => {
            features.insert(NirFeature::OpCall);
            visit_callee(callee, features);
            for arg in args {
                visit_value(arg, features);
            }
            if let Some(result) = result {
                visit_type(&result.ty, features);
            }
            if let Some(signature) = signature {
                visit_signature(signature, features);
            }
            visit_effects(
                &effects.memory,
                effects.may_call_external,
                effects.opaque,
                features,
            );
        }
        NirOp::ForeignCode { code, effects } => {
            features.insert(NirFeature::OpForeignCode);
            features.insert(match code.kind {
                NirForeignCodeKind::LegacyMachineBlock => NirFeature::ForeignLegacyMachineBlock,
                NirForeignCodeKind::InlineAssembly => NirFeature::ForeignInlineAssembly,
            });
            match &code.payload {
                NirForeignCodePayload::Structured(items) => {
                    features.insert(NirFeature::ForeignPayloadStructured);
                    for item in items {
                        visit_machine_item(item, features);
                    }
                }
                NirForeignCodePayload::Bytes { relocations, .. } => {
                    features.insert(NirFeature::ForeignPayloadBytes);
                    for relocation in relocations {
                        visit_foreign_target(relocation.target, features);
                    }
                }
            }
            visit_effects(
                &effects.memory,
                effects.may_call_external,
                effects.opaque,
                features,
            );
        }
        NirOp::Unsupported { .. } => {
            features.insert(NirFeature::OpUnsupported);
        }
    }
}

fn binary_feature(op: NirBinaryOp) -> NirFeature {
    match op {
        NirBinaryOp::Add => NirFeature::OpBinaryAdd,
        NirBinaryOp::Sub => NirFeature::OpBinarySub,
        NirBinaryOp::Mul => NirFeature::OpBinaryMul,
        NirBinaryOp::Div => NirFeature::OpBinaryDiv,
        NirBinaryOp::Mod => NirFeature::OpBinaryMod,
        NirBinaryOp::Lsh => NirFeature::OpBinaryLsh,
        NirBinaryOp::Rsh => NirFeature::OpBinaryRsh,
        NirBinaryOp::And => NirFeature::OpBinaryAnd,
        NirBinaryOp::Or => NirFeature::OpBinaryOr,
        NirBinaryOp::Xor => NirFeature::OpBinaryXor,
    }
}

fn compare_feature(op: NirCompareOp) -> NirFeature {
    match op {
        NirCompareOp::Eq => NirFeature::OpCompareEq,
        NirCompareOp::Ne => NirFeature::OpCompareNe,
        NirCompareOp::Lt => NirFeature::OpCompareLt,
        NirCompareOp::Le => NirFeature::OpCompareLe,
        NirCompareOp::Gt => NirFeature::OpCompareGt,
        NirCompareOp::Ge => NirFeature::OpCompareGe,
    }
}

fn visit_real_op(op: &NirRealOp, features: &mut BTreeSet<NirFeature>) {
    match op {
        NirRealOp::Copy {
            destination,
            source,
        } => {
            features.insert(NirFeature::OpRealCopy);
            visit_place(destination, features);
            visit_real_source(source, features);
        }
        NirRealOp::Unary {
            destination,
            operand,
            ..
        } => {
            features.insert(NirFeature::OpRealUnary);
            visit_place(destination, features);
            visit_real_source(operand, features);
        }
        NirRealOp::Binary {
            destination,
            left,
            right,
            ..
        } => {
            features.insert(NirFeature::OpRealBinary);
            visit_place(destination, features);
            visit_real_source(left, features);
            visit_real_source(right, features);
        }
        NirRealOp::Compare { left, right, .. } => {
            features.insert(NirFeature::OpRealCompare);
            visit_real_source(left, features);
            visit_real_source(right, features);
        }
        NirRealOp::IntegerToReal {
            destination,
            source,
            source_type,
        } => {
            features.insert(NirFeature::OpIntegerToReal);
            visit_place(destination, features);
            visit_value(source, features);
            visit_type(source_type, features);
        }
        NirRealOp::RealToInteger {
            result_type,
            source,
            ..
        } => {
            features.insert(NirFeature::OpRealToInteger);
            visit_type(result_type, features);
            visit_place(source, features);
        }
    }
}

fn visit_real_source(source: &NirRealSource, features: &mut BTreeSet<NirFeature>) {
    match source {
        NirRealSource::Place(place) => visit_place(place, features),
        NirRealSource::Static { .. } => {
            features.insert(NirFeature::ValueStaticAddr);
        }
    }
}

fn visit_callee(callee: &NirCallee, features: &mut BTreeSet<NirFeature>) {
    let feature = match callee {
        NirCallee::User { .. } => NirFeature::CalleeUser,
        NirCallee::Builtin(_) => NirFeature::CalleeBuiltin,
        NirCallee::Indirect { target, ty } => {
            visit_value(target, features);
            visit_type(ty, features);
            NirFeature::CalleeIndirect
        }
        NirCallee::Runtime { .. } => NirFeature::CalleeRuntime,
    };
    features.insert(feature);
}

fn visit_effects(
    effects: &NirMemoryEffects,
    may_call_external: bool,
    opaque: bool,
    features: &mut BTreeSet<NirFeature>,
) {
    visit_memory_access(&effects.reads, features);
    visit_memory_access(&effects.writes, features);
    if may_call_external {
        features.insert(NirFeature::EffectsMayCallExternal);
    }
    if opaque {
        features.insert(NirFeature::EffectsOpaque);
    }
}

fn visit_memory_access(access: &NirMemoryAccess, features: &mut BTreeSet<NirFeature>) {
    match access {
        NirMemoryAccess::None => {
            features.insert(NirFeature::EffectsPure);
        }
        NirMemoryAccess::Regions(regions) => {
            features.insert(NirFeature::EffectsRegions);
            for region in regions {
                features.insert(match region.kind {
                    NirMemoryRegionKind::Storage(NirStorageId::Local(_)) => {
                        NirFeature::RegionStorageLocal
                    }
                    NirMemoryRegionKind::Storage(NirStorageId::Param(_)) => {
                        NirFeature::RegionStorageParam
                    }
                    NirMemoryRegionKind::Storage(NirStorageId::Global(_)) => {
                        NirFeature::RegionStorageGlobal
                    }
                    NirMemoryRegionKind::Static(_) => NirFeature::RegionStatic,
                    NirMemoryRegionKind::AbsoluteRange(_) => NirFeature::RegionAbsolute,
                });
            }
        }
        NirMemoryAccess::Unknown => {
            features.insert(NirFeature::EffectsUnknown);
        }
        NirMemoryAccess::All => {
            features.insert(NirFeature::EffectsAll);
        }
    }
}

fn visit_machine_item(item: &NirMachineItem, features: &mut BTreeSet<NirFeature>) {
    match item {
        NirMachineItem::Byte(_) => {
            features.insert(NirFeature::MachineItemByte);
        }
        NirMachineItem::Word(_) => {
            features.insert(NirFeature::MachineItemWord);
        }
        NirMachineItem::StringLiteral(_) => {
            features.insert(NirFeature::MachineItemStringLiteral);
        }
        NirMachineItem::CharLiteral(_) => {
            features.insert(NirFeature::MachineItemCharLiteral);
        }
        NirMachineItem::Name(_) => {
            features.insert(NirFeature::MachineItemName);
        }
        NirMachineItem::AddressExpr { selector, atom, .. } => {
            features.insert(NirFeature::MachineItemAddressExpr);
            features.insert(match selector {
                None => NirFeature::MachineSelectorNone,
                Some(NirMachineByteSelector::Low) => NirFeature::MachineSelectorLow,
                Some(NirMachineByteSelector::High) => NirFeature::MachineSelectorHigh,
            });
            features.insert(match atom {
                NirMachineAtom::Number(_) => NirFeature::MachineAtomNumber,
                NirMachineAtom::Name(_) => NirFeature::MachineAtomName,
                NirMachineAtom::Current => NirFeature::MachineAtomCurrent,
            });
        }
        NirMachineItem::AddressByte { high, .. } => {
            features.insert(NirFeature::MachineItemAddressByte);
            features.insert(if *high {
                NirFeature::MachineSelectorHigh
            } else {
                NirFeature::MachineSelectorLow
            });
        }
        NirMachineItem::Relocation { target, .. } => {
            features.insert(NirFeature::MachineItemRelocation);
            visit_foreign_target(*target, features);
        }
    }
}

fn visit_foreign_target(target: NirForeignCodeTarget, features: &mut BTreeSet<NirFeature>) {
    features.insert(match target {
        NirForeignCodeTarget::Storage(_) => NirFeature::ForeignTargetStorage,
        NirForeignCodeTarget::Routine(_) => NirFeature::ForeignTargetRoutine,
        NirForeignCodeTarget::Absolute(_) => NirFeature::ForeignTargetAbsolute,
        NirForeignCodeTarget::InlineOffset(_) => NirFeature::ForeignTargetInlineOffset,
    });
}

fn visit_place(place: &NirPlace, features: &mut BTreeSet<NirFeature>) {
    if let Some(ty) = &place.ty {
        visit_type(ty, features);
    }
    match &place.kind {
        NirPlaceKind::Param { .. } => {
            features.insert(NirFeature::PlaceParam);
        }
        NirPlaceKind::Local { .. } => {
            features.insert(NirFeature::PlaceLocal);
        }
        NirPlaceKind::Global { .. } => {
            features.insert(NirFeature::PlaceGlobal);
        }
        NirPlaceKind::Absolute(_) => {
            features.insert(NirFeature::PlaceAbsolute);
        }
        NirPlaceKind::Deref { addr } => {
            features.insert(NirFeature::PlaceDeref);
            visit_value(addr, features);
        }
        NirPlaceKind::Index {
            base_addr,
            index,
            elem_ty,
            ..
        } => {
            features.insert(NirFeature::PlaceIndex);
            visit_value(base_addr, features);
            visit_value(index, features);
            visit_type(elem_ty, features);
        }
        NirPlaceKind::Field { base, ty, .. } => {
            features.insert(NirFeature::PlaceField);
            visit_place(base, features);
            visit_type(ty, features);
        }
    }
}

fn visit_value(value: &NirValue, features: &mut BTreeSet<NirFeature>) {
    match value {
        NirValue::IntegerConst { ty, .. } => {
            features.insert(if ty.storage_width() == ByteSize::ONE {
                NirFeature::ValueConstU8
            } else {
                NirFeature::ValueConstU16
            });
        }
        NirValue::Null { ty } => {
            features.insert(NirFeature::ValueNull);
            visit_type(ty, features);
        }
        NirValue::AddressConst { ty, .. } => {
            features.insert(NirFeature::ValueAddressConst);
            visit_type(ty, features);
        }
        NirValue::StaticAddr { ty, .. } => {
            features.insert(NirFeature::ValueStaticAddr);
            visit_type(ty, features);
        }
        NirValue::Temp { ty, .. } => {
            features.insert(NirFeature::ValueTemp);
            visit_type(ty, features);
        }
        NirValue::Param(_) => {
            features.insert(NirFeature::ValueParam);
        }
        NirValue::GlobalAddr(_) => {
            features.insert(NirFeature::ValueGlobalAddr);
        }
        NirValue::RoutineAddr { ty, .. } => {
            features.insert(NirFeature::ValueRoutineAddr);
            visit_type(ty, features);
        }
    }
}

fn visit_terminator(terminator: &NirTerminator, features: &mut BTreeSet<NirFeature>) {
    match terminator {
        NirTerminator::Open => {
            features.insert(NirFeature::TerminatorOpen);
        }
        NirTerminator::Fallthrough => {
            features.insert(NirFeature::TerminatorFallthrough);
        }
        NirTerminator::Goto(edge) => {
            features.insert(NirFeature::TerminatorGoto);
            for arg in &edge.args {
                visit_value(arg, features);
            }
        }
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            features.insert(NirFeature::TerminatorBranch);
            visit_value(condition, features);
            for edge in [then_edge, else_edge] {
                for arg in &edge.args {
                    visit_value(arg, features);
                }
            }
        }
        NirTerminator::Return(None) => {
            features.insert(NirFeature::TerminatorReturnVoid);
        }
        NirTerminator::Return(Some(value)) => {
            features.insert(NirFeature::TerminatorReturnValue);
            visit_value(value, features);
        }
        NirTerminator::Exit => {
            features.insert(NirFeature::TerminatorExit);
        }
    }
}
