use super::*;
use crate::mir6502::analysis::{
    known_callees::MirKnownCalleeSummaries, posthome::PostHomeAnalysisSnapshot,
};
use std::sync::OnceLock;
pub(in crate::mir6502) fn baseline() -> MirProgram {
    static P: OnceLock<MirProgram> = OnceLock::new();
    P.get_or_init(|| {
        let text =
            include_str!("../../../fixtures/runtime/argument_x_bitwise.act").replace("\r\n", "\n");
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
            &crate::mir6502::Mir6502Config { enable_argument_x_bitwise_forwarding:false, ..crate::mir6502::Mir6502Config::optimized() },
            0x3000,
        )
        .unwrap()
    })
    .clone()
}
#[test]
fn argument_x_bitwise_classifies_only_byte_immediates_in_a() {
    for op in [MirBinaryOp::And, MirBinaryOp::Or, MirBinaryOp::Xor] {
        for value in [0, 1, 127, 128, 254, 255] {
            let good = MirOp::Binary {
                op,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(value),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            };
            assert!(immediate_bitwise(&good));
            for change in 0..6 {
                let mut bad = good.clone();
                let MirOp::Binary {
                    ref mut dst,
                    ref mut left,
                    ref mut right,
                    ref mut width,
                    ref mut carry_in,
                    ref mut op,
                    ..
                } = bad
                else {
                    unreachable!()
                };
                match change {
                    0 => *dst = MirDef::Reg(MirReg::X),
                    1 => *left = MirValue::Def(MirDef::Reg(MirReg::X)),
                    2 => *right = MirValue::ConstU16(value.into()),
                    3 => *width = MirWidth::Word,
                    4 => *carry_in = Some(MirCarryIn::FromPrevious),
                    _ => *op = MirBinaryOp::Lsh,
                }
                assert!(!immediate_bitwise(&bad));
            }
        }
    }
}
#[test]
fn argument_x_bitwise_baseline_has_three_typed_gap_blockers() {
    let p = baseline();
    let memory = super::tests::memory(&p);
    let sums = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    for name in ["ForwardAnd", "ForwardOr", "ForwardXor"] {
        let r = p.routines.iter().find(|r| r.name == name).unwrap();
        let site = super::tests::site(r);
        let snap = PostHomeAnalysisSnapshot::new_with_known_callees(
            r,
            MirRoutineGeneration::initial(),
            &sums,
        )
        .unwrap();
        let c = PostHomeRewriteContext::new(&snap);
        assert_eq!(
            c.argument_x_forwarding(c.point(site), &memory),
            Err(Blocker::Gap)
        );
        assert_eq!(
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|o| immediate_bitwise(o))
                .count(),
            1
        );
    }
}
