#[path = "variant_values.rs"]
mod values;

pub fn check(source: &str, expected: &[u8]) {
    // Public compiler lanes as well as raw/optimized NIR; no capability override.
    values::check(source, expected);
}
