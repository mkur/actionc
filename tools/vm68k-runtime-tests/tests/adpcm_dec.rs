#[path = "common/adpcm.rs"]
mod adpcm;
mod common;
#[path = "common/reference.rs"]
mod reference;
const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/adpcm_dec/adpcm_dec.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/adpcm_dec/vectors.txt");
#[test]
fn adpcm_dec_raw_matches_every_c_state() {
    adpcm::execute(SOURCE, VECTORS, false, false);
}
#[test]
fn adpcm_dec_optimized_matches_every_c_state() {
    adpcm::execute(SOURCE, VECTORS, false, true);
}
