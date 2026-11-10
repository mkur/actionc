use super::*;
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::machine_values::MirMachineValueAvailability;
use crate::nir::SymbolId;

fn mem(id: u32) -> MirMem {
    MirMem::Global {
        id: SymbolId(id),
        offset: 0,
    }
}
fn load(reg: MirReg, src: MirMem) -> MirOp {
    MirOp::Load {
        dst: MirDef::Reg(reg),
        src: MirAddr::Direct(src),
        width: MirWidth::Byte,
    }
}
fn imm(reg: MirReg, value: u8) -> MirOp {
    MirOp::LoadImm {
        dst: MirDef::Reg(reg),
        value: u16::from(value),
        width: MirWidth::Byte,
    }
}
fn store(dst: MirMem, reg: MirReg) -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(dst),
        src: MirValue::Def(MirDef::Reg(reg)),
        width: MirWidth::Byte,
    }
}
fn program(ops: Vec<MirOp>) -> MirProgram {
    MirProgram {
        globals: (0..6)
            .map(|id| MirGlobal {
                id: SymbolId(id),
                name: format!("cell{id}"),
                kind: "byte".into(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Ordinary { offset: id as u16 },
                init: None,
            })
            .collect(),
        statics: vec![],
        machine_blocks: vec![],
        runtime_helpers: vec![],
        routines: vec![MirRoutine {
            id: RoutineId(0),
            name: "Neutral".into(),
            scalar_signature: None,
            inline: Default::default(),
            abi: MirRoutineAbi::ProgramEntry,
            frame: Default::default(),
            temps: vec![],
            effects: Default::default(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".into(),
                params: vec![],
                ops,
                terminator: MirTerminator::Return,
            }],
        }],
    }
}
fn plans(p: &MirProgram) -> Vec<MirPostHomeRewritePlan> {
    let r = &p.routines[0];
    let physical = physical_memory(p, p, 0x3000).unwrap();
    let map = MirMachineMemoryMap::from_routine(r, |m| physical.address(r.id, m));
    let s = PostHomeAnalysisSnapshot::new_with_known_callees_and_memory_map(
        r,
        MirRoutineGeneration::initial(),
        &MirKnownCalleeSummaries::default(),
        &map,
    )
    .unwrap();
    discover_load_stores(
        r,
        &PostHomeRewriteContext::new(&s),
        &MaterializeLayout::new(p, 0x3000),
        &physical,
    )
}

#[test]
fn register_copy_load_stores_select_all_registers_and_keep_live_a() {
    for reg in [MirReg::A, MirReg::X, MirReg::Y] {
        let mut prefix = vec![load(reg, mem(0))];
        if reg != MirReg::A {
            prefix.push(imm(MirReg::A, 0x55));
        }
        prefix.extend([
            load(MirReg::A, mem(0)),
            store(mem(1), MirReg::A),
            imm(MirReg::A, 7),
        ]);
        let p = program(prefix);
        let found = plans(&p);
        assert!(found.iter().any(|p|matches!(&p.replacement[0],MirOp::Store {src:MirValue::Def(MirDef::Reg(source)),..} if *source==reg)));
        let mut live = p.clone();
        let end = live.routines[0].blocks[0].ops.len() - 1;
        live.routines[0].blocks[0].ops[end] = store(mem(2), MirReg::A);
        assert_eq!(plans(&live).is_empty(), reg != MirReg::A);
    }
}

#[test]
fn register_copy_load_stores_require_values_and_protect_absolute_reads() {
    for src in [
        mem(0),
        MirMem::Absolute(0xBFFA),
        MirMem::Absolute(0xD20F),
        MirMem::Absolute(0xE2),
    ] {
        let unknown = program(vec![
            load(MirReg::A, src.clone()),
            store(mem(1), MirReg::A),
            imm(MirReg::A, 7),
        ]);
        assert!(plans(&unknown).is_empty());
        if matches!(src, MirMem::Absolute(_)) {
            let known = program(vec![
                load(MirReg::Y, src.clone()),
                load(MirReg::A, src),
                store(mem(1), MirReg::A),
                imm(MirReg::A, 7),
            ]);
            assert!(plans(&known).is_empty());
        }
    }
}

