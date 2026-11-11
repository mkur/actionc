use super::*;
use crate::mir6502::analysis::{
    known_callees::MirKnownCalleeSummaries, posthome::PostHomeAnalysisSnapshot,
};
use std::sync::OnceLock;
pub(in crate::mir6502) fn baseline() -> MirProgram {
    static P: OnceLock<MirProgram> = OnceLock::new();
    P.get_or_init(|| {
        let text = include_str!("../../../fixtures/runtime/argument_x_forwarding.act")
            .replace("\r\n", "\n");
        let ast = crate::parser::parse(&crate::lexer::tokenize(&text).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        crate::mir6502::materialize_program_with_origin(
            crate::mir6502::lower_program(&nir).unwrap(),
            &crate::mir6502::Mir6502Config {
                enable_argument_x_forwarding: false,
                ..crate::mir6502::Mir6502Config::optimized()
            },
            0x3000,
        )
        .unwrap()
    })
    .clone()
}
pub(in crate::mir6502) fn memory(p: &MirProgram) -> MirPhysicalMemory {
    let mut e = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
    crate::mir6502::emit::emit_program_with_physical_references(p, 0x3000, &mut e, true, p)
        .unwrap()
        .physical_memory
}
pub(in crate::mir6502) fn site(r: &MirRoutine) -> MirSite {
    r.blocks
        .iter()
        .flat_map(|b| {
            (0..b.ops.len()).map(move |i| MirSite::Op {
                block: b.id,
                op_index: i,
            })
        })
        .find(|s| shape(r, *s).is_ok())
        .unwrap()
}
pub(in crate::mir6502) fn fact(
    r: &MirRoutine,
    m: &MirPhysicalMemory,
    callees: &MirKnownCalleeSummaries,
) -> Result<Proof, Blocker> {
    let snap = PostHomeAnalysisSnapshot::new_with_known_callees(
        r,
        MirRoutineGeneration::initial(),
        callees,
    )
    .unwrap();
    let c = PostHomeRewriteContext::new(&snap);
    c.argument_x_forwarding(c.point(site(r)), m)
}
#[test]
fn argument_x_forwarding_proof_admits_fixed_private_and_parameter_homes() {
    let p = baseline();
    let m = memory(&p);
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    for (name, restored) in [
        ("Forward", true),
        ("ForwardByte", false),
        ("ForwardPrivate", true),
    ] {
        let r = p.routines.iter().find(|r| r.name == name).unwrap();
        let proof = fact(r, &m, &callees).unwrap();
        assert_eq!(proof.tail_restores_nz, restored);
        assert_eq!(
            replacement(r, &proof.shape).unwrap().len(),
            proof.shape.range().len()
        );
    }
    let r = p.routines.iter().find(|r| r.name == "Plain").unwrap();
    assert!(!r.blocks.iter().any(|b| (0..b.ops.len()).any(|i| {
        shape(
            r,
            MirSite::Op {
                block: b.id,
                op_index: i,
            },
        )
        .is_ok()
    })));
}
#[test]
fn argument_x_forwarding_proof_rejects_x_uses_clobbers_and_physical_aliases() {
    let p = baseline();
    let m = memory(&p);
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let base = p.routines.iter().find(|r| r.name == "Forward").unwrap();
    let i = shape(base, site(base)).unwrap().range().start;
    for (op, reason) in [
        (
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
                src: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
            },
            Blocker::XRead,
        ),
        (
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::X),
                value: 1,
                width: MirWidth::Byte,
            },
            Blocker::XClobber,
        ),
    ] {
        let mut r = base.clone();
        r.blocks[0].ops.insert(i + 1, op);
        assert_eq!(fact(&r, &m, &callees), Err(reason));
    }
    let mut r = base.clone();
    let q = MirMem::Param {
        id: crate::nir::ParamId(1),
        offset: 0,
    };
    r.blocks[0].ops.insert(
        i + 1,
        MirOp::Store {
            dst: MirAddr::Direct(q.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    );
    let mut aliases = MirPhysicalMemory::default();
    aliases.insert(r.id, q, 0xA1, true);
    assert_eq!(fact(&r, &aliases, &callees), Err(Blocker::Alias));
}
#[test]
fn argument_x_forwarding_proof_checks_both_nz_boundaries_and_call_facts() {
    let p = baseline();
    let m = memory(&p);
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let base = p.routines.iter().find(|r| r.name == "Forward").unwrap();
    let s = shape(base, site(base)).unwrap();
    let i = s.range().start;
    let end = s.range().end;
    let mut r = base.clone();
    r.blocks[0].ops.insert(
        i + 1,
        MirOp::Barrier {
            effects: MirEffects {
                reads: MirRegisterSet {
                    flags: true,
                    ..MirRegisterSet::default()
                },
                ..MirEffects::default()
            },
        },
    );
    let snap = PostHomeAnalysisSnapshot::new_with_known_callees(
        &r,
        MirRoutineGeneration::initial(),
        &callees,
    )
    .unwrap();
    let c = PostHomeRewriteContext::new(&snap);
    assert!(c.argument_x_forwarding(c.point(s.store), &m).is_err());
    let mut r = base.clone();
    r.blocks[0].ops.remove(end);
    assert_eq!(fact(&r, &m, &callees), Err(Blocker::CallFlags)); // No tail and no supplied scalar entry fact.
    assert_eq!(
        fact(&r, &m, &MirKnownCalleeSummaries::default()),
        Err(Blocker::CallFlags)
    );
    let byte = p.routines.iter().find(|r| r.name == "ForwardByte").unwrap();
    assert_eq!(
        fact(byte, &m, &MirKnownCalleeSummaries::default()),
        Err(Blocker::CallFlags)
    );
    let mut r = base.clone();
    let call = shape(&r, site(&r)).unwrap().call;
    let MirSite::Op { op_index, .. } = call else {
        unreachable!()
    };
    if let MirOp::Call { effects, .. } = &mut r.blocks[0].ops[op_index] {
        effects.reads.flags = true;
    }
    assert!(fact(&r, &m, &callees).is_ok()); // Its unchanged Y load restores N/Z before the machine input.
}
#[test]
fn argument_x_forwarding_proof_revalidates_owner_generation_and_replacement() {
    let p = baseline();
    let m = memory(&p);
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let r = p.routines.iter().find(|r| r.name == "Forward").unwrap();
    let proof = fact(r, &m, &callees).unwrap();
    let snap = PostHomeAnalysisSnapshot::new_with_known_callees(
        r,
        MirRoutineGeneration::initial(),
        &callees,
    )
    .unwrap();
    let c = PostHomeRewriteContext::new(&snap);
    let selected = replacement(r, &proof.shape).unwrap();
    assert!(validate(&proof, r, &c, &m, proof.shape.range(), &selected));
    let mut wrong = selected.clone();
    wrong.swap(0, 1);
    assert!(!validate(&proof, r, &c, &m, proof.shape.range(), &wrong));
    let mut stale = proof.clone();
    stale.point.generation = stale.point.generation.next();
    assert!(!validate(&stale, r, &c, &m, stale.shape.range(), &selected));
    let mut wrong = proof.clone();
    wrong.routine = RoutineId(u32::MAX);
    assert!(!validate(&wrong, r, &c, &m, wrong.shape.range(), &selected));
}
