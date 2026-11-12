use super::*;
use crate::mir6502::{
    analysis::posthome::PostHomeAnalysisSnapshot, rewrite::context::PostHomeRewriteContext,
};
use std::sync::OnceLock;

pub(in crate::mir6502) fn baseline() -> MirProgram {
    static PROGRAM: OnceLock<MirProgram> = OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            let source = include_str!("../../../fixtures/runtime/word_load_sharing.act")
                .replace("\r\n", "\n");
            let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
            let model = crate::semantic::analyze_with_options(
                &ast,
                crate::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            let nir = crate::nir::optimize_program(&crate::nir::lower_program(
                &crate::semantic::ir::lower_program(&ast, &model),
            ))
            .unwrap();
            crate::mir6502::materialize_program_with_origin(
                crate::mir6502::lower_program(&nir).unwrap(),
                &crate::mir6502::Mir6502Config {
                    enable_word_load_sharing: false,
                    ..crate::mir6502::Mir6502Config::optimized()
                },
                0x3000,
            )
            .unwrap()
        })
        .clone()
}

pub(in crate::mir6502) fn memory(p: &MirProgram) -> MirPhysicalMemory {
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
    let summary = crate::mir6502::emit::emit_program_with_physical_references(
        p,
        0x3000,
        &mut emitter,
        true,
        p,
    )
    .unwrap();
    let mut m = summary.physical_memory;
    m.protect_execution_ranges(
        summary
            .source_ranges
            .iter()
            .map(|r| (r.start, r.end))
            .collect(),
    );
    m
}

pub(in crate::mir6502) fn start(r: &MirRoutine) -> MirSite {
    r.blocks
        .iter()
        .find_map(|b| {
            b.ops
                .windows(WINDOW)
                .position(|ops| shape(ops).is_ok())
                .map(|op_index| MirSite::Op {
                    block: b.id,
                    op_index,
                })
        })
        .unwrap()
}

pub(in crate::mir6502) fn window(r: &MirRoutine) -> &[MirOp] {
    let MirSite::Op { block, op_index } = start(r) else {
        unreachable!()
    };
    &r.blocks.iter().find(|b| b.id == block).unwrap().ops[op_index..op_index + WINDOW]
}

pub(in crate::mir6502) fn fact(r: &MirRoutine, m: &MirPhysicalMemory) -> Result<Proof, Blocker> {
    let snapshot = PostHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
    let c = PostHomeRewriteContext::new(&snapshot);
    c.word_load_sharing(c.point(start(r)), m)
}

fn remapped(r: &MirRoutine, s: &Shape, addresses: [u16; 3]) -> MirPhysicalMemory {
    let mut m = MirPhysicalMemory::default();
    m.protect_execution_ranges(vec![]);
    for (pair, address) in [&s.source, &s.first, &s.second].into_iter().zip(addresses) {
        m.own_data(address, 2);
        for (n, mem) in pair.iter().enumerate() {
            if let MirMem::Global { id, offset } = mem {
                m.global(
                    *id,
                    address.wrapping_add(n as u16).wrapping_sub(*offset),
                    2,
                    true,
                );
            } else {
                m.insert(r.id, mem.clone(), address.wrapping_add(n as u16), true);
            }
        }
    }
    m
}

#[test]
fn word_load_sharing_proves_both_neutral_arrangements() {
    let p = baseline();
    let m = memory(&p);
    for name in ["Unlink", "Reset"] {
        let r = p.routines.iter().find(|r| r.name == name).unwrap();
        let proof = fact(r, &m).unwrap();
        let original = window(r);
        let selected = replacement(original).unwrap();
        assert_eq!(selected.len(), 6);
        assert!(validate(
            &proof,
            r,
            MirRoutineGeneration::initial(),
            &m,
            original,
            &selected
        ));
        assert_eq!(selected.iter().filter(|op| store(op).is_some()).count(), 4);
    }
}

