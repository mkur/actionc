/// Largest Action ABI argument frame captured directly by optimized callees.
///
/// Four bytes cover two CARD/pointer arguments or one CARD/pointer plus two
/// BYTE arguments. Larger frames use SArgs to avoid linear prologue growth.
pub(crate) const MAX_DIRECT_PARAM_CAPTURE_BYTES: u16 = 4;
