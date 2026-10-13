//! Captured-bound motion with bounded final code, home and image qualification.
use super::*;
use crate::mir6502::analysis::loop_motion::might_match;
use crate::mir6502::rewrite::motion::MirPreHomeMotionDriver;

fn occupied_zero_page(p: &MirProgram) -> BTreeSet<u8> {
    p.routines
        .iter()
        .flat_map(|r| {
            r.frame.fixed_zero_page.iter().map(|s| s.0).chain(
                r.frame
                    .zero_page_allocations
                    .iter()
                    .flat_map(|a| (0..a.size).map(move |n| a.start.0.wrapping_add(n))),
            )
        })
        .collect()
}

pub(in crate::mir6502) fn materialize_costed(
    original: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let mut off = config.clone();
    off.enable_captured_bound_motion = false;
    let fallback = |p, report| {
        crate::mir6502::materialize_resolved_program_without_motion_costing(
            p, &off, origin, runtime, report,
        )
    };
    if !original.routines.iter().any(might_match) {
        return fallback(original, report);
    }
    let mut selected = original.clone();
    let mut candidates = BTreeSet::new();
    for r in &mut selected.routines {
        if !might_match(r) {
            continue;
        }
        let count = MirPreHomeMotionDriver::default().run(r).map_err(|e| {
            vec![MirDiagnostic::routine(
                &r.name,
                format!("pre-home captured-bound motion failed: {e:?}"),
            )]
        })?;
        if count > 0 {
            candidates.insert(r.id);
        }
    }
    if candidates.is_empty() {
        return fallback(original, report);
    }
    crate::mir6502::verify::verify_program(&selected, MirPhase::PreMaterialization)?;
    let baseline = fallback(original.clone(), false)?;
    let Ok(mut proposed) = fallback(selected.clone(), false) else {
        return fallback(original, report);
    };
    let (Some(old), Some(new)) = (
        address_values::Cost::measure(&baseline, origin),
        address_values::Cost::measure(&proposed, origin),
    ) else {
        return fallback(original, report);
    };
    let mut accepted = candidates
        .iter()
        .copied()
        .filter(|&id| new.does_not_grow(&old, id))
        .collect::<BTreeSet<_>>();
    if !accepted.is_empty() && accepted != candidates {
        for r in &mut selected.routines {
            if candidates.contains(&r.id) && !accepted.contains(&r.id) {
                *r = original
                    .routines
                    .iter()
                    .find(|old| old.id == r.id)
                    .unwrap()
                    .clone();
            }
        }
        proposed = match fallback(selected.clone(), false) {
            Ok(p) => p,
            Err(_) => return fallback(original, report),
        };
    }
    // Final image and each accepted routine must remain within the baseline's
    // code and storage budget, including any longer-lived computed bound.
    if !occupied_zero_page(&proposed).is_subset(&occupied_zero_page(&baseline))
        || address_values::Cost::measure(&proposed, origin).is_none_or(|cost| {
            cost.bytes > old.bytes || accepted.iter().any(|&id| !cost.does_not_grow(&old, id))
        })
    {
        accepted.clear();
    }
    if report {
        let mut stats = MirPeepholeStats::default();
        for &id in &candidates {
            stats.record(
                id,
                if accepted.contains(&id) {
                    "captured-bound-motion-applied"
                } else {
                    "captured-bound-motion-declined-final-cost"
                },
            );
        }
        maybe_report_peepholes(&original, &stats, config);
    }
    if report && stats::reporting_enabled(config) {
        fallback(
            if accepted.is_empty() {
                original
            } else {
                selected
            },
            true,
        )
    } else {
        Ok(if accepted.is_empty() {
            baseline
        } else {
            proposed
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::loop_motion::tests::{captured, program};
    #[test]
    fn costed_motion_keeps_the_final_code_and_home_budget() {
        let input = captured();
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            let mut off = Mir6502Config::optimized();
            off.enable_captured_bound_motion = false;
            let on = Mir6502Config::optimized();
            let old = crate::mir6502::materialize_program_with_origin_and_runtime(
                input.clone(),
                &off,
                0x3000,
                runtime,
            )
            .unwrap();
            let new = crate::mir6502::materialize_program_with_origin_and_runtime(
                input.clone(),
                &on,
                0x3000,
                runtime,
            )
            .unwrap();
            let a = address_values::Cost::measure(&old, 0x3000).unwrap();
            let b = address_values::Cost::measure(&new, 0x3000).unwrap();
            assert!(b.bytes <= a.bytes && b.does_not_grow(&a, RoutineId(0)));
            assert_ne!(old, new, "motion should survive its cost gate");
        }
    }
    #[test]
    fn source_counterfactuals_keep_mutable_record_call_and_write_fallbacks() {
        let p = program(include_str!(
            "../../../surveys/tn/reproducers/captured-bounds.act"
        ));
        for r in &p.routines {
            let facts = crate::mir6502::analysis::loop_motion::tests::plans(r);
            if ["Count", "Snapshot", "Repeated"].contains(&r.name.as_str()) {
                assert!(!facts.is_empty(), "{}", r.name);
            } else {
                assert!(facts.is_empty(), "{}", r.name);
            }
        }
    }
}