#[test]
fn register_copy_physical_alias_and_barrier_invalidate_available_sources() {
    for alias in [false, true] {
        let mut p = program(vec![
            load(MirReg::Y, mem(0)),
            imm(MirReg::A, 11),
            store(mem(5), MirReg::A),
            load(MirReg::A, mem(0)),
            store(mem(1), MirReg::A),
            imm(MirReg::A, 7),
        ]);
        if alias {
            p.globals[5].backing = MirGlobalBacking::Alias {
                target: SymbolId(0),
                offset: 0,
            };
        }
        assert_eq!(plans(&p).is_empty(), alias);
        let r = &p.routines[0];
        let cfg = MirCfg::from_routine(r).unwrap();
        let conservative = MirMachineValueAvailability::analyze(r, &cfg);
        assert!(
            conservative
                .register_at(
                    MirSite::Op {
                        block: MirBlockId(0),
                        op_index: 3
                    },
                    MirReg::Y
                )
                .unwrap()
                .is_none()
        );
    }
    let mut p = program(vec![
        load(MirReg::Y, mem(0)),
        imm(MirReg::A, 11),
        load(MirReg::A, mem(0)),
        store(mem(1), MirReg::A),
        imm(MirReg::A, 7),
    ]);
    p.routines[0].blocks[0].ops.insert(
        2,
        MirOp::Barrier {
            effects: Default::default(),
        },
    );
    assert!(plans(&p).is_empty());
}

#[test]
fn register_copy_live_nz_requires_matching_provenance() {
    for matching in [false, true] {
        let mut ops = vec![load(MirReg::Y, mem(0))];
        if !matching {
            ops.push(imm(MirReg::A, 7));
        }
        ops.extend([load(MirReg::A, mem(0)), store(mem(1), MirReg::A)]);
        let mut p = program(ops);
        p.routines[0].blocks[0].terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::ZSet),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(1)),
        };
        p.routines[0].blocks.push(MirBlock {
            id: MirBlockId(1),
            label: "later".into(),
            params: vec![],
            ops: vec![],
            terminator: MirTerminator::Return,
        });
        assert_eq!(plans(&p).is_empty(), !matching);
    }
}

fn bytes(p: &MirProgram) -> Vec<u8> {
    let mut e = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
    crate::mir6502::emit::emit_program(p, 0x3000, &mut e).unwrap();
    e.finish_with_relocations().unwrap().bytes
}

#[test]
fn register_copy_rejects_equal_final_cost_without_losing_prior_optimizations() {
    let p = program(vec![
        load(MirReg::Y, mem(0)),
        imm(MirReg::A, 99),
        load(MirReg::A, mem(0)),
        store(mem(1), MirReg::A),
        load(MirReg::A, mem(0)),
    ]);
    let physical = physical_memory(&p, &p, 0x3000).unwrap();
    let (trial, _, affected) = apply(&p, 0x3000, &physical).unwrap();
    assert!(!affected.is_empty());
    assert_eq!(bytes(&trial).len(), bytes(&p).len());
    let selected = select_costed(
        p.clone(),
        &crate::mir6502::Mir6502Config::optimized(),
        0x3000,
        false,
    )
    .unwrap();
    assert_eq!(selected, p);
}

#[test]
fn register_copy_two_lane_fixed_point_executes_every_byte_pair_without_stale_batch_facts() {
    let lo = MirMem::ZeroPage(MirZpSlot(0));
    let hi = MirMem::ZeroPage(MirZpSlot(1));
    let mut p = program(vec![
        load(MirReg::Y, mem(0)),
        load(MirReg::A, mem(1)),
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::Def(MirDef::Reg(MirReg::Y)),
            width: MirWidth::Byte,
        },
        store(lo.clone(), MirReg::A),
        load(MirReg::A, mem(1)),
        store(hi.clone(), MirReg::A),
        load(MirReg::A, lo),
        store(mem(2), MirReg::A),
        load(MirReg::A, hi),
        store(mem(3), MirReg::A),
        load(MirReg::A, mem(0)),
        store(mem(4), MirReg::A),
        load(MirReg::A, mem(0)),
        store(mem(5), MirReg::A),
        load(MirReg::A, mem(1)),
        store(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0)), MirReg::A),
    ]);
    p.routines[0].frame.virtual_zero_page = vec![MirZpSlot(0), MirZpSlot(1)];
    p.routines[0]
        .frame
        .fixed_zero_page
        .push(MirFixedZpSlot(0xA0));
    p.routines[0].frame.zero_page_allocations = vec![
        MirZpAllocation {
            slot: MirZpSlot(0),
            start: MirFixedZpSlot(0xE2),
            size: 1,
        },
        MirZpAllocation {
            slot: MirZpSlot(1),
            start: MirFixedZpSlot(0xE3),
            size: 1,
        },
    ];
    crate::mir6502::verify_program(&p, MirPhase::PreEmission).unwrap();
    let selected = select_costed(
        p.clone(),
        &crate::mir6502::Mir6502Config::optimized(),
        0x3000,
        false,
    )
    .unwrap();
    assert_ne!(selected, p);
    assert!(
        selected.routines[0].blocks[0]
            .ops
            .iter()
            .all(|op| !matches!(
                op,
                MirOp::Load {
                    src: MirAddr::Direct(MirMem::ZeroPage(_)),
                    ..
                } | MirOp::Store {
                    dst: MirAddr::Direct(MirMem::ZeroPage(_)),
                    ..
                }
            ))
    );
    let before = bytes(&p);
    let after = bytes(&selected);
    assert!(after.len() < before.len());
    let mut original = [0u8; 65536];
    let mut rewritten = [0u8; 65536];
    original[0x3000..0x3000 + before.len()].copy_from_slice(&before);
    rewritten[0x3000..0x3000 + after.len()].copy_from_slice(&after);
    for word in 0..=65535u16 {
        let [low, high] = word.to_le_bytes();
        original[0x3000] = low;
        original[0x3001] = high;
        rewritten[0x3000] = low;
        rewritten[0x3001] = high;
        super::super::leaf_test_cpu::run_memory(&mut original, 0x3006);
        super::super::leaf_test_cpu::run_memory(&mut rewritten, 0x3006);
        assert_eq!(&original[0x3002..0x3006], &[low, high, low, low]);
        assert_eq!(&rewritten[0x3002..0x3006], &[low, high, low, low]);
        assert_eq!(rewritten[0xA0], high);
    }
}

