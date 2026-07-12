use crate::mir6502::ir::{MirProgram, RoutineId};
use crate::mir6502::passes::{Mir6502Config, MirPeepholeReportMode};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct MirPeepholeStats {
    counts: BTreeMap<(RoutineId, &'static str), usize>,
    sites: Vec<MirPeepholeSite>,
}

struct MirPeepholeSite {
    routine: RoutineId,
    rule: &'static str,
    detail: String,
}

impl MirPeepholeStats {
    pub(super) fn record(&mut self, routine: RoutineId, name: &'static str) {
        *self.counts.entry((routine, name)).or_default() += 1;
    }

    pub(super) fn record_many(&mut self, routine: RoutineId, name: &'static str, count: usize) {
        if count > 0 {
            *self.counts.entry((routine, name)).or_default() += count;
        }
    }

    pub(super) fn record_site(
        &mut self,
        routine: RoutineId,
        rule: &'static str,
        detail: impl Into<String>,
    ) {
        self.sites.push(MirPeepholeSite {
            routine,
            rule,
            detail: detail.into(),
        });
    }

    fn is_empty(&self) -> bool {
        self.counts.is_empty() && self.sites.is_empty()
    }

    pub(super) fn aggregate_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut aggregate = BTreeMap::new();
        for ((_routine, name), count) in &self.counts {
            *aggregate.entry(*name).or_default() += *count;
        }
        aggregate
    }

    fn per_routine_counts(&self) -> BTreeMap<RoutineId, BTreeMap<&'static str, usize>> {
        let mut per_routine = BTreeMap::new();
        for ((routine, name), count) in &self.counts {
            *per_routine
                .entry(*routine)
                .or_insert_with(BTreeMap::new)
                .entry(*name)
                .or_default() += *count;
        }
        per_routine
    }
}

pub(super) fn maybe_report_peepholes(
    program: &MirProgram,
    stats: &MirPeepholeStats,
    config: &Mir6502Config,
) {
    let mode = env_peephole_report_mode().unwrap_or(config.peephole_report);
    if mode == MirPeepholeReportMode::Off {
        return;
    }
    eprint!("{}", format_peephole_report(program, stats, mode));
}

fn env_peephole_report_mode() -> Option<MirPeepholeReportMode> {
    let value = std::env::var("ACTIONC_MIR6502_PEEPHOLES").ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "off" | "none" => Some(MirPeepholeReportMode::Off),
        "aggregate" | "summary" => Some(MirPeepholeReportMode::Aggregate),
        "1" | "true" | "on" | "full" | "routine" | "routines" | "per-routine" => {
            Some(MirPeepholeReportMode::PerRoutine)
        }
        "site" | "sites" | "detail" | "details" | "verbose" | "debug" => {
            Some(MirPeepholeReportMode::Sites)
        }
        _ => Some(MirPeepholeReportMode::PerRoutine),
    }
}

pub(super) fn format_peephole_report(
    program: &MirProgram,
    stats: &MirPeepholeStats,
    mode: MirPeepholeReportMode,
) -> String {
    let mut report = String::new();
    report.push_str("mir6502 peepholes:\n");
    if stats.is_empty() {
        report.push_str("  none\n");
        return report;
    }

    report.push_str("  aggregate:\n");
    for (name, count) in stats.aggregate_counts() {
        report.push_str(&format!("    {name}: {count}\n"));
    }

    if mode < MirPeepholeReportMode::PerRoutine {
        return report;
    }

    let routine_names = program
        .routines
        .iter()
        .map(|routine| (routine.id, routine.name.as_str()))
        .collect::<BTreeMap<_, _>>();
    report.push_str("  per-routine:\n");
    for (routine, counts) in stats.per_routine_counts() {
        let name = routine_names.get(&routine).copied().unwrap_or("<unknown>");
        report.push_str(&format!("    {name}:\n"));
        for (rule, count) in counts {
            report.push_str(&format!("      {rule}: {count}\n"));
        }
    }
    if mode < MirPeepholeReportMode::Sites {
        return report;
    }

    if !stats.sites.is_empty() {
        report.push_str("  sites:\n");
        for site in &stats.sites {
            let name = routine_names
                .get(&site.routine)
                .copied()
                .unwrap_or("<unknown>");
            report.push_str(&format!("    {name}: {}: {}\n", site.rule, site.detail));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::MirEffects;
    use crate::mir6502::{MirFrame, MirRoutine, MirRoutineAbi};

    #[test]
    fn peephole_report_includes_aggregate_and_per_routine_counts() {
        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![
                MirRoutine {
                    id: RoutineId(0),
                    name: "Cold".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: Vec::new(),
                    effects: MirEffects::default(),
                },
                MirRoutine {
                    id: RoutineId(1),
                    name: "Hot".to_string(),
                    abi: MirRoutineAbi::Action,
                    frame: MirFrame::default(),
                    temps: Vec::new(),
                    blocks: Vec::new(),
                    effects: MirEffects::default(),
                },
            ],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };
        let mut stats = MirPeepholeStats::default();
        stats.record(RoutineId(1), "fold-a");
        stats.record(RoutineId(1), "fold-a");
        stats.record(RoutineId(0), "fold-b");
        stats.record_site(RoutineId(1), "fold-a", "block=b0 op=#1");

        let report = format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);

        assert!(report.contains("mir6502 peepholes:\n"));
        assert!(report.contains("  aggregate:\n"));
        assert!(report.contains("    fold-a: 2\n"));
        assert!(report.contains("    fold-b: 1\n"));
        assert!(report.contains("  per-routine:\n"));
        assert!(report.contains("    Cold:\n      fold-b: 1\n"));
        assert!(report.contains("    Hot:\n      fold-a: 2\n"));
        assert!(report.contains("  sites:\n"));
        assert!(report.contains("    Hot: fold-a: block=b0 op=#1\n"));
    }
}
