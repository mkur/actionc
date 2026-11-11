use super::*;
use crate::mir6502::{
    analysis::{posthome::PostHomeAnalysisSnapshot, sites::MirRoutineGeneration},
    rewrite::context::PostHomeRewriteContext,
};
use std::sync::OnceLock;

pub(in crate::mir6502) fn baseline() -> MirProgram {
    static PROGRAM: OnceLock<MirProgram> = OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            let text = include_str!("../../../fixtures/runtime/pointer_load_sharing.act")
                .replace("\r\n", "\n");
            let ast = crate::parser::parse(&crate::lexer::tokenize(&text).unwrap()).unwrap();
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
                &crate::mir6502::Mir6502Config::optimized(),
                0x3000,
            )
            .unwrap()
        })
        .clone()
}
pub(in crate::mir6502) fn memory(p: &MirProgram) -> MirPhysicalMemory {
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
    crate::mir6502::emit::emit_program_with_physical_references(p, 0x3000, &mut emitter, true, p)
        .unwrap()
        .physical_memory
}
pub(in crate::mir6502) fn start(r: &MirRoutine) -> usize {
    r.blocks[0]
        .ops
        .windows(WINDOW)
        .position(|ops| shape(ops).is_ok())
        .unwrap()
}
pub(in crate::mir6502) fn forward(p: &mut MirProgram) -> &mut MirRoutine {
    p.routines.iter_mut().find(|r| r.name == "Forward").unwrap()
}
pub(in crate::mir6502) fn fact(r: &MirRoutine, m: &MirPhysicalMemory) -> Result<Proof, Blocker> {
    let snapshot = PostHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
    let context = PostHomeRewriteContext::new(&snapshot);
    context.pointer_load_sharing(
        context.point(MirSite::Op {
            block: r.blocks[0].id,
            op_index: start(r),
        }),
        m,
    )
}

#[test]
fn pointer_load_sharing_proof_finds_the_neutral_miss_and_rejects_controls() {
    let mut p = baseline();
    let m = memory(&p);
    let r = forward(&mut p);
    let proof = fact(r, &m).unwrap();
    assert_eq!(proof.shape.capture, MirFixedZpSlot(0xA2));
    assert_eq!(proof.shape.pointer, MirFixedZpSlot(0xAC));
    let i = start(r);
    let original = &r.blocks[0].ops[i..i + WINDOW];
    let selected = replacement(original).unwrap();
    assert_eq!(selected.len(), 9);
    assert!(validate(
        &proof,
        r,
        MirRoutineGeneration::initial(),
        &m,
        original,
        &selected
    ));
    for name in ["PlusZero", "PlusTwo", "Subtract", "Offset", "Plain"] {
        let r = p.routines.iter().find(|r| r.name == name).unwrap();
        assert!(
            !r.blocks
                .iter()
                .any(|b| b.ops.windows(WINDOW).any(|ops| shape(ops).is_ok())),
            "{name}"
        );
    }
}

#[test]
fn pointer_load_sharing_proof_uses_physical_ranges_and_ordinary_storage() {
    let mut p = baseline();
    let m = memory(&p);
    let r = forward(&mut p);
    let proof = fact(r, &m).unwrap();
    for address in [0xA1u16, 0xA2, 0xA3, 0xAB, 0xAC, 0xAD] {
        let mut aliases = MirPhysicalMemory::default();
        for (n, mem) in proof.shape.source.iter().enumerate() {
            aliases.insert(r.id, mem.clone(), address + n as u16, true);
        }
        assert_eq!(fact(r, &aliases), Err(Blocker::Overlap), "{address:04X}");
    }
    assert_eq!(
        fact(r, &MirPhysicalMemory::default()),
        Err(Blocker::Placement)
    );
    for (address, normal) in [(0xD000, true), (0x7000, false)] {
        let mut other = MirPhysicalMemory::default();
        for (n, mem) in proof.shape.source.iter().enumerate() {
            other.insert(r.id, mem.clone(), address + n as u16, normal);
        }
        assert_eq!(fact(r, &other), Err(Blocker::Storage));
    }
}

#[test]
fn pointer_load_sharing_proof_rejects_changed_carry_consumer_and_partial_words() {
    let mut p = baseline();
    let m = memory(&p);
    let base = forward(&mut p).clone();
    let i = start(&base);
    for index in [1, 4] {
        let mut r = base.clone();
        if let MirOp::Binary { carry_in, .. } = &mut r.blocks[0].ops[i + index] {
            *carry_in = None;
        }
        assert_eq!(shape(&r.blocks[0].ops[i..i + WINDOW]), Err(Blocker::Shape));
    }
    let mut r = base.clone();
    if let MirOp::LoadIndirect { offset, .. } = &mut r.blocks[0].ops[i + 10] {
        *offset = 1;
    }
    assert_eq!(shape(&r.blocks[0].ops[i..i + WINDOW]), Err(Blocker::Shape));
    let mut r = base.clone();
    r.frame
        .params
        .iter_mut()
        .find(|s| s.name.as_deref() == Some("q"))
        .unwrap()
        .storage_size = 1;
    assert_eq!(fact(&r, &m), Err(Blocker::Storage));
    let mut ops = base.blocks[0].ops[i..i + WINDOW].to_vec();
    ops.insert(
        3,
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 0,
            width: MirWidth::Byte,
        },
    );
    assert_eq!(shape(&ops), Err(Blocker::Shape));
}

#[test]
fn pointer_load_sharing_proof_revalidates_generation_owner_layout_and_replacement() {
    let mut p = baseline();
    let m = memory(&p);
    let r = forward(&mut p);
    let proof = fact(r, &m).unwrap();
    let i = start(r);
    let original = &r.blocks[0].ops[i..i + WINDOW];
    let selected = replacement(original).unwrap();
    assert!(!validate(
        &proof,
        r,
        MirRoutineGeneration::initial().next(),
        &m,
        original,
        &selected
    ));
    let mut other = r.clone();
    other.id = RoutineId(r.id.0 + 1);
    assert!(!validate(
        &proof,
        &other,
        MirRoutineGeneration::initial(),
        &m,
        original,
        &selected
    ));
    let mut moved = MirPhysicalMemory::default();
    for (n, mem) in proof.shape.source.iter().enumerate() {
        moved.insert(r.id, mem.clone(), 0x7000 + n as u16, true);
    }
    assert!(!validate(
        &proof,
        r,
        MirRoutineGeneration::initial(),
        &moved,
        original,
        &selected
    ));
    let mut wrong = selected.clone();
    wrong.swap(1, 3);
    assert!(!validate(
        &proof,
        r,
        MirRoutineGeneration::initial(),
        &m,
        original,
        &wrong
    ));
}
