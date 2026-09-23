mod support;
use support::*;

#[test]
fn byte_captures_and_boolean_results_survive_alias_and_call_clobbers() {
    narrow_comparison::captured_values_survive_clobbers(false);
}

fn source() -> String {
    let mut s = String::from(
        "BYTE a=$7100,b=$7101\nBYTE ARRAY out=$7200\n\
        BYTE FUNC Ret(BYTE x,y) RETURN(x<y)\n\
        PROC Save(BYTE value) out(32)=value RETURN\n\
        PROC Work(BYTE x,y) BYTE saved\n",
    );
    for (i, op) in ["=", "#", "<", "<=", ">", ">="].iter().enumerate() {
        s.push_str(&format!(
            "out({})=(x{op}y)\nIF x{op}y THEN out({})=1 ELSE out({})=0 FI\n",
            2 * i,
            12 + 2 * i,
            12 + 2 * i
        ));
    }
    s.push_str(
        "out(24)=(x<BYTE(128)) out(26)=(BYTE(128)<x)\n\
        saved=(x>=y) out(28)=Ret(x,y) out(30)=saved Save(saved)\n\
        x==+1 out(34)=(x>y) RETURN\nPROC Main() Work(a,b) RETURN\n",
    );
    s
}

#[test]
fn byte_predicates_and_boolean_consumers_cover_runtime_boundary_pairs() {
    let s = source();
    for optimize in [false, true] {
        narrow_comparison::check_shape(&s, optimize, 1);
        let image = compile(&s, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&s.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let caller = caller(image.entry);
        for a in [0u8, 1, 0x7f, 0x80, 0xfe, 0xff] {
            for b in [0u8, 1, 0x7f, 0x80, 0xfe, 0xff] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&[a, b]);
                    h.bus.ram[0x7200..0x7224].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    let relations = [a == b, a != b, a < b, a <= b, a > b, a >= b];
                    let expected = relations.into_iter().chain(relations).chain([
                        a < 128,
                        128 < a,
                        a < b,
                        a >= b,
                        a >= b,
                        a.wrapping_add(1) > b,
                    ]);
                    for (i, truth) in expected.enumerate() {
                        assert_eq!(
                            h.bus.ram[0x7200 + 2 * i],
                            u8::from(truth),
                            "{optimize}/{a}/{b}/{mask}/{i}"
                        );
                        assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                    }
                    if a == 0x80 && b == 0x7f && mask == 0 {
                        narrow_comparison::record("byte-comparisons", optimize, &s, &image, &h);
                    }
                }
            }
        }
    }
}
