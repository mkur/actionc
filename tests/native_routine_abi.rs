use std::path::Path;

use actionc::foreign::{ForeignRelocationEncoding, ForeignSymbolUse};
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502::{self, MirCallTarget, MirOp, MirRoutineAbi, MirStorageBase, RoutineId};
use actionc::nir::{
    self, LocalId, NirActivationModel, NirCallConvention, NirCallEffects, NirCallee,
    NirDataAddressEncoding, NirDataAddressTarget, NirDataFragment, NirDataImage, NirForeignCode,
    NirForeignCodeKind, NirForeignCodePayload, NirForeignCodeTarget, NirForeignRelocation,
    NirLocalBacking, NirLocalPurpose, NirMachineEffects, NirMemoryAccess, NirMemoryEffects,
    NirMemoryRegion, NirMemoryRegionKind, NirObjectLayout, NirOp, NirPlaceKind,
    NirRoutinePlacement, NirStorageDuration, NirStorageId, NirStorageIdentityDomain,
    NirStorageInit, NirTypeKind,
};
use actionc::semantic::ir::{SemActivationModel, SemItem};
use actionc::semantic::{SemanticOptions, analyze_with_options, ir};
use actionc::source::Span;
use actionc::target::{ByteOffset, ByteSize, TargetId};
use actionc::{mir68k, mir65816};

fn classic_baseline() -> nir::NirProgram {
    fixture_for_target(
        Path::new("fixtures/runtime/native_routine_abi_baseline.act"),
        TargetId::Atari6502,
    )
}

fn fixture_for_target(relative: &Path, target: TargetId) -> nir::NirProgram {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let loaded = load_program_with_expanded_source(&source)
        .unwrap_or_else(|diagnostics| panic!("load {}: {diagnostics:?}", source.display()));
    let model = analyze_with_options(
        &loaded.program,
        SemanticOptions::modern().with_target(target),
    )
    .unwrap_or_else(|diagnostics| panic!("analyze {}: {diagnostics:?}", source.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let program = nir::lower_program(&semir);
    nir::verify_program(&program)
        .unwrap_or_else(|diagnostics| panic!("verify {}: {diagnostics:?}", source.display()));
    program
}

#[test]
fn semantic_profiles_resolve_activation_before_nir_lowering() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/nir/activation_storage.act");
    for (target, expected) in [
        (TargetId::Atari6502, SemActivationModel::ClassicStatic),
        (
            TargetId::Wdc65816Native,
            SemActivationModel::NativeReentrant,
        ),
        (TargetId::Wdc65816Small, SemActivationModel::NativeReentrant),
        (TargetId::Motorola68000, SemActivationModel::NativeReentrant),
    ] {
        let loaded = load_program_with_expanded_source(&source)
            .unwrap_or_else(|diagnostics| panic!("load {}: {diagnostics:?}", source.display()));
        let model = analyze_with_options(
            &loaded.program,
            SemanticOptions::modern().with_target(target),
        )
        .unwrap_or_else(|diagnostics| panic!("analyze {}: {diagnostics:?}", source.display()));
        let semir = ir::lower_program(&loaded.program, &model);
        assert!(
            semir
                .modules
                .iter()
                .flat_map(|module| &module.items)
                .filter_map(|item| match item {
                    SemItem::Routine(routine) => Some(routine),
                    _ => None,
                })
                .all(|routine| routine.activation == expected),
            "{target}"
        );
    }
}

#[test]
fn target_profiles_select_activation_duration_and_final_object_layouts() {
    let source = Path::new("fixtures/nir/activation_storage.act");
    for (target, activation, duration, pointer_layout, word_alignment, pair_layout) in [
        (
            TargetId::Atari6502,
            NirActivationModel::ClassicStatic,
            NirStorageDuration::RoutineStatic,
            NirObjectLayout::new(ByteSize::new(2), ByteSize::ONE),
            ByteSize::ONE,
            NirObjectLayout::new(ByteSize::new(3), ByteSize::ONE),
        ),
        (
            TargetId::Wdc65816Native,
            NirActivationModel::NativeReentrant,
            NirStorageDuration::Automatic,
            NirObjectLayout::new(ByteSize::new(3), ByteSize::ONE),
            ByteSize::new(2),
            NirObjectLayout::new(ByteSize::new(4), ByteSize::new(2)),
        ),
        (
            TargetId::Wdc65816Small,
            NirActivationModel::NativeReentrant,
            NirStorageDuration::Automatic,
            NirObjectLayout::new(ByteSize::new(2), ByteSize::ONE),
            ByteSize::new(2),
            NirObjectLayout::new(ByteSize::new(4), ByteSize::new(2)),
        ),
        (
            TargetId::Motorola68000,
            NirActivationModel::NativeReentrant,
            NirStorageDuration::Automatic,
            NirObjectLayout::new(ByteSize::new(4), ByteSize::new(2)),
            ByteSize::new(2),
            NirObjectLayout::new(ByteSize::new(4), ByteSize::new(2)),
        ),
    ] {
        let program = fixture_for_target(source, target);
        let worker = program
            .routines
            .iter()
            .find(|routine| routine.name == "Worker")
            .expect("Worker routine");
        assert_eq!(worker.activation, activation, "{target}");
        assert!(
            worker.params.iter().all(|param| param.duration == duration),
            "{target}"
        );
        assert_eq!(
            worker
                .params
                .iter()
                .find(|param| param.name == "source")
                .expect("array parameter")
                .layout,
            pointer_layout,
            "{target}"
        );

        let local = |name: &str| {
            worker
                .locals
                .iter()
                .find(|local| local.name == name)
                .unwrap_or_else(|| panic!("{name} local for {target}"))
        };
        assert_eq!(local("local").duration, duration, "{target}");
        assert_eq!(local("alias").duration, duration, "{target}");
        assert_eq!(local("globalAlias").duration, NirStorageDuration::External);
        assert_eq!(local("absolute").duration, NirStorageDuration::External);
        assert_eq!(
            local("externalAlias").duration,
            NirStorageDuration::External
        );
        assert_eq!(local("word").layout.alignment, word_alignment, "{target}");
        assert_eq!(local("pair").layout, pair_layout, "{target}");

        let stats = nir::collect_program_stats(&program);
        let duration_name = match duration {
            NirStorageDuration::Automatic => "automatic",
            NirStorageDuration::RoutineStatic => "routine_static",
            NirStorageDuration::External => unreachable!(),
        };
        assert!(
            stats
                .storage
                .duration_counts
                .get(duration_name)
                .is_some_and(|count| *count > 0),
            "{target}"
        );

        let storage = nir::analyze_program_storage(&program);
        let storage = storage.routine("Worker").expect("Worker storage analysis");
        let expected_domain = match activation {
            NirActivationModel::ClassicStatic => NirStorageIdentityDomain::Routine(worker.id),
            NirActivationModel::NativeReentrant => NirStorageIdentityDomain::Invocation(worker.id),
        };
        assert_eq!(
            storage.storage_by_name("local").unwrap().identity_domain,
            expected_domain,
            "{target}"
        );
        assert_eq!(
            storage.storage_by_name("alias").unwrap().identity_domain,
            expected_domain,
            "{target}"
        );
        for name in ["globalAlias", "absolute", "externalAlias"] {
            assert_eq!(
                storage.storage_by_name(name).unwrap().identity_domain,
                NirStorageIdentityDomain::External,
                "{name} for {target}"
            );
        }
    }
}

#[test]
fn verifier_rejects_inconsistent_activation_duration_and_layout_facts() {
    let mut wrong_activation = classic_baseline();
    wrong_activation.routines[0].activation = NirActivationModel::NativeReentrant;
    let diagnostics = nir::verify_program(&wrong_activation).expect_err("wrong activation");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("activation does not match the selected target ABI")
    }));

    let mut wrong_duration = classic_baseline();
    wrong_duration
        .routines
        .iter_mut()
        .find_map(|routine| routine.params.first_mut())
        .expect("fixture parameter")
        .duration = NirStorageDuration::Automatic;
    let diagnostics = nir::verify_program(&wrong_duration).expect_err("wrong duration");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("duration does not match routine activation")
    }));

    let mut wrong_alignment = classic_baseline();
    wrong_alignment
        .routines
        .iter_mut()
        .find_map(|routine| routine.locals.first_mut())
        .expect("fixture local")
        .layout
        .alignment = ByteSize::new(3);
    let diagnostics = nir::verify_program(&wrong_alignment).expect_err("wrong alignment");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("alignment must be a nonzero power of two")
    }));
}

