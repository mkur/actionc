use super::*;

fn source_slot(base: MirStorageBase) -> MirStorageSlot {
    MirStorageSlot {
        purpose: MirStoragePurpose::Storage,
        id: MirStorageId(0),
        name: Some("source".into()),
        storage: MirStorageClass::Scalar,
        storage_size: 1,
        scalar_width: Some(MirWidth::Byte),
        base,
        offset: 0,
        mutable: true,
        init: None,
    }
}

fn relocation(mem: MirMem) -> MirMachineItem {
    MirMachineItem::Relocation {
        kind: crate::asm6502::InlineAsmRelocationKind::Absolute16,
        target: MirInlineAsmTarget::Memory(mem),
        addend: 0,
        requires_zero_page: false,
        span: crate::source::Span { start: 0, end: 0 },
    }
}

#[test]
fn source_address_and_data_relocation_do_not_expose_neighboring_spill() {
    let local = crate::nir::LocalId(0);
    let mut p = program(vec![MirOp::LeaAddr {
        dst: MirDef::Reg(MirReg::A),
        target: MirMem::Local {
            id: local,
            offset: 0,
        },
        width: MirWidth::Word,
    }]);
    p.routines[0]
        .frame
        .locals
        .push(source_slot(MirStorageBase::Local(local)));
    let (after, _, events) = run(&p, &clean(&p));
    assert_eq!(result(&events)["accepted"], true, "{events:?}");
    assert!(after.routines[0].frame.spills.is_empty());
    assert_eq!(after.routines[0].frame.locals, p.routines[0].frame.locals);
    assert_eq!(after.routines[0].blocks, p.routines[0].blocks);

    // An initializer or typed assembly reference to source storage does not
    // expose unrelated compiler homes either, including in retained witnesses.
    let mut historical = p.clone();
    historical.statics.push(MirStatic {
        id: crate::nir::SymbolId(0),
        name: "source_address".into(),
        ty: "CARD".into(),
        image: MirDataImage {
            bytes: vec![0, 0],
            relocations: vec![MirDataRelocation {
                offset: 0,
                kind: MirDataRelocationKind::Word16,
                target: MirDataRelocationTarget::Local {
                    routine: RoutineId(0),
                    id: local,
                },
                addend: 0,
                span: crate::source::Span { start: 0, end: 0 },
            }],
        },
        display: String::new(),
        alignment: 1,
        mutable: false,
        section: "data".into(),
    });
    historical.routines[0].blocks[0]
        .ops
        .push(MirOp::MachineBlock {
            id: MirMachineBlockId(0),
            effects: Default::default(),
        });
    historical.machine_blocks.push(MirMachineBlock {
        id: MirMachineBlockId(0),
        source_span: None,
        items: vec![
            MirMachineItem::Byte(0xad),
            relocation(MirMem::Local {
                id: local,
                offset: 0,
            }),
        ],
    });
    assert_eq!(count(&discover(&historical, &[]).0), 1);
    assert_eq!(count(&discover(&p, &[&historical]).0), 1);
}

#[test]
fn typed_machine_exposure_retains_only_the_named_spill_in_current_and_old_programs() {
    let mut p = program(vec![MirOp::MachineBlock {
        id: MirMachineBlockId(0),
        effects: Default::default(),
    }]);
    p.routines[0].frame.spills.push(MirSpillId(1));
    p.machine_blocks.push(MirMachineBlock {
        id: MirMachineBlockId(0),
        source_span: None,
        items: vec![MirMachineItem::Byte(0xad), relocation(home())],
    });
    let expected = BTreeMap::from([(RoutineId(0), BTreeSet::from([MirSpillId(1)]))]);
    assert_eq!(discover(&p, &[]).0, expected);
    let mut current = p.clone();
    current.routines[0].blocks[0].ops.clear();
    assert_eq!(discover(&current, &[&p]).0, expected);
    // Unresolved assembly retains the frame, including historical references.
    p.machine_blocks[0].items = vec![MirMachineItem::Name("unknown".into())];
    assert!(discover(&current, &[&p]).0.is_empty());
}

#[test]
fn source_backing_retains_a_spill_even_when_only_a_historical_frame_exposes_it() {
    let p = program(vec![imm(7)]);
    let mut historical = p.clone();
    historical.routines[0]
        .frame
        .locals
        .push(source_slot(MirStorageBase::Spill(MirSpillId(0))));
    assert!(discover(&historical, &[]).0.is_empty());
    assert!(discover(&p, &[&historical]).0.is_empty());
}

