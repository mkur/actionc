use super::*;
use crate::mir6502::{
    Mir6502Config,
    analysis::{posthome::PostHomeAnalysisSnapshot, sites::MirRoutineGeneration},
};
use std::sync::OnceLock;

pub(in crate::mir6502) fn baseline() -> MirProgram {
    static PROGRAM: OnceLock<MirProgram> = OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            let text = include_str!("../../../fixtures/runtime/pointer_byte_plus_one.act")
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
                &Mir6502Config {
                    enable_pointer_byte_plus_one_fusion: false,
                    ..Mir6502Config::optimized()
                },
                0x3000,
            )
            .unwrap()
        })
        .clone()
}

fn forward() -> MirRoutine {
    baseline()
        .routines
        .into_iter()
        .find(|r| r.name == "Forward")
        .unwrap()
}
fn site(r: &MirRoutine) -> MirSite {
    let b = &r.blocks[0];
    MirSite::Op {
        block: b.id,
        op_index: b
            .ops
            .iter()
            .position(|o| matches!(o, MirOp::OffsetPointerByIndirectByte { .. }))
            .unwrap(),
    }
}
fn fact(r: &MirRoutine) -> Result<Proof, Blocker> {
    let snap = PostHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
    let ctx = PostHomeRewriteContext::new(&snap);
    ctx.pointer_byte_plus_one(ctx.point(site(r)))
}

#[test]
fn pointer_byte_plus_one_facts_identify_the_neutral_pair_and_controls() {
    let p = baseline();
    let r = p.routines.iter().find(|r| r.name == "Forward").unwrap();
    let f = fact(r).unwrap();
    assert_eq!(f.routine, r.id);
    assert_eq!(f.shape.dst, MirFixedZpSlot(0xAC));
    for name in ["PlusZero", "PlusTwo", "Subtract", "Address"] {
        assert_eq!(
            fact(p.routines.iter().find(|r| r.name == name).unwrap()),
            Err(Blocker::Shape)
        );
    }
}

#[test]
fn pointer_byte_plus_one_facts_reject_live_machine_state_and_incomplete_source() {
    for reg in [MirReg::A, MirReg::X] {
        let mut r = forward();
        let MirSite::Op { op_index, .. } = site(&r) else {
            unreachable!()
        };
        r.blocks[0].ops.insert(
            op_index + 2,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x680)),
                src: MirValue::Def(MirDef::Reg(reg)),
                width: MirWidth::Byte,
            },
        );
        assert_eq!(fact(&r), Err(Blocker::MachineState));
    }
    let mut r = forward();
    let i = r.blocks[0]
        .ops
        .iter()
        .position(|o| {
            matches!(
                o,
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAF))),
                    ..
                }
            )
        })
        .unwrap();
    r.blocks[0].ops.remove(i);
    assert_eq!(fact(&r), Err(Blocker::Source));
    r.blocks[0].ops.insert(
        i,
        MirOp::Barrier {
            effects: MirEffects {
                memory_writes: MirMemoryEffect::Regions(vec![MirMemoryRegion {
                    kind: MirMemoryRegionKind::ZeroPage,
                    offset: 0xAF,
                    size: 1,
                }]),
                ..MirEffects::default()
            },
        },
    );
    assert_eq!(
        fact(&r),
        Err(Blocker::Source),
        "a declared may-write is not initialization"
    );
    r.blocks[0].ops[i] = MirOp::AddByteToWordMem {
        mem: MirMem::FixedZeroPage(MirFixedZpSlot(0xAE)),
        value: MirValue::ConstU8(0),
    };
    assert_eq!(
        fact(&r),
        Err(Blocker::Source),
        "conditional high-byte repair is not initialization"
    );
}

#[test]
fn pointer_byte_plus_one_facts_reject_pair_overlap_wrap_offsets_and_stale_points() {
    let r = forward();
    let f = fact(&r).unwrap();
    for dst in [0xAD, 0xAE, 0xAF, 0xFF] {
        assert!(!valid_pairs(MirFixedZpSlot(dst), f.shape.source, 0));
    }
    assert!(!valid_pairs(f.shape.dst, f.shape.source, 256));
    for offset in [0, 1, 255] {
        assert!(valid_pairs(f.shape.dst, f.shape.source, offset));
    }
    let snap = PostHomeAnalysisSnapshot::new(&r, MirRoutineGeneration::initial().next()).unwrap();
    assert_eq!(
        PostHomeRewriteContext::new(&snap).pointer_byte_plus_one(f.point),
        Err(Blocker::Site)
    );
}

#[test]
fn pointer_byte_plus_one_incoming_pair_requires_a_definition_on_both_paths() {
    for both in [false, true] {
        let mut r = forward();
        let mut original = r.blocks.remove(0);
        let MirSite::Op { op_index, .. } = site(&MirRoutine {
            blocks: vec![original.clone()],
            ..r.clone()
        }) else {
            unreachable!()
        };
        let tail = original.ops.split_off(op_index);
        let high = original.ops.pop().unwrap();
        assert!(matches!(
            high,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAF))),
                ..
            }
        ));
        original.terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::CSet),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        };
        let block = |id, ops, terminator| MirBlock {
            id: MirBlockId(id),
            label: format!("proof{id}"),
            params: vec![],
            ops,
            terminator,
        };
        r.blocks = vec![
            original,
            block(
                1,
                vec![high.clone()],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            ),
            block(
                2,
                if both { vec![high] } else { vec![] },
                MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            ),
            block(3, tail, MirTerminator::Return),
        ];
        let snap = PostHomeAnalysisSnapshot::new(&r, MirRoutineGeneration::initial()).unwrap();
        let ctx = PostHomeRewriteContext::new(&snap);
        let f = ctx.pointer_byte_plus_one(ctx.point(MirSite::Op {
            block: MirBlockId(3),
            op_index: 0,
        }));
        assert_eq!(f.is_ok(), both, "{f:?}");
    }
}
