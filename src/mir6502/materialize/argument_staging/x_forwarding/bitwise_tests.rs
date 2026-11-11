use super::*;
use crate::mir6502::analysis::argument_x_forwarding::{
    bitwise_tests::baseline,
    tests::{memory, site},
};
fn config() -> Mir6502Config {
    Mir6502Config {
        enable_argument_x_bitwise_forwarding: true,
        ..Mir6502Config::optimized()
    }
}
#[test]
fn argument_x_bitwise_selection_preserves_stores_frames_and_reports() {
    let old = baseline();
    let new = select_costed(
        old.clone(),
        &config(),
        0x3000,
        crate::runtime::Runtime::ActionCart,
    )
    .unwrap();
    assert_eq!(
        image_size(&old, 0x3000, crate::runtime::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::runtime::Runtime::ActionCart).unwrap(),
        3
    );
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
        assert_eq!(stores(a), stores(b));
        assert_eq!(
            a.blocks.iter().map(|b| b.ops.len()).sum::<usize>(),
            b.blocks.iter().map(|b| b.ops.len()).sum::<usize>()
        );
        assert_eq!(a == b, !a.name.starts_with("Forward"));
    }
    assert_eq!(
        select_costed(
            new.clone(),
            &config(),
            0x3000,
            crate::runtime::Runtime::ActionCart
        )
        .unwrap(),
        new
    );
    assert_eq!(
        select_costed(
            old,
            &Mir6502Config {
                peephole_report: crate::mir6502::MirPeepholeReportMode::Sites,
                ..config()
            },
            0x3000,
            crate::runtime::Runtime::ActionCart
        )
        .unwrap(),
        new
    );
}
#[test]
fn argument_x_bitwise_proof_keeps_x_alias_and_tail_rejections() {
    let p = baseline();
    let m = memory(&p);
    let sums = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let base = p.routines.iter().find(|r| r.name == "ForwardAnd").unwrap();
    let shape = facts::shape(base, site(base)).unwrap();
    let start = shape.range().start;
    let check = |r: &MirRoutine| {
        let snap =
            crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot::new_with_known_callees(
                r,
                crate::mir6502::analysis::sites::MirRoutineGeneration::initial(),
                &sums,
            )
            .unwrap();
        let c = PostHomeRewriteContext::new(&snap);
        c.argument_x_forwarding_with_policy(
            c.point(shape.store),
            &m,
            facts::GapPolicy::ImmediateBitwise,
        )
    };
    assert!(check(base).is_ok());
    for change in 0..5 {
        let mut r = base.clone();
        match change {
            0 => r.blocks[0].ops.insert(
                start + 1,
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
                    src: MirValue::Def(MirDef::Reg(MirReg::X)),
                    width: MirWidth::Byte,
                },
            ),
            1 => r.blocks[0].ops.insert(
                start + 1,
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::X),
                    value: 0,
                    width: MirWidth::Byte,
                },
            ),
            2 => {
                let copy = r.blocks[0].ops[start].clone();
                r.blocks[0].ops.insert(start + 1, copy);
            }
            3 => {
                let i = r.blocks[0]
                    .ops
                    .iter()
                    .position(facts::immediate_bitwise)
                    .unwrap();
                let op = r.blocks[0].ops[i].clone();
                r.blocks[0].ops[shape.range().end] = op;
            }
            _ => {
                let i = r.blocks[0]
                    .ops
                    .iter()
                    .position(facts::immediate_bitwise)
                    .unwrap();
                if let MirOp::Binary { ref mut right, .. } = r.blocks[0].ops[i] {
                    *right = MirValue::Def(MirDef::Reg(MirReg::Y));
                }
            }
        };
        assert!(check(&r).is_err(), "case {change}");
    }
}
fn fragment(op: MirBinaryOp, mask: u8, enabled: bool, origin: u16) -> (Vec<u8>, u16, u16) {
    let mut p = baseline();
    let r = p
        .routines
        .iter_mut()
        .find(|r| r.name == "ForwardAnd")
        .unwrap();
    let s = facts::shape(r, site(r)).unwrap();
    let i = r.blocks[0]
        .ops
        .iter()
        .position(facts::immediate_bitwise)
        .unwrap();
    if let MirOp::Binary {
        op: ref mut operation,
        right: ref mut value,
        ..
    } = r.blocks[0].ops[i]
    {
        *operation = op;
        *value = MirValue::ConstU8(mask);
    }
    let mut ops = if enabled {
        facts::replacement(r, &s).unwrap()
    } else {
        r.blocks[0].ops[s.range()].to_vec()
    };
    let MirSite::Op { op_index: call, .. } = s.call else {
        unreachable!()
    };
    ops.extend_from_slice(&r.blocks[0].ops[s.range().end..call]);
    let low = match &r.blocks[0].ops[s.range().end - 2] {
        MirOp::Load {
            src: MirAddr::Direct(m),
            ..
        } => m.clone(),
        _ => panic!(),
    };
    ops.insert(
        0,
        MirOp::Store {
            dst: MirAddr::Direct(low),
            src: MirValue::Def(MirDef::Reg(MirReg::Y)),
            width: MirWidth::Byte,
        },
    );
    r.blocks[0].ops = ops;
    r.blocks[0].terminator = MirTerminator::Return;
    let mut e = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = crate::mir6502::emit::emit_program(&p, origin, &mut e).unwrap();
    let out = crate::mir6502::codegen_output(
        e.finish_with_relocations().unwrap(),
        origin,
        summary,
        crate::runtime::Runtime::ActionCart,
        &p,
    );
    let range = out
        .map
        .routine_ranges
        .iter()
        .find(|r| r.name == "ForwardAnd")
        .unwrap();
    let tag = out
        .map
        .storage_symbols
        .iter()
        .find(|s| {
            s.name == "tag"
                && s.scope == crate::codegen::CodegenSymbolScope::Routine("ForwardAnd".into())
        })
        .unwrap()
        .address;
    let at = usize::from(range.start - origin) + 7 + usize::from(enabled);
    assert_eq!(
        &out.bytes[at..at + 2],
        &[
            match op {
                MirBinaryOp::And => 0x29,
                MirBinaryOp::Or => 0x09,
                _ => 0x49,
            },
            mask
        ]
    );
    let mut bytes = out.bytes[..usize::from(range.end - origin - 1)].to_vec();
    assert_eq!(out.bytes[bytes.len()], 0x60);
    bytes.extend([
        0x8D, 0, 8, 0x8E, 1, 8, 0x8C, 2, 8, 0x08, 0x68, 0x8D, 3, 8, 0xBA, 0x8E, 4, 8, 0x60,
    ]);
    (bytes, range.start, tag)
}
#[test]
fn argument_x_bitwise_emitted_oracle_sweeps_values_masks_carry_and_overflow() {
    let mut count = 0;
    for origin in [0x3000u16, 0x41C7] {
        for op in [MirBinaryOp::And, MirBinaryOp::Or, MirBinaryOp::Xor] {
            for mask in [0u8, 1, 127, 128, 254, 255] {
                let old = fragment(op, mask, false, origin);
                let new = fragment(op, mask, true, origin);
                assert_eq!(old.0.len() - new.0.len(), 1);
                for input in 0..=255u8 {
                    for cv in [0u8, 1, 0x40, 0x41] {
                        let forwarded = input.rotate_left(1) ^ 0xA5;
                        let expected = match op {
                            MirBinaryOp::And => input & mask,
                            MirBinaryOp::Or => input | mask,
                            _ => input ^ mask,
                        };
                        let mut observations = Vec::new();
                        for (bytes, entry, tag) in [&old, &new] {
                            let mut ram = [0xD6u8; 65536];
                            let at = usize::from(origin);
                            ram[at..at + bytes.len()].copy_from_slice(bytes);
                            ram[usize::from(*tag)] = input;
                            ram[0xE0] = 0x37;
                            ram[0x7FF] = 0x5A;
                            ram[0x805] = 0xA5;
                            ram[0x700..0x70D].copy_from_slice(&[
                                0xA9,
                                0x20 | cv,
                                0x48,
                                0xA9,
                                forwarded,
                                0xA2,
                                0x5A,
                                0xA0,
                                0x37,
                                0x28,
                                0x4C,
                                *entry as u8,
                                (*entry >> 8) as u8,
                            ]);
                            crate::mir6502::materialize::leaf_test_cpu::run_memory(&mut ram, 0x700);
                            assert_eq!(
                                &ram[0x800..0x805],
                                &[
                                    0x37,
                                    forwarded,
                                    expected,
                                    0x30 | cv | (expected & 0x80) | u8::from(expected == 0) * 2,
                                    0xFF
                                ]
                            );
                            assert_eq!(&ram[0xE0..=0xE2], &[0x37, forwarded, expected]);
                            assert_eq!(ram[0x7FF], 0x5A);
                            assert_eq!(ram[0x805], 0xA5);
                            observations.push(ram[0x800..0x805].to_vec());
                            count += 1;
                        }
                        assert_eq!(observations[0], observations[1]);
                    }
                }
            }
        }
    }
    assert_eq!(count, 73728);
}

