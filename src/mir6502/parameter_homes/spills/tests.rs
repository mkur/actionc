use super::*;

fn nir() -> crate::nir::NirProgram {
    let source =
        include_str!("../../../../fixtures/runtime/cross_call_spills.act").replace("\r\n", "\n");
    let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let n = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::optimize_program(&n).unwrap()
}

fn program() -> MirProgram {
    let n = nir();
    let p = crate::mir6502::lower_program(&n).unwrap();
    crate::mir6502::materialize_program(
        p,
        &crate::mir6502::Mir6502Config {
            enable_small_leaf_inlining: false,
            ..crate::mir6502::Mir6502Config::optimized()
        },
    )
    .unwrap()
}

#[test]
fn loop_carried_byte_spills_use_preserved_free_lanes() {
    let mut p = program();
    let homes = discover(&p, true);
    assert_eq!(homes.len(), 1, "{p:#?}");
    let h = &homes[0];
    let r = p.routines.iter().find(|r| r.id == h.owner).unwrap();
    assert_eq!(r.name, "Hold");
    let mut calls = ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry);
    for call in r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .filter(|op| matches!(op, MirOp::Call { .. }))
    {
        assert!(calls.writes(call).unwrap().preserves_fixed_byte(h.fixed));
    }
    place(&mut p, true, &crate::mir6502::trace::Trace::disabled());
    crate::mir6502::verify_program(&p, MirPhase::PostMaterialization).unwrap();
    let r = p.routines.iter().find(|r| r.id == h.owner).unwrap();
    assert!(!r.frame.spills.contains(&h.spill));
    assert!(
        r.frame
            .zero_page_allocations
            .iter()
            .any(|a| a.slot == h.slot && a.start == h.fixed)
    );
}

#[test]
fn spill_placement_respects_ownership_calls_and_capacity() {
    let p = program();
    let homes = discover(&p, true);
    assert_eq!(homes.len(), 1);
    let h = &homes[0];
    let index = p.routines.iter().position(|r| r.id == h.owner).unwrap();
    let mut full = p.clone();
    full.routines[index]
        .frame
        .fixed_zero_page
        .extend((0xe0..=0xef).map(MirFixedZpSlot));
    assert!(discover(&full, true).is_empty());
    let mut exposed = p.clone();
    exposed.routines[index].blocks[0].ops.insert(
        0,
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::StorageAddrByte {
                mem: MirMem::Spill {
                    id: h.spill,
                    offset: 0,
                },
                byte: 0,
            },
            width: MirWidth::Byte,
        },
    );
    assert!(discover(&exposed, true).is_empty());
    let mut unknown = p.clone();
    for op in unknown.routines[index]
        .blocks
        .iter_mut()
        .flat_map(|b| &mut b.ops)
    {
        if let MirOp::Call { target, .. } = op {
            *target = MirCallTarget::Routine(RoutineId(999));
        }
    }
    assert!(discover(&unknown, true).is_empty());
    let mut clobbered = p.clone();
    let callee = clobbered
        .routines
        .iter_mut()
        .find(|r| r.name == "TouchByte")
        .unwrap();
    for byte in 0xe0..=0xef {
        callee.blocks[0].ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(byte)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        );
    }
    assert!(discover(&clobbered, true).is_empty());
}

fn executable(p: MirProgram) -> Executable {
    crate::mir6502::finalize_output(
        &nir(),
        p,
        0x3000,
        crate::runtime::Runtime::ActionCart,
        &crate::mir6502::trace::Trace::disabled(),
    )
    .unwrap()
}

#[test]
fn final_spill_proof_rechecks_callees_scratch_and_numeric_observers() {
    let p = program();
    let old = executable(p.clone());
    let mut new = p;
    let homes = place(&mut new, true, &crate::mir6502::trace::Trace::disabled());
    assert_eq!(homes.len(), 1);
    assert!(verify(&homes, &old, &executable(new.clone()), &old.program, &old).is_ok());
    let h = &homes[0];
    let previous = old
        .memory
        .address(
            h.owner,
            &MirMem::Spill {
                id: h.spill,
                offset: 0,
            },
        )
        .unwrap();
    for (callee, numeric, byte) in [
        (true, false, h.fixed.0 as u16),
        (false, false, h.fixed.0 as u16),
        (false, true, previous),
    ] {
        // Keep the measured layout, then change the expanded effects. This
        // directly tests residence replay, independently of frame validation.
        let mut bad = executable(new.clone());
        let r = bad
            .program
            .routines
            .iter_mut()
            .find(|r| {
                if callee {
                    r.name == "TouchByte"
                } else {
                    r.id == h.owner
                }
            })
            .unwrap();
        r.blocks[0].ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(if numeric {
                    MirMem::Absolute(byte)
                } else {
                    MirMem::FixedZeroPage(MirFixedZpSlot(byte as u8))
                }),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        );
        assert!(
            verify(&homes, &old, &bad, &old.program, &old).is_err(),
            "callee={callee} numeric={numeric}"
        );
    }
}

#[test]
fn multiple_leaf_gate_and_entry_initialization_apply_to_spills() {
    let mut p = program();
    let h = discover(&p, true).remove(0);
    let r = p.routines.iter_mut().find(|r| r.id == h.owner).unwrap();
    for b in &mut r.blocks {
        if let Some(index) = b.ops.iter().position(|op| matches!(op, MirOp::Call { .. })) {
            b.ops.insert(index, b.ops[index].clone());
            break;
        }
    }
    assert!(discover(&p, false).is_empty());
    assert_eq!(discover(&p, true).len(), 1);
    let r = p.routines.iter_mut().find(|r| r.id == h.owner).unwrap();
    r.blocks[0].ops.retain(|op| {
        !classify_op(op).homes.writes.contains(&MirHomeByte::Spill {
            id: h.spill,
            offset: 0,
        })
    });
    assert!(discover(&p, true).is_empty());
}
