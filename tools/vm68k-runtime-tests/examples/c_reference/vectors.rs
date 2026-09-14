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
    let text = match name {
        "insertsort" => include_str!("../../../../fixtures/runtime/tacle/insertsort/vectors.txt"),
        "matrix1" => include_str!("../../../../fixtures/runtime/tacle/matrix1/vectors.txt"),
        _ => unreachable!(),
    };
    let mut total = 0;
    let mut count = 0;
    for line in text
        .lines()
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let f: Vec<_> = line.split_whitespace().collect();
        if name == "matrix1" && (f[0] != "LONGINT" || f[1] != "10x10x10") {
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
