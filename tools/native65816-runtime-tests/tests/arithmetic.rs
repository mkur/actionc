mod support;
use support::*;

#[test]
fn scalar_arithmetic_and_all_comparisons_match_host_boundary_cases() {
    for (ty, bytes, signed) in [
        ("BYTE", 1, false),
        ("CARD", 2, false),
        ("INT", 2, true),
        ("SIZE", 3, false),
        ("LONGCARD", 4, false),
        ("LONGINT", 4, true),
    ] {
        // Action! unary minus on SIZE produces INT in NIR. Request a SIZE
        // subtraction explicitly when testing modular 24-bit negation.
        let negation = if bytes == 3 { "SIZE(0)-a" } else { "-a" };
        let source = format!(
            r#"
{ty} a,b,sum,difference,negative,band,bor,bxor
BYTE flags
PROC Arithmetic()
  sum=a+b
  difference=a-b
  negative={negation}
  band=a AND b
  bor=a OR b
  bxor=a XOR b
RETURN
PROC Main()
  Arithmetic()
  flags=0
  IF a=b THEN flags==+1 FI
  IF a<>b THEN flags==+2 FI
  IF a<b THEN flags==+4 FI
  IF a<=b THEN flags==+8 FI
  IF a>b THEN flags==+16 FI
  IF a>=b THEN flags==+32 FI
RETURN
"#
        );
        let mask = ((1u64 << (bytes * 8)) - 1) as u32;
        let sign_bit = 1u32 << (bytes * 8 - 1);
        let interpret = |n: u32| {
            if signed && n & sign_bit != 0 {
                i64::from(n) - (1i64 << (bytes * 8))
            } else {
                i64::from(n)
            }
        };
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            let caller = caller(image.entry);
            for (a, b) in [
                (0, 0),
                (mask, 1),
                (sign_bit, mask),
                (sign_bit - 1, 1),
                (0, mask),
                (0x55 & mask, 0xaa & mask),
            ] {
                let mut h = Harness::new(&image, &caller, 4);
                for (name, value) in [("a", a), ("b", b)] {
                    let address =
                        image.data.iter().find(|d| d.name == name).unwrap().address as usize;
                    h.bus.ram[address..address + bytes]
                        .copy_from_slice(&value.to_le_bytes()[..bytes]);
                }
                h.run();
                h.guards(4);
                for (name, expected) in [
                    ("sum", a.wrapping_add(b)),
                    ("difference", a.wrapping_sub(b)),
                    ("negative", 0u32.wrapping_sub(a)),
                    ("band", a & b),
                    ("bor", a | b),
                    ("bxor", a ^ b),
                ] {
                    assert_eq!(
                        h.global(&image, name, bytes),
                        expected & mask,
                        "{ty}/{optimize}/{a:x}/{b:x}/{name}"
                    );
                }
                let (a, b) = (interpret(a), interpret(b));
                let flags = u32::from(a == b)
                    | (u32::from(a != b) << 1)
                    | (u32::from(a < b) << 2)
                    | (u32::from(a <= b) << 3)
                    | (u32::from(a > b) << 4)
                    | (u32::from(a >= b) << 5);
                assert_eq!(
                    h.global(&image, "flags", 1),
                    flags,
                    "{ty}/{optimize}/{a}/{b}"
                );
            }
        }
    }
}
