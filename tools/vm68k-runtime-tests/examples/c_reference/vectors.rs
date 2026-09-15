//! Pinned oracle vectors are numeric little-endian words, independent of 68K.
use super::image::Program;

fn words(hex: &str, width: usize) -> Vec<u32> {
    assert_eq!(hex.len() % (width * 2), 0);
    hex.as_bytes()
        .chunks_exact(width * 2)
        .map(|chunk| {
            let s = std::str::from_utf8(chunk).unwrap();
            (0..width)
                .map(|i| u32::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap() << (8 * i))
                .sum()
        })
        .collect()
}
const STATS: [&str; 6] = ["itersI", "minI", "maxI", "itersA", "minA", "maxA"];

pub fn execute(name: &str, program: &Program) -> (usize, u64) {
    if name.starts_with("jfdctint") {
        return execute_dct(program);
    }
    let text = vector_text(name);
    let mut total = 0;
    let mut count = 0;
    for line in text
        .lines()
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let f: Vec<_> = line.split_whitespace().collect();
        if name.starts_with("matrix1") && (f[0] != "LONGINT" || f[1] != "10x10x10") {
            continue;
        }
        let mut vm = program.machine();
        let command;
        if name == "insertsort" {
            assert_eq!(f.len(), 7);
            command = f[1].parse().unwrap();
            for (stat, value) in STATS.iter().zip(words(f[2], 4)) {
                program.write(&mut vm, stat, 4, &[value]);
            }
            program.write(&mut vm, "input", 4, &words(f[3], 4));
            program.write(&mut vm, "values", 4, &[0xa5a5a5a5; 11]);
        } else {
            assert_eq!(f.len(), 12);
            command = f[3].parse().unwrap();
            for (i, array) in ["matrixA", "matrixB", "matrixC"].iter().enumerate() {
                program.write(&mut vm, array, 4, &words(f[4 + i], 4));
            }
        }
        program.write(&mut vm, "command", 1, &[command]);
        let run = vm.run(2_000_000);
        assert!(
            matches!(run.outcome, actionc_vm68k_tests::Outcome::Completed),
            "{name}/{}: {run:#?}",
            if name == "insertsort" { f[0] } else { f[2] }
        );
        total += run.steps;
        program.check(&vm, "command", 1, &[command]);
        if name == "insertsort" {
            for (stat, value) in STATS.iter().zip(words(f[4], 4)) {
                program.check(&vm, stat, 4, &[value]);
            }
            program.check(&vm, "input", 4, &words(f[3], 4));
            program.check(&vm, "values", 4, &words(f[5], 4));
            program.check(&vm, "result", 1, &[f[6].parse().unwrap()]);
        } else {
            for (i, array) in ["matrixA", "matrixB", "matrixC"].iter().enumerate() {
                program.check(&vm, array, 4, &words(f[7 + i], 4));
            }
            program.check(&vm, "checksum", 4, &words(f[10], 4));
            program.check(&vm, "result", 2, &words(f[11], 2));
        }
        count += 1;
    }
    assert_eq!(count, if name == "insertsort" { 209 } else { 22 });
    (count, total)
}

fn execute_dct(program: &Program) -> (usize, u64) {
    let mut count = 0;
    let mut instructions = 0;
    for line in include_str!("../../../../fixtures/runtime/tacle/jfdctint/vectors.txt")
        .lines()
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let f: Vec<_> = line.split_whitespace().collect();
        assert_eq!(f.len(), 9);
        let mut vm = program.machine();
        for (name, field) in [("testCommand", 1), ("testShift", 2)] {
            program.write(&mut vm, name, 1, &[f[field].parse().unwrap()]);
        }
        program.write(&mut vm, "testInput", 4, &words(f[3], 4));
        program.write(&mut vm, "testRows", 4, &[0xcccccccc; 64]);
        let run = vm.run(2_000_000);
        assert!(
            matches!(run.outcome, actionc_vm68k_tests::Outcome::Completed),
            "DCT/{}: {run:#?}",
            f[0]
        );
        for (name, field) in [
            ("testInput", 3),
            ("testInitial", 4),
            ("testRows", 5),
            ("block", 6),
        ] {
            program.check(&vm, name, 4, &words(f[field], 4));
        }
        for (name, field, width) in [("checksum", 7, 4), ("result", 8, 2)] {
            program.check(&vm, name, width, &words(f[field], width));
        }
        for (name, field) in [("testCommand", 1), ("testShift", 2)] {
            program.check(&vm, name, 1, &[f[field].parse().unwrap()]);
        }
        count += 1;
        instructions += run.steps;
    }
    assert_eq!(count, 181);
    (count, instructions)
}

fn vector_text(name: &str) -> &'static str {
    match name {
        "insertsort" => include_str!("../../../../fixtures/runtime/tacle/insertsort/vectors.txt"),
        "matrix1" | "matrix1-multidimensional" => {
            include_str!("../../../../fixtures/runtime/tacle/matrix1/vectors.txt")
        }
        "jfdctint" | "jfdctint-multidimensional" => {
            include_str!("../../../../fixtures/runtime/tacle/jfdctint/vectors.txt")
        }
        _ => unreachable!(),
    }
}

/// Headline runs also check full final state against the upstream default
/// vector, independently of the separately instrumented DCT corpus builds.
pub fn check_default(name: &str, program: &Program, vm: &actionc_vm68k_tests::Machine) {
    let selected: Vec<Vec<_>> = vector_text(name)
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .filter(|f| {
            if name.starts_with("matrix1") {
                f[0] == "LONGINT" && f[1] == "10x10x10" && f[2] == "upstream"
            } else {
                f[0] == "upstream"
            }
        })
        .collect();
    assert_eq!(selected.len(), 1);
    let f = &selected[0];
    if name == "insertsort" {
        for (stat, value) in STATS.iter().zip(words(f[4], 4)) {
            program.check(vm, stat, 4, &[value]);
        }
        program.check(vm, "values", 4, &words(f[5], 4));
        program.check(vm, "input", 4, &[0; 11]);
        program.check(vm, "result", 1, &[f[6].parse().unwrap()]);
        program.check(vm, "command", 1, &[0]);
    } else if name.starts_with("matrix1") {
        for (i, array) in ["matrixA", "matrixB", "matrixC"].iter().enumerate() {
            program.check(vm, array, 4, &words(f[7 + i], 4));
        }
        program.check(vm, "checksum", 4, &words(f[10], 4));
        program.check(vm, "result", 2, &words(f[11], 2));
        program.check(vm, "command", 1, &[0]);
    } else {
        program.check(vm, "block", 4, &words(f[6], 4));
        program.check(vm, "checksum", 4, &words(f[7], 4));
        program.check(vm, "result", 2, &words(f[8], 2));
    }
}
