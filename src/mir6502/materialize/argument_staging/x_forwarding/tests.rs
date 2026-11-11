use super::*;
use crate::mir6502::analysis::argument_x_forwarding::tests::{baseline, memory, site};
use crate::mir6502::rewrite::driver::MirRewriteError;
fn selected(p: MirProgram) -> MirProgram {
    select_costed(
        p,
        &Mir6502Config {
            enable_argument_x_bitwise_forwarding: false,
            enable_precise_callee_entry_flags: false,
            enable_argument_x_forwarding: true,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap()
}
fn ids(r: &MirRoutine) -> BTreeMap<MirBlockId, Vec<usize>> {
    r.blocks
        .iter()
        .map(|b| (b.id, (0..b.ops.len()).collect()))
        .collect()
}
#[test]
fn argument_x_forwarding_selection_saves_four_bytes_without_removing_stores_or_ops() {
    let old = baseline();
    let m = memory(&old);
    let (new, stats, affected) = apply(&old, &m, None).unwrap();
    assert_eq!(affected.len(), 3);
    assert_eq!(selected(old.clone()), new);
    assert_eq!(selected(new.clone()), new);
    assert_eq!(
        image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        4
    );
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        assert_eq!(
            a.blocks.iter().map(|b| b.ops.len()).sum::<usize>(),
            b.blocks.iter().map(|b| b.ops.len()).sum::<usize>()
        );
        let stores = |r: &MirRoutine| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|o| matches!(o, MirOp::Store { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(stores(a), stores(b));
        if affected.contains(&a.id) {
            assert_eq!(stats.count_for(a.id, "argument-x-forwarding-selected"), 1);
        } else {
            assert_eq!(a, b);
        }
    }
    let report = select_costed(
        old,
        &Mir6502Config {
            enable_argument_x_bitwise_forwarding: false,
            enable_precise_callee_entry_flags: false,
            enable_argument_x_forwarding: true,
            peephole_report: crate::mir6502::MirPeepholeReportMode::Sites,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap();
    assert_eq!(report, new);
}
#[test]
fn argument_x_forwarding_driver_rejects_missing_altered_and_stale_witnesses() {
    let p = baseline();
    let m = memory(&p);
    let r = p
        .routines
        .iter()
        .find(|r| r.name == "Forward")
        .unwrap()
        .clone();
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
        let mut bad = plan.clone();
        match change {
            0 => bad.witness = None,
            1 => bad.replacement.swap(0, 1),
            _ => {
                if let Some(MirPostHomeWitness::ArgumentXForwarding(p)) = bad.witness.as_mut() {
                    p.address += 1;
                }
            }
        }
        let result = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
            &mut r.clone(),
            &summaries,
            &m,
            |_, _| vec![bad.clone()],
        );
        assert!(
            matches!(result, Err(MirRewriteError::InvalidDeclaration { .. })),
            "{result:?}"
        );
    }
    let result = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
        &mut r.clone(),
        &summaries,
        &m,
        |_, _| vec![plan.clone()],
    );
    assert!(matches!(result, Err(MirRewriteError::StalePlan { .. })));
    let result = MirPostHomeRewriteDriver::default()
        .run_fixed_point(&mut r.clone(), |_, _| vec![plan.clone()]);
    assert!(matches!(
        result,
        Err(MirRewriteError::InvalidDeclaration { .. })
    ));
}
#[test]
fn argument_x_forwarding_keeps_unqualified_flags_and_selects_independent_routines() {
    let mut p = baseline();
    let r = p
        .routines
        .iter_mut()
        .find(|r| r.name == "ForwardByte")
        .unwrap();
    let s = facts::shape(r, site(r)).unwrap();
    let MirSite::Op { op_index, .. } = s.call else {
        unreachable!()
    };
    if let MirOp::Call { effects, .. } = &mut r.blocks[0].ops[op_index] {
        effects.reads.flags = true;
    }
    let unchanged = r.clone();
    let new = selected(p.clone());
    assert_eq!(
        new.routines.iter().find(|r| r.id == unchanged.id).unwrap(),
        &unchanged
    );
    assert_eq!(
        image_size(&p, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        2
    );
}
#[test]
fn argument_x_forwarding_unknown_cost_keeps_the_entire_baseline() {
    let mut p = baseline();
    let r = p.routines.iter_mut().find(|r| r.name == "Plain").unwrap();
    let call = r.blocks[0]
        .ops
        .iter_mut()
        .find(|o| matches!(o, MirOp::Call { .. }))
        .unwrap();
    if let MirOp::Call { target, .. } = call {
        *target = MirCallTarget::Routine(RoutineId(u32::MAX));
    }
    assert_eq!(selected(p.clone()), p);
}

#[test]
fn argument_x_forwarding_checks_budget_tail_limits_and_implicit_x_inputs() {
    let p = baseline();
    let m = memory(&p);
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let base = p.routines.iter().find(|r| r.name == "Forward").unwrap();
    let s = facts::shape(base, site(base)).unwrap();
    let i = s.range().start;
    let check = |r: &MirRoutine| {
        let snap =
            crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot::new_with_known_callees(
                r,
                crate::mir6502::analysis::sites::MirRoutineGeneration::initial(),
                &summaries,
            )
            .unwrap();
        let c = PostHomeRewriteContext::new(&snap);
        c.argument_x_forwarding(c.point(s.store), &m)
    };
    let MirSite::Op { op_index: call, .. } = s.call else {
        unreachable!()
    };
    let n = facts::MAX_REGION - (call - i + 1);
    for (extra, allowed) in [(0, true), (1, false)] {
        let mut r = base.clone();
        r.blocks[0].ops.splice(
            i + 1..i + 1,
            std::iter::repeat_n(
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 0,
                    width: MirWidth::Byte,
                },
                n + extra,
            ),
        );
        assert_eq!(check(&r).is_ok(), allowed);
    }
    let mut r = base.clone();
    r.blocks[0].ops.splice(
        s.range().end..s.range().end,
        std::iter::repeat_n(
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::Y),
                value: 0,
                width: MirWidth::Byte,
            },
            facts::MAX_TAIL,
        ),
    );
    assert!(check(&r).is_err());
    let mut r = base.clone();
    r.blocks[0].ops.insert(
        i + 1,
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::AbsoluteIndexedX {
                base: MirMem::Absolute(0x7000),
            },
            width: MirWidth::Byte,
        },
    );
    assert_eq!(check(&r), Err(facts::Blocker::XRead));
    let mut r = base.clone();
    let indirect = (i + 1..s.range().end - 1)
        .find(|&n| matches!(r.blocks[0].ops[n], MirOp::LoadIndirect { .. }))
        .unwrap();
    if let MirOp::LoadIndirect { consumer, .. } = &mut r.blocks[0].ops[indirect] {
        *consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(255),
        });
    }
    // Unsupported address-consumer forms remain conservative even when no X
    // value appears as an explicit operand.
    assert!(check(&r).is_err());
}

