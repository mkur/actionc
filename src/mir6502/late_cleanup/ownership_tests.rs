use super::*;

fn storage_observation_fixture(read: u8) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(
        "PROC Resident(CARD p) RETURN PROC Worker() RETURN",
    ).unwrap()).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let mut p = crate::mir6502::lower_program(&nir).unwrap();
    let r = &mut p.routines[0];
    let MirStorageBase::Param(id) = r.frame.params[0].base else { panic!() };
    r.frame.parameter_homes = Some(vec![MirParamBinding {
        param: id, home: MirParamHome::ZeroPage(MirFixedZpSlot(0xe2)),
    }]);
    r.frame.fixed_zero_page.extend([MirFixedZpSlot(0xe2), MirFixedZpSlot(0xe3)]);
    let mut p = crate::mir6502::materialize_program(p, &Mir6502Config::default()).unwrap();
    let r = &mut p.routines[0];
    match read {
        0 => {},
        1 => r.blocks[0].ops.push(load(MirMem::Param { id, offset: 1 })),
        2 => r.blocks[0].ops.push(load(MirMem::FixedZeroPage(MirFixedZpSlot(0xe3)))),
        3 => r.blocks[0].ops.push(MirOp::LoadIndirect {
            consumer: consumer(0xe2), dst: MirDef::Reg(MirReg::A), offset: 0,
        }),
        _ => unreachable!(),
    }
    let r = &mut p.routines[1];
    let mem = MirMem::ZeroPage(MirZpSlot(0));
    r.frame.virtual_zero_page = vec![MirZpSlot(0)];
    r.frame.zero_page_allocations = vec![MirZpAllocation {
        slot: MirZpSlot(0), start: MirFixedZpSlot(0xe3), size: 1,
    }];
    r.blocks[0].ops = vec![imm(23), write(mem.clone()), imm(128), write(mem.clone()), load(mem), sink()];
    p
}

#[test]
fn storage_observation_baseline_has_two_independent_vetoes() {
    for read in 0..4 {
        let p = storage_observation_fixture(read);
        let m = measure(&p, &[&p], 0x3000, crate::runtime::Runtime::ActionCart, &Trace::disabled()).unwrap();
        let worker = &p.routines[1];
        let op = &worker.blocks[0].ops[1];
        let ownership = StorageOwnership::new(&p);
        assert!(ownership.private_store(worker.id, op, None).is_some());
        assert!(ownership.private_store(worker.id, op, Some(&m.memory)).is_none());
        let reads = read_aliases(&p, &m.memory, true);
        assert_eq!(reads.iter().any(|(a, n)| a.is_some_and(|a| 0xe3u16.wrapping_sub(a) < *n)), read != 0);
        assert_eq!(apply(&p, Some(&m.memory), false, &Trace::disabled()).unwrap(), p);
    }
}

#[test]
fn storage_observation_inventory_keeps_owners_aliases_and_lane_extents() {
    use crate::mir6502::analysis::record_bases::{SourceBinding, SourceObject};
    let mut p = storage_observation_fixture(0);
    let mut other = p.routines[0].clone();
    other.id = RoutineId(200);
    p.routines.push(other);
    p.globals.push(fixed_global(0xe4, 1));
    let mut alias = fixed_global(0xe4, 1);
    alias.id = crate::nir::SymbolId(1);
    p.globals.push(alias);
    let m = measure(&p, &[&p], 0x3000, crate::runtime::Runtime::ActionCart, &Trace::disabled()).unwrap();
    let records = m.memory.source_storage().unwrap();
    let aliases: Vec<_> = records.iter().filter(|s| s.contains(0xe3)).collect();
    assert_eq!(aliases.len(), 2);
    assert_eq!(records.iter().filter(|s| s.contains(0xe4)
        && matches!(s.object, SourceObject::Global(_))).count(), 2);
    for owner in [p.routines[0].id, RoutineId(200)] {
        assert!(aliases.iter().any(|s| matches!(&s.object,
            SourceObject::Routine { owner: r, base: MirStorageBase::Param(_), .. } if *r == owner)
            && s.binding == Some(SourceBinding::Parameter(MirParamHome::ZeroPage(MirFixedZpSlot(0xe2))))));
    }
    for a in 0..=255 {
        assert_eq!(m.memory.source_storage_disjoint(a), !matches!(a, 0xe2..=0xe4));
    }
    let r = &mut p.routines[0];
    r.frame.parameter_homes.as_mut().unwrap()[0].home = MirParamHome::Lanes(vec![
        MirParamLane::Reg(MirReg::A), MirParamLane::ZeroPage(MirFixedZpSlot(0xe3)),
    ]);
    r.blocks[0].ops.retain(|op| !matches!(op, MirOp::Store {
        dst: MirAddr::Direct(MirMem::Param { offset: 0, .. }), ..
    }));
    let m = measure(&p, &[&p], 0x3000, crate::runtime::Runtime::ActionCart, &Trace::disabled()).unwrap();
    let lanes: Vec<_> = m.memory.source_storage().unwrap().iter().filter(|s|
        matches!(s.object, SourceObject::Routine { owner, .. } if owner == p.routines[0].id)).collect();
    assert_eq!(lanes.len(), 1);
    assert_eq!((lanes[0].offset, lanes[0].address, lanes[0].bytes), (1, 0xe3, 1));
    let mut missing = m.memory.clone();
    missing.protect_source_storage(None);
    assert!(!missing.source_storage_disjoint(0xe4));
}