#[test]
fn verifier_rejects_load_time_relocations_to_automatic_storage() {
    let mut program = fixture_for_target(
        Path::new("fixtures/nir/activation_storage.act"),
        TargetId::Motorola68000,
    );
    let pointer = program.target_layout.data_pointer;
    let worker = program
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Worker")
        .expect("Worker routine");
    let target = worker
        .locals
        .iter()
        .find(|local| local.name == "local")
        .expect("automatic target")
        .id;
    let owner = worker
        .locals
        .iter_mut()
        .find(|local| local.name == "pair")
        .expect("initialized owner");
    owner.init = Some(NirStorageInit::Bytes {
        image: NirDataImage {
            bytes: vec![0; usize::from(pointer.size_bytes)],
            fragments: vec![NirDataFragment::Address {
                offset: ByteOffset::ZERO,
                encoding: NirDataAddressEncoding::Pointer {
                    address_space: pointer.address_space,
                    width: pointer.size_bytes,
                },
                target: NirDataAddressTarget::Storage(NirStorageId::Local(target)),
                addend: 0,
                span: actionc::source::Span::new(0, 0),
            }],
        },
        zero_fill: ByteSize::ZERO,
        mutable: true,
        section: "local".to_string(),
    });

    let diagnostics = nir::verify_program(&program).expect_err("automatic load-time relocation");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("load-time address fragment targets automatic local storage")
    }));
}

#[test]
fn verifier_rejects_invalid_non_owning_local_storage() {
    let mut missing_target = fixture_for_target(
        Path::new("fixtures/nir/activation_storage.act"),
        TargetId::Motorola68000,
    );
    let alias = missing_target
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Worker")
        .and_then(|routine| {
            routine
                .locals
                .iter_mut()
                .find(|local| local.name == "alias")
        })
        .expect("local alias");
    let NirLocalBacking::Alias { target, .. } = &mut alias.backing else {
        panic!("expected local alias")
    };
    *target = LocalId(999);
    let diagnostics = nir::verify_program(&missing_target).expect_err("missing alias target");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("references missing local id 999")
    }));

    let mut independently_initialized = fixture_for_target(
        Path::new("fixtures/nir/activation_storage.act"),
        TargetId::Motorola68000,
    );
    let external = independently_initialized
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Worker")
        .and_then(|routine| {
            routine
                .locals
                .iter_mut()
                .find(|local| local.name == "globalAlias")
        })
        .expect("global alias");
    external.init = Some(NirStorageInit::ZeroFill {
        bytes: ByteSize::ONE,
        mutable: true,
        section: "local".to_string(),
    });
    let diagnostics =
        nir::verify_program(&independently_initialized).expect_err("external alias init");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot have independent storage initialization")
    }));

    let mut cyclic_aliases = fixture_for_target(
        Path::new("fixtures/nir/activation_storage.act"),
        TargetId::Motorola68000,
    );
    let worker = cyclic_aliases
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Worker")
        .expect("Worker routine");
    let alias_id = worker
        .locals
        .iter()
        .find(|local| local.name == "alias")
        .expect("alias")
        .id;
    let external_alias = worker
        .locals
        .iter_mut()
        .find(|local| local.name == "externalAlias")
        .expect("external alias");
    external_alias.duration = NirStorageDuration::Automatic;
    external_alias.backing = NirLocalBacking::Alias {
        target: alias_id,
        target_name: "alias".to_string(),
        offset: ByteOffset::ZERO,
    };
    let external_alias_id = external_alias.id;
    let alias = worker
        .locals
        .iter_mut()
        .find(|local| local.id == alias_id)
        .expect("alias");
    let NirLocalBacking::Alias { target, .. } = &mut alias.backing else {
        unreachable!()
    };
    *target = external_alias_id;
    let diagnostics = nir::verify_program(&cyclic_aliases).expect_err("alias cycle");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("participates in an alias cycle")
    }));
}

