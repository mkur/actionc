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

const BORROW: &str = "BYTE answer\nBYTE FUNC Read(BYTE POINTER p)\nRETURN(p^)\nBYTE FUNC Relay(BYTE POINTER p)\nRETURN(Read(p))\nPROC Run()\nBYTE ARRAY data(4)\ndata(0)=17\nanswer=Relay(data)\nRETURN\nPROC Main()\nRun()\nRETURN\n";

fn run_summary(p: &MirProgram) -> Option<Summary> {
    summarize(p, p.routines.iter().find(|r| r.name == "Run").unwrap())
}

#[test]
fn call_footprints_follow_actual_objects_through_multiple_frames() {
    for optimized in [false, true] {
        let p = lower(BORROW, optimized);
        let summary = run_summary(&p).unwrap();
        let owner = p.routines.iter().find(|r| r.name == "Run").unwrap().id;
        let local = Object::Local(owner, LocalId(0));
        assert!(
            summary
                .reads
                .iter()
                .any(|a| a.pointer.object == local && a.pointer.offset == 0 && a.bytes == 1)
        );
        assert!(summary.writes.iter().any(|a| a.pointer.object == local));
        assert_eq!(summary.borrowed.get(&local), Some(&4));
        let source = BORROW.replace("answer=Relay(data)", "answer=Relay(BYTE POINTER(@data+3))");
        let shifted = lower(&source, optimized);
        let summary = run_summary(&shifted).unwrap();
        assert!(
            summary
                .reads
                .iter()
                .any(|a| a.pointer.object == local && a.pointer.offset == 3)
        );
        for offset in [4, 65535] {
            let source = BORROW.replace(
                "answer=Relay(data)",
                &format!("answer=Relay(BYTE POINTER(@data+{offset}))"),
            );
            assert!(run_summary(&lower(&source, optimized)).is_none());
        }
    }
}

#[test]
fn bounded_copies_use_hidden_result_parameter_order_and_private_pointer_homes() {
    let source = "TYPE Data=[CARD n BYTE b]\nData result\nCARD number\nData FUNC Make(CARD n)\nData item\nitem.n=n\nitem.b=7\nRETURN(item)\nPROC Run()\nresult=Make(number)\nRETURN\nPROC Main()\nRun()\nRETURN\n";
    for optimized in [false, true] {
        let p = lower(source, optimized);
        let summary = run_summary(&p).unwrap();
        let global = p.globals.iter().find(|g| g.name == "result").unwrap().id;
        assert!(
            summary
                .writes
                .iter()
                .any(|a| a.pointer.object == Object::Global(global) && a.bytes == 3)
        );
        assert!(
            summary
                .writes
                .iter()
                .any(|a| matches!(a.pointer.object, Object::Local(..)) && a.bytes == 3)
        );
    }
}