#[test]
fn generic_source_read_write_effects_allow_private_tail_removal() {
    for effect in [MirMemoryEffect::Unknown, MirMemoryEffect::All] {
        let p = program(vec![
            MirOp::Barrier {
                effects: MirEffects {
                    memory_reads: effect.clone(),
                    memory_writes: effect,
                    opaque: true,
                    ..Default::default()
                },
            },
            imm(7),
        ]);
        let (after, _, events) = run(&p, &clean(&p));
        assert_eq!(result(&events)["accepted"], true, "{events:?}");
        assert!(after.routines[0].frame.spills.is_empty());
        assert_eq!(after.routines[0].blocks, p.routines[0].blocks);
    }
}

#[test]
fn unused_source_declarations_and_aliases_override_private_allocation() {
    for alias in [false, true] {
        let mut p = program(vec![imm(7)]);
        if alias {
            p.globals.push(MirGlobal {
                id: crate::nir::SymbolId(0),
                name: "base".into(),
                kind: "BYTE".into(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Absolute(0x2fff),
                init: None,
            });
            p.globals.push(MirGlobal {
                id: crate::nir::SymbolId(1),
                name: "alias".into(),
                backing: MirGlobalBacking::Alias {
                    target: crate::nir::SymbolId(0),
                    offset: 1,
                },
                ..p.globals[0].clone()
            });
        } else {
            p.routines[0]
                .frame
                .locals
                .push(source_slot(MirStorageBase::Absolute(0x3000)));
        }
        let (after, _, events) = run(&p, &clean(&p));
        assert_eq!(after, p);
        assert_eq!(
            result(&events)["reason"],
            "ObservableAllocation",
            "{events:?}"
        );
    }
}

#[test]
fn explicit_physical_regions_and_abi_homes_retain_allocations() {
    for write in [false, true] {
        let region = MirMemoryEffect::Regions(vec![MirMemoryRegion {
            kind: MirMemoryRegionKind::AbsoluteRange,
            offset: 0x3000,
            size: 1,
        }]);
        let mut effects = MirEffects::default();
        if write {
            effects.memory_writes = region;
        } else {
            effects.memory_reads = region;
        }
        let p = program(vec![MirOp::Barrier { effects }, imm(7)]);
        let (after, _, events) = run(&p, &clean(&p));
        assert_eq!(after, p);
        assert_eq!(result(&events)["reason"], "ObservableAllocation");
    }
    let p = program(vec![imm(7)]);
    let removed = discover(&p, &[]).0;
    let mut old = measure(
        &p,
        &[&p],
        0x3000,
        crate::runtime::Runtime::ActionCart,
        &Trace::disabled(),
    )
    .unwrap();
    for write in [false, true] {
        let mut observed = p.clone();
        observed.routines[0].blocks[0]
            .ops
            .push(MirOp::RuntimeHelper {
                helper: MirRuntimeHelper::SArgs,
                args: if write {
                    vec![]
                } else {
                    vec![MirArgHome::Absolute(0x3000)]
                },
                result: write.then_some(MirResultHome::Absolute(0x3000)),
                additional_results: vec![],
                effects: MirEffects {
                    memory_reads: MirMemoryEffect::Unknown,
                    ..Default::default()
                },
            });
        assert!(!unobserved_allocations(&observed, &old, &removed));
    }
    // An incomplete source inventory cannot certify a private allocation.
    old.memory.protect_source_storage(None);
    assert!(!unobserved_allocations(&p, &old, &removed));
}

#[test]
fn inspectable_machine_reads_and_writes_retain_only_overlapping_allocations() {
    for opcode in [0xad, 0x8d] {
        for address in [0x3000u16, 0x7000] {
            let mut p = program(vec![MirOp::MachineBlock {
                id: MirMachineBlockId(0),
                effects: MirEffects {
                    memory_reads: MirMemoryEffect::Unknown,
                    memory_writes: MirMemoryEffect::Unknown,
                    ..Default::default()
                },
            }]);
            p.routines[0].blocks[0].terminator = MirTerminator::Unreachable;
            p.machine_blocks.push(MirMachineBlock {
                id: MirMachineBlockId(0),
                source_span: None,
                items: [opcode, address as u8, (address >> 8) as u8, 0x60]
                    .into_iter()
                    .map(MirMachineItem::Byte)
                    .collect(),
            });
            let (after, _, events) = run(&p, &clean(&p));
            if address == 0x3000 {
                assert_eq!(after, p);
                assert_eq!(result(&events)["reason"], "ObservableAllocation");
            } else {
                assert!(after.routines[0].frame.spills.is_empty(), "{events:?}");
                assert_eq!(result(&events)["accepted"], true);
            }
        }
    }
}
