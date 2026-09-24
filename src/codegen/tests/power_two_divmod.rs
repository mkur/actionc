use super::*;

fn compile(source: &str, mir: bool, runtime: Runtime, profile: CodegenProfile) -> CodegenOutput {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    if mir {
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(&semir)).unwrap();
        crate::mir6502::generate_output_with_config_and_runtime(
            &nir,
            0x3000,
            &crate::mir6502::Mir6502Config::optimized(),
            runtime,
        )
        .unwrap()
    } else {
        generate_semir_profile_at_origin_with_runtime(&semir, 0x3000, profile, runtime).unwrap()
    }
}

fn has_divmod_helper(output: &CodegenOutput) -> bool {
    output.routine_addresses.iter().any(|routine| {
        let name = routine.name.to_ascii_lowercase();
        name.contains("::div")
            || name.contains("::rem")
            || name.contains("::mod")
            || name.contains("::udiv")
            || name.contains("::umod")
    })
}

fn memory(output: &CodegenOutput) -> [u8; 65536] {
    let mut memory = [0xCC; 65536];
    let start = usize::from(output.origin);
    memory[start..start + output.bytes.len()].copy_from_slice(&output.bytes);
    memory
}

fn read(memory: &[u8], address: usize, width: usize) -> u32 {
    let mut bytes = [0; 4];
    bytes[..width].copy_from_slice(&memory[address..address + width]);
    u32::from_le_bytes(bytes)
}

#[test]
fn unsigned_power_two_divmod_executes_at_source_width_in_both_backends() {
    for (ty, width) in [("BYTE", 1), ("CARD", 2), ("LONGCARD", 4)] {
        for shift in 0..width * 8 {
            let divisor = 1u32 << shift;
            let source = format!(
                "{ty} input=$6E0,q=$600,r=$604 BYTE nq=$608,nr=$609\n\
                 PROC Main() q=input/${divisor:X} r=input MOD ${divisor:X}\n\
                 nq=input/${divisor:X} nr=input MOD ${divisor:X} RETURN"
            );
            let mask = u32::MAX >> (32 - width * 8);
            let inputs: Vec<u32> = if width == 1 {
                (0..=255).collect()
            } else {
                [
                    0,
                    1,
                    7,
                    8,
                    9,
                    127,
                    128,
                    255,
                    256,
                    257,
                    32767,
                    32768,
                    65535,
                    65536,
                    0x8000_0000,
                    u32::MAX,
                    divisor - 1,
                    divisor,
                    divisor.wrapping_add(1),
                    divisor.wrapping_mul(2).wrapping_sub(1),
                ]
                .into_iter()
                .map(|input| input & mask)
                .collect()
            };
            for mir in [false, true] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let output = compile(&source, mir, runtime, CodegenProfile::Modern);
                    assert!(
                        !has_divmod_helper(&output),
                        "{ty}/{shift}/{mir}/{runtime:?}: {:?}",
                        output.routine_addresses
                    );
                    // Some word shifts select cartridge-resident shift code.
                    // This small executor has no cartridge ROM; execute the
                    // standalone form and check helper selection in both.
                    if runtime == Runtime::ActionCart {
                        continue;
                    }
                    let mut memory = memory(&output);
                    for input in &inputs {
                        memory[0x600..0x60C].fill(0xCC);
                        memory[0x6E0..0x6E0 + width].copy_from_slice(&input.to_le_bytes()[..width]);
                        indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
                        let context = format!("{ty}/{shift}/{mir}/{runtime:?}/{input}");
                        assert_eq!(read(&memory, 0x600, width), input / divisor, "{context}");
                        assert_eq!(read(&memory, 0x604, width), input % divisor, "{context}");
                        assert_eq!(memory[0x608], (input / divisor) as u8, "{context}");
                        assert_eq!(memory[0x609], (input % divisor) as u8, "{context}");
                        assert_eq!(read(&memory, 0x6E0, width), *input, "{context}");
                        assert_eq!(&memory[0x60A..0x60C], &[0xCC; 2], "{context}");
                    }
                }
            }
        }
    }
}

#[test]
fn power_two_divmod_preserves_calls_indirect_reads_and_in_place_results() {
    for ty in ["BYTE", "CARD", "LONGCARD"] {
        let width = match ty {
            "BYTE" => 1,
            "CARD" => 2,
            _ => 4,
        };
        for divisor in [1, 8, 128] {
            let source = format!(
                "{ty} input=$6E0,q=$600,r=$604,iq=$608,ir=$60C\n\
                 {ty} POINTER p BYTE calls=$620\n\
                 {ty} FUNC Next() calls==+1 RETURN(input)\n\
                 PROC Main() calls=0 p=@input\n\
                 q=Next()/{divisor} r=Next() MOD {divisor}\n\
                 iq=p^/{divisor} ir=p^ MOD {divisor}\n\
                 input=input/{divisor} RETURN"
            );
            for mir in [false, true] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let output = compile(&source, mir, runtime, CodegenProfile::Modern);
                    if runtime == Runtime::ActionCart {
                        continue;
                    }
                    let mut memory = memory(&output);
                    let input = 0xFEDC_BA99u32 >> (32 - width * 8);
                    memory[0x6E0..0x6E0 + width].copy_from_slice(&input.to_le_bytes()[..width]);
                    indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
                    let context = format!("{ty}/{divisor}/{mir}/{runtime:?}");
                    for address in [0x600, 0x608, 0x6E0] {
                        assert_eq!(read(&memory, address, width), input / divisor, "{context}");
                    }
                    for address in [0x604, 0x60C] {
                        assert_eq!(read(&memory, address, width), input % divisor, "{context}");
                    }
                    assert_eq!(memory[0x620], 2, "{context}");
                }
            }
        }
    }
}