#[test]
fn pointer_calls_reject_escape_partial_access_reentry_and_unknown_effects() {
    for reason in [
        "global retention",
        "return",
        "partial parameter",
        "pointer arithmetic",
        "parameter write",
        "absolute",
        "unknown",
        "recursion",
        "machine",
        "barrier",
        "OS",
        "all effects",
        "absolute ABI",
        "budget",
    ] {
        let mut p = lower(BORROW, true);
        let call = p
            .routines
            .iter()
            .find(|r| r.name == "Relay")
            .unwrap()
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .find(|op| matches!(op, MirOp::Call { .. }))
            .unwrap()
            .clone();
        let r = p.routines.iter_mut().find(|r| r.name == "Read").unwrap();
        let MirOp::Load {
            dst,
            src: MirAddr::Direct(MirMem::Param { id, .. }),
            ..
        } = &r.blocks[0].ops[0]
        else {
            panic!()
        };
        let (dst, param) = (dst.clone(), *id);
        let ptr = MirValue::Def(dst.clone());
        let extra = match reason {
            "global retention" => MirOp::Store {
                dst: MirAddr::Direct(MirMem::Global {
                    id: p.globals[0].id,
                    offset: 0,
                }),
                src: ptr.clone(),
                width: MirWidth::Byte,
            },
            "return" => MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
                src: ptr.clone(),
                width: MirWidth::Word,
            },
            "partial parameter" => MirOp::Load {
                dst: MirDef::VTemp(MirTempId(900)),
                src: MirAddr::Direct(MirMem::Param {
                    id: param,
                    offset: 1,
                }),
                width: MirWidth::Byte,
            },
            "pointer arithmetic" => MirOp::Binary {
                dst: MirDef::VTemp(MirTempId(900)),
                left: ptr,
                right: MirValue::ConstU16(2),
                width: MirWidth::Word,
                op: MirBinaryOp::Xor,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            "parameter write" => MirOp::Store {
                dst: MirAddr::Direct(MirMem::Param {
                    id: param,
                    offset: 0,
                }),
                src: MirValue::ConstU16(1),
                width: MirWidth::Word,
            },
            "absolute" => MirOp::Load {
                dst: MirDef::VTemp(MirTempId(900)),
                src: MirAddr::Direct(MirMem::Absolute(0xD000)),
                width: MirWidth::Byte,
            },
            "unknown" | "recursion" | "OS" | "all effects" | "absolute ABI" => {
                let mut call = call;
                let MirOp::Call {
                    target,
                    effects,
                    abi,
                    args,
                    ..
                } = &mut call
                else {
                    panic!()
                };
                args[0].value = MirValue::Def(dst);
                match reason {
                    "unknown" => *target = MirCallTarget::Routine(RoutineId(999)),
                    "recursion" => {}
                    "OS" => effects.may_call_os = true,
                    "all effects" => effects.memory_reads = MirMemoryEffect::All,
                    "absolute ABI" => {
                        args[0].home = MirArgHome::Absolute(0x9000);
                        abi.params[0] = args[0].home.clone();
                    }
                    _ => unreachable!(),
                }
                call
            }
            "machine" => MirOp::MachineBlock {
                id: MirMachineBlockId(0),
                effects: MirEffects::default(),
            },
            "barrier" => MirOp::Barrier {
                effects: MirEffects::default(),
            },
            "budget" => {
                for n in 0..513 {
                    r.blocks[0].ops.push(MirOp::LoadImm {
                        dst: MirDef::VTemp(MirTempId(1000 + n)),
                        value: 0,
                        width: MirWidth::Byte,
                    });
                }
                MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(999)),
                    value: 0,
                    width: MirWidth::Byte,
                }
            }
            _ => unreachable!(),
        };
        r.blocks[0].ops.push(extra);
        assert!(run_summary(&p).is_none(), "{reason}");
    }
}

#[test]
fn pointer_homes_cannot_expose_stale_or_partial_pointer_bytes() {
    let source = "BYTE answer\nPROC Run()\nBYTE local\nBYTE POINTER p\nlocal=37\np=@local\nanswer=p^\nRETURN\nPROC Main()\nRun()\nRETURN\n";
    let p = lower(source, false);
    assert!(run_summary(&p).is_some());
    let owner = p.routines.iter().position(|r| r.name == "Run").unwrap();
    let r = &p.routines[owner];
    let home = r
        .frame
        .locals
        .iter()
        .find(|s| s.name.as_deref() == Some("p"))
        .unwrap();
    let MirStorageBase::Local(id) = home.base else {
        panic!()
    };
    for reason in [
        "address",
        "early read",
        "partial read",
        "successor",
        "data exposure",
        "alias",
    ] {
        let mut bad = p.clone();
        let r = &mut bad.routines[owner];
        match reason {
            "address" => r.blocks[0].ops.push(MirOp::LeaAddr {
                dst: MirDef::VTemp(MirTempId(900)),
                target: MirMem::Local { id, offset: 0 },
                width: MirWidth::Word,
            }),
            "early read" | "partial read" => r.blocks[0].ops.insert(
                0,
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(900)),
                    src: MirAddr::Direct(MirMem::Local { id, offset: 0 }),
                    width: if reason == "partial read" {
                        MirWidth::Byte
                    } else {
                        MirWidth::Word
                    },
                },
            ),
            "successor" => {
                let mut next = r.blocks[0].clone();
                next.id = MirBlockId(1);
                next.ops = vec![MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(900)),
                    src: MirAddr::Direct(MirMem::Local { id, offset: 0 }),
                    width: MirWidth::Word,
                }];
                r.blocks[0].terminator = MirTerminator::Jump(MirEdge::plain(next.id));
                r.blocks.push(next);
            }
            "data exposure" => {
                let slot = r
                    .frame
                    .locals
                    .iter_mut()
                    .find(|s| s.base == MirStorageBase::Local(id))
                    .unwrap();
                slot.init = Some(MirStorageInit::Bytes {
                    image: MirDataImage {
                        bytes: vec![0, 0],
                        relocations: vec![MirDataRelocation {
                            offset: 0,
                            kind: MirDataRelocationKind::Word16,
                            target: MirDataRelocationTarget::Local { routine: r.id, id },
                            addend: 0,
                            span: crate::source::Span::new(0, 0),
                        }],
                    },
                    zero_fill: 0,
                    mutable: true,
                    section: "data".into(),
                });
            }
            "alias" => r.frame.locals[0].base = MirStorageBase::Absolute(0x9000),
            _ => unreachable!(),
        }
        assert!(run_summary(&bad).is_none(), "{reason}");
    }
}

