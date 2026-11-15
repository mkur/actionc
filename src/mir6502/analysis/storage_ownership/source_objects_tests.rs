use super::*;

fn program() -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(
        "CARD FUNC One(CARD POINTER p) CARD h h=p^ RETURN(h)\nCARD FUNC Two(CARD POINTER p) CARD h h=p^ RETURN(h)"
    ).unwrap()).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    crate::mir6502::lower_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model))).unwrap()
}

#[test]
fn object_storage_address_control_is_logical_and_snapshot_scoped() {
    let p = program();
    let owner = p.routines[0].id;
    let MirStorageBase::Local(id) = p.routines[0].frame.locals[0].base else { panic!() };
    let home = MirMem::Local { id, offset: 0 };
    let param = MirMem::Param { id: crate::nir::ParamId(0), offset: 0 };
    let facts = ObjectAddressControl::new(&p);
    assert!(facts.compiler_controls_address(owner, &home));
    assert!(facts.compiler_controls_address(owner, &param)); // pointee read does not expose p
    assert!(facts.is_current(&p));
    let mut placed = p.clone();
    placed.routines[0].frame.local_homes = Some(vec![MirLocalBinding {
        local: id, home: MirLocalHome::ZeroPage(MirFixedZpSlot(0xe4)),
    }]);
    assert!(!facts.is_current(&placed));
    assert!(ObjectAddressControl::new(&placed).compiler_controls_address(owner, &home));
    // The old role is not widened to temporary merely by address control.
    assert_eq!(StorageOwnership::new(&placed).role(owner, &home), StorageRole::ProgramOrAbi);
    assert!(!facts.compiler_controls_address(owner, &MirMem::Absolute(0xe4)));
    assert!(!facts.compiler_controls_address(owner, &MirMem::FixedZeroPage(MirFixedZpSlot(0xa0))));
    assert!(!facts.compiler_controls_address(RoutineId(9000), &home));
    assert!(!facts.compiler_controls_address(owner, &MirMem::Local { id, offset: 2 }));

    for mode in 0..5 {
        let mut exposed = p.clone();
        match mode {
            0 => exposed.routines[0].blocks[0].ops.push(MirOp::LeaAddr {
                dst: MirDef::Reg(MirReg::A), target: MirMem::Local { id, offset: 1 }, width: MirWidth::Word,
            }),
            1 => {
                let mut alias = exposed.routines[0].frame.locals[0].clone();
                alias.base = MirStorageBase::LocalAlias { id: crate::nir::LocalId(99), target: id };
                exposed.routines[0].frame.locals.push(alias);
            }
            2 => exposed.statics.push(MirStatic {
                id: crate::nir::SymbolId(9000), name: "address".into(), ty: "CARD".into(), display: String::new(),
                alignment: 1, mutable: false, section: "data".into(),
                image: MirDataImage { bytes: vec![0, 0], relocations: vec![MirDataRelocation {
                    offset: 0, kind: MirDataRelocationKind::Word16,
                    target: MirDataRelocationTarget::Local { routine: owner, id }, addend: 1,
                    span: crate::source::Span { start: 0, end: 0 },
                }] },
            }),
            3 => exposed.routines[0].blocks[0].ops.push(MirOp::MachineBlock {
                id: MirMachineBlockId(999), effects: Default::default(),
            }), // an unresolved payload grants no proof
            _ => exposed.routines[0].frame.locals[0].base = MirStorageBase::Absolute(0x600),
        }
        let old = ObjectAddressControl::new(&exposed);
        assert!(!old.compiler_controls_address(owner, &home), "mode {mode}");
        assert!(old.compiler_controls_address(p.routines[1].id, &home), "owner-qualified identity");
        // A newly unexposed snapshot cannot erase historical exposure.
        assert!(facts.compiler_controls_address(owner, &home));
        assert!(![&facts, &old].iter().all(|f| f.compiler_controls_address(owner, &home)));
    }
    let mut physical = p.clone();
    physical.routines[0].blocks[0].ops.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::Absolute(0xe4)), src: MirValue::ConstU8(9), width: MirWidth::Byte,
    });
    // Explicit numeric effects require a separate placement check, not a
    // placement-dependent logical exposure classification.
    assert!(ObjectAddressControl::new(&physical).compiler_controls_address(owner, &home));
}
