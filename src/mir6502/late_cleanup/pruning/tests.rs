use super::*;
use crate::mir6502::{late_cleanup::tests::*, trace::Mir6502Trace};

fn run(
    p: &MirProgram,
    log: &Preservation,
) -> (
    MirProgram,
    serde_json::Value,
    Vec<crate::mir6502::trace::TraceEvent>,
) {
    let session = Mir6502Trace::default();
    let selected = select(
        p.clone(),
        &[p],
        log,
        None,
        &Mir6502Config::optimized(),
        0x3000,
        crate::runtime::Runtime::ActionCart,
        &session.context(),
    )
    .unwrap();
    (selected, session.manifest(), session.events())
}
fn result(events: &[crate::mir6502::trace::TraceEvent]) -> &serde_json::Value {
    &events
        .iter()
        .find(|e| e.kind == "late-spill-pruning-result")
        .unwrap()
        .data
}

#[test]
fn prunes_unused_tail_with_exact_delta_and_bounded_work() {
    let mut p = program(vec![imm(23), store(), load(home())]);
    p.routines[0].frame.spills.push(MirSpillId(1));
    p.routines[0]
        .frame
        .zero_page_allocations
        .push(MirZpAllocation {
            slot: MirZpSlot(0),
            start: MirFixedZpSlot(0xe0),
            size: 2,
        });
    p.routines[0].frame.virtual_zero_page.push(MirZpSlot(0));
    let (after, manifest, events) = run(&p, &clean(&p));
    assert_closed_attempts(&events);
    assert_eq!(result(&events)["accepted"], true, "{events:?}");
    assert_eq!(result(&events)["removed_spills"], 1);
    assert_eq!(
        result(&events)["image_bytes_before"].as_u64().unwrap()
            - result(&events)["image_bytes_after"].as_u64().unwrap(),
        1
    );
    assert_eq!(manifest["work"]["emission_invocations"], 2);
    assert!(manifest["work"]["materializations"].is_null());
    let mut expected = p.clone();
    expected.routines[0].frame.spills.pop();
    assert_eq!(after, expected);
    let (again, manifest, events) = run(&after, &clean(&after));
    assert_eq!(again, after);
    assert_eq!(result(&events)["reason"], "NoUnusedSpills");
    assert!(manifest["work"]["emission_invocations"].is_null());
}

#[test]
fn interior_hole_preserves_core_allocation_without_costing() {
    let mut p = program(vec![load(MirMem::Spill {
        id: MirSpillId(1),
        offset: 0,
    })]);
    p.routines[0].frame.spills.push(MirSpillId(1));
    let (after, manifest, events) = run(&p, &clean(&p));
    assert_eq!(after, p);
    assert_eq!(result(&events)["reason"], "CoreAllocation");
    assert!(manifest["work"]["emission_invocations"].is_null());
}

#[test]
fn structured_address_and_historical_frame_exposure_retain_declarations() {
    let address = MirOp::LeaAddr {
        dst: MirDef::Reg(MirReg::A),
        target: home(),
        width: MirWidth::Byte,
    };
    let p = program(vec![address.clone()]);
    assert_eq!(discover(&p, &[&p]).0.len(), 0);
    let p = program(vec![imm(7)]);
    let historical = program(vec![address]);
    assert!(discover(&p, &[&historical]).0.is_empty());
    for item in [
        MirMachineItem::Name("spill0".into()),
        MirMachineItem::Relocation {
            kind: crate::asm6502::InlineAsmRelocationKind::Absolute16,
            target: MirInlineAsmTarget::Memory(home()),
            addend: 0,
            requires_zero_page: false,
            span: crate::source::Span { start: 0, end: 0 },
        },
    ] {
        let mut machine = program(vec![MirOp::MachineBlock {
            id: MirMachineBlockId(0),
            effects: Default::default(),
        }]);
        machine.machine_blocks.push(MirMachineBlock {
            id: MirMachineBlockId(0),
            source_span: None,
            items: vec![item],
        });
        assert!(discover(&machine, &[]).0.is_empty());
    }
    let mut p = p;
    p.routines[0].blocks[0].terminator = MirTerminator::Branch {
        cond: MirCond::BoolValue(MirValue::PointerCell(home())),
        then_edge: MirEdge::plain(MirBlockId(0)),
        else_edge: MirEdge::plain(MirBlockId(0)),
    };
    assert!(discover(&p, &[]).0.is_empty());
}

