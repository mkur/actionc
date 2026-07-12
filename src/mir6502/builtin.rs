#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MirBuiltinResolution {
    Resolved {
        address: u16,
    },
    #[allow(dead_code)]
    Deferred {
        reason: &'static str,
    },
    Unsupported {
        reason: &'static str,
    },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MirBuiltinEntry {
    pub name: &'static str,
    pub resolution: MirBuiltinResolution,
}

pub(super) const BUILTIN_TARGETS: &[MirBuiltinEntry] = &[
    resolved("Print", 0xA47F),
    resolved("PrintE", 0xA46C),
    resolved("PrintD", 0xA486),
    resolved("PrintDE", 0xA473),
    resolved("PrintB", 0xA4E4),
    resolved("PrintBE", 0xA4EC),
    resolved("PrintBD", 0xA4F4),
    resolved("PrintBDE", 0xA508),
    resolved("PrintC", 0xA4E6),
    resolved("PrintCE", 0xA4EE),
    resolved("PrintCD", 0xA4F6),
    resolved("PrintCDE", 0xA50A),
    resolved("PrintI", 0xA512),
    resolved("PrintIE", 0xA536),
    resolved("PrintID", 0xA519),
    resolved("PrintIDE", 0xA53C),
    unsupported("PrintH", "resident entry point is not modeled"),
    resolved("PrintF", 0xA3CC),
    resolved("Put", 0xA4CE),
    resolved("PutE", 0xA4CC),
    resolved("PutD", 0xA4D1),
    resolved("PutDE", 0xA4DA),
    resolved("InputB", 0xA588),
    resolved("InputBD", 0xA58A),
    resolved("InputC", 0xA588),
    resolved("InputCD", 0xA58A),
    resolved("InputI", 0xA588),
    resolved("InputID", 0xA58A),
    resolved("InputS", 0xA48C),
    resolved("InputSD", 0xA493),
    resolved("InputMD", 0xA499),
    resolved("GetD", 0xA4AD),
    resolved("Open", 0xA444),
    resolved("Close", 0xA479),
    resolved("XIO", 0xA4DE),
    resolved("Note", 0xA60D),
    resolved("Point", 0xA634),
    resolved("Graphics", 0xA654),
    resolved("SetColor", 0xA6CE),
    resolved("Plot", 0xA6C3),
    resolved("DrawTo", 0xA68C),
    resolved("Fill", 0xA6E9),
    resolved("Position", 0xA6AE),
    resolved("Locate", 0xA6BB),
    resolved("Sound", 0xA704),
    resolved("SndRst", 0xA721),
    resolved("Paddle", 0xAD37),
    resolved("PTrig", 0xA737),
    resolved("Stick", 0xA74E),
    resolved("STrig", 0xAD2F),
    resolved("SCompare", 0xA864),
    resolved("SCopy", 0xA898),
    resolved("SCopyS", 0xA8AF),
    resolved("SAssign", 0xA8D8),
    resolved("ValB", 0xA59A),
    resolved("ValC", 0xA59A),
    resolved("ValI", 0xA59A),
    resolved("Rand", 0xA6F1),
    resolved("Peek", 0xA767),
    resolved("PeekC", 0xA767),
    resolved("Poke", 0xA777),
    resolved("PokeC", 0xA781),
    resolved("Error", 0x04CB),
    resolved("Break", 0xA7DA),
    resolved("Zero", 0xA78A),
    resolved("SetBlock", 0xA790),
    resolved("MoveBlock", 0xA7B3),
];

pub(super) fn resolve_builtin_target(name: &str) -> MirBuiltinResolution {
    let normalized = normalize_builtin_name(name);
    BUILTIN_TARGETS
        .iter()
        .find(|entry| normalize_builtin_name(entry.name) == normalized)
        .map(|entry| entry.resolution)
        .unwrap_or(MirBuiltinResolution::Unknown)
}

const fn resolved(name: &'static str, address: u16) -> MirBuiltinEntry {
    MirBuiltinEntry {
        name,
        resolution: MirBuiltinResolution::Resolved { address },
    }
}

const fn unsupported(name: &'static str, reason: &'static str) -> MirBuiltinEntry {
    MirBuiltinEntry {
        name,
        resolution: MirBuiltinResolution::Unsupported { reason },
    }
}

fn normalize_builtin_name(name: &str) -> String {
    name.chars()
        .filter(|ch| !matches!(ch, '_' | '-' | ' '))
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{BUILTIN_TARGETS, MirBuiltinResolution, resolve_builtin_target};

    #[test]
    fn resolves_confirmed_resident_builtins() {
        assert_eq!(
            resolve_builtin_target("PrintE"),
            MirBuiltinResolution::Resolved { address: 0xA46C }
        );
        assert_eq!(
            resolve_builtin_target("put-e"),
            MirBuiltinResolution::Resolved { address: 0xA4CC }
        );
        assert_eq!(
            resolve_builtin_target("SCompare"),
            MirBuiltinResolution::Resolved { address: 0xA864 }
        );
        assert_eq!(
            resolve_builtin_target("Break"),
            MirBuiltinResolution::Resolved { address: 0xA7DA }
        );
        assert_eq!(
            resolve_builtin_target("Error"),
            MirBuiltinResolution::Resolved { address: 0x04CB }
        );
    }

    #[test]
    fn classifies_currently_unresolved_seeded_builtins() {
        assert_eq!(
            resolve_builtin_target("PrintH"),
            MirBuiltinResolution::Unsupported {
                reason: "resident entry point is not modeled"
            }
        );
        assert_eq!(
            resolve_builtin_target("NotAThing"),
            MirBuiltinResolution::Unknown
        );
    }

    #[test]
    fn builtin_inventory_has_no_duplicate_normalized_names() {
        for (index, entry) in BUILTIN_TARGETS.iter().enumerate() {
            let normalized = entry.name.to_ascii_uppercase();
            assert!(
                !BUILTIN_TARGETS[index + 1..]
                    .iter()
                    .any(|other| other.name.to_ascii_uppercase() == normalized),
                "duplicate builtin inventory entry for {}",
                entry.name
            );
        }
    }
}
