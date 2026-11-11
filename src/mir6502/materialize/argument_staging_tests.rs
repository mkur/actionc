use super::*;
use std::sync::OnceLock;

fn baseline() -> MirProgram {
    static BASE: OnceLock<MirProgram> = OnceLock::new();
    BASE.get_or_init(|| {
        let source =
            include_str!("../../../fixtures/runtime/argument_staging.act").replace("\r\n", "\n");
        let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        let config = Mir6502Config {
            enable_argument_staging_reduction: false,
            ..Mir6502Config::optimized()
        };
        crate::mir6502::materialize_program_with_origin_and_runtime(
            crate::mir6502::lower_program(&nir).unwrap(),
            &config,
            0x3000,
            crate::runtime::Runtime::Standalone,
        )
        .unwrap()
    })
    .clone()
}

fn selected(p: MirProgram) -> MirProgram {
    let eligible = p.routines.iter().map(|r| r.id).collect();
    select_costed(
        p,
        &Mir6502Config::optimized(),
        0x3000,
        crate::runtime::Runtime::Standalone,
        &eligible,
    )
    .unwrap()
}

#[test]
fn argument_staging_keeps_all_stores_homes_source_reads_and_calls() {
    let old = baseline();
    let new = selected(old.clone());
    assert_ne!(old, new);
    assert_eq!(new.globals, old.globals);
    assert_eq!(new.machine_blocks, old.machine_blocks);
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        let stores = |r: &MirRoutine| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|o| matches!(o, MirOp::Store { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        let calls = |r: &MirRoutine| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|o| matches!(o, MirOp::Call { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        let source_reads = |r: &MirRoutine| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter_map(|o| match o {
                    MirOp::Load {
                        src: MirAddr::Direct(mem),
                        width,
                        ..
                    } if !peepholes::mem_is_private_scratch(mem) => Some((mem.clone(), *width)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(stores(a), stores(b));
        assert_eq!(calls(a), calls(b));
        assert_eq!(source_reads(a), source_reads(b));
        if ["ForwardAdd", "ForwardSub"].contains(&a.name.as_str()) {
            assert_ne!(a, b);
        } else {
            assert_eq!(a, b);
        }
    }
    assert_eq!(
        image_size(&old, 0x3000, crate::runtime::Runtime::Standalone).unwrap()
            - image_size(&new, 0x3000, crate::runtime::Runtime::Standalone).unwrap(),
        2
    );
    let eligible = old.routines.iter().map(|r| r.id).collect();
    let reported = select_costed(
        old,
        &Mir6502Config {
            peephole_report: crate::mir6502::MirPeepholeReportMode::PerRoutine,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::runtime::Runtime::Standalone,
        &eligible,
    )
    .unwrap();
    assert_eq!(new, reported);
}

#[test]
fn argument_staging_unknown_emission_cost_retains_the_complete_fallback() {
    let mut old = baseline();
    let control = old
        .routines
        .iter_mut()
        .find(|r| r.name == "FourByte")
        .unwrap();
    if let Some(MirOp::Call { target, .. }) = control.blocks[0]
        .ops
        .iter_mut()
        .find(|o| matches!(o, MirOp::Call { .. }))
    {
        *target = super::super::super::ir::MirCallTarget::Routine(RoutineId(u32::MAX));
    }
    assert!(image_size(&old, 0x3000, crate::runtime::Runtime::Standalone).is_none());
    let eligible = old.routines.iter().map(|r| r.id).collect();
    let new = select_costed(
        old.clone(),
        &Mir6502Config::optimized(),
        0x3000,
        crate::runtime::Runtime::Standalone,
        &eligible,
    )
    .unwrap();
    assert_eq!(new, old);
}

fn forward(p: &mut MirProgram) -> &mut MirRoutine {
    p.routines
        .iter_mut()
        .find(|r| r.name == "ForwardAdd")
        .unwrap()
}

fn capture_store(r: &MirRoutine) -> usize {
    (5..r.blocks[0].ops.len())
        .find(|&i| arithmetic_capture(r, &r.blocks[0], i).is_some())
        .unwrap()
}

fn apply_one(p: &MirProgram) -> (MirProgram, MirPeepholeStats, BTreeSet<RoutineId>) {
    let id = p
        .routines
        .iter()
        .find(|r| r.name == "ForwardAdd")
        .unwrap()
        .id;
    // Reuse the valid original placement when testing deliberately unsupported
    // operations or ABI shapes; those may themselves be non-emittable.
    let original = baseline();
    let physical = register_copies::physical_memory(&original, &original, 0x3000).unwrap();
    apply(p, &physical, &BTreeSet::from([id])).unwrap()
}

#[test]
fn argument_staging_rejects_intervening_registers_flags_calls_and_opaque_effects() {
    let intruders = [
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 7,
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA3))),
            src: MirValue::Def(MirDef::Reg(MirReg::X)),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::FromPrevious),
            carry_out: MirCarryOut::Produce,
        },
        MirOp::Barrier {
            effects: MirEffects::default(),
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::FixedIndirectIndexedY {
                zp: MirFixedZpSlot(0xAC),
            },
            width: MirWidth::Byte,
        },
    ];
    for op in intruders {
        let mut p = baseline();
        let r = forward(&mut p);
        let i = capture_store(r);
        r.blocks[0].ops.insert(i + 1, op);
        let (after, _, affected) = apply_one(&p);
        assert!(affected.is_empty());
        assert_eq!(after, p);
    }
    let mut p = baseline();
    let r = forward(&mut p);
    let i = capture_store(r);
    let call = r.blocks[0]
        .ops
        .iter()
        .find(|o| matches!(o, MirOp::Call { .. }))
        .unwrap()
        .clone();
    r.blocks[0].ops.insert(i + 1, call);
    assert!(apply_one(&p).2.is_empty());
}

#[test]
fn argument_staging_rejects_hardware_noncanonical_and_unknown_callees() {
    for kind in 0..4 {
        let mut p = baseline();
        let r = forward(&mut p);
        let call = r.blocks[0]
            .ops
            .iter()
            .position(|o| matches!(o, MirOp::Call { .. }))
            .unwrap();
        if kind == 0 {
            if let MirOp::Load { src, .. } = &mut r.blocks[0].ops[call - 1] {
                *src = MirAddr::Direct(MirMem::Absolute(0xD20F));
            }
        } else if let MirOp::Call {
            target,
            effects,
            abi,
            ..
        } = &mut r.blocks[0].ops[call]
        {
            match kind {
                1 => {
                    *target = MirCallTarget::Runtime {
                        name: "foreign".into(),
                        address: Some(0x7000),
                    }
                }
                2 => effects.opaque = true,
                _ => abi.params[0] = MirArgHome::Reg(MirReg::Y),
            }
        }
        assert!(apply_one(&p).2.is_empty());
    }
}

#[test]
fn argument_staging_physical_overlap_and_window_limits_block_selection() {
    let mut p = baseline();
    let r = forward(&mut p);
    let i = capture_store(r);
    let home = arithmetic_capture(r, &r.blocks[0], i).unwrap();
    let id = r.id;
    let mut physical = MirPhysicalMemory::default();
    physical.insert(id, home, 0xA3, true);
    let (_, stats, affected) = apply(&p, &physical, &BTreeSet::from([id])).unwrap();
    assert!(affected.is_empty());
    assert_eq!(stats.count_for(id, "argument-staging-blocked-overlap"), 1);
    for extra in [50, 51] {
        let mut p = baseline();
        let r = forward(&mut p);
        let i = capture_store(r);
        let op = r.blocks[0].ops[i + 1].clone();
        r.blocks[0]
            .ops
            .splice(i + 1..i + 1, std::iter::repeat_n(op, extra));
        assert_eq!(!apply_one(&p).2.is_empty(), extra == 50);
    }
}

#[test]
fn argument_staging_keeps_observed_captures_and_independent_profitable_routines() {
    let mut p = baseline();
    let r = forward(&mut p);
    let i = capture_store(r);
    let home = arithmetic_capture(r, &r.blocks[0], i).unwrap();
    r.blocks[0].ops.push(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(home.clone()),
        width: MirWidth::Byte,
    });
    let new = selected(p.clone());
    let r = new
        .routines
        .iter()
        .find(|r| r.name == "ForwardAdd")
        .unwrap();
    assert!(
        r.blocks[0]
            .ops
            .iter()
            .any(|o| matches!(o,MirOp::Store{dst:MirAddr::Direct(mem),..} if mem==&home))
    );
    assert!(
        r.blocks[0]
            .ops
            .iter()
            .any(|o| matches!(o,MirOp::Load{src:MirAddr::Direct(mem),..} if mem==&home))
    );
    assert_ne!(new, p);
    let mut p = baseline();
    let r = forward(&mut p);
    let i = capture_store(r);
    r.blocks[0].ops.insert(
        i + 1,
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 1,
            width: MirWidth::Byte,
        },
    );
    let new = selected(p.clone());
    assert_eq!(
        p.routines.iter().find(|r| r.name == "ForwardAdd"),
        new.routines.iter().find(|r| r.name == "ForwardAdd")
    );
    assert_ne!(
        p.routines.iter().find(|r| r.name == "ForwardSub"),
        new.routines.iter().find(|r| r.name == "ForwardSub")
    );
}

