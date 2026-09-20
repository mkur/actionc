mod support;
use support::*;

#[test]
fn scalar_comma_groups_preserve_native_arguments_and_record_fields() {
    let source = "TYPE Fields=[CARD first,second LONGCARD wide BYTE tag,other LONGINT signed]\n\
        Fields POINTER target LONGINT result\n\
        LONGINT FUNC Mixed(CARD first,second LONGCARD wide BYTE tag,other LONGINT signed)\n\
          target.first=first target.second=second target.wide=wide\n\
          target.tag=tag target.other=other target.signed=signed RETURN(signed)\n\
        PROC Main() target=Fields POINTER($12FFF9)\n\
          result=Mixed($8123,$FEDC,LONGCARD($89ABCDEF),$AA,$55,LONGINT(-70000)) RETURN";
    for newline in ["\n", "\r\n"] {
        for optimize in [false, true] {
            let image = compile(&source.replace('\n', newline), optimize);
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                h.bus.map(0x12fff8, &[0xa5; 16], true);
                h.run();
                h.guards(mask);
                assert_eq!(h.global(&image, "result", 4), (-70000i32) as u32);
                // Independent native layout: words at 0/2, LONGCARD at 4,
                // bytes at 8/9, LONGINT at 10; the record crosses a bank.
                let mut expected = vec![
                    0xa5, 0x23, 0x81, 0xdc, 0xfe, 0xef, 0xcd, 0xab, 0x89, 0xaa, 0x55,
                ];
                expected.extend_from_slice(&(-70000i32).to_le_bytes());
                expected.push(0xa5);
                assert_eq!(&h.bus.ram[0x12fff8..0x130008], expected.as_slice());
            }
        }
    }
}
