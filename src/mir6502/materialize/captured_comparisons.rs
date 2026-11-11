//! Qualify capture-preserving comparison selection after home and branch layout.
use super::*;

fn occupied_zero_page(program: &MirProgram) -> BTreeSet<u8> {
    program
        .routines
        .iter()
        .flat_map(|routine| {
            routine
                .frame
                .fixed_zero_page
                .iter()
                .map(|slot| slot.0)
                .chain(routine.frame.zero_page_allocations.iter().flat_map(|slot| {
                    (0..slot.size).map(move |offset| slot.start.0.wrapping_add(offset))
                }))
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
    off.enable_captured_byte_comparisons = false;
    let fallback = |program, report| {
        crate::mir6502::materialize_resolved_program_without_captured_compare_costing(
            program, &off, origin, runtime, report,
        )
    };
    if !original
        .routines
        .iter()
        .any(|routine| captured_byte_compare_rank(routine) > 0)
    {
        return fallback(original, report);
    }
    let mut selected = original.clone();
    let mut stats = MirPeepholeStats::default();
    let mut candidates = BTreeSet::new();
    for routine in &mut selected.routines {
        if captured_byte_compare_rank(routine) > 0 {
            run_analyzed_captured_byte_comparisons(routine, &mut stats)?;
            if stats.count_for(routine.id, "captured-byte-compare-selected") > 0 {
                candidates.insert(routine.id);
            }
        }
    }
    if candidates.is_empty() {
        if report {
            maybe_report_peepholes(&original, &stats, config);
        }
        return fallback(original, report);
    }
    let baseline = fallback(original.clone(), false)?;
    let mut proposed = fallback(selected.clone(), false).ok();
    let old = address_values::Cost::measure(&baseline, origin);
    let mut accepted = old
        .as_ref()
        .zip(
            proposed
                .as_ref()
                .and_then(|p| address_values::Cost::measure(p, origin)),
        )
        .map_or_else(BTreeSet::new, |(old, new)| {
            candidates
                .iter()
                .copied()
                .filter(|&id| new.does_not_grow(old, id))
                .collect()
        });
    if !accepted.is_empty() && accepted != candidates {
        for routine in &mut selected.routines {
            if candidates.contains(&routine.id) && !accepted.contains(&routine.id) {
                *routine = original
                    .routines
                    .iter()
                    .find(|r| r.id == routine.id)
                    .unwrap()
                    .clone();
            }
        }
        proposed = fallback(selected.clone(), false).ok();
    }
    // Recheck the combined result after rollback. Allocation and branch
    // relaxation can change costs in routines outside the selected suffix.
    let final_ok = proposed.as_ref().is_some_and(|program| {
        occupied_zero_page(program).is_subset(&occupied_zero_page(&baseline))
            && old
                .as_ref()
                .zip(address_values::Cost::measure(program, origin))
                .is_some_and(|(old, new)| {
                    new.bytes <= old.bytes && accepted.iter().all(|&id| new.does_not_grow(old, id))
                })
    });
    if !final_ok {
        accepted.clear();
    }
    if report {
        for &id in &candidates {
            stats.record_many(
                id,
                if accepted.contains(&id) {
                    "captured-byte-compare-applied"
                } else {
                    "captured-byte-compare-declined-final-cost"
                },
                stats.count_for(id, "captured-byte-compare-selected"),
            );
        }
        maybe_report_peepholes(&original, &stats, config);
        if stats::reporting_enabled(config) {
            return fallback(
                if accepted.is_empty() {
                    original
                } else {
                    selected
                },
                true,
            );
        }
    }
    Ok(if accepted.is_empty() {
        baseline
    } else {
        proposed.unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::MirPeepholeReportMode;

    #[test]
    fn captured_compare_cost_gate_keeps_existing_dual_indirect_selection() {
        let source = "MODULE\nBYTE index=$E0,result=$0680\n\
            PROC Keep(BYTE POINTER p)\n\
              IF index<=p^ THEN result=17 ELSE result=29 FI\nRETURN\n\
            BYTE FUNC Pair(BYTE POINTER p,q)\n\
              IF p^>q^ THEN RETURN(17) ELSE RETURN(29) FI\n\
            PROC Main()\nKeep($7000)\nresult=Pair($7000,$7100)\nRETURN\n";
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let raw = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let nir = crate::nir::optimize_program(&raw).unwrap();
        let input = crate::mir6502::lower_program(&nir).unwrap();
        assert_eq!(
            input
                .routines
                .iter()
                .map(captured_byte_compare_rank)
                .sum::<usize>(),
            2
        );
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            for origin in [0x3000, 0x41C7] {
                let mut off = Mir6502Config::optimized();
                off.enable_captured_byte_comparisons = false;
                let old = crate::mir6502::materialize_program_with_origin_and_runtime(
                    crate::mir6502::lower_program(&nir).unwrap(),
                    &off,
                    origin,
                    runtime,
                )
                .unwrap();
                let new = crate::mir6502::materialize_program_with_origin_and_runtime(
                    crate::mir6502::lower_program(&nir).unwrap(),
                    &Mir6502Config::optimized(),
                    origin,
                    runtime,
                )
                .unwrap();
                let reported = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input.clone(),
                    &Mir6502Config {
                        peephole_report: MirPeepholeReportMode::PerRoutine,
                        ..Mir6502Config::optimized()
                    },
                    origin,
                    runtime,
                )
                .unwrap();
                assert_eq!(
                    new, reported,
                    "reporting must preserve final selection and placement"
                );
                let find = |p: &MirProgram, name| {
                    p.routines.iter().find(|r| r.name == name).unwrap().clone()
                };
                assert_eq!(find(&old, "Pair"), find(&new, "Pair"));
                assert_ne!(find(&old, "Keep"), find(&new, "Keep"));
                let old_cost = address_values::Cost::measure(&old, origin).unwrap();
                let new_cost = address_values::Cost::measure(&new, origin).unwrap();
                assert!(new_cost.bytes < old_cost.bytes);
                for r in &new.routines {
                    assert!(new_cost.does_not_grow(&old_cost, r.id));
                }
                assert!(occupied_zero_page(&new).is_subset(&occupied_zero_page(&old)));
            }
        }
    }
}