fn write(mem: MirMem) -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(mem),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }
}

fn consumer(lo: u8) -> MirAddressConsumer {
    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(lo),
    })
}

fn fixture(mem: &MirMem) -> MirProgram {
    let mut p = program(vec![
        MirOp::MaterializeAddress {
            consumer: consumer(0xac),
            value: MirValue::ConstU16(0x680),
        },
        MirOp::MaterializeAddress {
            consumer: consumer(0xae),
            value: MirValue::ConstU16(0x681),
        },
        imm(23),
        write(mem.clone()),
        MirOp::LoadIndirect {
            consumer: consumer(0xae),
            dst: MirDef::Reg(MirReg::A),
            offset: 0,
        },
        imm(128),
        write(mem.clone()),
        load(mem.clone()),
        sink(),
    ]);
    p.routines[0].frame.fixed_zero_page = (0xac..=0xaf).map(MirFixedZpSlot).collect();
    p.routines[0].frame.virtual_zero_page.push(MirZpSlot(0));
    p.routines[0]
        .frame
        .zero_page_allocations
        .push(MirZpAllocation {
            slot: MirZpSlot(0),
            start: MirFixedZpSlot(0xe2),
            size: 1,
        });
    p
}

#[test]
fn same_definition_lifetime_rule_for_spill_allocated_zp_and_fixed_pointer_scratch() {
    for mem in [
        home(),
        MirMem::ZeroPage(MirZpSlot(0)),
        MirMem::FixedZeroPage(MirFixedZpSlot(0xac)),
    ] {
        let p = fixture(&mem);
        let (selected, _, events) = run(&p, &clean(&p));
        let mut expected = p.clone();
        expected.routines[0].blocks[0].ops.remove(3);
        assert_eq!(selected, expected, "{mem:?}: {events:?}");
        assert_eq!(selected.routines[0].frame, p.routines[0].frame);
        let result = &events
            .iter()
            .find(|e| e.kind == "late-cleanup-result")
            .unwrap()
            .data;
        assert_eq!(result["removed_stores"], 1);
        assert_eq!(
            result["image_bytes_before"].as_u64().unwrap()
                - result["image_bytes_after"].as_u64().unwrap(),
            if mem == home() { 3 } else { 2 }
        );
        // A real home read before the overwrite keeps the first definition.
        let mut read = p.clone();
        read.routines[0].blocks[0].ops[4] = load(mem.clone());
        assert_eq!(run(&read, &clean(&read)).0, read, "{mem:?}");
    }
}

fn fixed_global(address: u16, bytes: u16) -> MirGlobal {
    MirGlobal {
        id: crate::nir::SymbolId(0),
        name: "source_cell".into(),
        kind: "BYTE".into(),
        width: Some(MirWidth::Byte),
        storage_size: bytes,
        backing: MirGlobalBacking::Absolute(address),
        init: None,
    }
}