#[test]
fn argument_x_bitwise_driver_rechecks_the_gap_policy_and_replacement() {
    let p = baseline();
    let m = memory(&p);
    let sums = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let original = p.routines.iter().find(|r| r.name == "ForwardAnd").unwrap();
    let mut saved = None;
    MirPostHomeRewriteDriver::default()
        .run_fixed_point_with_physical_memory(&mut original.clone(), &sums, &m, |r, c| {
            if saved.is_some() {
                return vec![];
            }
            let ids = r
                .blocks
                .iter()
                .map(|b| (b.id, (0..b.ops.len()).collect::<Vec<_>>()))
                .collect();
            let plans = discover_with_policy(
                r,
                c,
                &m,
                &ids,
                &mut BTreeSet::new(),
                facts::GapPolicy::ImmediateBitwise,
            );
            saved = plans.first().cloned();
            plans
        })
        .unwrap();
    let plan = saved.unwrap();
    for change in 0..3 {
        let mut bad = plan.clone();
        match change {
            0 => bad.witness = None,
            1 => {
                let Some(MirPostHomeWitness::ArgumentXForwarding(ref mut p)) = bad.witness else {
                    panic!()
                };
                p.gap_policy = facts::GapPolicy::Basic;
            }
            _ => bad.replacement.swap(0, 1),
        }
        assert!(matches!(
            MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
                &mut original.clone(),
                &sums,
                &m,
                |_, _| vec![bad.clone()]
            ),
            Err(crate::mir6502::rewrite::driver::MirRewriteError::InvalidDeclaration { .. })
        ));
    }
}
