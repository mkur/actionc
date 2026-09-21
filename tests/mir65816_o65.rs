use actionc::{
    lexer,
    mir65816::{self, o65, relocation},
    nir, parser, semantic,
    target::TargetId,
};
fn mir(source: &str, optimize: bool) -> mir65816::Mir65816Program {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
    )
    .unwrap();
    let n = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let n = if optimize {
        nir::optimize_program_with_promotion(&n, nir::NirPromotionPolicy::Native65816).unwrap()
    } else {
        n
    };
    mir65816::lower_program(&n).unwrap()
}
#[test]
fn retains_typed_targets_and_builds_independent_sections() {
    for optimize in [false, true] {
        let program = mir(
            "CARD output,initial=[7] BYTE ARRAY table=[1 2 3] CARD FUNC Add(CARD n) RETURN(n+1) PROC Main() output=Add(initial)+CARD(table(1)) RETURN",
            optimize,
        );
        let machine = mir65816::emit::materialize(&program).unwrap();
        let pending = relocation::collect(&program, &machine).unwrap();
        assert!(
            pending
                .iter()
                .any(|f| matches!(f.target, relocation::Target::StackOverflow))
        );
        assert!(
            pending
                .iter()
                .any(|f| matches!(f.target, relocation::Target::Data(_)))
        );
        let artifact = o65::prepare(&program, &Default::default()).unwrap();
        assert_eq!(artifact.profile().imports[0].name, o65::profile::OVERFLOW);
        assert!(artifact.section_sizes().iter().all(|s| *s > 0));
        assert_eq!(artifact.profile().routines.len(), 2);
        assert!(
            artifact
                .profile()
                .relocations
                .iter()
                .any(|r| matches!(r.target, o65::profile::Reference::Import(0)))
        );
    }
}
#[test]
fn rejects_overlapping_fixups_and_alias_cycles() {
    let mut program = mir("CARD value PROC Main() value=7 RETURN", false);
    let mut machine = mir65816::emit::materialize(&program).unwrap();
    let f = machine.routines[0].code.fixups[0].clone();
    machine.routines[0].code.fixups.push(f);
    assert!(
        relocation::collect(&program, &machine)
            .unwrap_err()
            .contains("overlapping")
    );
    let d = &mut program.data[0];
    d.placement = mir65816::Mir65816DataPlacement::Alias {
        target: d.id,
        offset: actionc::target::ByteOffset::ZERO,
    };
    assert!(o65::prepare(&program, &Default::default()).is_err());
}
#[test]
fn rejects_unsupported_profiles_and_unchecked_bindings() {
    let p = mir("PROC Main() RETURN", false);
    let mut options = o65::Options::default();
    options.profile = "future".into();
    assert!(o65::prepare(&p, &options).unwrap_err().contains("profile"));
    options.profile = o65::profile::ID.into();
    options.imports.push(o65::Binding {
        symbol: 1,
        name: "Host".into(),
        stack_peak: 0,
        checks_stack: false,
        domains: 3,
        irq_effect: Default::default(),
    });
    assert!(
        o65::prepare(&p, &options)
            .unwrap_err()
            .contains("unchecked")
    );
}