#[test]
fn bounded_pointer_writes_and_call_contexts_keep_the_actual_object_bounds() {
    let source = "BYTE answer\nPROC Write(BYTE ARRAY p)\np(1)=90\nRETURN\nPROC Run()\nBYTE ARRAY data(4),small(1)\nWrite(data)\nanswer=data(1)\nRETURN\nPROC Main()\nRun()\nRETURN\n";
    for optimized in [false, true] {
        let p = lower(source, optimized);
        let summary = run_summary(&p).unwrap();
        let owner = p.routines.iter().find(|r| r.name == "Run").unwrap().id;
        assert!(
            summary
                .writes
                .iter()
                .any(|a| a.pointer.object == Object::Local(owner, LocalId(0))
                    && a.pointer.offset == 1
                    && a.bytes == 1)
        );
        let bad = source.replace("Write(data)", "Write(data)\nWrite(small)");
        assert!(
            run_summary(&lower(&bad, optimized)).is_none(),
            "callee facts must be specialized per actual call"
        );
    }
    let word = source.replace(
        "PROC Write(BYTE ARRAY p)\np(1)=90",
        "PROC Write(CARD POINTER p)\np^=32768",
    );
    assert!(
        run_summary(&lower(
            &word.replace("Write(data)", "Write(CARD POINTER(@data+2))"),
            true
        ))
        .is_some()
    );
    assert!(
        run_summary(&lower(
            &word.replace("Write(data)", "Write(CARD POINTER(@data+3))"),
            true
        ))
        .is_none()
    );
}

#[test]
fn deeper_call_graphs_and_runtime_argument_capture_stay_bounded() {
    for count in [4, 17] {
        let mut source =
            String::from("BYTE answer\nPROC Step0(BYTE POINTER p)\nanswer=p^\nRETURN\n");
        for n in 1..count {
            source.push_str(&format!(
                "PROC Step{n}(BYTE POINTER p)\nStep{}(p)\nRETURN\n",
                n - 1
            ));
        }
        source.push_str(&format!("PROC Run()\nBYTE local\nlocal=37\nStep{}(@local)\nRETURN\nPROC Main()\nRun()\nRETURN\n", count-1));
        assert_eq!(run_summary(&lower(&source, true)).is_some(), count == 4);
    }
    let source = "TYPE Data=[CARD n BYTE b]\nData result\nData FUNC Make(CARD n BYTE b)\nData item\nitem.n=n\nitem.b=b\nRETURN(item)\nPROC Run()\nresult=Make(17,9)\nRETURN\nPROC Main()\nRun()\nRETURN\n";
    assert!(
        run_summary(&lower(source, true)).is_none(),
        "five argument bytes require the runtime SArgs prologue"
    );
}