#[test]
fn verifier_rejects_malformed_automatic_initializer_storage() {
    let mut load_time = fixture_for_target(
        Path::new("fixtures/nir/native_entry_initialization.act"),
        TargetId::Motorola68000,
    );
    let first = load_time
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Initialized")
        .and_then(|routine| {
            routine
                .locals
                .iter_mut()
                .find(|local| local.name == "first")
        })
        .expect("automatic initialized local");
    first.init = Some(NirStorageInit::ZeroFill {
        bytes: ByteSize::ONE,
        mutable: true,
        section: "local".to_string(),
    });
    let diagnostics = nir::verify_program(&load_time).expect_err("automatic load-time image");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("must use executable entry initialization")
    }));

    let mut missing_owner = fixture_for_target(
        Path::new("fixtures/nir/native_entry_initialization.act"),
        TargetId::Motorola68000,
    );
    let backing = missing_owner
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Initialized")
        .and_then(|routine| {
            routine
                .locals
                .iter_mut()
                .find(|local| matches!(local.purpose, NirLocalPurpose::AggregateBacking { .. }))
        })
        .expect("automatic aggregate backing");
    backing.purpose = NirLocalPurpose::AggregateBacking {
        owner: LocalId(999),
    };
    let diagnostics = nir::verify_program(&missing_owner).expect_err("missing backing owner");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("references a missing or invalid descriptor owner")
    }));
}

#[test]
fn effect_regions_use_object_layout_not_element_type_width() {
    let mut program = fixture_for_target(
        Path::new("fixtures/nir/activation_storage.act"),
        TargetId::Motorola68000,
    );
    {
        let worker = program
            .routines
            .iter_mut()
            .find(|routine| routine.name == "Worker")
            .expect("Worker routine");
        let bytes = worker
            .locals
            .iter()
            .find(|local| local.name == "bytes")
            .expect("array local")
            .id;
        worker.blocks[0].ops.push(NirOp::Call {
            callee: NirCallee::Builtin("ObserveLastByte".to_string()),
            args: Vec::new(),
            result: None,
            signature: Some(nir::NirCallableSignature::empty_proc(
                NirCallConvention::Runtime,
            )),
            effects: NirCallEffects {
                memory: NirMemoryEffects {
                    reads: NirMemoryAccess::Regions(vec![NirMemoryRegion {
                        kind: NirMemoryRegionKind::Storage(NirStorageId::Local(bytes)),
                        offset: ByteOffset::new(3),
                        size: ByteSize::ONE,
                    }]),
                    writes: NirMemoryAccess::None,
                },
                may_call_external: false,
                opaque: false,
            },
        });
    }
    nir::verify_program(&program).expect("effect within four-byte local object");

    let worker = program
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Worker")
        .expect("Worker routine");
    let NirOp::Call { effects, .. } = worker.blocks[0].ops.last_mut().expect("inserted call")
    else {
        unreachable!()
    };
    let NirMemoryAccess::Regions(regions) = &mut effects.memory.reads else {
        unreachable!()
    };
    regions[0].offset = ByteOffset::new(4);
    let diagnostics = nir::verify_program(&program).expect_err("effect past local object");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("exceeds storage size 4") })
    );
}

#[test]
fn classic_nir_baseline_keeps_fixed_local_initializers_and_aliases() {
    let program = classic_baseline();
    let bump = program
        .routines
        .iter()
        .find(|routine| routine.name == "Bump")
        .expect("Bump routine");

    let initial = bump
        .locals
        .iter()
        .find(|local| local.name == "initial")
        .expect("initialized local");
    assert!(matches!(initial.backing, NirLocalBacking::Ordinary));
    assert!(matches!(
        initial.init,
        Some(NirStorageInit::Bytes { ref image, .. }) if image.bytes == [7]
    ));

    let alias = bump
        .locals
        .iter()
        .find(|local| local.name == "alias")
        .expect("local alias");
    assert!(matches!(alias.backing, NirLocalBacking::Alias { .. }));

    assert!(bump.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(
            op,
            NirOp::AddrOf { place, .. }
                if matches!(&place.kind, nir::NirPlaceKind::Local { name, .. } if name == "local")
        )
    }));
}

