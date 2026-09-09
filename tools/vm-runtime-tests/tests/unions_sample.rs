#[path = "support/variant_values.rs"]
mod support;

#[test]
fn documented_union_sample_matches_printed_values_in_all_public_atari_lanes() {
    let capture = "CARD ARRAY captured=$600 BYTE count=$620\nPROC Capture(CARD value) captured(count)=value count==+1 RETURN\n";
    let source = include_str!("../../../samples/union-views.act")
        .replace("PROC Main()", &format!("{capture}PROC Main()\ncount=0"))
        .replace("PrintCE(", "Capture(")
        .replace("PrintBE(", "Capture(");
    let end = source.rfind("RETURN").unwrap();
    let source = format!("{} DO OD\n{}", &source[..end], &source[end..]);
    let mut expected = vec![0xCC; 0x500];
    for (index, value) in [4728u16, 18, 165, 0].iter().enumerate() {
        expected[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
    }
    expected[0x20] = 4;
    support::check(&source, &expected);
}