#[test]
fn signed_power_two_divmod_and_compatibility_keep_helpers() {
    for (ty, width, input, expected_q, expected_r, profile) in [
        (
            "INT",
            2,
            (-9i32) as u32,
            (-1i32) as u32,
            (-1i32) as u32,
            CodegenProfile::Modern,
        ),
        (
            "LONGINT",
            4,
            (-9i32) as u32,
            (-1i32) as u32,
            (-1i32) as u32,
            CodegenProfile::Modern,
        ),
        ("CARD", 2, 65535, 8191, 7, CodegenProfile::Compat),
    ] {
        for mir in [false, true] {
            if mir && profile == CodegenProfile::Compat {
                continue;
            }
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let source = format!(
                    "{ty} input=$6E0,q=$600,r=$604 PROC Main() q=input/8 r=input MOD 8 RETURN"
                );
                let output = compile(&source, mir, runtime, profile);
                assert!(has_divmod_helper(&output));
                let mut memory = memory(&output);
                memory[0x6E0..0x6E0 + width].copy_from_slice(&input.to_le_bytes()[..width]);
                indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
                assert_eq!(
                    &memory[0x600..0x600 + width],
                    &expected_q.to_le_bytes()[..width]
                );
                assert_eq!(
                    &memory[0x604..0x604 + width],
                    &expected_r.to_le_bytes()[..width]
                );
            }
        }
    }
}

#[test]
fn power_two_divmod_uses_promoted_signedness_and_preserves_nested_values() {
    let source = "INT input=$6E0 CARD q=$600,r=$602,nested=$604\n\
        LONGCARD wideq=$608,wider=$60C BYTE byteq=$610,byter=$611\n\
        PROC Main() q=input/CARD(8) r=input MOD CARD(8)\n\
        nested=(CARD(input)/8)+(CARD(input) MOD 8)\n\
        wideq=input/LONGCARD(8) wider=input MOD LONGCARD(8)\n\
        byteq=BYTE(input)/8 byter=BYTE(input) MOD 8 RETURN";
    for mir in [false, true] {
        let output = compile(source, mir, Runtime::Standalone, CodegenProfile::Modern);
        assert!(!has_divmod_helper(&output));
        for input in [-32768i16, -257, -9, -1, 0, 9, 32767] {
            let mut memory = memory(&output);
            memory[0x6E0..0x6E2].copy_from_slice(&input.to_le_bytes());
            indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
            let narrow = input as u16;
            let wide = i32::from(input) as u32;
            assert_eq!(read(&memory, 0x600, 2), u32::from(narrow / 8));
            assert_eq!(read(&memory, 0x602, 2), u32::from(narrow % 8));
            assert_eq!(read(&memory, 0x604, 2), u32::from(narrow / 8 + narrow % 8));
            assert_eq!(read(&memory, 0x608, 4), wide / 8);
            assert_eq!(read(&memory, 0x60C, 4), wide % 8);
            assert_eq!(memory[0x610], (input as u8) / 8);
            assert_eq!(memory[0x611], (input as u8) % 8);
        }
    }
}

#[test]
fn power_two_divmod_accepts_named_and_folded_divisors() {
    let source = "CONST LONGCARD divisor=65536 CARD word=$6E0,q=$600,r=$602\n\
        LONGCARD wide=$6E4,wq=$604,wr=$608\n\
        PROC Main() q=word/(2+6) r=word MOD (2+6)\n\
        wq=wide/divisor wr=wide MOD divisor RETURN";
    for mir in [false, true] {
        let output = compile(source, mir, Runtime::Standalone, CodegenProfile::Modern);
        assert!(!has_divmod_helper(&output));
        let mut memory = memory(&output);
        memory[0x6E0..0x6E2].copy_from_slice(&65535u16.to_le_bytes());
        memory[0x6E4..0x6E8].copy_from_slice(&0xFEDC_BA98u32.to_le_bytes());
        indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
        assert_eq!(read(&memory, 0x600, 2), 8191);
        assert_eq!(read(&memory, 0x602, 2), 7);
        assert_eq!(read(&memory, 0x604, 4), 0xFEDC);
        assert_eq!(read(&memory, 0x608, 4), 0xBA98);
    }
}

#[test]
fn power_two_divmod_keeps_complete_volatile_dividend_reads() {
    for (ty, width) in [("BYTE", 1), ("CARD", 2), ("LONGCARD", 4)] {
        for expression in ["input MOD 1", "input MOD 8", "input/CARD(256)"] {
            let source = format!(
                "VOLATILE {ty} input=$6E0 {ty} result=$600 PROC Main() result={expression} RETURN"
            );
            for mir in [false, true] {
                let output = compile(&source, mir, Runtime::Standalone, CodegenProfile::Modern);
                assert!(
                    !has_divmod_helper(&output),
                    "{ty}/{expression}/{mir}: {:?}",
                    output.routine_addresses
                );
                let mut reads = vec![0; width];
                let mut offset = 0;
                while offset < output.bytes.len() {
                    let (name, mode, len) = decode_6502_opcode(output.bytes[offset]).unwrap();
                    if mode == AddressingMode::Absolute && matches!(name, "LDA" | "LDX" | "LDY") {
                        let address = usize::from(u16::from_le_bytes([
                            output.bytes[offset + 1],
                            output.bytes[offset + 2],
                        ]));
                        if (0x6E0..0x6E0 + width).contains(&address) {
                            reads[address - 0x6E0] += 1;
                        }
                    }
                    offset += len;
                }
                assert_eq!(reads, vec![1; width], "{ty}/{expression}/{mir}");
            }
        }
    }
}