#[test]
fn native_initializers_are_explicit_ordered_entry_operations() {
    for target in [
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
        TargetId::Motorola68000,
    ] {
        let program = fixture_for_target(
            Path::new("fixtures/nir/native_entry_initialization.act"),
            target,
        );
        let initialized = program
            .routines
            .iter()
            .find(|routine| routine.name == "Initialized")
            .expect("Initialized routine");
        assert!(
            initialized.locals.iter().all(|local| local.init.is_none()),
            "automatic locals cannot retain load-time images for {target}"
        );
        let backing = initialized
            .locals
            .iter()
            .find(|local| {
                matches!(
                    local.purpose,
                    NirLocalPurpose::AggregateBacking { owner }
                        if initialized.locals.iter().any(|candidate| {
                            candidate.id == owner && candidate.name == "words"
                        })
                )
            })
            .unwrap_or_else(|| panic!("word-array backing for {target}"));
        assert_eq!(backing.duration, NirStorageDuration::Automatic);
        assert!(matches!(backing.backing, NirLocalBacking::Ordinary));

        let ops = &initialized.blocks[0].ops;
        let local_store = |name: &str| {
            ops.iter().position(|op| {
                matches!(
                    op,
                    NirOp::Store {
                        place: nir::NirPlace {
                            kind: NirPlaceKind::Local { name: candidate, .. },
                            ..
                        },
                        ..
                    } if candidate == name
                )
            })
        };
        let first = local_store("first").expect("first initializer store");
        let second = local_store("second").expect("second initializer store");
        let pointer = local_store("ptr").expect("pointer initializer store");
        let descriptor = local_store("words").expect("descriptor pointer store");
        let backing_copy = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    NirOp::CopyBytes {
                        destination: nir::NirPlace {
                            kind: NirPlaceKind::Local { name, .. },
                            ..
                        },
                        ..
                    } if name == "words.__backing"
                )
            })
            .expect("word backing template copy");
        let body = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    NirOp::Load {
                        place: nir::NirPlace {
                            kind: NirPlaceKind::Local { name, .. },
                            ..
                        },
                        ..
                    } if name == "second"
                )
            })
            .expect("first source-body operation");
        assert!(first < second && second < pointer);
        assert!(pointer < descriptor && descriptor < backing_copy && backing_copy < body);
        assert!(
            local_store("alias").is_none(),
            "an alias is not initialized"
        );
        assert!(
            local_store("plain").is_none(),
            "an uninitialized local stays uninitialized"
        );
        assert!(
            program.statics.iter().all(|static_data| {
                !static_data.name.starts_with("__nir_init_")
                    || (!static_data.mutable && static_data.section == "rodata")
            }),
            "entry templates must be immutable rodata for {target}"
        );

        for routine_name in ["EarlyReturn", "Nested"] {
            let routine = program
                .routines
                .iter()
                .find(|routine| routine.name == routine_name)
                .expect("entry-initialized routine");
            assert!(matches!(
                routine.blocks[0].ops.first(),
                Some(NirOp::Store { .. })
            ));
        }
        match target {
            TargetId::Wdc65816Native | TargetId::Wdc65816Small => {
                mir65816::lower_program(&program)
                    .unwrap_or_else(|error| panic!("MIR65816 entry initialization: {error:?}"));
            }
            TargetId::Motorola68000 => {
                mir68k::lower_program(&program)
                    .unwrap_or_else(|error| panic!("MIR68K entry initialization: {error:?}"));
            }
            TargetId::Atari6502 => unreachable!(),
        }
    }
}

#[test]
fn classic_initializers_remain_load_time_storage_images() {
    let program = fixture_for_target(
        Path::new("fixtures/nir/native_entry_initialization.act"),
        TargetId::Atari6502,
    );
    let initialized = program
        .routines
        .iter()
        .find(|routine| routine.name == "Initialized")
        .expect("Initialized routine");
    for name in ["first", "second", "ptr", "bytes", "words", "pair", "holder"] {
        assert!(
            initialized
                .locals
                .iter()
                .find(|local| local.name == name)
                .is_some_and(|local| local.init.is_some()),
            "classic `{name}` keeps its load-time initializer"
        );
    }
    assert!(
        initialized
            .locals
            .iter()
            .all(|local| !matches!(local.purpose, NirLocalPurpose::AggregateBacking { .. }))
    );
    assert!(
        initialized.blocks[0].ops.iter().all(|op| {
            !matches!(
                op,
                NirOp::Store {
                    place: nir::NirPlace {
                        kind: NirPlaceKind::Local { name, .. },
                        ..
                    },
                    ..
                } if matches!(name.as_str(), "first" | "second" | "ptr")
            )
        }),
        "classic initialization must not be repeated by entry stores"
    );
}

#[test]
fn classic_nir_has_structured_routine_and_call_conventions() {
    let program = classic_baseline();

    for (index, routine) in program.routines.iter().enumerate() {
        assert_eq!(routine.id.0 as usize, index);
        assert_eq!(routine.convention, NirCallConvention::TargetPublic);
        assert_eq!(routine.signature.convention, routine.convention);
        assert!(matches!(
            routine.entry.placement,
            NirRoutinePlacement::Relocatable
        ));
    }
    assert_eq!(
        program
            .routines
            .iter()
            .filter(|routine| routine.entry.program)
            .map(|routine| routine.name.as_str())
            .collect::<Vec<_>>(),
        ["Main"]
    );

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let indirect = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op {
            NirOp::Call {
                callee: NirCallee::Indirect { ty, .. },
                signature: Some(signature),
                ..
            } => Some((ty, signature)),
            _ => None,
        })
        .expect("indirect callback call");
    let NirTypeKind::Callable {
        signature,
        convention,
        ..
    } = indirect.0.kind
    else {
        panic!("indirect target must retain callable facts");
    };
    assert_eq!(signature, indirect.1.id);
    assert_eq!(convention, indirect.1.convention);
    assert_eq!(convention, NirCallConvention::TargetPublic);
}

#[test]
fn convention_is_part_of_callable_signature_identity() {
    let public = nir::NirCallableSignature::empty_proc(NirCallConvention::TargetPublic);
    let runtime = nir::NirCallableSignature::empty_proc(NirCallConvention::Runtime);
    assert_ne!(public.id, runtime.id);
}

