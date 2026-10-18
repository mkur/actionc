#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir6502Config {
    /// Final-layout, same-block record base reuse with exact exit state repair.
    pub enable_record_base_reuse: bool,
    /// Admit indirect field writes only with complete disjoint destination proofs.
    pub enable_record_base_writes: bool,
    /// Trace parameter bytes after complete explicit same-block assignments.
    pub enable_record_base_parameter_origins: bool,
    pub select_runtime_helpers: bool,
    #[doc(hidden)]
    pub select_widening_byte_multiply: bool,
    pub enable_peepholes: bool,
    /// Replace proven small constant fill loops with direct byte stores.
    pub enable_tiny_constant_fills: bool,
    /// Forward captured bytes into small copies, subject to final size costing.
    pub enable_copy_value_forwarding: bool,
    pub enable_constructor_store_elision: bool,
    pub enable_constructor_home_elision: bool,
    /// Forward ordered scalar argument captures, with independent store proofs.
    pub enable_argument_capture_forwarding: bool,
    /// Forward scalar pattern bindings and remove proven unobservable homes.
    pub enable_pattern_binding_forwarding: bool,
    pub enable_word_inc_update: bool,
    pub enable_direct_byte_word_update: bool,
    /// Allow bounded code growth for exact small counted loops. The selector
    /// retains its own trip-count and growth limits; this is intentionally
    /// enabled only by the optimized configuration.
    pub enable_small_loop_unrolling: bool,
    /// Inline eligible scalar routines only after materialization proves bounded
    /// growth and a conservative cycle saving in the expanded caller.
    pub enable_small_leaf_inlining: bool,
    /// Complete address values and proven same-block source-pointer forwarding.
    pub enable_address_values: bool,
    /// Select indexed address values only after final code/home and image costing.
    pub enable_indexed_address_values: bool,
    /// Defer proven argument snapshots across pure predicates, with final costing.
    pub enable_call_predicate_arguments: bool,
    /// Carry an exclusive normalized Boolean into its proven fixed ABI home.
    /// Requires argument snapshot selection and its final cost/placement gate.
    pub enable_call_predicate_abi: bool,
    /// Retain two proven loop bases, guarded by final code/home/placement cost.
    pub enable_comparison_loop_pointers: bool,
    /// Share one unsigned comparison across continuation and mismatch ordering.
    pub enable_comparison_loop_fusion: bool,
    /// Carry bounded private comparison induction in Y.
    pub enable_comparison_loop_y: bool,
    /// Select native right shifts/rotates on private scratch when A is dead.
    pub enable_private_memory_shifts: bool,
    /// Remove pure register bitwise work only when its result and N/Z are dead.
    pub enable_dead_register_arithmetic: bool,
    /// Move pure header arithmetic from stable captures to a proven preheader.
    pub enable_captured_bound_motion: bool,
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
            enable_record_base_reuse: false,
            enable_record_base_writes: false,
            enable_record_base_parameter_origins: false,
            select_runtime_helpers: true,
            select_widening_byte_multiply: false,
            enable_peepholes: true,
            enable_tiny_constant_fills: true,
            enable_copy_value_forwarding: true,
            enable_constructor_store_elision: true,
            enable_constructor_home_elision: true,
            enable_argument_capture_forwarding: true,
            enable_pattern_binding_forwarding: true,
            enable_word_inc_update: true,
            enable_direct_byte_word_update: false,
            enable_small_loop_unrolling: false,
            enable_small_leaf_inlining: false,
            enable_address_values: false,
            enable_indexed_address_values: false,
            enable_call_predicate_arguments: false,
            enable_call_predicate_abi: false,
            enable_comparison_loop_pointers: false,
            enable_comparison_loop_fusion: false,
            enable_comparison_loop_y: false,
            enable_private_memory_shifts: false,
            enable_dead_register_arithmetic: false,
            enable_captured_bound_motion: false,
            peephole_report: MirPeepholeReportMode::Off,
        }
    }
}

impl Mir6502Config {
    pub fn optimized() -> Self {
        Self {
            enable_record_base_reuse: true,
            enable_record_base_writes: true,
            enable_record_base_parameter_origins: true,
            enable_word_inc_update: true,
            enable_direct_byte_word_update: true,
            enable_small_loop_unrolling: true,
            enable_small_leaf_inlining: true,
            enable_address_values: true,
            enable_indexed_address_values: true,
            enable_call_predicate_arguments: true,
            enable_call_predicate_abi: true,
            enable_comparison_loop_pointers: true,
            enable_comparison_loop_fusion: true,
            enable_comparison_loop_y: true,
            enable_private_memory_shifts: true,
            enable_dead_register_arithmetic: true,
            enable_captured_bound_motion: true,
            ..Self::default()
        }
    }
}
