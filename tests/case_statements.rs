use actionc::semantic::SemanticOptions;
use actionc::target::TargetId;

#[test]
fn incomplete_language_capabilities_are_independent_and_not_public() {
    for options in [SemanticOptions::default(), SemanticOptions::modern()] {
        for target in [
            TargetId::Atari6502,
            TargetId::Wdc65816Small,
            TargetId::Wdc65816Native,
            TargetId::Motorola68000,
        ] {
            let options = options.with_target(target);
            assert!(!options.case_statements);
            assert!(!options.enum_types);
            let cases = SemanticOptions {
                case_statements: true,
                ..options
            };
            assert!(!cases.enum_types);
            let enums = SemanticOptions {
                enum_types: true,
                ..options
            };
            assert!(!enums.case_statements);
        }
    }
}
