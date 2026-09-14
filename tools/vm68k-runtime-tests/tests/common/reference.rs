//! Decode the committed reference format before target memory serialization.
#![allow(dead_code)] // Each integration-test binary uses a subset of helpers.
pub fn text(source: &str, crlf: bool) -> String {
    let lf = source.replace("\r\n", "\n");
    if crlf { lf.replace('\n', "\r\n") } else { lf }
}
pub fn bytes(hex: &str) -> Vec<u8> {
    if hex == "-" {
        return Vec::new();
    }
    assert_eq!(hex.len() % 2, 0);
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
pub fn words(bytes: &[u8], width: usize) -> Vec<u32> {
    assert!(matches!(width, 1 | 2 | 4));
    assert_eq!(bytes.len() % width, 0);
    bytes
        .chunks_exact(width)
        .map(|bytes| {
            bytes
                .iter()
                .enumerate()
                .fold(0, |v, (i, b)| v | u32::from(*b) << (8 * i))
        })
        .collect()
}
pub fn replace_once(source: &mut String, old: &str, new: &str) {
    assert_eq!(source.matches(old).count(), 1, "source anchor: {old}");
    *source = source.replace(old, new);
}