#[test]
fn absolute_reads_and_writes_retain_unused_allocation() {
    for write in [false, true] {
        let op = if write {
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x3000)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            }
        } else {
            load(MirMem::Absolute(0x3000))
        };
        let p = program(vec![imm(7), op]);
        let (after, manifest, events) = run(&p, &clean(&p));
        assert_eq!(after, p);
        assert_eq!(result(&events)["reason"], "ObservableAllocation");
        assert_eq!(manifest["work"]["emission_invocations"], 1);
    }
}

#[test]
fn failed_earlier_replay_restores_whole_pruning_baseline() {
    let p = program(vec![imm(7)]);
    let mut log = Preservation::new(true);
    let mut before = p.clone();
    before.routines[0].blocks[0].ops.insert(0, imm(99));
    log.select(Family::CheckedRanges, before, |_| Ok(p.clone()))
        .unwrap();
    let (after, manifest, events) = run(&p, &log);
    assert_eq!(after, p);
    assert_closed_attempts(&events);
    assert_eq!(result(&events)["reason"], "EarlierDecision(CheckedRanges)");
    assert_eq!(manifest["work"]["emission_invocations"], 2);
}

#[test]
fn declaration_guard_rejects_operations_mappings_and_live_home_changes() {
    let p = program(vec![imm(7)]);
    let removed = BTreeMap::from([(RoutineId(0), BTreeSet::from([MirSpillId(0)]))]);
    let after = erase(&p, &removed);
    assert!(only_declarations(&p, &after, &removed));
    let mut bad = after.clone();
    bad.routines[0].blocks[0].ops.clear();
    assert!(!only_declarations(&p, &bad, &removed));
    let mut bad = after;
    bad.routines[0]
        .frame
        .fixed_zero_page
        .push(MirFixedZpSlot(0xff));
    assert!(!only_declarations(&p, &bad, &removed));
}

#[test]
fn moved_deferred_storage_rolls_back_despite_stable_core_addresses() {
    let mut p = program(vec![imm(7)]);
    p.routines[0].frame.locals.push(MirStorageSlot {
        purpose: MirStoragePurpose::Storage,
        id: MirStorageId(0),
        name: None,
        storage: MirStorageClass::Array,
        storage_size: 300,
        scalar_width: None,
        base: MirStorageBase::Local(crate::nir::LocalId(0)),
        offset: 0,
        mutable: true,
        init: Some(MirStorageInit::ZeroFill {
            bytes: 300,
            mutable: true,
            section: "data".into(),
        }),
    });
    let (after, manifest, events) = run(&p, &clean(&p));
    assert_eq!(after, p);
    assert_eq!(result(&events)["reason"], "PhysicalAllocation");
    assert_eq!(manifest["work"]["emission_invocations"], 2);
}

#[test]
fn shared_cost_gate_rejects_zero_saving_and_individual_routine_growth() {
    let p = program(vec![imm(7)]);
    let old = measure(
        &p,
        &[&p],
        0x3000,
        crate::runtime::Runtime::ActionCart,
        &Trace::disabled(),
    )
    .unwrap();
    let mut new = measure(
        &p,
        &[&p],
        0x3000,
        crate::runtime::Runtime::ActionCart,
        &Trace::disabled(),
    )
    .unwrap();
    assert!(!improves_cost(&p, &old, &new));
    new.image_bytes -= 1;
    assert!(improves_cost(&p, &old, &new));
    *new.instruction_bytes.get_mut(&RoutineId(0)).unwrap() += 1;
    assert!(!improves_cost(&p, &old, &new));
}

#[test]
fn shared_terminal_boundary_preserves_prior_cleanup_on_pruning_rejection() {
    let mut p = program(vec![
        imm(23),
        store(),
        imm(83),
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill {
                id: MirSpillId(1),
                offset: 0,
            }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        load(MirMem::Spill {
            id: MirSpillId(1),
            offset: 0,
        }),
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x7000)),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ]);
    p.routines[0].frame.spills.push(MirSpillId(1));
    let preservation = clean(&p);
    let session = Mir6502Trace::default();
    let selected = super::super::select(
        p.clone(),
        &preservation,
        None,
        &Mir6502Config {
            enable_late_spill_pruning: true,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::runtime::Runtime::ActionCart,
        &session.context(),
    )
    .unwrap();
    assert_eq!(selected.routines[0].frame, p.routines[0].frame);
    assert!(selected.routines[0].blocks[0].ops.len() < p.routines[0].blocks[0].ops.len());
    assert_eq!(result(&session.events())["reason"], "CoreAllocation");
    assert_eq!(session.manifest()["work"]["emission_invocations"], 2);
}