fn fragment(enabled: bool, origin: u16) -> (Vec<u8>, u16) {
    let mut p = baseline();
    let r = p.routines.iter_mut().find(|r| r.name == "Forward").unwrap();
    let s = facts::shape(r, site(r)).unwrap();
    let range = s.range();
    let mut ops = if enabled {
        facts::replacement(r, &s).unwrap()
    } else {
        r.blocks[0].ops[range].to_vec()
    };
    let MirSite::Op { op_index: call, .. } = s.call else {
        unreachable!()
    };
    ops.extend_from_slice(&r.blocks[0].ops[s.range().end..call]);
    r.blocks[0].ops = ops;
    r.blocks[0].terminator = MirTerminator::Return;
    let mut e = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = crate::mir6502::emit::emit_program(&p, origin, &mut e).unwrap();
    let output = crate::mir6502::codegen_output(
        e.finish_with_relocations().unwrap(),
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
    bytes.extend([
        0x8D, 0, 8, 0x8E, 1, 8, 0x8C, 2, 8, 0x08, 0x68, 0x8D, 3, 8, 0xBA, 0x8E, 4, 8, 0x60,
    ]);
    (bytes, range.start)
}
#[test]
fn argument_x_forwarding_emitted_byte_oracle_preserves_state_and_aliased_reads() {
    use crate::mir6502::materialize::leaf_test_cpu;
    let p = baseline();
    let r = p.routines.iter().find(|r| r.name == "Forward").unwrap();
    let m = memory(&p);
    let q = facts::shape(r, site(r)).unwrap();
    let source = if let MirOp::Load {
        src: MirAddr::Direct(mem),
        ..
    } = &r.blocks[0].ops[q.range().start + 1]
    {
        mem.clone()
    } else {
        panic!()
    };
    let source_address = m.address(r.id, &source).unwrap();
    let mut executions = 0;
    for origin in [0x3000u16, 0x41C7] {
        let old = fragment(false, origin);
        let new = fragment(true, origin);
        assert_eq!(old.0.len() - new.0.len(), 1);
        // Parameters precede code and have the same origin-relative positions.
        let source = source_address + (origin - 0x3000);
        for forwarded in 0..=255u16 {
            for pointer in [
                0x7200u16,
                0x72FF,
                0x7FFF,
                0xFFFF,
                0xA0,
                0xA1,
                0xA2,
                0xA3,
                0xAC,
                0xAD,
                source,
                source + 1,
            ] {
                for carry in [false, true] {
                    let mut states = Vec::new();
                    for (bytes, entry) in [&old, &new] {
                        let mut ram = [0xD6u8; 65536];
                        let at = usize::from(origin);
                        ram[at..at + bytes.len()].copy_from_slice(bytes);
                        ram[usize::from(pointer)] = 0x80;
                        ram[usize::from(source)..usize::from(source) + 2]
                            .copy_from_slice(&pointer.to_le_bytes());
                        ram[0xA0] = 0x37;
                        // The original captured high byte is the supplied accumulator.
                        ram[0x700..0x70A].copy_from_slice(&[
                            0xA9,
                            forwarded as u8,
                            0xA2,
                            0x5A,
                            0xA0,
                            0x7F,
                            if carry { 0x38 } else { 0x18 },
                            0x4C,
                            *entry as u8,
                            (*entry >> 8) as u8,
                        ]);
                        ram[0x7FF] = 0x5A;
                        ram[0x805] = 0xA5;
                        let next = pointer.wrapping_add(1).to_le_bytes();
                        let tag = match pointer {
                            0xA0 => 0x37,
                            0xA1 => forwarded as u8,
                            0xA2 => next[0],
                            0xA3 => next[1],
                            0xAC => pointer as u8,
                            0xAD => (pointer >> 8) as u8,
                            a if a == source => pointer as u8,
                            a if a == source + 1 => (pointer >> 8) as u8,
                            _ => 0x80,
                        };
                        leaf_test_cpu::run_memory(&mut ram, 0x700);
                        assert_eq!(&ram[0x800..0x803], &[0x37, forwarded as u8, next[0]]);
                        let flags = 0x30
                            | u8::from(pointer == 0xFFFF)
                            | (u8::from(next[0] == 0) << 1)
                            | (u8::from(pointer == 0x7FFF) << 6)
                            | (next[0] & 0x80);
                        assert_eq!(ram[0x803], flags);
                        assert_eq!(ram[0x804], 0xFF);
                        assert_eq!(ram[0xA1], forwarded as u8);
                        assert_eq!(ram[0xA4], tag);
                        assert_eq!(ram[0x7FF], 0x5A);
                        assert_eq!(ram[0x805], 0xA5);
                        states.push((ram[0x800..0x805].to_vec(), ram[0xA0..=0xAF].to_vec()));
                        executions += 1;
                    }
                    assert_eq!(states[0], states[1]);
                }
            }
        }
    }
    assert_eq!(executions, 24576);
}
