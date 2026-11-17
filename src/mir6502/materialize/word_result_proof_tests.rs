use super::*;
use crate::mir6502::ir::MirGlobalInit;
use crate::mir6502::trace::{Mir6502Trace, TraceOptions};

fn slot(base: MirStorageBase, size: u16) -> MirStorageSlot {
    MirStorageSlot {
        purpose: crate::mir6502::ir::MirStoragePurpose::Storage,
        id: MirStorageId(0),
        name: None,
        storage: MirStorageClass::Scalar,
        storage_size: size,
        scalar_width: Some(MirWidth::Word),
        base,
        offset: 0,
        mutable: true,
        init: None,
    }
}

pub(super) fn program() -> MirProgram {
    let mut program = empty_test_program();
    let mut routine = ssa_lite_edge_test_routine(vec![MirBlock {
        id: MirBlockId(0),
        label: "entry".into(),
        params: Vec::new(),
        ops: Vec::new(),
        terminator: MirTerminator::Return,
    }]);
    routine.id = RoutineId(0);
    routine.frame.locals = vec![slot(MirStorageBase::Local(LocalId(0)), 2)];
    routine.frame.params = vec![slot(MirStorageBase::Param(ParamId(0)), 2)];
    program.routines = vec![routine];
    program.globals.push(MirGlobal {
        id: SymbolId(0),
        name: "destination".into(),
        kind: "scalar".into(),
        width: Some(MirWidth::Word),
        storage_size: 2,
        backing: MirGlobalBacking::Ordinary { offset: 0 },
        init: None,
    });
    program
}

fn local(offset: u16) -> MirMem {
    MirMem::Local {
        id: LocalId(0),
        offset,
    }
}
fn global(offset: u16) -> MirMem {
    MirMem::Global {
        id: SymbolId(0),
        offset,
    }
}

#[test]
fn word_result_proof_is_invariant_under_origin_and_preceding_storage_changes() {
    for origin in [0x0100, 0x3000, 0x41c7] {
        for padding in [0, 17, 257] {
            let mut program = program();
            if padding != 0 {
                let mut preceding = program.globals[0].clone();
                preceding.id = SymbolId(9);
                preceding.storage_size = padding;
                program.globals[0].backing = MirGlobalBacking::Ordinary { offset: padding };
                program.globals.insert(0, preceding);
            }
            let layout = MaterializeLayout::new(&program, origin);
            for mem in [
                local(0),
                global(0),
                MirMem::Param {
                    id: ParamId(0),
                    offset: 0,
                },
            ] {
                assert!(layout.ordinary_range_is_outside_zero_page(RoutineId(0), &mem, 2));
                let address = layout.mem_address(RoutineId(0), &mem).unwrap();
                assert!(address >= origin + padding);
            }
            assert!(!layout.ordinary_range_is_outside_zero_page(RoutineId(1), &local(0), 2));
        }
    }
}

#[test]
fn word_result_proof_rejects_incomplete_ranges_and_storage_budget_overflow() {
    let mut program = program();
    program.globals.clear();
    program.routines[0].frame.params.clear();
    for (origin, proven) in [
        (0x00ff, false),
        (0x0100, true),
        (0xfffe, true),
        (0xffff, false),
    ] {
        let layout = MaterializeLayout::new(&program, origin);
        assert_eq!(
            layout.ordinary_range_is_outside_zero_page(RoutineId(0), &local(0), 2),
            proven
        );
        for (offset, bytes) in [(0, 0), (1, 2), (2, 1), (u16::MAX, 2)] {
            assert!(!layout.ordinary_range_is_outside_zero_page(
                RoutineId(0),
                &local(offset),
                bytes
            ));
        }
    }
    // An initializer larger than u16 must not disappear through a length cast.
    program.globals.push(MirGlobal {
        id: SymbolId(7),
        name: "large".into(),
        kind: "scalar".into(),
        width: None,
        storage_size: 0,
        backing: MirGlobalBacking::Ordinary { offset: 0 },
        init: Some(MirGlobalInit::Bytes {
            image: MirDataImage {
                bytes: vec![0; 65536],
                relocations: Vec::new(),
            },
            zero_fill: 0,
            mutable: true,
            section: "data".into(),
            array: None,
        }),
    });
    assert!(
        !MaterializeLayout::new(&program, 0x3000).ordinary_range_is_outside_zero_page(
            RoutineId(0),
            &local(0),
            2
        )
    );
}

