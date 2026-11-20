use super::*;

fn nir() -> crate::nir::NirProgram {
    nir_with_caller("")
}

fn nir_with_caller(caller: &str) -> crate::nir::NirProgram {
    let source =
        include_str!("../../../../fixtures/runtime/cross_call_spills.act").replace("\r\n", "\n") + caller;
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
fn spill_shares_a_source_home_in_an_unrelated_routine() {
    let n = nir_with_caller("\nBYTE FUNC Outer(BYTE keep) TouchByte(keep) RETURN(keep)");
    let input = crate::mir6502::lower_program(&n).unwrap();
    let mut p = crate::mir6502::materialize_program(input, &crate::mir6502::Mir6502Config {
        enable_small_leaf_inlining: false, ..crate::mir6502::Mir6502Config::optimized()
    }).unwrap();
    let first = discover(&p, true).into_iter().find(|h|
        p.routines.iter().any(|r| r.id == h.owner && r.name == "Hold")).unwrap();
    let outer = p.routines.iter_mut().find(|r| r.name == "Outer").unwrap();
    let MirStorageBase::Param(param) = outer.frame.params[0].base else { panic!() };
    install_bindings(outer, vec![MirParamBinding { param, home: MirParamHome::ZeroPage(first.fixed) }]);
    let next = discover(&p, true).into_iter().find(|h| h.owner == first.owner && h.spill == first.spill).unwrap();
    assert_eq!(next.fixed, first.fixed);
    // An escaped entry may run while the other routine's value is live.
    let outer = p.routines.iter_mut().find(|r| r.name == "Outer").unwrap();
    outer.blocks[0].ops.push(MirOp::Move { dst: MirDef::Reg(MirReg::A),
        src: MirValue::RoutineAddrByte { id: first.owner, byte: 0 }, width: MirWidth::Byte });
    assert!(!discover(&p, true).iter().any(|p|
        p.owner == first.owner && p.spill == first.spill && p.fixed == first.fixed));
}

#[test]
fn a_second_nested_call_while_the_parameter_is_live_blocks_sharing() {
    let n = nir_with_caller("\nPROC Wrapper() Hold() RETURN\nBYTE FUNC Outer(BYTE keep) Wrapper() RETURN(keep)");
    let input = crate::mir6502::lower_program(&n).unwrap();
    let mut p = crate::mir6502::materialize_program(input, &crate::mir6502::Mir6502Config {
        enable_small_leaf_inlining: false, ..crate::mir6502::Mir6502Config::optimized()
    }).unwrap();
    let h = discover(&p, true).into_iter().find(|h|
        p.routines.iter().any(|r| r.id == h.owner && r.name == "Hold")).unwrap();
    let outer = p.routines.iter_mut().find(|r| r.name == "Outer").unwrap();
    let MirStorageBase::Param(param) = outer.frame.params[0].base else { panic!() };
    install_bindings(outer, vec![MirParamBinding { param, home: MirParamHome::ZeroPage(h.fixed) }]);
    let body = &mut outer.blocks[0].ops;
    let call = body.iter().find(|op| matches!(op, MirOp::Call { .. })).unwrap().clone();
    let load = MirOp::Load { dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(MirMem::Param { id: param, offset: 0 }), width: MirWidth::Byte };
    *body = vec![load, call.clone()];
    let chosen = |p: &MirProgram| discover(p, true).into_iter().find(|p| p.owner == h.owner && p.spill == h.spill).map(|p| p.fixed);
    assert_eq!(chosen(&p), Some(h.fixed));
    let body = &mut p.routines.iter_mut().find(|r| r.name == "Outer").unwrap().blocks[0].ops;
    body.insert(0, call);
    assert_ne!(chosen(&p), Some(h.fixed));
}

#[test]
fn an_unknown_call_before_initialization_leaves_the_spill_byte_available() {
    let mut p = program();
    let h = discover(&p, true).remove(0);
    let r = p.routines.iter_mut().find(|r| r.id == h.owner).unwrap();
    let mut call = r.blocks.iter().flat_map(|b| &b.ops).find(|op| matches!(op, MirOp::Call { .. })).unwrap().clone();
    let MirOp::Call { target, .. } = &mut call else { unreachable!() };
    *target = MirCallTarget::Routine(RoutineId(999));
    r.blocks[0].ops.insert(0, call);
    assert!(discover(&p, true).iter().any(|p| p.spill == h.spill && p.fixed == h.fixed));
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
    let fixed = calls.fixed_storage();
    for call in r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .filter(|op| matches!(op, MirOp::Call { .. }))
    {
        assert!(calls.writes(call).unwrap().preserves_fixed_byte(h.fixed, &fixed));
    }
    place_at(&mut p, true, 0x3000, &crate::mir6502::trace::Trace::disabled());
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
    let homes = place_at(&mut new, true, 0x3000, &crate::mir6502::trace::Trace::disabled());
    assert_eq!(homes.len(), 1);
    assert!(verify_at(&homes, &old, &executable(new.clone()), &old.program, &old, 0x3000).is_ok());
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
        let (block, index) = if !callee && !numeric {
            r.blocks.iter().enumerate().find_map(|(b, block)|
                block.ops.iter().position(|op| matches!(op, MirOp::Call { .. })).map(|i| (b, i))).unwrap()
        } else { (0, 0) };
        r.blocks[block].ops.insert(
            index,
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
            verify_at(&homes, &old, &bad, &old.program, &old, 0x3000).is_err(),
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

#[test]
fn spill_interference_tracks_dead_stores_word_lanes_and_loop_calls() {
    use interference::{Byte, FrameLives};
    use crate::mir6502::analysis::sites::MirSite;
    let mut p = program();
    let h = discover(&p, true).remove(0);
    let r = p.routines.iter_mut().find(|r| r.id == h.owner).unwrap();
    let spill = Byte::Home(MirHomeByte::Spill { id: h.spill, offset: 0 });
    let cfg = MirCfg::from_routine(r).unwrap();
    let lives = FrameLives::new(r, &cfg).unwrap();
    assert!(!lives.get(spill).unwrap().across_calls.is_empty());

    // Inject accesses into the loop while the spill survives the call.
    let site = *lives.get(spill).unwrap().across_calls.first().unwrap();
    let MirSite::Op { block, op_index } = site else { panic!() };
    let other = MirZpSlot(900);
    let b = r.blocks.iter_mut().find(|b| b.id == block).unwrap();
    b.ops.insert(op_index, MirOp::Store { dst: MirAddr::Direct(MirMem::ZeroPage(other)),
        src: MirValue::ConstU8(1), width: MirWidth::Word });
    let lives = FrameLives::new(r, &cfg).unwrap();
    for offset in [0, 1] {
        let lane = Byte::Home(MirHomeByte::VirtualZeroPage { slot: other, offset });
        assert!(lives.get(lane).unwrap().occupied.contains(&site));
        assert!(!lives.get(lane).unwrap().occupied.is_disjoint(&lives.get(spill).unwrap().occupied));
        assert!(lives.get(lane).unwrap().across_calls.is_empty());
    }
    assert!(lives.get(Byte::Home(MirHomeByte::VirtualZeroPage { slot: other, offset: 2 })).is_none());
}

#[test]
fn one_frame_shares_only_disjoint_spill_local_and_parameter_lanes() {
    let n = nir_with_caller("\nPROC Witness(BYTE x) BYTE y y=x TouchByte(y) RETURN\nPROC Entry() Witness(1) RETURN");
    let input = crate::mir6502::lower_program(&n).unwrap();
    let frame = input.routines.iter().find(|r| r.name == "Witness").unwrap().frame.clone();
    let template = crate::mir6502::materialize_program(input, &crate::mir6502::Mir6502Config {
        enable_small_leaf_inlining: false, ..crate::mir6502::Mir6502Config::optimized()
    }).unwrap();
    let emit = |p| crate::mir6502::finalize_output(&n, p, 0x3000, crate::runtime::Runtime::ActionCart,
        &crate::mir6502::trace::Trace::disabled()).unwrap();
    let fixed = MirFixedZpSlot(0xef);
    let load = |mem| MirOp::Load { dst: MirDef::Reg(MirReg::A), src: MirAddr::Direct(mem), width: MirWidth::Byte };
    let save = |mem| MirOp::Store { dst: MirAddr::Direct(mem), src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte };
    for kind in ["spill", "local", "param"] {
        let mut p = template.clone();
        let index = p.routines.iter().position(|r| r.name == "Witness").unwrap();
        let r = &mut p.routines[index];
        let call = r.blocks.iter().flat_map(|b| &b.ops).find(|op| matches!(op, MirOp::Call { .. })).unwrap().clone();
        r.frame = frame.clone();
        let MirStorageBase::Param(param) = r.frame.params[0].base else { panic!() };
        let local = crate::nir::LocalId(0);
        let mut declaration = r.frame.params[0].clone();
        declaration.id = MirStorageId(42);
        declaration.name = Some("witness".into());
        declaration.base = MirStorageBase::Local(local);
        r.frame.locals = vec![declaration];
        let first = match kind {
            "param" => {
                install_bindings(r, vec![MirParamBinding { param, home: MirParamHome::ZeroPage(fixed) }]);
                MirMem::Param { id: param, offset: 0 }
            }
            "local" => {
                r.frame.local_homes = Some(vec![MirLocalBinding { local, home: MirLocalHome::ZeroPage(fixed) }]);
                r.frame.reserve_named_lane(fixed);
                MirMem::Local { id: local, offset: 0 }
            }
            _ => MirMem::Spill { id: MirSpillId(40), offset: 0 },
        };
        let second = MirMem::Spill { id: MirSpillId(41), offset: 0 };
        r.frame.spills = if kind == "spill" { vec![MirSpillId(40), MirSpillId(41)] } else { vec![MirSpillId(41)] };
        r.frame.fixed_zero_page.extend((0xe0..0xef).map(MirFixedZpSlot));
        r.blocks = [vec![save(first.clone())], vec![load(first.clone()), save(second.clone())],
            vec![call, load(second)]].into_iter().enumerate().map(|(i,ops)| MirBlock {
            id: MirBlockId(i as u32), label: format!("body{i}"), params: vec![], ops,
            terminator: if i == 2 { MirTerminator::Return } else { MirTerminator::Jump(MirEdge::plain(MirBlockId(i as u32 + 1))) },
        }).collect();
        let old = emit(p.clone());
        let homes = place_at(&mut p, true, 0x3000, &crate::mir6502::trace::Trace::disabled());
        let selected: Vec<_> = homes.iter().filter(|h| h.owner == p.routines[index].id).collect();
        assert_eq!(selected.len(), if kind == "spill" { 2 } else { 1 }, "{kind}");
        assert!(selected.iter().all(|h| h.fixed == fixed), "{kind}");
        crate::mir6502::verify_program(&p, MirPhase::PreEmission).unwrap();
        let new = emit(p.clone());
        verify_at(&homes, &old, &new, &old.program, &old, 0x3000).unwrap();
        // Reading the first occupant again after storing the second makes the
        // overlap illegal. Leave both physical bindings in place to test replay.
        let first = if kind == "spill" { MirMem::ZeroPage(selected[0].slot) } else { first };
        p.routines[index].blocks[2].ops.push(load(first));
        assert!(crate::mir6502::verify_program(&p, MirPhase::PreEmission).is_err(), "{kind}");
        let mut bad = new;
        bad.program = p;
        assert!(verify_at(&homes, &old, &bad, &old.program, &old, 0x3000).is_err(), "{kind}");
    }
}