#[test]
fn word_load_sharing_rejects_named_physical_aliases_and_unowned_bytes() {
    let p = baseline();
    let r = p.routines.iter().find(|r| r.name == "Unlink").unwrap();
    let s = shape(window(r)).unwrap();
    for addresses in [
        [0x7000, 0x7000, 0xAC],
        [0x7000, 0x7001, 0xAC],
        [0x7001, 0x7000, 0xAC],
    ] {
        assert_eq!(fact(r, &remapped(r, &s, addresses)), Err(Blocker::Overlap));
    }
    assert_eq!(
        fact(r, &MirPhysicalMemory::default()),
        Err(Blocker::Placement)
    );
    let mut code = remapped(r, &s, [0x7000, 0x7100, 0xAC]);
    code.protect_execution_ranges(vec![(0x7001, 0x7002)]);
    assert_eq!(fact(r, &code), Err(Blocker::Storage));
    let mut unowned = MirPhysicalMemory::default();
    unowned.protect_execution_ranges(vec![]);
    for (pair, address) in [&s.source, &s.first].into_iter().zip([0x7000, 0x7100]) {
        for (i, mem) in pair.iter().enumerate() {
            unowned.insert(r.id, mem.clone(), address + i as u16, true);
        }
    }
    assert_eq!(fact(r, &unowned), Err(Blocker::Storage));
}

#[test]
fn word_load_sharing_proves_parameter_and_local_sources() {
    let p = baseline();
    for parameter in [false, true] {
        let mut r = p
            .routines
            .iter()
            .find(|r| r.name == "Unlink")
            .unwrap()
            .clone();
        let mut slot = r.frame.locals[0].clone();
        slot.id = MirStorageId(99);
        slot.offset = 0;
        slot.base = if parameter {
            MirStorageBase::Param(crate::nir::ParamId(99))
        } else {
            MirStorageBase::Local(crate::nir::LocalId(99))
        };
        let source = if parameter {
            r.frame.params.push(slot);
            MirMem::Param {
                id: crate::nir::ParamId(99),
                offset: 0,
            }
        } else {
            r.frame.locals.push(slot);
            MirMem::Local {
                id: crate::nir::LocalId(99),
                offset: 0,
            }
        };
        let MirSite::Op { block, op_index } = start(&r) else {
            unreachable!()
        };
        let ops = &mut r.blocks.iter_mut().find(|b| b.id == block).unwrap().ops;
        for i in [0, 2, 4, 6] {
            if let MirOp::Load { src, .. } = &mut ops[op_index + i] {
                *src = MirAddr::Direct(if i == 0 || i == 4 {
                    source.clone()
                } else {
                    next(&source).unwrap()
                });
            }
        }
        let s = shape(window(&r)).unwrap();
        assert!(fact(&r, &remapped(&r, &s, [0x7000, 0x7100, 0xAC])).is_ok());
        let slots = if parameter {
            &mut r.frame.params
        } else {
            &mut r.frame.locals
        };
        slots
            .iter_mut()
            .find(|s| s.id == MirStorageId(99))
            .unwrap()
            .storage_size = 1;
        assert_eq!(
            fact(&r, &remapped(&r, &s, [0x7000, 0x7100, 0xAC])),
            Err(Blocker::Storage)
        );
    }
}

#[test]
fn word_load_sharing_rejects_unsupported_cells_shapes_and_footprints() {
    let p = baseline();
    let base = p.routines.iter().find(|r| r.name == "Unlink").unwrap();
    let MirSite::Op { block, op_index } = start(base) else {
        unreachable!()
    };
    for address in [0xA0, 0xA2, 0xA4, 0xE0, 0xFE] {
        let mut r = base.clone();
        let ops = &mut r.blocks.iter_mut().find(|b| b.id == block).unwrap().ops;
        for (i, lane) in [(5, 0), (7, 1)] {
            if let MirOp::Store { dst, .. } = &mut ops[op_index + i] {
                *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(address + lane)));
            }
        }
        let s = shape(window(&r)).unwrap();
        assert_eq!(
            fact(&r, &remapped(&r, &s, [0x7000, 0x7100, u16::from(address)])),
            Err(Blocker::Storage)
        );
    }
    let s = shape(window(base)).unwrap();
    let mut partial = remapped(base, &s, [0x7000, 0x7100, 0xAC]);
    if let MirMem::Global { id, .. } = &s.source[0] {
        partial.global(*id, 0x7000, 1, true);
    }
    assert!(fact(base, &partial).is_err());
    let mut ops = window(base).to_vec();
    ops[4] = ops[2].clone();
    assert_eq!(shape(&ops), Err(Blocker::Shape));
    ops.insert(
        1,
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 1,
            width: MirWidth::Byte,
        },
    );
    assert_eq!(shape(&ops), Err(Blocker::Shape));
}