#[test]
fn word_result_proof_does_not_certify_aliases_descriptors_or_nonordinary_homes() {
    let mut program = program();
    for backing in [
        MirGlobalBacking::Absolute(0xa0),
        MirGlobalBacking::Alias {
            target: SymbolId(0),
            offset: 0,
        },
    ] {
        let mut alias = program.globals[0].clone();
        alias.id = SymbolId(1);
        alias.backing = backing;
        program.globals.push(alias);
        assert!(
            !MaterializeLayout::new(&program, 0x3000).ordinary_range_is_outside_zero_page(
                RoutineId(0),
                &MirMem::Global {
                    id: SymbolId(1),
                    offset: 0
                },
                2
            )
        );
        program.globals.pop();
    }
    program.routines[0].frame.locals.push(slot(
        MirStorageBase::LocalAlias {
            id: LocalId(1),
            target: LocalId(0),
        },
        2,
    ));
    let layout = MaterializeLayout::new(&program, 0x3000);
    for mem in [
        MirMem::Local {
            id: LocalId(1),
            offset: 0,
        },
        MirMem::Local {
            id: LocalId(99),
            offset: 0,
        },
        MirMem::Absolute(0x3000),
        MirMem::Static {
            id: SymbolId(0),
            offset: 0,
        },
        MirMem::Spill {
            id: MirSpillId(0),
            offset: 0,
        },
        MirMem::ZeroPage(MirZpSlot(0)),
        MirMem::FixedZeroPage(MirFixedZpSlot(0xa0)),
    ] {
        assert!(!layout.ordinary_range_is_outside_zero_page(RoutineId(0), &mem, 2));
    }
    program.routines[0].frame.params[0].base = MirStorageBase::ParamAbiOnly(ParamId(0));
    program.routines[0].frame.locals[0].storage = MirStorageClass::Array;
    program.globals[0].init = Some(MirGlobalInit::RoutineAddress {
        routine: RoutineId(0),
        descriptor_size: 2,
        size_word: None,
        mutable: false,
        section: "data".into(),
    });
    let layout = MaterializeLayout::new(&program, 0x3000);
    for mem in [
        local(0),
        global(0),
        MirMem::Param {
            id: ParamId(0),
            offset: 0,
        },
    ] {
        assert!(!layout.ordinary_range_is_outside_zero_page(RoutineId(0), &mem, 2));
    }
}

fn load(mem: MirMem) -> MirOp {
    MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(mem),
        width: MirWidth::Byte,
    }
}
fn store(mem: MirMem) -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(mem),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }
}

fn result_program() -> MirProgram {
    let mut program = program();
    let mut callee = program.routines[0].clone();
    callee.id = RoutineId(1);
    callee.name = "Callee".into();
    callee.frame = MirFrame::default();
    callee.blocks[0].ops = vec![load(MirMem::Absolute(0x600)), store(return_slot_mem(1))];
    program.routines.push(callee);
    program.routines[0].blocks[0].ops = vec![
        MirOp::Call {
            additional_results: Vec::new(),
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: MirCallAbi {
                library: None,
                additional_results: Vec::new(),
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet {
                    a: true,
                    x: true,
                    y: true,
                    flags: true,
                    sp: false,
                },
                preserves: MirRegisterSet::default(),
            },
            args: Vec::new(),
            result: None,
            effects: MirEffects::default(),
        },
        load(return_slot_mem(0)),
        store(local(0)),
        load(return_slot_mem(1)),
        store(local(1)),
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 0,
            width: MirWidth::Byte,
        },
    ];
    program
}