#[test]
fn register_copy_ram_scratch_cleanup_preserves_explicit_alias_and_successor_reads() {
    let scratch = MirMem::Spill {
        id: MirSpillId(0),
        offset: 0,
    };
    for observed in [0, 1, 2] {
        let mut p = program(vec![
            load(MirReg::Y, mem(0)),
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                width: MirWidth::Byte,
            },
            store(scratch.clone(), MirReg::A),
            imm(MirReg::A, 77),
            load(MirReg::A, scratch.clone()),
            store(mem(1), MirReg::A),
            imm(MirReg::A, 7),
        ]);
        p.routines[0].frame.spills.push(MirSpillId(0));
        if observed == 1 {
            let physical = physical_memory(&p, &p, 0x3000).unwrap();
            let address = physical.address(RoutineId(0), &scratch).unwrap();
            p.globals.push(MirGlobal {
                id: SymbolId(6),
                name: "external_alias".into(),
                kind: "byte".into(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Absolute(address),
                init: None,
            });
            p.routines[0].blocks[0]
                .ops
                .extend([load(MirReg::X, mem(6)), store(mem(2), MirReg::X)]);
        } else if observed == 2 {
            p.routines[0].blocks[0].terminator = MirTerminator::Jump(MirEdge::plain(MirBlockId(1)));
            p.routines[0].blocks.push(MirBlock {
                id: MirBlockId(1),
                label: "successor".into(),
                params: vec![],
                ops: vec![load(MirReg::X, scratch.clone()), store(mem(2), MirReg::X)],
                terminator: MirTerminator::Return,
            });
        }
        let selected = select_costed(
            p.clone(),
            &crate::mir6502::Mir6502Config::optimized(),
            0x3000,
            false,
        )
        .unwrap();
        assert_ne!(selected, p);
        let retained = selected.routines[0]
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(|op| matches!(op,MirOp::Store {dst:MirAddr::Direct(mem),..} if *mem==scratch));
        assert_eq!(retained, observed != 0);
    }
}

#[test]
fn register_copy_final_transaction_preserves_published_callee_accumulator() {
    let mut p = program(vec![
        imm(MirReg::A, 11),
        imm(MirReg::Y, 22),
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::Def(MirDef::Reg(MirReg::Y)),
            width: MirWidth::Byte,
        },
        store(mem(0), MirReg::A),
    ]);
    p.routines[0].abi = MirRoutineAbi::Action;
    let mut caller = program(vec![]).routines.remove(0);
    caller.id = RoutineId(1);
    caller.name = "Caller".into();
    let clobbers = MirRegisterSet {
        a: true,
        x: true,
        y: true,
        flags: true,
        ..Default::default()
    };
    caller.blocks[0].ops.push(MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(0)),
        abi: MirCallAbi {
            params: vec![],
            result: None,
            additional_results: vec![],
            clobbers,
            preserves: Default::default(),
        },
        args: vec![],
        result: None,
        additional_results: vec![],
        effects: MirEffects {
            clobbers,
            ..Default::default()
        },
    });
    p.routines.push(caller);
    let map = physical_memory(&p, &p, 0x3000).unwrap();
    let (trial, _, affected) = apply(&p, 0x3000, &map).unwrap();
    assert!(affected.contains(&RoutineId(0)));
    assert!(bytes(&trial).len() < bytes(&p).len());
    let selected = select_costed(
        p.clone(),
        &crate::mir6502::Mir6502Config::optimized(),
        0x3000,
        false,
    )
    .unwrap();
    assert_eq!(selected, p);
}