#[test]
fn word_load_sharing_revalidates_ownership_generation_and_replacement() {
    let p = baseline();
    let m = memory(&p);
    let r = p.routines.iter().find(|r| r.name == "Unlink").unwrap();
    let proof = fact(r, &m).unwrap();
    let original = window(r);
    let selected = replacement(original).unwrap();
    assert!(!validate(
        &proof,
        r,
        MirRoutineGeneration::initial().next(),
        &m,
        original,
        &selected
    ));
    let mut owner = r.clone();
    owner.id = RoutineId(99);
    assert!(!validate(
        &proof,
        &owner,
        MirRoutineGeneration::initial(),
        &m,
        original,
        &selected
    ));
    assert!(!validate(
        &proof,
        r,
        MirRoutineGeneration::initial(),
        &remapped(r, &proof.shape, [0x7000, 0x7100, 0xAC]),
        original,
        &selected
    ));
    let mut wrong = selected.clone();
    wrong.swap(1, 2);
    assert!(!validate(
        &proof,
        r,
        MirRoutineGeneration::initial(),
        &m,
        original,
        &wrong
    ));
}

#[test]
fn word_load_sharing_rejects_every_pair_overlap_and_wrapping_footprint() {
    let p = baseline();
    let mut r = p
        .routines
        .iter()
        .find(|r| r.name == "Unlink")
        .unwrap()
        .clone();
    let mut slot = r.frame.locals[0].clone();
    slot.id = MirStorageId(99);
    slot.base = MirStorageBase::Local(crate::nir::LocalId(99));
    slot.offset = 0;
    r.frame.locals.push(slot);
    let MirSite::Op { block, op_index } = start(&r) else {
        unreachable!()
    };
    let ops = &mut r.blocks.iter_mut().find(|b| b.id == block).unwrap().ops;
    for (i, lane) in [(5, 0), (7, 1)] {
        if let MirOp::Store { dst, .. } = &mut ops[op_index + i] {
            *dst = MirAddr::Direct(MirMem::Local {
                id: crate::nir::LocalId(99),
                offset: lane,
            });
        }
    }
    let s = shape(window(&r)).unwrap();
    for addresses in [
        [0x7000, 0x7100, 0x7100],
        [0x7000, 0x7100, 0x7101],
        [0x7000, 0x7101, 0x7100],
        [0x7000, 0x7100, 0x7000],
        [0x7000, 0x7100, 0x7001],
        [0x7001, 0x7100, 0x7000],
    ] {
        assert_eq!(
            fact(&r, &remapped(&r, &s, addresses)),
            Err(Blocker::Overlap),
            "{addresses:?}"
        );
    }
    assert_eq!(
        fact(&r, &remapped(&r, &s, [0xFFFF, 0x7100, 0x7200])),
        Err(Blocker::Footprint)
    );
    for i in 0..3 {
        let mut m = remapped(&r, &s, [0x7000, 0x7100, 0x7200]);
        let base = [0x7000, 0x7100, 0x7200][i];
        m.protect_execution_ranges(vec![(base + 1, base + 2)]);
        assert_eq!(fact(&r, &m), Err(Blocker::Storage));
    }
    let mut m = remapped(&r, &s, [0xD000, 0x7100, 0x7200]);
    assert_eq!(fact(&r, &m), Err(Blocker::Storage));
    if let MirMem::Global { id, .. } = s.source[0] {
        m.global(id, 0x7000, 2, false);
    }
    assert_eq!(fact(&r, &m), Err(Blocker::Storage));
}