#[test]
fn declared_source_zp_overrides_private_placement_including_tn_cells() {
    for address in [0xe0, 0xe1, 0xe6, 0xe7, 0xe8, 0xe9, 0xac, 0xad] {
        let mem = if address >= 0xe0 {
            MirMem::ZeroPage(MirZpSlot(0))
        } else {
            MirMem::FixedZeroPage(MirFixedZpSlot(address as u8))
        };
        let mut p = fixture(&mem);
        p.globals.push(fixed_global(address, 1));
        if address >= 0xe0 {
            p.routines[0].frame.zero_page_allocations[0].start = MirFixedZpSlot(address as u8);
        }
        // The declaration has no MIR reads: emission must still publish it in
        // the source-storage inventory, independently of the reference map.
        let measured = measure(
            &p,
            &[&p],
            0x3000,
            crate::runtime::Runtime::ActionCart,
            &Trace::disabled(),
        )
        .unwrap();
        let ownership = StorageOwnership::new(&p);
        assert!(
            ownership
                .private_store(RoutineId(0), &write(mem), Some(&measured.memory))
                .is_none(),
            "{address:x}"
        );
    }
}

#[test]
fn missing_source_inventory_cannot_certify_private_storage() {
    let p = fixture(&home());
    let mut memory = MirPhysicalMemory::default();
    memory.insert(RoutineId(0), home(), 0x3000, true);
    assert!(
        StorageOwnership::new(&p)
            .private_store(RoutineId(0), &store(), Some(&memory))
            .is_none()
    );
}

