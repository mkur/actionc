use super::*;
use crate::mir6502::analysis::pointer_load_sharing::tests::{baseline, forward, memory, start};
use crate::mir6502::rewrite::driver::MirRewriteError;

fn selected(p: MirProgram) -> MirProgram {
    select_costed(
        p,
        &Mir6502Config {
            enable_pointer_load_sharing: true,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap()
}
fn ids(r: &crate::mir6502::ir::MirRoutine) -> BTreeMap<MirBlockId, Vec<usize>> {
    r.blocks
        .iter()
        .map(|b| (b.id, (0..b.ops.len()).collect()))
        .collect()
}

#[test]
fn pointer_load_sharing_selection_saves_six_bytes_and_is_idempotent() {
    let old = baseline();
    let m = memory(&old);
    let (proposed, stats, affected) = apply(&old, &m, None).unwrap();
    assert_eq!(affected.len(), 1);
    let id = old
        .routines
        .iter()
        .find(|r| r.name == "Forward")
        .unwrap()
        .id;
    assert_eq!(stats.count_for(id, "pointer-load-sharing-candidate"), 1);
    assert_eq!(stats.count_for(id, "pointer-load-sharing-selected"), 1);
    let new = selected(old.clone());
    assert_eq!(new, proposed);
    assert_eq!(selected(new.clone()), new);
    assert_eq!(
        image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        6
    );
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        if a.name != "Forward" {
            assert_eq!(a, b);
        }
    }
    let reported = select_costed(
        old,
        &Mir6502Config {
            enable_pointer_load_sharing: true,
            peephole_report: crate::mir6502::MirPeepholeReportMode::Sites,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap();
    assert_eq!(reported, new);
}

#[test]
fn pointer_load_sharing_driver_rejects_missing_changed_and_stale_proofs() {
    let mut p = baseline();
    let m = memory(&p);
    let r = forward(&mut p).clone();
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let mut saved = None;
    MirPostHomeRewriteDriver::default()
        .run_fixed_point_with_physical_memory(&mut r.clone(), &summaries, &m, |r, c| {
            if saved.is_some() {
                return vec![];
            }
            let plans = discover(r, c, &m, &ids(r), &mut BTreeSet::new());
            saved = plans.first().cloned();
            plans
        })
        .unwrap();
    let plan = saved.unwrap();
    for change in 0..3 {
        let mut wrong = plan.clone();
        match change {
            0 => wrong.pointer_load_sharing = None,
            1 => wrong.replacement.swap(1, 3),
            _ => wrong.pointer_load_sharing.as_mut().unwrap().source_address += 1,
        }
        let error = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
            &mut r.clone(),
            &summaries,
            &m,
            |_, _| vec![wrong.clone()],
        );
        assert!(
            matches!(error, Err(MirRewriteError::InvalidDeclaration { .. })),
            "{error:?}"
        );
    }
    let no_map = MirPostHomeRewriteDriver::default()
        .run_fixed_point(&mut r.clone(), |_, _| vec![plan.clone()]);
    assert!(matches!(
        no_map,
        Err(MirRewriteError::InvalidDeclaration { .. })
    ));
    let stale = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
        &mut r.clone(),
        &summaries,
        &m,
        |_, _| vec![plan.clone()],
    );
    assert!(matches!(stale, Err(MirRewriteError::StalePlan { .. })));
}

#[test]
fn pointer_load_sharing_selection_rebuilds_neighbors_and_keeps_blocked_routines() {
    let mut old = baseline();
    let r = forward(&mut old);
    let i = start(r);
    let window = r.blocks[0].ops[i..i + facts::WINDOW].to_vec();
    r.blocks[0]
        .ops
        .splice(i + facts::WINDOW..i + facts::WINDOW, window);
    let good = r.clone();
    let blocked = old
        .routines
        .iter_mut()
        .find(|r| r.name == "PlusZero")
        .unwrap();
    blocked.blocks = good.blocks.clone();
    let i = start(blocked);
    for (n, slot) in [(2, 0xAC), (5, 0xAD)] {
        if let MirOp::Store { dst, .. } = &mut blocked.blocks[0].ops[i + n] {
            *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot)));
        }
    }
    // Both exact windows in this routine must remain blocked.
    let j = i + facts::WINDOW;
    for (n, slot) in [(2, 0xAC), (5, 0xAD)] {
        if let MirOp::Store { dst, .. } = &mut blocked.blocks[0].ops[j + n] {
            *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot)));
        }
    }
    let blocked = blocked.clone();
    let m = memory(&old);
    let (proposed, stats, affected) = apply(&old, &m, None).unwrap();
    assert_eq!(affected, BTreeSet::from([good.id]));
    assert_eq!(
        stats.count_for(good.id, "pointer-load-sharing-candidate"),
        2
    );
    assert_eq!(stats.count_for(good.id, "pointer-load-sharing-selected"), 2);
    assert_eq!(
        proposed
            .routines
            .iter()
            .find(|r| r.id == blocked.id)
            .unwrap(),
        &blocked
    );
    assert_eq!(selected(old.clone()), proposed);
    assert_eq!(
        image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&proposed, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        12
    );
}

