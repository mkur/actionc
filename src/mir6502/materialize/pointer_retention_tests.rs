use super::*;

fn fixture(text: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    let mut config = Mir6502Config::optimized();
    config.enable_pointer_call_retention = false;
    config.enable_pointer_loop_retention = false;
    config.enable_pointer_promotion = false;
    crate::mir6502::materialize_program_with_origin(
        crate::mir6502::lower_program(&nir).unwrap(),
        &config,
        0x3000,
    )
    .unwrap()
}
fn probe(body: &str) -> MirProgram {
    fixture(&format!(
        "BYTE observed\nPROC Touch=*()\nASM\n{body}\nENDASM\nPROC Probe(BYTE POINTER p)\nobserved=p(1)\nTouch()\nobserved=p(2)\nRETURN\nPROC Main()\nProbe(BYTE POINTER($6000))\nRETURN\n"
    ))
}
fn changed(p: &MirProgram) -> bool {
    let map = memory(p, p, 0x3000).unwrap();
    apply(p, p, &map).unwrap().0 != *p
}
#[test]
fn pointer_retention_calls_preserve_exact_pairs_and_reject_each_clobbered_lane() {
    assert!(changed(&probe("lda #3\nsta.z $A0\nrts")));
    for address in [0xAC, 0xAD] {
        assert!(!changed(&probe(&format!(
            "lda #3\nsta.z ${address:02X}\nrts"
        ))));
    }
    assert!(!changed(&probe("lda ($A0),y\nsta ($A2),y\nrts")));
}
#[test]
fn pointer_retention_calls_keep_source_mutations_and_opaque_barriers() {
    let mut p = probe("rts");
    let caller = p.routines.iter().find(|r| r.name == "Probe").unwrap();
    let source = caller.frame.params[0].base.clone();
    let MirStorageBase::Param(id) = source else {
        panic!()
    };
    let address = memory(&p, &p, 0x3000)
        .unwrap()
        .address(caller.id, &MirMem::Param { id, offset: 0 })
        .unwrap();
    let touch = p.routines.iter_mut().find(|r| r.name == "Touch").unwrap();
    // A separate absolute spelling of the caller's parameter must invalidate it.
    touch.blocks[0].ops = vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 9,
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(address)),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ];
    touch.blocks[0].terminator = MirTerminator::Return;
    assert!(!changed(&p));
    let mut p = probe("rts");
    let touch = p.routines.iter_mut().find(|r| r.name == "Touch").unwrap();
    if let MirOp::MachineBlock { effects, .. } = &mut touch.blocks[0].ops[0] {
        effects.opaque = true;
    }
    assert!(!changed(&p));
}
#[test]
fn pointer_retention_calls_include_transitive_and_argument_writes() {
    let p = fixture(
        "BYTE observed\nPROC Leaf=*()\nASM\nlda #5\nsta.z $AD\nrts\nENDASM\nPROC Touch()\nLeaf()\nRETURN\nPROC Probe(BYTE POINTER p)\nobserved=p(1)\nTouch()\nobserved=p(2)\nRETURN\nPROC Main()\nProbe(BYTE POINTER($6000))\nRETURN\n",
    );
    assert!(!changed(&p));
    let mut p = probe("rts");
    let map = memory(&p, &p, 0x3000).unwrap();
    let calls = CallWrites::analyze(&p, &map);
    let r = p.routines.iter_mut().find(|r| r.name == "Probe").unwrap();
    for op in &mut r.blocks[0].ops {
        if let MirOp::Call { args, .. } = op {
            args.push(MirCallArg {
                value: MirValue::ConstU8(7),
                width: MirWidth::Byte,
                home: MirArgHome::FixedZeroPage(MirFixedZpSlot(0xAC)),
            });
        }
        if matches!(op, MirOp::Call { .. }) {
            assert!(calls.for_call(op, r.id, &map).unwrap().contains(&0xAC));
        }
    }
}
#[test]
fn pointer_retention_calls_replay_cost_and_keep_frames() {
    let p = probe("lda #3\nsta.z $A0\nrts");
    let selected = select_costed(p.clone(), &Mir6502Config::optimized(), 0x3000, false).unwrap();
    assert_ne!(selected, p);
    assert!(
        address_values::Cost::measure(&selected, 0x3000)
            .unwrap()
            .bytes
            < address_values::Cost::measure(&p, 0x3000).unwrap().bytes
    );
    for (a, b) in selected.routines.iter().zip(&p.routines) {
        assert_eq!(a.frame, b.frame);
    }
    crate::mir6502::verify_program(&selected, MirPhase::PreEmission).unwrap();
}