#[test]
fn ownership_cleanup_executes_all_bytes_in_both_runtimes_and_origins() {
    use crate::runtime::Runtime;
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            for mem in [
                home(),
                MirMem::ZeroPage(MirZpSlot(0)),
                MirMem::FixedZeroPage(MirFixedZpSlot(0xac)),
            ] {
                let mut p = fixture(&mem);
                p.routines[0].blocks[0]
                    .ops
                    .insert(5, write(MirMem::Absolute(0x7001)));
                let selected = select(
                    p.clone(),
                    &clean(&p),
                    None,
                    &Mir6502Config {
                        enable_late_dead_register_cleanup: false,
                        ..Mir6502Config::optimized()
                    },
                    origin,
                    runtime,
                    &Trace::disabled(),
                )
                .unwrap();
                assert_eq!(count_ops(&p) - count_ops(&selected), 1);
                for program in [&p, &selected] {
                    let mut emitter =
                        crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
                    let summary = emit::emit_program(program, origin, &mut emitter).unwrap();
                    let output = crate::mir6502::codegen_output(
                        emitter.finish_with_relocations().unwrap(),
                        origin,
                        summary,
                        runtime,
                        program,
                    );
                    for input in 0..=255u8 {
                        let mut ram = [0u8; 65536];
                        let start = usize::from(origin);
                        ram[start..start + output.bytes.len()].copy_from_slice(&output.bytes);
                        ram[0x681] = input;
                        for a in [0xe0, 0xe1, 0xe6, 0xe7, 0xe8, 0xe9] {
                            ram[a] = 0xa5;
                        }
                        crate::mir6502::materialize::leaf_test_cpu::run_memory(
                            &mut ram,
                            usize::from(output.routine_addresses[0].address),
                        );
                        assert_eq!((ram[0x7000], ram[0x7001], ram[0x681]), (128, input, input));
                        for a in [0xe0, 0xe1, 0xe6, 0xe7, 0xe8, 0xe9] {
                            assert_eq!(ram[a], 0xa5);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn role_requires_a_declared_home_or_reserved_typed_pointer_use() {
    let mut p = fixture(&home());
    let own = StorageOwnership::new(&p);
    for slot in [0xa0, 0xa1, 0xa3, 0xaa, 0xe0, 0xe6, 0xe8] {
        assert_eq!(
            own.role(RoutineId(0), &MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
            StorageRole::ProgramOrAbi
        );
    }
    assert_eq!(
        own.role(RoutineId(0), &MirMem::Absolute(0xac)),
        StorageRole::ProgramOrAbi
    );
    assert_eq!(
        own.role(RoutineId(0), &MirMem::FixedZeroPage(MirFixedZpSlot(0xac))),
        StorageRole::PointerScratch
    );
    p.routines[0].frame.fixed_zero_page.clear();
    assert_eq!(
        StorageOwnership::new(&p).role(RoutineId(0), &MirMem::FixedZeroPage(MirFixedZpSlot(0xac))),
        StorageRole::ProgramOrAbi
    );
    p.routines[0].blocks[0].ops.push(MirOp::LeaAddr {
        dst: MirDef::Reg(MirReg::A),
        target: home(),
        width: MirWidth::Word,
    });
    assert_eq!(
        StorageOwnership::new(&p).role(RoutineId(0), &home()),
        StorageRole::ProgramOrAbi
    );
}

#[test]
fn explicit_physical_and_abi_readers_keep_all_private_placements() {
    for mem in [
        home(),
        MirMem::ZeroPage(MirZpSlot(0)),
        MirMem::FixedZeroPage(MirFixedZpSlot(0xac)),
    ] {
        let mut p = fixture(&mem);
        p.routines[0].blocks[0].ops[4] = MirOp::Barrier {
            effects: MirEffects {
                memory_reads: MirMemoryEffect::Regions(vec![MirMemoryRegion {
                    kind: MirMemoryRegionKind::AbsoluteRange,
                    offset: 0,
                    size: u16::MAX,
                }]),
                opaque: true,
                ..Default::default()
            },
        };
        assert_eq!(run(&p, &clean(&p)).0, p, "{mem:?}");
        let measured = measure(
            &p,
            &[&p],
            0x3000,
            crate::runtime::Runtime::ActionCart,
            &Trace::disabled(),
        )
        .unwrap();
        let address = measured.memory.address(RoutineId(0), &mem).unwrap();
        p.routines[0].blocks[0].ops[4] = MirOp::Barrier {
            effects: MirEffects {
                memory_reads: MirMemoryEffect::Regions(vec![MirMemoryRegion {
                    kind: if address < 0x100 {
                        MirMemoryRegionKind::ZeroPage
                    } else {
                        MirMemoryRegionKind::AbsoluteRange
                    },
                    offset: address,
                    size: 1,
                }]),
                ..Default::default()
            },
        };
        assert_eq!(run(&p, &clean(&p)).0, p, "explicit physical region {mem:?}");
    }
    let mem = MirMem::FixedZeroPage(MirFixedZpSlot(0xac));
    let mut p = fixture(&mem);
    // A helper explicitly consumes this physical ABI byte even when its other
    // memory effects are empty. Being used as pointer scratch elsewhere does
    // not erase that argument use.
    p.routines[0].blocks[0].ops[4] = MirOp::RuntimeHelper {
        helper: MirRuntimeHelper::SArgs,
        args: vec![MirArgHome::FixedZeroPage(MirFixedZpSlot(0xac))],
        result: None,
        additional_results: vec![],
        effects: Default::default(),
    };
    // Logical discovery must retain it, without attempting helper emission.
    assert_eq!(apply(&p, None, false, &Trace::disabled()).unwrap(), p);
}

#[test]
fn unknown_source_memory_does_not_expose_non_escaping_temporaries() {
    for mem in [home(), MirMem::ZeroPage(MirZpSlot(0))] {
        for effect in [MirMemoryEffect::Unknown, MirMemoryEffect::All] {
            let mut p = fixture(&mem);
            p.routines[0].blocks[0].ops[4] = MirOp::Barrier {
                effects: MirEffects {
                    memory_reads: effect,
                    opaque: true,
                    ..Default::default()
                },
            };
            let mut expected = p.clone();
            expected.routines[0].blocks[0].ops.remove(3);
            assert_eq!(run(&p, &clean(&p)).0, expected);
        }
    }
}

#[test]
fn typed_machine_reference_exposes_a_temporary_even_with_unknown_reads() {
    let mut p = fixture(&home());
    p.routines[0].blocks[0].ops[4] = MirOp::MachineBlock {
        id: MirMachineBlockId(0),
        effects: MirEffects {
            memory_reads: MirMemoryEffect::Unknown,
            ..Default::default()
        },
    };
    p.machine_blocks.push(MirMachineBlock {
        id: MirMachineBlockId(0),
        source_span: None,
        items: vec![
            MirMachineItem::Byte(0xad),
            MirMachineItem::Relocation {
                kind: crate::asm6502::InlineAsmRelocationKind::Absolute16,
                target: MirInlineAsmTarget::Memory(home()),
                addend: 0,
                requires_zero_page: false,
                span: crate::source::Span { start: 0, end: 0 },
            },
        ],
    });
    assert_eq!(run(&p, &clean(&p)).0, p);
}

#[test]
fn explicit_numeric_alias_and_called_pointer_pair_keep_definitions() {
    let mem = MirMem::FixedZeroPage(MirFixedZpSlot(0xac));
    let mut p = fixture(&mem);
    p.routines[0].blocks[0].ops[4] = load(MirMem::Absolute(0xac));
    assert_eq!(run(&p, &clean(&p)).0, p);
    let mut callee = fixture(&mem).routines.remove(0);
    callee.id = RoutineId(1);
    callee.name = "ObservePair".into();
    callee.abi = MirRoutineAbi::Action;
    callee.blocks[0].ops = vec![MirOp::LoadIndirect {
        consumer: consumer(0xac),
        dst: MirDef::Reg(MirReg::A),
        offset: 0,
    }];
    p.routines.push(callee);
    p.routines[0].blocks[0].ops[4] = MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(1)),
        abi: MirCallAbi {
            params: vec![],
            result: None,
            additional_results: vec![],
            clobbers: Default::default(),
            preserves: Default::default(),
        },
        args: vec![],
        result: None,
        additional_results: vec![],
        effects: Default::default(),
    };
    assert_eq!(run(&p, &clean(&p)).0, p);
}

#[test]
fn storage_observation_certificates_require_logical_definitions_and_current_bindings() {
    use crate::mir6502::{parameter_homes::InvocationHomes, analysis::sites::MirSite};
    for read in [1, 2, 3] {
        let p = storage_observation_fixture(read);
        let m = measure(&p, &[&p], 0x3000, crate::runtime::Runtime::ActionCart, &Trace::disabled()).unwrap();
        let record = &m.memory.source_storage().unwrap()[0];
        let facts = InvocationHomes::new(&p);
        assert!(facts.is_current(&p));
        assert!(facts.certifies_storage(record), "{}", crate::mir6502::format_program(&p));
        let r = &p.routines[0];
        let site = MirSite::Op { block: r.blocks[0].id, op_index: r.blocks[0].ops.len() - 1 };
        let mem = if read == 1 { MirMem::Param { id: crate::nir::ParamId(0), offset: 1 } }
            else { MirMem::FixedZeroPage(MirFixedZpSlot(0xe3)) };
        assert!(facts.read_object(r.id, site, &mem).is_some());
        assert!(facts.read_object(r.id, site, &MirMem::Absolute(0xe3)).is_none());
        assert!(facts.read_object(r.id, MirSite::Op { block: r.blocks[0].id, op_index: 0 }, &mem).is_none());
        let mut bad = p.clone();
        for op in &mut bad.routines[0].blocks[0].ops {
            if let MirOp::Store { dst, .. } = op
                && matches!(dst, MirAddr::Direct(MirMem::Param { offset: 1, .. })) {
                *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xe3)));
            }
        }
        assert!(!InvocationHomes::new(&bad).certifies_storage(record));
        let mut moved = p.clone();
        moved.routines[0].frame.parameter_homes.as_mut().unwrap()[0].home = MirParamHome::ZeroPage(MirFixedZpSlot(0xe4));
        moved.routines[0].frame.fixed_zero_page.extend([MirFixedZpSlot(0xe4), MirFixedZpSlot(0xe5)]);
        assert!(!InvocationHomes::new(&moved).certifies_storage(record));
    }
}

#[test]
fn storage_observation_local_certificate_excludes_entry_dependence() {
    use crate::mir6502::parameter_homes::InvocationHomes;
    let ast = crate::parser::parse(&crate::lexer::tokenize(
        "CARD output=$7002 PROC Resident() CARD kept kept=23 output=kept RETURN PROC Main() RETURN",
    ).unwrap()).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let mut p = crate::mir6502::lower_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model))).unwrap();
    let r = &mut p.routines[0];
    let MirStorageBase::Local(id) = r.frame.locals[0].base else { panic!() };
    r.frame.local_homes = Some(vec![MirLocalBinding { local: id, home: MirLocalHome::ZeroPage(MirFixedZpSlot(0xe4)) }]);
    r.frame.fixed_zero_page.extend([MirFixedZpSlot(0xe4), MirFixedZpSlot(0xe5)]);
    let mut p = crate::mir6502::materialize_program(p, &Mir6502Config::default()).unwrap();
    p.routines[0].blocks[0].ops.push(load(MirMem::Local { id, offset: 1 }));
    let m = measure(&p, &[&p], 0x3000, crate::runtime::Runtime::ActionCart, &Trace::disabled()).unwrap();
    let record = m.memory.source_storage().unwrap().iter().find(|s| s.address == 0xe4).unwrap();
    assert!(InvocationHomes::new(&p).certifies_storage(record));
    p.routines[0].blocks[0].ops.retain(|op| !matches!(op,
        MirOp::Store { dst: MirAddr::Direct(MirMem::Local { offset: 1, .. }), .. }));
    assert!(!InvocationHomes::new(&p).certifies_storage(record));
}