#[test]
fn pointer_load_sharing_unknown_emission_cost_restores_the_baseline() {
    let mut old = baseline();
    let call = old
        .routines
        .iter_mut()
        .find(|r| r.name == "PlusTwo")
        .unwrap()
        .blocks[0]
        .ops
        .iter_mut()
        .find(|op| matches!(op, MirOp::Call { .. }))
        .unwrap();
    if let MirOp::Call { target, .. } = call {
        *target = MirCallTarget::Routine(RoutineId(u32::MAX));
    }
    assert!(image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).is_none());
    assert_eq!(selected(old.clone()), old);
}

fn fragment(enabled: bool, origin: u16) -> (Vec<u8>, u16, u16) {
    let mut p = baseline();
    let r = forward(&mut p);
    let i = start(r);
    let ops = r.blocks[0].ops[i..i + facts::WINDOW].to_vec();
    let source = facts::shape(&ops).unwrap().source[0].clone();
    let id = r.id;
    r.blocks[0].ops = if enabled {
        facts::replacement(&ops).unwrap()
    } else {
        ops
    };
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = crate::mir6502::emit::emit_program_with_physical_references(
        &p,
        origin,
        &mut emitter,
        true,
        &p,
    )
    .unwrap();
    let source = summary.physical_memory.address(id, &source).unwrap();
    let output = crate::mir6502::codegen_output(
        emitter.finish_with_relocations().unwrap(),
        origin,
        summary,
        crate::compiler::Runtime::ActionCart,
        &p,
    );
    let range = output
        .map
        .routine_ranges
        .iter()
        .find(|r| r.name == "Forward")
        .unwrap();
    let mut bytes = output.bytes[..usize::from(range.end - origin - 1)].to_vec();
    assert_eq!(output.bytes[bytes.len()], 0x60);
    // Capture registers before PHP/PLA/TSX alter their flags or carriers.
    bytes.extend([
        0x8D, 0, 8, 0x8E, 1, 8, 0x8C, 2, 8, 0x08, 0x68, 0x8D, 3, 8, 0xBA, 0x8E, 4, 8, 0x60,
    ]);
    (bytes, range.start, source)
}

