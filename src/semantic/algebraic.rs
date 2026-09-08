//! Independent acceptance gates for the algebraic-type implementation slices.
//! Public profiles enable a gate only after its producer and consumer paths
//! have been verified together. These are not source-language feature switches.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AlgebraicTypeCapabilities {
    pub aggregate_values: bool,
    pub variants: bool,
    pub aggregate_calls: bool,
    pub indirect_aggregate_calls: bool,
    pub generic_types: bool,
    pub case_guards: bool,
}

impl AlgebraicTypeCapabilities {
    pub const DISABLED: Self = Self {
        aggregate_values: false,
        variants: false,
        aggregate_calls: false,
        indirect_aggregate_calls: false,
        generic_types: false,
        case_guards: false,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::SemanticOptions;
    use crate::target::TargetId;

    #[test]
    fn only_verified_algebraic_capabilities_are_open_in_public_profiles() {
        for target in [
            TargetId::Atari6502,
            TargetId::Wdc65816Native,
            TargetId::Wdc65816Small,
            TargetId::Motorola68000,
        ] {
            assert_eq!(SemanticOptions::default().with_target(target).algebraic_types,
                AlgebraicTypeCapabilities::DISABLED);
            assert_eq!(SemanticOptions::modern().with_target(target).algebraic_types,
                AlgebraicTypeCapabilities { aggregate_values: true, variants: true, aggregate_calls: true, indirect_aggregate_calls: true, ..AlgebraicTypeCapabilities::DISABLED });
        }
    }

    #[test]
    fn enabling_aggregate_values_does_not_enable_unfinished_consumers() {
        let capabilities = AlgebraicTypeCapabilities {
            aggregate_values: true,
            ..AlgebraicTypeCapabilities::DISABLED
        };
        assert!(!capabilities.variants);
        assert!(!capabilities.aggregate_calls);
        assert!(!capabilities.indirect_aggregate_calls);
        assert!(!capabilities.generic_types);
        assert!(!capabilities.case_guards);
    }
}