#[test]
fn argument_staging_posthome_plans_cannot_reuse_a_stale_generation() {
    use crate::mir6502::rewrite::driver::MirRewriteError;
    let p = baseline();
    let physical = register_copies::physical_memory(&p, &p, 0x3000).unwrap();
    let callees = p.routines.iter().map(|r| r.id).collect();
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let mut r = p
        .routines
        .iter()
        .find(|r| r.name == "ForwardAdd")
        .unwrap()
        .clone();
    let mut saved: Option<MirPostHomeRewritePlan> = None;
    let result = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
        &mut r,
        &summaries,
        &physical,
        |r, c| {
            if let Some(ref plan) = saved {
                return vec![plan.clone()];
            }
            let plans = discover(r, c, &physical, &callees, &mut BTreeSet::new());
            saved = plans.first().cloned();
            plans
        },
    );
    assert!(matches!(result, Err(MirRewriteError::StalePlan { .. })));
}

#[test]
fn argument_staging_emitted_fragments_cover_every_first_word() {
    let old = baseline();
    let new = selected(old.clone());
    for p in [old, new] {
        for name in ["ForwardAdd", "ForwardSub"] {
            let mut p = p.clone();
            let r = p.routines.iter_mut().find(|r| r.name == name).unwrap();
            let start = capture_store(r) - 5;
            // Execute the emitted arithmetic/setup fragment without SArgs.
            // Replace the callee with independent, explicit ABI-byte stores.
            r.blocks[0].ops.drain(..start);
            let call = r.blocks[0]
                .ops
                .iter()
                .position(|o| matches!(o, MirOp::Call { .. }))
                .unwrap();
            let mut observe = Vec::new();
            for (offset, reg) in [(0, MirReg::A), (1, MirReg::X), (2, MirReg::Y)] {
                observe.push(MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x800 + offset)),
                    src: MirValue::Def(MirDef::Reg(reg)),
                    width: MirWidth::Byte,
                });
            }
            for (offset, slot) in [(3, 0xA3), (4, 0xA4)] {
                observe.push(MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
                    width: MirWidth::Byte,
                });
                observe.push(MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x800 + offset)),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                });
            }
            r.blocks[0].ops.splice(call..call + 1, observe);
            let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
            let summary = crate::mir6502::emit::emit_program(&p, 0x3000, &mut emitter).unwrap();
            let output = crate::mir6502::codegen_output(
                emitter.finish_with_relocations().unwrap(),
                0x3000,
                summary,
                crate::runtime::Runtime::Standalone,
                &p,
            );
            let param = |n: &str| {
                output
                    .map
                    .storage_symbols
                    .iter()
                    .find(|s| {
                        s.scope == crate::codegen::CodegenSymbolScope::Routine(name.to_owned())
                            && s.name == n
                    })
                    .unwrap()
                    .address as usize
            };
            let first = param("first");
            let second = param("second");
            let tag = param("tag");
            let entry = output
                .routine_addresses
                .iter()
                .find(|r| r.name == name)
                .unwrap()
                .address as usize;
            let mut memory = [0u8; 65536];
            memory[0x3000..0x3000 + output.bytes.len()].copy_from_slice(&output.bytes);
            for value in 0..=u16::MAX {
                memory[first..first + 2].copy_from_slice(&value.to_le_bytes());
                memory[second..second + 2].copy_from_slice(&0x8F73u16.to_le_bytes());
                memory[tag] = 0x80;
                memory[0x805] = 0xA5;
                super::super::leaf_test_cpu::run_memory(&mut memory, entry);
                let expected = if name == "ForwardAdd" {
                    value.wrapping_add(1)
                } else {
                    value.wrapping_sub(1)
                };
                assert_eq!(
                    &memory[0x800..0x805],
                    &[expected as u8, (expected >> 8) as u8, 0x73, 0x8F, 0x80],
                    "{name}/{value:04X}"
                );
                assert_eq!(memory[0x805], 0xA5);
                assert_eq!(&memory[first..first + 2], &value.to_le_bytes());
            }
        }
    }
}
