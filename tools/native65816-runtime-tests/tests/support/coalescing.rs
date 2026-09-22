//! Verified target probes retaining optimized loop MIR in both frontend modes.
use super::*;
pub fn prepared(source: &str, optimize: bool) -> native65816::Prepared {
    let optimized = prepare(source, true);
    let mut p = prepare(source, optimize);
    for r in &mut p.mir.routines {
        if r.name.to_ascii_lowercase().contains("rotation") {
            let original = optimized
                .mir
                .routines
                .iter()
                .find(|s| s.id == r.id)
                .unwrap();
            assert_eq!(r.name, original.name);
            *r = original.clone();
        }
    }
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
}
