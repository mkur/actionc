mod support;
use support::*;

fn number(h: &Harness, at: u32, bytes: usize) -> u32 {
    h.bus.ram[at as usize..at as usize + bytes]
        .iter()
        .enumerate()
        .fold(0, |n, (i, b)| n | (u32::from(*b) << (8 * i)))
}

#[test]
fn accumulator_homes_widen_bytes_and_words_with_dirty_high_bits() {
    for (from, input_bytes) in [("BYTE", 1), ("CARD", 2)] {
        for (to, output_bytes) in [("CARD", 2), ("SIZE", 3), ("LONGCARD", 4)] {
            if input_bytes >= output_bytes {
                continue;
            }
            for optimize in [false, true] {
                let image = compile(
                    &format!(
                        "{from} input {to} output \
                    {to} FUNC Work({from} value) RETURN({to}(value)) \
                    PROC Main() output=Work(input) RETURN"
                    ),
                    optimize,
                );
                let input = context::symbol(&image, "input");
                let output = context::symbol(&image, "output");
                let routine = image.routines.iter().find(|r| r.name == "Work").unwrap();
                if to != "SIZE" {
                    assert_eq!(routine.fixed_frame, if output_bytes == 4 { 6 } else { 4 });
                }
                for value in [0u32, 1, 0x7f, 0x80, 0xff, 0x100, 0x8000, 0xffff] {
                    let expected = value & if input_bytes == 1 { 0xff } else { 0xffff };
                    for mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller(image.entry), mask);
                        h.bus.ram[input as usize..input as usize + input_bytes]
                            .copy_from_slice(&value.to_le_bytes()[..input_bytes]);
                        h.bus.ram[output as usize..output as usize + output_bytes].fill(0xa5);
                        h.run();
                        h.guards(mask);
                        assert_eq!(
                            number(&h, output, output_bytes),
                            expected,
                            "{from}/{to}/{optimize}/{value}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn accumulator_arithmetic_retains_word_wrap_before_widening() {
    for (op, expected) in [
        ("+", 0x8001u16.wrapping_add(7)),
        ("-", 0x8001u16.wrapping_sub(7)),
        ("&", 0x8001u16 & 7),
        ("OR", 0x8001u16 | 7),
        ("XOR", 0x8001u16 ^ 7),
        ("LSH", 4),
        ("RSH", 0x2000),
    ] {
        let operand = if matches!(op, "LSH" | "RSH") { 2 } else { 7 };
        let rhs = if matches!(op, "LSH" | "RSH") {
            operand.to_string()
        } else {
            format!("CARD({operand})")
        };
        for optimize in [false, true] {
            let image = compile(
                &format!(
                    "CARD input SIZE output \
                SIZE FUNC Work(CARD value) RETURN(SIZE(value {op} {rhs})) \
                PROC Main() output=Work(input) RETURN"
                ),
                optimize,
            );
            let input = context::symbol(&image, "input") as usize;
            let output = context::symbol(&image, "output");
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                h.bus.ram[input..input + 2].copy_from_slice(&0x8001u16.to_le_bytes());
                h.run();
                h.guards(mask);
                assert_eq!(
                    number(&h, output, 3),
                    u32::from(expected),
                    "{op}/{optimize}"
                );
            }
        }
    }
    for (op, input, expected) in [("+", 0xffffu16, 6u32), ("-", 0u16, 0xfff9)] {
        let image = compile(
            &format!(
                "SIZE output SIZE FUNC Work(CARD value) RETURN(SIZE(value {op} CARD(7))) PROC Main() output=Work({input}) RETURN"
            ),
            true,
        );
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        h.run();
        h.guards(0);
        assert_eq!(number(&h, context::symbol(&image, "output"), 3), expected);
    }
}

#[test]
fn direct_global_loads_widen_without_reading_neighbor_bytes() {
    for (ty, bytes, value) in [("BYTE", 1, 0x80u32), ("CARD", 2, 0x8001)] {
        for optimize in [false, true] {
            let mut options = layout();
            options.data_origin = 0x12ffff;
            let compiled = prepare(&format!("{ty} input BYTE neighbor LONGCARD output PROC Main() output=LONGCARD(input) RETURN"), optimize)
                .compile(&options).unwrap();
            let image =
                actionc::mir65816::image::Image::from_json(&compiled.image.to_json().unwrap())
                    .unwrap();
            let input = context::symbol(&image, "input");
            let neighbor = context::symbol(&image, "neighbor");
            let output = context::symbol(&image, "output");
            let mut h = Harness::new(&image, &caller(image.entry), 0);
            h.bus.ram[input as usize..input as usize + bytes]
                .copy_from_slice(&value.to_le_bytes()[..bytes]);
            h.bus.ram[neighbor as usize] = 0xa5;
            h.run();
            h.guards(0);
            assert_eq!(number(&h, output, 4), value);
            assert_eq!(h.bus.ram[neighbor as usize], 0xa5);
            assert!(!h.bus.reads.contains(&neighbor));
            assert_eq!(
                h.bus
                    .reads
                    .iter()
                    .filter(|a| (input..input + bytes as u32).contains(a))
                    .count(),
                bytes
            );
        }
    }
}

#[test]
fn metadata_expression_survives_the_multiply_helper_and_restores_guards() {
    for optimize in [false, true] {
        let image = compile(
            "CARD input SIZE output \
            SIZE FUNC MetadataBytes(CARD blocks) RETURN(SIZE(blocks)*12+SIZE(blocks RSH 2)) \
            PROC Main() output=MetadataBytes(input) RETURN",
            optimize,
        );
        let input = context::symbol(&image, "input") as usize;
        let output = context::symbol(&image, "output");
        for blocks in [0u16, 1, 3, 4, 512, 4096, 0x8000, 0xffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                h.bus.ram[input..input + 2].copy_from_slice(&blocks.to_le_bytes());
                h.run();
                h.guards(mask);
                assert_eq!(
                    number(&h, output, 3),
                    u32::from(blocks) * 12 + u32::from(blocks >> 2)
                );
            }
        }
    }
}