#[test]
fn verifier_rejects_call_convention_mismatches_and_missing_signatures() {
    let mut mismatched = classic_baseline();
    let call = mismatched
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Recur")
        .expect("Recur routine")
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.ops)
        .find_map(|op| match op {
            NirOp::Call {
                signature: Some(signature),
                ..
            } => Some(signature),
            _ => None,
        })
        .expect("recursive call signature");
    call.convention = NirCallConvention::Runtime;
    let diagnostics = nir::verify_program(&mismatched).expect_err("mismatched convention");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("call convention does not match")
    }));

    let mut missing = classic_baseline();
    let signature = missing
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Recur")
        .expect("Recur routine")
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.ops)
        .find_map(|op| match op {
            NirOp::Call { signature, .. } => Some(signature),
            _ => None,
        })
        .expect("recursive call");
    *signature = None;
    let diagnostics = nir::verify_program(&missing).expect_err("missing convention");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("no callable signature or convention")
    }));
}

#[test]
fn classic_mir6502_baseline_reuses_one_frame_for_recursive_calls() {
    let program = classic_baseline();
    let recur_index = program
        .routines
        .iter()
        .position(|routine| routine.name == "Recur")
        .expect("Recur routine");
    let recur_nir = &program.routines[recur_index];
    let recur_id = recur_nir.id;
    assert!(recur_nir.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(op, NirOp::Call { callee: NirCallee::User { id, .. }, .. } if *id == recur_id)
    }));

    let mir = mir6502::lower_program(&program).expect("lower classic routine-storage baseline");
    let recur = mir
        .routines
        .iter()
        .find(|routine| routine.name == "Recur")
        .expect("Recur MIR routine");
    assert_eq!(recur.abi, MirRoutineAbi::Action);
    assert!(matches!(
        recur.frame.params[0].base,
        MirStorageBase::Param(_)
    ));
    assert!(matches!(
        recur.frame.locals[0].base,
        MirStorageBase::Local(_)
    ));
    assert!(recur.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(
            op,
            MirOp::Call {
                target: MirCallTarget::Routine(RoutineId(id)),
                ..
            } if *id == recur_id.0
        )
    }));
}

#[test]
fn native_optimization_preserves_activation_identity_and_escaped_homes() {
    for target in [
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
        TargetId::Motorola68000,
    ] {
        let lowered = fixture_for_target(
            Path::new("fixtures/runtime/native_routine_abi_baseline.act"),
            target,
        );
        let optimized = nir::optimize_program(&lowered).unwrap_or_else(|diagnostics| {
            panic!("optimize native ABI fixture for {target}: {diagnostics:?}")
        });
        nir::verify_program(&optimized).unwrap_or_else(|diagnostics| {
            panic!("verify optimized native ABI fixture for {target}: {diagnostics:?}")
        });

        for optimized_routine in &optimized.routines {
            let lowered_routine = lowered
                .routines
                .iter()
                .find(|routine| routine.id == optimized_routine.id)
                .expect("lowered routine identity");
            assert_eq!(
                optimized_routine.activation,
                NirActivationModel::NativeReentrant,
                "{target}"
            );
            for param in &optimized_routine.params {
                let lowered_param = lowered_routine
                    .params
                    .iter()
                    .find(|candidate| candidate.id == param.id)
                    .expect("lowered parameter identity");
                assert_eq!(param.duration, lowered_param.duration, "{target}");
            }
            for local in &optimized_routine.locals {
                let lowered_local = lowered_routine
                    .locals
                    .iter()
                    .find(|candidate| candidate.id == local.id)
                    .expect("lowered local identity");
                assert_eq!(local.duration, lowered_local.duration, "{target}");
            }
        }

        let bump = optimized
            .routines
            .iter()
            .find(|routine| routine.name == "Bump")
            .expect("optimized Bump routine");
        let escaped = bump
            .locals
            .iter()
            .find(|local| local.name == "local")
            .expect("address-escaped local retains a home");
        assert_eq!(escaped.duration, NirStorageDuration::Automatic, "{target}");
        assert!(bump.blocks.iter().flat_map(|block| &block.ops).any(|op| {
            matches!(op, NirOp::AddrOf {
                place: nir::NirPlace {
                    kind: NirPlaceKind::Local { id, .. },
                    ..
                },
                ..
            } if *id == escaped.id)
        }));

        let recur = optimized
            .routines
            .iter()
            .find(|routine| routine.name == "Recur")
            .expect("optimized Recur routine");
        assert!(recur.blocks.iter().flat_map(|block| &block.ops).any(|op| {
            matches!(op, NirOp::Call { callee: NirCallee::User { id, .. }, .. }
                if *id == recur.id)
        }));

        match target {
            TargetId::Wdc65816Native | TargetId::Wdc65816Small => {
                mir65816::lower_program(&optimized).unwrap_or_else(|diagnostics| {
                    panic!("lower optimized {target}: {diagnostics:?}")
                });
            }
            TargetId::Motorola68000 => {
                mir68k::lower_program(&optimized).unwrap_or_else(|diagnostics| {
                    panic!("lower optimized {target}: {diagnostics:?}")
                });
            }
            TargetId::Atari6502 => unreachable!(),
        }
    }
}