#[test]
fn word_result_rewrite_uses_structural_proof_and_retains_callee_and_exit_checks() {
    for (origin, padding) in [(0x3000, 0), (0x41c7, 257)] {
        for unknown_callee in [false, true] {
            let mut program = result_program();
            program.globals[0].storage_size += padding;
            let known = if unknown_callee {
                MirKnownCalleeSummaries::default()
            } else {
                MirKnownCalleeSummaries::analyze(&program)
            };
            let session = Mir6502Trace::new(TraceOptions {
                core_layout_census: true,
                ..Default::default()
            });
            let layout =
                MaterializeLayout::new(&program, origin).with_census(&session.context(), "test");
            let original = program.routines[0].blocks[0].ops.clone();
            let mut stats = MirPeepholeStats::default();
            stats.trace = session.context();
            run_analyzed_known_callee_word_result_placements(
                &mut program.routines[0],
                &layout,
                Some(&known),
                &mut stats,
            )
            .unwrap();
            assert_eq!(
                program.routines[0].blocks[0].ops.len(),
                original.len() - usize::from(!unknown_callee)
            );
            let events = session.events();
            for event in events.iter().filter(|e| e.kind == "core-layout-round") {
                assert!(
                    event.data["observations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|o| o["observation"]["predicate"] != "address-query")
                );
            }
            if !unknown_callee {
                assert_eq!(program.routines[0].blocks[0].ops[1], store(local(1)));
                assert!(
                    events
                        .iter()
                        .any(|e| e.kind == "core-layout-round" && e.data["applied_count"] == 1)
                );
            }
        }
    }
}

#[test]
fn word_result_fallback_requires_resolved_nonoverlapping_addresses() {
    // Below-ZP origin, a real alias, missing storage and a collapsed word at
    // $FFFF exercise the fallback rather than granting structural permission.
    for case in [
        "safe-low-origin",
        "return-overlap",
        "alias",
        "missing",
        "saturated",
    ] {
        let mut program = result_program();
        program.globals.clear();
        program.routines[0].frame.params.clear();
        let origin = match case {
            "safe-low-origin" => 0x80,
            "return-overlap" => 0xa0,
            "saturated" => 0xffff,
            _ => 0x3000,
        };
        match case {
            "alias" => {
                program.routines[0].frame.locals[0].base = MirStorageBase::Local(LocalId(1));
                program.routines[0].frame.locals.push(slot(
                    MirStorageBase::LocalAlias {
                        id: LocalId(0),
                        target: LocalId(1),
                    },
                    2,
                ));
            }
            "missing" => program.routines[0].frame.locals.clear(),
            _ => {}
        }
        let known = MirKnownCalleeSummaries::analyze(&program);
        let layout = MaterializeLayout::new(&program, origin);
        assert!(!layout.ordinary_range_is_outside_zero_page(RoutineId(0), &local(0), 2));
        let original = program.routines[0].blocks[0].ops.clone();
        let mut stats = MirPeepholeStats::default();
        run_analyzed_known_callee_word_result_placements(
            &mut program.routines[0],
            &layout,
            Some(&known),
            &mut stats,
        )
        .unwrap();
        let expected = matches!(case, "safe-low-origin" | "alias");
        assert_eq!(
            program.routines[0].blocks[0].ops.len(),
            original.len() - usize::from(expected),
            "{case}"
        );
        if !expected {
            assert_eq!(program.routines[0].blocks[0].ops, original, "{case}");
        }
    }
}

#[test]
fn word_result_proof_does_not_override_live_accumulator_or_flags() {
    for live_a in [false, true] {
        let mut program = result_program();
        let caller = &mut program.routines[0];
        caller.blocks[0].ops.pop();
        if live_a {
            caller.blocks[0].ops.push(store(MirMem::Absolute(0x602)));
        } else {
            caller.blocks[0].terminator = MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::ZSet),
                then_edge: MirEdge {
                    target: MirBlockId(1),
                    args: Vec::new(),
                },
                else_edge: MirEdge {
                    target: MirBlockId(2),
                    args: Vec::new(),
                },
            };
            for id in [1, 2] {
                caller.blocks.push(MirBlock {
                    id: MirBlockId(id),
                    label: format!("exit{id}"),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                });
            }
        }
        let known = MirKnownCalleeSummaries::analyze(&program);
        let layout = MaterializeLayout::new(&program, 0x3000);
        assert!(layout.ordinary_range_is_outside_zero_page(RoutineId(0), &local(0), 2));
        let original = program.routines[0].clone();
        run_analyzed_known_callee_word_result_placements(
            &mut program.routines[0],
            &layout,
            Some(&known),
            &mut MirPeepholeStats::default(),
        )
        .unwrap();
        assert_eq!(program.routines[0], original);
    }
}
