//! Union views retain ordinary typed places and conservative aggregate homes.
use actionc::{
    lexer::tokenize,
    nir,
    parser::parse,
    semantic::{self, SemanticOptions},
    target::TargetId,
};

#[test]
fn union_member_places_keep_offsets_widths_and_addressable_homes_on_all_targets() {
    let source = "TYPE View=UNION [BYTE low CARD word INT signedWord LONGCARD wide BYTE POINTER link] \
        BYTE result PROC Main() View value BYTE POINTER ptr \
        value.wide=$12345678 value.low=7 value.word==+1 ptr=@value.low ptr^=9 \
        result=value.signedWord<0 RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let mut options = SemanticOptions::modern().with_target(target);
        options.algebraic_types.unions = true;
        let model = semantic::analyze_with_options(&ast, options).unwrap();
        let raw = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        nir::verify_program(&raw).unwrap();
        let analysis = nir::analyze_program_storage(&raw);
        let home = analysis
            .routines
            .iter()
            .find_map(|r| r.storage_by_name("value"))
            .unwrap();
        assert!(!home.is_promotable());
        assert!(home.requires_addressable_home());
        assert!(!home.is_value_trackable());
        let mut widths = std::collections::BTreeSet::new();
        for op in raw
            .routines
            .iter()
            .flat_map(|r| &r.blocks)
            .flat_map(|b| &b.ops)
        {
            if let nir::NirOp::Store { place, .. } | nir::NirOp::Load { place, .. } = op {
                if let nir::NirPlaceKind::Field { offset, ty, .. } = &place.kind {
                    assert_eq!(offset.get(), 0);
                    widths.insert(ty.width.unwrap().get());
                }
            }
        }
        assert!(
            widths.is_superset(&[1, 2, 4].into_iter().collect()),
            "{widths:?}"
        );
        let optimized = nir::optimize_program(&raw).unwrap();
        nir::verify_program(&optimized).unwrap();
        match target {
            TargetId::Atari6502 => {
                actionc::mir6502::lower_program(&optimized).unwrap();
            }
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(&optimized).unwrap();
            }
            _ => {
                actionc::mir65816::lower_program(&optimized).unwrap();
            }
        }
    }
}

#[test]
fn union_values_do_not_gain_scalar_operations_or_cross_nominal_assignments() {
    for statement in [
        "a=b",
        "n=a",
        "n=a+1",
        "n=-a",
        "n=a=b",
        "n=CARD(a)",
        "IF a THEN RETURN FI",
        "CASE a OF\nWHEN 0 THEN\nRETURN\nESAC",
    ] {
        let source = format!(
            "TYPE First=UNION [CARD word] TYPE Second=UNION [CARD word] First a Second b CARD n PROC Main()\n{statement}\nRETURN"
        );
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let mut options = SemanticOptions::modern();
        options.algebraic_types.unions = true;
        assert!(
            semantic::analyze_with_options(&ast, options).is_err(),
            "{statement}"
        );
    }
}