#[test]
fn mir68k_plans_reentrant_frames_and_balanced_stack_calls() {
    let program = fixture_for_target(
        Path::new("fixtures/runtime/native_frame_plan.act"),
        TargetId::Motorola68000,
    );
    let mir = mir68k::lower_program(&program).expect("lower native 68k frame-plan fixture");

    let sum6 = mir
        .routines
        .iter()
        .find(|routine| routine.name == "Sum6")
        .expect("Sum6 MIR routine");
    let first_param = sum6.frame.parameters.first().expect("first Sum6 parameter");
    assert!(
        first_param.frame_object.is_some(),
        "the mutated parameter must receive an invocation-local home"
    );
    assert!(
        sum6.frame.parameters[1..]
            .iter()
            .all(|parameter| parameter.frame_object.is_none()),
        "immutable non-address-taken parameters may remain incoming values"
    );

    let probe = mir
        .routines
        .iter()
        .find(|routine| routine.name == "FrameProbe")
        .expect("FrameProbe MIR routine");
    assert_eq!(probe.frame.extent.get() % 2, 0);
    assert!(probe.frame.outgoing.size.get() >= 12);
    assert!(probe.frame.objects.iter().any(|object| {
        object.addressable && matches!(object.owner, mir68k::Mir68kFrameObjectOwner::Local(_))
    }));
    assert!(probe.frame.objects.iter().any(|object| {
        matches!(object.owner, mir68k::Mir68kFrameObjectOwner::Local(_)) && object.size.get() >= 4
    }));
    assert!(probe.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(
            op,
            mir68k::Mir68kOp::Call { plan, .. }
                if plan.arguments.len() == 6
                    && plan.outgoing_bytes.get() >= 12
                    && plan.net_stack_delta == 0
        )
    }));

    let recur = mir
        .routines
        .iter()
        .find(|routine| routine.name == "Recur")
        .expect("Recur MIR routine");
    assert!(recur.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(
            op,
            mir68k::Mir68kOp::Call {
                target: mir68k::Mir68kCallTarget::Direct(id),
                plan: mir68k::Mir68kCallPlan {
                    activation: mir68k::Mir68kCallActivation::Fresh,
                    net_stack_delta: 0,
                    ..
                },
                ..
            } if *id == recur.id.0
        )
    }));
    assert!(
        mir.routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .any(|block| {
                block.ops.iter().any(|op| {
                    matches!(
                        op,
                        mir68k::Mir68kOp::Call {
                            target: mir68k::Mir68kCallTarget::Indirect(_, _),
                            ..
                        }
                    )
                })
            })
    );
}

#[test]
fn mir65816_plans_independent_native_and_small_model_frames() {
    for (target, pointer_width, call_form, expected_return_form) in [
        (
            TargetId::Wdc65816Native,
            ByteSize::new(3),
            mir65816::Mir65816CallForm::FarJsl,
            mir65816::Mir65816ReturnForm::FarRtl,
        ),
        (
            TargetId::Wdc65816Small,
            ByteSize::new(2),
            mir65816::Mir65816CallForm::NearJsr,
            mir65816::Mir65816ReturnForm::NearRts,
        ),
    ] {
        let program =
            fixture_for_target(Path::new("fixtures/runtime/native_frame_plan.act"), target);
        let mir = mir65816::lower_program(&program)
            .unwrap_or_else(|diagnostics| panic!("lower {target} frame plan: {diagnostics:?}"));
        assert_eq!(mir.code_pointer_width, pointer_width, "{target}");
        assert_eq!(
            mir.task_switch_state.required,
            vec![
                mir65816::Mir65816SavedState::Accumulator,
                mir65816::Mir65816SavedState::X,
                mir65816::Mir65816SavedState::Y,
                mir65816::Mir65816SavedState::StackPointer,
                mir65816::Mir65816SavedState::DirectPage,
                mir65816::Mir65816SavedState::DataBank,
                mir65816::Mir65816SavedState::ProgramBank,
                mir65816::Mir65816SavedState::ProcessorStatus,
            ],
            "{target}"
        );

        let sum6 = mir
            .routines
            .iter()
            .find(|routine| routine.name == "Sum6")
            .expect("Sum6 MIR routine");
        assert!(sum6.frame.parameters[0].frame_object.is_some(), "{target}");
        assert!(
            sum6.frame.parameters[1..]
                .iter()
                .all(|parameter| parameter.frame_object.is_none()),
            "{target}"
        );

        let probe = mir
            .routines
            .iter()
            .find(|routine| routine.name == "FrameProbe")
            .expect("FrameProbe MIR routine");
        assert_eq!(
            probe.frame.strategy,
            mir65816::Mir65816FrameStrategy::HardwareStackRelative,
            "{target}"
        );
        assert_eq!(probe.frame.bank, 0, "{target}");
        assert!(probe.frame.extent.get() <= u32::from(u8::MAX), "{target}");
        assert!(probe.frame.outgoing_bytes.get() >= 6, "{target}");
        assert!(probe.blocks.iter().flat_map(|block| &block.ops).any(|op| {
            matches!(
                op,
                mir65816::Mir65816Op::Call { plan, .. }
                    if plan.arguments.len() == 6
                        && plan.call_form == call_form
                        && plan.code_pointer_width == pointer_width
                        && plan.net_stack_delta == 0
            )
        }));
        assert!(probe.blocks.iter().any(|block| {
            matches!(
                block.terminator,
                mir65816::Mir65816Terminator::Return {
                    form,
                    restored_mode: mir65816::Mir65816ModeState {
                        native_mode: true,
                        accumulator: mir65816::Mir65816RegisterWidth::Bits16,
                        index: mir65816::Mir65816RegisterWidth::Bits16,
                    },
                    ..
                } if form == expected_return_form
            )
        }));

        let recur = mir
            .routines
            .iter()
            .find(|routine| routine.name == "Recur")
            .expect("Recur MIR routine");
        assert!(recur.blocks.iter().flat_map(|block| &block.ops).any(|op| {
            matches!(
                op,
                mir65816::Mir65816Op::Call {
                    target: mir65816::Mir65816CallTarget::Direct(id),
                    plan: mir65816::Mir65816CallPlan {
                        activation: mir65816::Mir65816CallActivation::Fresh,
                        ..
                    },
                    ..
                } if *id == recur.id.0
            )
        }));
        assert!(
            mir.routines
                .iter()
                .flat_map(|routine| &routine.blocks)
                .any(|block| {
                    block.ops.iter().any(|op| {
                        matches!(
                            op,
                            mir65816::Mir65816Op::Call {
                                target: mir65816::Mir65816CallTarget::Indirect(_, width),
                                ..
                            } if *width == pointer_width
                        )
                    })
                })
        );
    }
}

