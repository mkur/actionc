#[path = "support/variant_values.rs"]
mod support;

const SAMPLE: &str = include_str!("../../../samples/algebraic-types.act");

#[test]
fn algebraic_types_sample_matches_documented_output_in_every_atari_lane() {
    let capture = "CARD ARRAY captured=$600 BYTE count=$620\nPROC Capture(CARD value) captured(count)=value count==+1 RETURN\n";
    let source = SAMPLE
        .replace("PROC Main()", &format!("{capture}PROC Main()\ncount=0"))
        .replace("PrintBE(", "Capture(")
        .replace("PrintIE(", "Capture(");
    let end = source.rfind("RETURN").unwrap();
    let source = format!("{} DO OD\n{}", &source[..end], &source[end..]);
    let mut expected = vec![0xCC; 0x500];
    for (index, value) in [65u16, 0, 9, 10, 7, 5].iter().enumerate() {
        expected[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
    }
    expected[0x20] = 6;
    support::check(&source, &expected);
}