#[test]
fn word_load_sharing_requires_owned_private_bytes_and_exact_register_operations() {
    let p = baseline();
    let reset = p.routines.iter().find(|r| r.name == "Reset").unwrap();
    let mut r = reset.clone();
    let s = shape(window(&r)).unwrap();
    r.frame.virtual_zero_page.clear();
    r.frame.zero_page_allocations.clear();
    assert_eq!(fact(&r, &memory(&p)), Err(Blocker::Storage));
    let mut raw = reset.clone();
    let MirSite::Op { block, op_index } = start(&raw) else {
        unreachable!()
    };
    for (i, lane) in [(0, 0), (2, 1), (4, 0), (6, 1)] {
        if let MirOp::Load { src, .. } =
            &mut raw.blocks.iter_mut().find(|b| b.id == block).unwrap().ops[op_index + i]
        {
            *src = MirAddr::Direct(MirMem::Absolute(0x7000 + lane));
        }
    }
    let sr = shape(window(&raw)).unwrap();
    assert_eq!(
        fact(&raw, &remapped(&raw, &sr, [0x7000, 0xE0, 0x7100])),
        Err(Blocker::Storage)
    );
    for index in 0..WINDOW {
        let mut ops = window(reset).to_vec();
        match &mut ops[index] {
            MirOp::Load { dst, .. } => *dst = MirDef::Reg(MirReg::X),
            MirOp::Store { src, .. } => *src = MirValue::Def(MirDef::Reg(MirReg::Y)),
            _ => unreachable!(),
        }
        assert_eq!(shape(&ops), Err(Blocker::Shape));
    }
    let call = p
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .find(|op| matches!(op, MirOp::Call { .. }))
        .unwrap();
    let mut ops = window(reset).to_vec();
    ops[3] = call.clone();
    assert_eq!(shape(&ops), Err(Blocker::Shape));
    assert!(destination(reset, &s.first, &memory(&p), 0xE0));
}

#[test]
fn word_load_sharing_schedule_matches_all_word_values_and_status_bits() {
    let p = baseline();
    let r = p.routines.iter().find(|r| r.name == "Unlink").unwrap();
    let original = window(r);
    let selected = replacement(original).unwrap();
    let s = shape(original).unwrap();
    let homes = s
        .source
        .iter()
        .chain(&s.first)
        .chain(&s.second)
        .collect::<Vec<_>>();
    let execute = |ops: &[MirOp], mut cells: [u8; 6], mut status: u8| {
        let mut a = 0xA7;
        for op in ops {
            if let Some(mem) = load(op) {
                a = cells[homes.iter().position(|h| *h == mem).unwrap()];
                status = (status & !0x82) | (a & 0x80) | if a == 0 { 2 } else { 0 };
            } else {
                let mem = store(op).unwrap();
                cells[homes.iter().position(|h| *h == mem).unwrap()] = a;
            }
        }
        (cells, a, status)
    };
    for value in 0u16..=u16::MAX {
        let [lo, hi] = value.to_le_bytes();
        for status in [0, 0xFF, 0x49, 0xB6] {
            let input = [lo, hi, 0x37, 0xA5, 0x19, 0xC6];
            let expected = (
                [lo, hi, lo, hi, lo, hi],
                hi,
                (status & !0x82) | (hi & 0x80) | if hi == 0 { 2 } else { 0 },
            );
            assert_eq!(execute(original, input, status), expected);
            assert_eq!(execute(&selected, input, status), expected);
        }
    }
}

#[test]
fn word_load_sharing_accepts_owned_ram_spills_and_ordinary_global_destinations() {
    let p = baseline();
    for globals in [false, true] {
        let mut r = p
            .routines
            .iter()
            .find(|r| r.name == "Unlink")
            .unwrap()
            .clone();
        let MirSite::Op { block, op_index } = start(&r) else {
            unreachable!()
        };
        if globals {
            let id = p
                .globals
                .iter()
                .find(|g| g.name == "startAddress")
                .unwrap()
                .id;
            for (i, lane) in [(5, 0), (7, 1)] {
                if let MirOp::Store { dst, .. } =
                    &mut r.blocks.iter_mut().find(|b| b.id == block).unwrap().ops[op_index + i]
                {
                    *dst = MirAddr::Direct(MirMem::Global { id, offset: lane });
                }
            }
        } else {
            r.frame.spills.extend([MirSpillId(90), MirSpillId(91)]);
            for (i, id) in [(1, 90), (3, 91)] {
                if let MirOp::Store { dst, .. } =
                    &mut r.blocks.iter_mut().find(|b| b.id == block).unwrap().ops[op_index + i]
                {
                    *dst = MirAddr::Direct(MirMem::Spill {
                        id: MirSpillId(id),
                        offset: 0,
                    });
                }
            }
        }
        let s = shape(window(&r)).unwrap();
        let m = remapped(
            &r,
            &s,
            [0x7000, 0x7100, if globals { 0x7200 } else { 0xAC }],
        );
        assert!(fact(&r, &m).is_ok());
        if !globals {
            r.frame.spills.clear();
            assert_eq!(fact(&r, &m), Err(Blocker::Storage));
        }
    }
}