#[test]
fn native_verifier_rejects_fixed_addresses_for_automatic_storage() {
    let mut program = fixture_for_target(
        Path::new("fixtures/runtime/native_frame_plan.act"),
        TargetId::Motorola68000,
    );
    let probe = program
        .routines
        .iter_mut()
        .find(|routine| routine.name == "FrameProbe")
        .expect("FrameProbe routine");
    let local = probe
        .locals
        .iter()
        .find(|local| local.name == "local")
        .expect("automatic local")
        .id;
    probe.blocks[0].ops.insert(
        0,
        NirOp::ForeignCode {
            code: NirForeignCode {
                target: TargetId::Motorola68000,
                kind: NirForeignCodeKind::InlineAssembly,
                payload: NirForeignCodePayload::Bytes {
                    bytes: vec![0; 4],
                    relocations: vec![NirForeignRelocation {
                        offset: ByteOffset::ZERO,
                        encoding: ForeignRelocationEncoding::Address {
                            width: ByteSize::new(4),
                        },
                        target: NirForeignCodeTarget::Storage(NirStorageId::Local(local)),
                        addend: 0,
                        required_address_bits: Some(32),
                        symbol_use: ForeignSymbolUse::Address,
                        span: Span::new(0, 0),
                    }],
                },
                source: "automatic local relocation probe".to_string(),
                span: Span::new(0, 0),
            },
            effects: NirMachineEffects {
                memory: NirMemoryEffects {
                    reads: NirMemoryAccess::All,
                    writes: NirMemoryAccess::All,
                },
                may_call_external: true,
                opaque: true,
            },
        },
    );

    let diagnostics = nir::verify_program(&program).expect_err("fixed automatic address");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("inline assembly cannot embed a fixed address for automatic local `local`")
    }));
}

#[test]
fn native_verifier_rejects_fixed_entries_and_internal_address_escape() {
    for placement in [
        NirRoutinePlacement::CurrentLocation,
        NirRoutinePlacement::Absolute(actionc::target::AddressValue::code(0x2000)),
    ] {
        let mut program = fixture_for_target(
            Path::new("fixtures/runtime/native_frame_plan.act"),
            TargetId::Motorola68000,
        );
        program
            .routines
            .iter_mut()
            .find(|routine| routine.name == "FrameProbe")
            .expect("FrameProbe routine")
            .entry
            .placement = placement;
        let diagnostics = nir::verify_program(&program).expect_err("fixed native entry");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("under native reentrant activation")
                || diagnostic
                    .message
                    .contains("incompatible with native reentrant activation")
        }));
    }

    let mut escaped_internal = fixture_for_target(
        Path::new("fixtures/runtime/native_frame_plan.act"),
        TargetId::Motorola68000,
    );
    let thunk = escaped_internal
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Thunk")
        .expect("addressed Thunk routine");
    thunk.convention = NirCallConvention::TargetInternal;
    thunk.signature.convention = NirCallConvention::TargetInternal;
    let diagnostics = nir::verify_program(&escaped_internal).expect_err("internal address escape");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot expose target-internal routine")
    }));
}

