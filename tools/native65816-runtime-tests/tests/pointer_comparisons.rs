mod support;
use support::*;

#[test]
fn pointer_captures_and_boolean_results_survive_alias_and_call_clobbers() {
    narrow_comparison::captured_values_survive_clobbers(true);
}

const SOURCE: &str = "BYTE POINTER a=$7100,b=$7103\nBYTE ARRAY out=$7200\n\
    BYTE FUNC Equal(BYTE POINTER x,y) RETURN(x=y)\n\
    PROC Save(BYTE value) out(16)=value RETURN\n\
    PROC Work(BYTE POINTER x,y) BYTE saved\n\
    out(0)=(x=y) out(2)=(x#y)\n\
    IF x=y THEN out(4)=1 ELSE out(4)=0 FI\n\
    IF x#y THEN out(6)=1 ELSE out(6)=0 FI\n\
    out(8)=(x=BYTE POINTER(0)) out(10)=(BYTE POINTER(0)#x)\n\
    saved=(x#y) out(12)=Equal(x,y) out(14)=saved Save(saved)\n\
    IF x=BYTE POINTER(0) THEN out(18)=1 ELSE out(18)=0 FI\n\
    IF BYTE POINTER(0)#x THEN out(20)=1 ELSE out(20)=0 FI\n\
    out(22)=(x=BYTE POINTER($010001)) x=BYTE POINTER(0) out(24)=(x=y)\n\
    RETURN\nPROC Main() Work(a,b) RETURN\n";

#[test]
fn pointer_equality_and_null_tests_cover_each_byte_and_boolean_consumer() {
    let values = [
        0u32, 1, 0xffff, 0x10000, 0x10001, 0x12ffff, 0x130000, 0xffffff,
    ];
    for optimize in [false, true] {
        narrow_comparison::check_shape(SOURCE, optimize, 3);
        let image = compile(SOURCE, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&SOURCE.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let caller = caller(image.entry);
        for a in values {
            for b in values {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&a.to_le_bytes()[..3]);
                    h.bus.ram[0x7103..0x7106].copy_from_slice(&b.to_le_bytes()[..3]);
                    h.bus.ram[0x7200..0x721a].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    for (i, truth) in [
                        a == b,
                        a != b,
                        a == b,
                        a != b,
                        a == 0,
                        a != 0,
                        a == b,
                        a != b,
                        a != b,
                        a == 0,
                        a != 0,
                        a == 0x10001,
                        0 == b,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        assert_eq!(
                            h.bus.ram[0x7200 + 2 * i],
                            u8::from(truth),
                            "{optimize}/{a:x}/{b:x}/{mask}/{i}"
                        );
                        assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                    }
                    if a == 0x10000 && b == 0 && mask == 0 {
                        narrow_comparison::record(
                            "pointer-comparisons",
                            optimize,
                            SOURCE,
                            &image,
                            &h,
                        );
                    }
                }
            }
        }
    }
}
