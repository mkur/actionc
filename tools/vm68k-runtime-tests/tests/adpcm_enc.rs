#[path = "common/adpcm.rs"]
mod adpcm;
mod common;
#[path = "common/reference.rs"]
mod reference;
const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/adpcm_enc/adpcm_enc.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/adpcm_enc/vectors.txt");
#[test]
fn adpcm_enc_raw_matches_every_c_state() {
    adpcm::execute(SOURCE, VECTORS, true, false);
}
#[test]
fn adpcm_enc_optimized_matches_every_c_state() {
    adpcm::execute(SOURCE, VECTORS, true, true);
}