fn oracle_program() -> MirProgram {
    program(vec![
        load(MirMem::Absolute(0x600)),
        MirOp::Binary {
            op: MirBinaryOp::Xor,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(0xa5),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x601)),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ])
}

fn emit_output(
    p: &MirProgram,
    origin: u16,
    runtime: crate::runtime::Runtime,
) -> crate::codegen::CodegenOutput {
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = emit::emit_program(p, origin, &mut emitter).unwrap();
    let emission = emitter.finish_with_relocations().unwrap();
    crate::mir6502::codegen_output(emission, origin, summary, runtime, p)
}

#[test]
fn pruned_tail_executes_all_byte_inputs_in_both_runtimes_and_origins() {
    use crate::runtime::Runtime;
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let p = oracle_program();
            let selected = select(
                p.clone(),
                &[&p],
                &clean(&p),
                None,
                &Mir6502Config::optimized(),
                origin,
                runtime,
                &Trace::disabled(),
            )
            .unwrap();
            assert!(selected.routines[0].frame.spills.is_empty());
            let before = emit_output(&p, origin, runtime);
            let after = emit_output(&selected, origin, runtime);
            assert_eq!(
                crate::codegen::format_load_file(&before).len()
                    - crate::codegen::format_load_file(&after).len(),
                1
            );
            for o in [&before, &after] {
                for input in 0..=255 {
                    let mut ram = [0u8; 65536];
                    let start = usize::from(origin);
                    ram[start..start + o.bytes.len()].copy_from_slice(&o.bytes);
                    ram[0x600] = input;
                    ram[0x5ff] = 0x5a;
                    ram[0x602] = 0xc3;
                    crate::mir6502::materialize::leaf_test_cpu::run_memory(
                        &mut ram,
                        usize::from(o.routine_addresses[0].address),
                    );
                    assert_eq!(ram[0x601], input ^ 0xa5);
                    assert_eq!((ram[0x5ff], ram[0x600], ram[0x602]), (0x5a, input, 0xc3));
                }
            }
        }
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn pruned_tail_mads_reconstruction_relocation_lf_crlf() {
    use crate::runtime::Runtime;
    let dir = std::env::temp_dir().join(format!("actionc-late-spills-mads-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let p = oracle_program();
        let outputs = [0x3000, 0x41c7].map(|origin| {
            let selected = select(
                p.clone(),
                &[&p],
                &clean(&p),
                None,
                &Mir6502Config::optimized(),
                origin,
                runtime,
                &Trace::disabled(),
            )
            .unwrap();
            assert!(selected.routines[0].frame.spills.is_empty());
            emit_output(&selected, origin, runtime)
        });
        for (index, origin) in [0x3000, 0x41c7].into_iter().enumerate() {
            let listing = crate::compiler::artifacts::format_listing_with_source(&outputs[0], "")
                .replace("\r\n", "\n")
                .replace(
                    "ACTIONC_ORIGIN = $3000",
                    &format!("ACTIONC_ORIGIN = ${origin:04X}"),
                );
            for crlf in [false, true] {
                let name = format!("{runtime:?}-{origin:x}-{crlf}");
                let asm = dir.join(format!("{name}.asm"));
                let xex = dir.join(format!("{name}.xex"));
                std::fs::write(
                    &asm,
                    if crlf {
                        listing.replace('\n', "\r\n")
                    } else {
                        listing.clone()
                    },
                )
                .unwrap();
                let result = std::process::Command::new(
                    std::env::var_os("ACTIONC_MADS").unwrap_or_else(|| "mads".into()),
                )
                .arg(&asm)
                .arg(format!("-o:{}", xex.display()))
                .arg("-s")
                .output()
                .unwrap();
                assert!(
                    result.status.success(),
                    "{} {}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    std::fs::read(&xex).unwrap(),
                    crate::codegen::format_load_file(&outputs[index])
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_machine_reader_retains_zero_initialized_unused_storage() {
    let mut p = program(vec![MirOp::MachineBlock {
        id: MirMachineBlockId(0),
        effects: MirEffects {
            memory_reads: MirMemoryEffect::Unknown,
            ..Default::default()
        },
    }]);
    p.machine_blocks.push(MirMachineBlock {
        id: MirMachineBlockId(0),
        source_span: None,
        items: vec![MirMachineItem::Byte(0xb1), MirMachineItem::Byte(0x80)],
    });
    let (after, manifest, events) = run(&p, &clean(&p));
    assert_eq!(after, p);
    assert_eq!(result(&events)["reason"], "ObservableAllocation");
    assert_eq!(manifest["work"]["emission_invocations"], 1);
}
