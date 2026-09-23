use super::*;

#[test]
fn indexed_first_byte_argument_preserves_later_register_arguments() {
    for (arguments, expected) in [
        ("xs(n),ys(n),0", [23, 71, 0]),
        ("xs(n),191-ys(n),0", [23, 120, 0]),
        ("xs(n),ys(n),colors(n)", [23, 71, 6]),
        ("xs(n),ys(n),colors(n)+1", [23, 71, 7]),
        ("xs(n RSH 1),ys(n),colors(n)", [23, 71, 6]),
        ("pixel^,ys(n),colors(n)", [23, 71, 6]),
    ] {
        let source = format!(
            "BYTE ARRAY xs(256)=$5000,ys(256)=$5100,colors(256)=$5200 \
             BYTE n=$0600,first=$0601,second=$0602,third=$0603 \
             BYTE POINTER pixel \
             PROC Capture(BYTE a,b,c) first=a second=b third=c RETURN \
             PROC Main() pixel=xs Capture({arguments}) RETURN"
        );
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let model = analyze(&ast).unwrap();
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        for profile in [CodegenProfile::Compat, CodegenProfile::Modern] {
            let output =
                generate_semir_standalone_profile_at_origin(&semir, 0x3000, profile).unwrap();
            for n in [0u8, 1, 9, 127, 128, 255] {
                let mut memory = [0u8; 65536];
                let origin = usize::from(output.origin);
                memory[origin..origin + output.bytes.len()].copy_from_slice(&output.bytes);
                memory[0x11] = 0xFF; // SArgs checks the Atari BREAK flag.
                memory[0x600] = n;
                memory[0x5000..0x5100].fill(23);
                memory[0x5100..0x5200].fill(71);
                memory[0x5200..0x5300].fill(6);
                indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
                assert_eq!(
                    &memory[0x601..0x604],
                    &expected,
                    "{profile:?}/{arguments}/{n}"
                );
            }
        }
    }
}

#[test]
fn indexed_first_byte_argument_preserves_word_argument() {
    for suffix in ["", "done=1"] {
        let source = format!(
            "BYTE ARRAY xs(256)=$5000 CARD ARRAY words(256)=$5100 \
             BYTE n=$0600,first=$0601,done=$0604 CARD second=$0602 \
             PROC Capture(BYTE a,CARD b) first=a second=b RETURN \
             PROC Main() Capture(xs(n),words(n)) {suffix} RETURN"
        );
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let model = analyze(&ast).unwrap();
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        for profile in [CodegenProfile::Compat, CodegenProfile::Modern] {
            let output =
                generate_semir_standalone_profile_at_origin(&semir, 0x3000, profile).unwrap();
            for n in [0u8, 1, 127, 128, 255] {
                let mut memory = [0u8; 65536];
                let origin = usize::from(output.origin);
                memory[origin..origin + output.bytes.len()].copy_from_slice(&output.bytes);
                memory[0x11] = 0xFF;
                memory[0x600] = n;
                memory[0x5000..0x5100].fill(23);
                let word_address = 0x5100 + usize::from(n) * 2;
                memory[word_address..word_address + 2].copy_from_slice(&[0x34, 0xAB]);
                indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
                assert_eq!(
                    &memory[0x601..0x605],
                    &[23, 0x34, 0xAB, u8::from(!suffix.is_empty())],
                    "{profile:?}/{suffix}/{n}"
                );
            }
        }
    }
}
