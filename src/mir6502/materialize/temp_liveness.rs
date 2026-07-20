pub(super) use crate::mir6502::analysis::temp_liveness::{MirTempLiveSet, MirTempLiveness};

use super::stats::MirPeepholeStats;
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::ir::{MirRoutine, RoutineId};

pub(super) fn analyze_temp_liveness(routine: &MirRoutine) -> MirTempLiveness {
    let cfg = MirCfg::from_routine(routine)
        .unwrap_or_else(|errors| panic!("invalid MIR CFG before temp liveness: {errors:?}"));
    MirTempLiveness::analyze(routine, &cfg)
}

pub(super) fn record_temp_liveness_observability(
    routine_id: RoutineId,
    liveness: &MirTempLiveness,
    peephole_stats: &mut MirPeepholeStats,
) {
    let live_in_lanes = liveness
        .blocks()
        .iter()
        .map(|block| block.live_in.exact_len())
        .sum();
    let live_out_lanes = liveness
        .blocks()
        .iter()
        .map(|block| block.live_out.exact_len())
        .sum();
    let live_in_full_temps = liveness
        .blocks()
        .iter()
        .map(|block| block.live_in.full_len())
        .sum();
    let live_out_full_temps = liveness
        .blocks()
        .iter()
        .map(|block| block.live_out.full_len())
        .sum();

    peephole_stats.record_many(routine_id, "temp-liveness-live-in-lanes", live_in_lanes);
    peephole_stats.record_many(routine_id, "temp-liveness-live-out-lanes", live_out_lanes);
    peephole_stats.record_many(
        routine_id,
        "temp-liveness-live-in-full-temps",
        live_in_full_temps,
    );
    peephole_stats.record_many(
        routine_id,
        "temp-liveness-live-out-full-temps",
        live_out_full_temps,
    );
}