#[test]
fn native_acceptance_corpus_proves_recursive_activation_contracts() {
    const FIXTURES: &[(&str, &str)] = &[
        ("fixtures/native/recursive_scalar.act", "SumTo"),
        ("fixtures/native/recursive_permutation.act", "Permute"),
        ("fixtures/native/recursive_queens.act", "Queens"),
        ("fixtures/native/reentrancy_boundaries.act", "Reenter"),
    ];

    for target in [
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        for (fixture, recursive_routine) in FIXTURES {
            let lowered = fixture_for_target(Path::new(fixture), target);
            let recursive = lowered
                .routines
                .iter()
                .find(|routine| routine.name == *recursive_routine)
                .unwrap_or_else(|| panic!("{recursive_routine} in {fixture}"));
            assert!(
                recursive
                    .blocks
                    .iter()
                    .flat_map(|block| &block.ops)
                    .any(|op| {
                        matches!(op, NirOp::Call { callee: NirCallee::User { id, .. }, .. }
                    if *id == recursive.id)
                    })
            );
            let optimized = nir::optimize_program(&lowered).unwrap_or_else(|diagnostics| {
                panic!("optimize {fixture} for {target}: {diagnostics:?}")
            });
            for (path, program) in [("lowered", &lowered), ("optimized", &optimized)] {
                nir::verify_program(program).unwrap_or_else(|diagnostics| {
                    panic!("verify {path} {fixture} for {target}: {diagnostics:?}")
                });
                assert!(
                    program.routines.iter().all(|routine| {
                        routine.activation == NirActivationModel::NativeReentrant
                            && routine.convention == NirCallConvention::TargetPublic
                    }),
                    "{path} {fixture} for {target}"
                );
                match target {
                    TargetId::Motorola68000 => {
                        mir68k::lower_program(program).unwrap_or_else(|diagnostics| {
                            panic!("lower {path} {fixture} for {target}: {diagnostics:?}")
                        });
                    }
                    TargetId::Wdc65816Native | TargetId::Wdc65816Small => {
                        mir65816::lower_program(program).unwrap_or_else(|diagnostics| {
                            panic!("lower {path} {fixture} for {target}: {diagnostics:?}")
                        });
                    }
                    TargetId::Atari6502 => unreachable!(),
                }
            }
        }

        let boundary = fixture_for_target(
            Path::new("fixtures/native/reentrancy_boundaries.act"),
            target,
        );
        let indirect_calls = boundary
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .filter(|op| {
                matches!(
                    op,
                    NirOp::Call {
                        callee: NirCallee::Indirect { .. },
                        ..
                    }
                )
            })
            .count();
        assert!(indirect_calls >= 3, "mutual/public calls for {target}");
        for name in ["Even", "Odd", "Nested", "Outer", "Reenter"] {
            let routine = boundary
                .routines
                .iter()
                .find(|routine| routine.name == name)
                .unwrap_or_else(|| panic!("{name} routine for {target}"));
            assert!(
                matches!(routine.blocks[0].ops.first(), Some(NirOp::Store { .. })),
                "{name} repeats its initializer at entry for {target}"
            );
        }

        let reenter_nir = boundary
            .routines
            .iter()
            .find(|routine| routine.name == "Reenter")
            .expect("Reenter NIR routine");
        assert!(
            reenter_nir
                .blocks
                .iter()
                .flat_map(|block| &block.ops)
                .any(|op| {
                    matches!(op, NirOp::Call { callee: NirCallee::User { id, .. }, .. }
                if *id == reenter_nir.id)
                })
        );
        let local = reenter_nir
            .locals
            .iter()
            .find(|local| local.name == "local")
            .expect("Reenter local")
            .id;

        match target {
            TargetId::Motorola68000 => {
                let mir = mir68k::lower_program(&boundary).expect("lower re-entry 68k");
                let indirect_plans = mir
                    .routines
                    .iter()
                    .flat_map(|routine| &routine.blocks)
                    .flat_map(|block| &block.ops)
                    .filter_map(|op| match op {
                        mir68k::Mir68kOp::Call {
                            target: mir68k::Mir68kCallTarget::Indirect(_, _),
                            plan,
                            ..
                        } => Some(plan),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert!(indirect_plans.len() >= 3);
                assert!(indirect_plans.iter().all(|plan| {
                    plan.convention == NirCallConvention::TargetPublic
                        && plan.activation == mir68k::Mir68kCallActivation::Fresh
                }));
                let routine = mir
                    .routines
                    .iter()
                    .find(|routine| routine.name == "Reenter")
                    .expect("Reenter MIR68K routine");
                let object = routine
                    .frame
                    .objects
                    .iter()
                    .find(|object| object.owner == mir68k::Mir68kFrameObjectOwner::Local(local))
                    .expect("Reenter MIR68K local object")
                    .id;
                let outer = routine
                    .frame
                    .address_in(mir68k::Mir68kActivationId(1), object)
                    .expect("outer activation address");
                let inner = routine
                    .frame
                    .address_in(mir68k::Mir68kActivationId(2), object)
                    .expect("inner activation address");
                assert_ne!(outer, inner);
                assert!(
                    routine
                        .blocks
                        .iter()
                        .flat_map(|block| &block.ops)
                        .any(|op| {
                            matches!(
                                op,
                                mir68k::Mir68kOp::Call {
                                    target: mir68k::Mir68kCallTarget::Direct(id),
                                    plan: mir68k::Mir68kCallPlan {
                                        activation: mir68k::Mir68kCallActivation::Fresh,
                                        convention: NirCallConvention::TargetPublic,
                                        ..
                                    },
                                    ..
                                } if *id == routine.id.0
                            )
                        })
                );
            }
            TargetId::Wdc65816Native | TargetId::Wdc65816Small => {
                let mir = mir65816::lower_program(&boundary).expect("lower re-entry 65816");
                let indirect_plans = mir
                    .routines
                    .iter()
                    .flat_map(|routine| &routine.blocks)
                    .flat_map(|block| &block.ops)
                    .filter_map(|op| match op {
                        mir65816::Mir65816Op::Call {
                            target: mir65816::Mir65816CallTarget::Indirect(_, _),
                            plan,
                            ..
                        } => Some(plan),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert!(indirect_plans.len() >= 3);
                assert!(indirect_plans.iter().all(|plan| {
                    plan.convention == NirCallConvention::TargetPublic
                        && plan.activation == mir65816::Mir65816CallActivation::Fresh
                }));
                let routine = mir
                    .routines
                    .iter()
                    .find(|routine| routine.name == "Reenter")
                    .expect("Reenter MIR65816 routine");
                let object = routine
                    .frame
                    .objects
                    .iter()
                    .find(|object| object.owner == mir65816::Mir65816FrameObjectOwner::Local(local))
                    .expect("Reenter MIR65816 local object")
                    .id;
                let outer = routine
                    .frame
                    .address_in(mir65816::Mir65816ActivationId(1), object)
                    .expect("outer activation address");
                let inner = routine
                    .frame
                    .address_in(mir65816::Mir65816ActivationId(2), object)
                    .expect("inner activation address");
                assert_ne!(outer, inner);
                assert!(
                    routine
                        .blocks
                        .iter()
                        .flat_map(|block| &block.ops)
                        .any(|op| {
                            matches!(
                                op,
                                mir65816::Mir65816Op::Call {
                                    target: mir65816::Mir65816CallTarget::Direct(id),
                                    plan: mir65816::Mir65816CallPlan {
                                        activation: mir65816::Mir65816CallActivation::Fresh,
                                        convention: NirCallConvention::TargetPublic,
                                        ..
                                    },
                                    ..
                                } if *id == routine.id.0
                            )
                        })
                );
            }
            TargetId::Atari6502 => unreachable!(),
        }
    }
}
