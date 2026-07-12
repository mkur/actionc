#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir6502Config {
    pub select_runtime_helpers: bool,
    pub enable_peepholes: bool,
    pub enable_word_inc_update: bool,
    pub enable_direct_byte_word_update: bool,
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
            enable_peepholes: true,
            enable_word_inc_update: true,
            enable_direct_byte_word_update: false,
            peephole_report: MirPeepholeReportMode::Off,
        }
    }
}

impl Mir6502Config {
    pub fn optimized() -> Self {
        Self {
            enable_word_inc_update: true,
            enable_direct_byte_word_update: true,
            ..Self::default()
        }
    }
}
