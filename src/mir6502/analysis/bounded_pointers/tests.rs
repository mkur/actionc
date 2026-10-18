use super::*;

pub(super) fn lower(source: &str, optimized: bool) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let raw = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let nir = if optimized {
        crate::nir::optimize_program(&raw).unwrap()
    } else {
        raw
    };
    crate::mir6502::lower_program(&nir).unwrap()
}

const SOURCE: &str = "BYTE ARRAY data(4)\nPROC A(CARD param)\nBYTE ARRAY local(4)\nRETURN\nPROC B(BYTE param)\nBYTE local\nRETURN\nPROC Main()\nRETURN\n";

#[test]
fn named_objects_have_owning_frames_and_checked_extents() {
    let p = lower(SOURCE, false);
    let a = p.routines.iter().find(|r| r.name == "A").unwrap();
    let b = p.routines.iter().find(|r| r.name == "B").unwrap();
    for mem in [
        MirMem::Local {
            id: LocalId(0),
            offset: 0,
        },
        MirMem::Param {
            id: ParamId(0),
            offset: 0,
        },
    ] {
        let pa = Pointer::named(&p, a, &mem).unwrap();
        let pb = Pointer::named(&p, b, &mem).unwrap();
        assert_ne!(pa.object, pb.object);
        assert!(pa.covers(2));
        assert!(!pb.covers(2));
        assert_eq!(pa.object.memory(0), mem);
        assert_eq!(pa.object.owner(), a.id);
    }
    let id = p.globals.iter().find(|g| g.name == "data").unwrap().id;
    let base = Pointer::named(&p, a, &MirMem::Global { id, offset: 0 }).unwrap();
    assert_eq!(base.extent, 4);
    assert!(base.add(2).unwrap().covers(2));
    assert!(!base.add(3).unwrap().covers(2));
    assert!(!base.add(4).unwrap().covers(1));
    assert!(base.add(5).is_none());
    assert!(base.sub(1).is_none());
    assert!(base.add(3).unwrap().add(u16::MAX).is_none());
    assert!(!base.covers(0));
    let edge = Pointer {
        offset: 65534,
        extent: 65535,
        ..base
    };
    assert!(edge.covers(1));
    assert!(!edge.covers(2));
    for mem in [
        MirMem::Absolute(0xD000),
        MirMem::Local {
            id: LocalId(999),
            offset: 0,
        },
        MirMem::Global { id, offset: 5 },
    ] {
        assert!(Pointer::named(&p, a, &mem).is_none());
    }
    for backing in [
        MirGlobalBacking::Absolute(0x9000),
        MirGlobalBacking::Alias {
            target: id,
            offset: 0,
        },
    ] {
        let mut bad = p.clone();
        bad.globals.iter_mut().find(|g| g.id == id).unwrap().backing = backing;
        assert!(
            Pointer::named(&bad, &bad.routines[0], &MirMem::Global { id, offset: 0 }).is_none()
        );
    }
}

#[test]
fn pointer_temps_require_preceding_complete_same_block_definitions() {
    let mut p = lower(SOURCE, false);
    let r = &mut p.routines[0];
    let id = MirTempId(500);
    let use_id = MirTempId(501);
    let temp = MirValue::Def(MirDef::VTemp(id));
    let lea = MirOp::LeaAddr {
        dst: MirDef::VTemp(id),
        target: MirMem::Local {
            id: LocalId(0),
            offset: 0,
        },
        width: MirWidth::Word,
    };
    let read = MirOp::Load {
        dst: MirDef::VTemp(use_id),
        src: MirAddr::Deref {
            ptr: temp.clone(),
            offset: 1,
        },
        width: MirWidth::Word,
    };
    r.temps.extend([MirTemp { id }, MirTemp { id: use_id }]);
    r.blocks[0].ops = vec![lea.clone(), read.clone()];
    for reason in [
        "valid",
        "redefined",
        "used before",
        "partial lane",
        "successor",
        "terminator",
    ] {
        let mut program = p.clone();
        let r = &mut program.routines[0];
        match reason {
            "redefined" => r.blocks[0].ops.push(lea.clone()),
            "used before" => r.blocks[0].ops.swap(0, 1),
            "partial lane" => r.blocks[0].ops.push(MirOp::Move {
                dst: MirDef::VTemp(use_id),
                src: MirValue::Def(MirDef::VTempByte { id, byte: 0 }),
                width: MirWidth::Byte,
            }),
            "successor" => {
                let mut b = r.blocks[0].clone();
                b.id = MirBlockId(1);
                b.ops = vec![read.clone()];
                r.blocks[0].ops.pop();
                r.blocks[0].terminator = MirTerminator::Jump(MirEdge::plain(b.id));
                r.blocks.push(b);
            }
            "terminator" => {
                r.blocks[0].terminator = MirTerminator::Branch {
                    cond: MirCond::BoolValue(temp.clone()),
                    then_edge: MirEdge::plain(MirBlockId(0)),
                    else_edge: MirEdge::plain(MirBlockId(0)),
                }
            }
            _ => {}
        }
        let r = &program.routines[0];
        let defs = MirTempUseDefIndex::from_routine(r);
        let pointer = Pointer::named(
            &program,
            r,
            &MirMem::Local {
                id: LocalId(0),
                offset: 0,
            },
        )
        .unwrap();
        let mut values = Values::new(&program, r, r.blocks[0].id, &defs);
        assert_eq!(
            values
                .define(
                    &MirDef::VTemp(id),
                    MirWidth::Word,
                    Value::Pointer(pointer),
                    if reason == "used before" { 1 } else { 0 }
                )
                .is_some(),
            reason == "valid",
            "{reason}"
        );
        if reason == "valid" {
            assert_eq!(
                values
                    .derived(&temp, Some(&MirValue::ConstU8(1)), 2, 0)
                    .unwrap()
                    .offset,
                2
            );
            assert!(
                values
                    .derived(&temp, Some(&MirValue::ConstU16(65535)), 2, 0)
                    .is_none()
            );
            assert!(
                values
                    .derived(&temp, Some(&MirValue::Def(MirDef::VTemp(use_id))), 1, 0)
                    .is_none()
            );
            assert_eq!(
                values.address(&MirAddr::Deref {
                    ptr: temp.clone(),
                    offset: 2
                }),
                pointer.add(2)
            );
        }
    }
}

#[test]
fn immutable_static_objects_are_bounded_but_code_and_numeric_addresses_are_not() {
    let p = lower(
        "PROC Use(BYTE ARRAY data)\nRETURN\nPROC Main()\nUse(\"abc\")\nRETURN\n",
        false,
    );
    let r = &p.routines[0];
    let defs = MirTempUseDefIndex::from_routine(r);
    let values = Values::new(&p, r, r.blocks[0].id, &defs);
    let s = &p.statics[0];
    let Value::Pointer(pointer) = values.value(&MirValue::StaticAddr(s.id)).unwrap() else {
        panic!()
    };
    assert_eq!(pointer.extent as usize, s.image.bytes.len());
    assert!(pointer.covers(pointer.extent));
    for value in [
        MirValue::RoutineAddr(r.id),
        MirValue::StorageAddrByte {
            mem: MirMem::Static {
                id: s.id,
                offset: 0,
            },
            byte: 0,
        },
        MirValue::PointerCell(MirMem::Absolute(0x9000)),
    ] {
        assert!(values.value(&value).is_none());
    }
    assert!(
        values
            .address(&MirAddr::Deref {
                ptr: MirValue::ConstU16(0x9000),
                offset: 0
            })
            .is_none()
    );
}