#[test]
fn pointer_load_sharing_emitted_sequences_preserve_complete_exit_state_and_aliases() {
    use crate::mir6502::materialize::leaf_test_cpu;
    let mut executions = 0;
    for origin in [0x3000u16, 0x41C7] {
        let before = fragment(false, origin);
        let after = fragment(true, origin);
        assert_eq!(before.0.len() - after.0.len(), 6);
        assert_eq!(before.2, after.2);
        let source = before.2;
        let pointers = (0..=255u16)
            .map(|low| 0x7000 + low)
            .chain([
                0,
                1,
                0xFF,
                0x100,
                0x7FFF,
                0x8000,
                0xFF00,
                0xFFFF,
                0xA0,
                0xA2,
                0xA3,
                0xAC,
                0xAD,
                0xAE,
                0xAF,
                source,
                source + 1,
            ])
            .collect::<Vec<_>>();
        for (bytes, entry, _) in [before, after] {
            for &q in &pointers {
                for value in [0u8, 1, 127, 128, 254, 255] {
                    for carry in [false, true] {
                        let mut ram = [0xD6u8; 65536];
                        let at = usize::from(origin);
                        ram[at..at + bytes.len()].copy_from_slice(&bytes);
                        ram[usize::from(q)] = value;
                        ram[usize::from(source)..usize::from(source) + 2]
                            .copy_from_slice(&q.to_le_bytes());
                        ram[0x7FF] = 0x5A;
                        ram[0x805] = 0xA5;
                        ram[0x700..0x70A].copy_from_slice(&[
                            0xA9,
                            0x55,
                            0xA2,
                            0xAA,
                            0xA0,
                            0x7F,
                            if carry { 0x38 } else { 0x18 },
                            0x4C,
                            entry as u8,
                            (entry >> 8) as u8,
                        ]);
                        let capture = q.wrapping_add(1).to_le_bytes();
                        let pointer = q.to_le_bytes();
                        let byte = if q == 0xA2 {
                            capture[0]
                        } else if q == 0xA3 {
                            capture[1]
                        } else if q == 0xAC || q == source {
                            pointer[0]
                        } else if q == 0xAD || q == source + 1 {
                            pointer[1]
                        } else {
                            value
                        };
                        let flags = 0x30
                            | u8::from(q == 0xFFFF)
                            | (u8::from(byte == 0) << 1)
                            | (u8::from(q == 0x7FFF) << 6)
                            | (byte & 0x80);
                        leaf_test_cpu::run_memory(&mut ram, 0x700);
                        assert_eq!(
                            &ram[0x800..0x805],
                            &[byte, 0xAA, 0, flags, 0xFF],
                            "{origin:04X}/{q:04X}/{value}/{carry}"
                        );
                        assert_eq!(&ram[0xA2..=0xA3], &capture);
                        assert_eq!(&ram[0xAC..=0xAD], &pointer);
                        assert_eq!(&ram[usize::from(source)..usize::from(source) + 2], &pointer);
                        assert_eq!(ram[0x7FF], 0x5A);
                        assert_eq!(ram[0x805], 0xA5);
                        executions += 1;
                    }
                }
            }
        }
    }
    assert_eq!(executions, 13104);
}

#[test]
fn pointer_load_sharing_proof_rejects_pair_aliases_and_accepts_live_exit_users() {
    use crate::mir6502::analysis::pointer_load_sharing::tests::fact;
    let mut p = baseline();
    let m = memory(&p);
    let r = forward(&mut p).clone();
    let i = start(&r);
    for (lo, hi) in [(0xAB, 0xAC), (0xAC, 0xAD), (0xAD, 0xAE)] {
        let mut bad = r.clone();
        for (index, slot) in [(2, lo), (5, hi)] {
            if let MirOp::Store { dst, .. } = &mut bad.blocks[0].ops[i + index] {
                *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot)));
            }
        }
        assert_eq!(fact(&bad, &m), Err(facts::Blocker::Overlap));
    }
    let mut live = r.clone();
    for (n, reg) in [MirReg::A, MirReg::X, MirReg::Y].into_iter().enumerate() {
        live.blocks[0].ops.insert(
            i + facts::WINDOW + n,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x686 + n as u16)),
                src: MirValue::Def(MirDef::Reg(reg)),
                width: MirWidth::Byte,
            },
        );
    }
    assert!(fact(&live, &m).is_ok());
    for consumer in [
        MirAddressConsumer::LiveIndirectIndexedY(INDEX_POINTER_PAIR.pointer_pair()),
        MirAddressConsumer::ScaledIndirectIndexedY(INDEX_POINTER_PAIR.pointer_pair()),
        MirAddressConsumer::PagedIndirectIndexedY(INDEX_POINTER_PAIR.pointer_pair()),
    ] {
        let mut bad = r.clone();
        if let MirOp::LoadIndirect { consumer: c, .. } = &mut bad.blocks[0].ops[i + 10] {
            *c = consumer;
        }
        assert!(facts::shape(&bad.blocks[0].ops[i..i + facts::WINDOW]).is_err());
    }
    for op in [
        MirOp::Barrier {
            effects: MirEffects::default(),
        },
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 0,
            width: MirWidth::Byte,
        },
    ] {
        let mut bad = r.clone();
        bad.blocks[0].ops.insert(i + 6, op);
        assert!(
            !bad.blocks[0]
                .ops
                .windows(facts::WINDOW)
                .any(|ops| facts::shape(ops).is_ok())
        );
    }
    for offset in [1, 255, 256] {
        let mut bad = r.clone();
        if let MirOp::LoadIndirect { offset: k, .. } = &mut bad.blocks[0].ops[i + 10] {
            *k = offset;
        }
        assert!(facts::shape(&bad.blocks[0].ops[i..i + facts::WINDOW]).is_err());
    }
}
