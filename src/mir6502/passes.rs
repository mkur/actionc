#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir6502Config {
    pub select_runtime_helpers: bool,
    #[doc(hidden)]
    pub select_widening_byte_multiply: bool,
    pub enable_peepholes: bool,
    pub enable_word_inc_update: bool,
    pub enable_direct_byte_word_update: bool,
    /// Allow bounded code growth for exact small counted loops. The selector
    /// retains its own trip-count and growth limits; this is intentionally
    /// enabled only by the optimized configuration.
    pub enable_small_loop_unrolling: bool,
    /// Inline private byte leaves only after materialization proves bounded
    /// growth and a conservative cycle saving in the expanded caller.
    pub enable_small_leaf_inlining: bool,
    pub peephole_report: MirPeepholeReportMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MirPeepholeReportMode {
    Off,
    Aggregate,
    PerRoutine,
    Sites,
}

impl Default for Mir6502Config {
    fn default() -> Self {
        Self {
            select_runtime_helpers: true,
            select_widening_byte_multiply: false,
            enable_peepholes: true,
            enable_word_inc_update: true,
            enable_direct_byte_word_update: false,
            enable_small_loop_unrolling: false,
            enable_small_leaf_inlining: false,
            peephole_report: MirPeepholeReportMode::Off,
        }
    }
}

impl Mir6502Config {
    pub fn optimized() -> Self {
        Self {
            enable_word_inc_update: true,
            enable_direct_byte_word_update: true,
            enable_small_loop_unrolling: true,
            enable_small_leaf_inlining: true,
            ..Self::default()
        }
    }
}
