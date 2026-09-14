mod common;
#[path = "common/reference.rs"]
mod reference;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;
use reference::{bytes, text, words};
const CORE: &str = include_str!("../../../fixtures/runtime/tacle/sha/kernel.inc");
const DRIVER: &str = include_str!("../../../fixtures/runtime/tacle/sha/sha.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/sha/vectors.txt");
fn execute(optimize: bool) {
    let source = common::Source::new(&text(DRIVER, optimize));
    std::fs::write(
        source.0.parent().unwrap().join("kernel.inc"),
        text(CORE, optimize),
    )
    .unwrap();
    let image = compile_file(
        &source.0,
        &NativeCompileOptions {
            optimize,
            ..Default::default()
        },
    )
    .unwrap()
    .image;
    let vectors = text(VECTORS, optimize);
    let mut executed = 0;
    for line in vectors
        .lines()
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let f: Vec<_> = line.split_whitespace().collect();
        assert_eq!(f.len(), 5);
        let command = f[1].parse().unwrap();
        assert!(command <= 2);
        let state = words(&bytes(f[2]), 4);
        let message = bytes(f[3]);
        let expected = words(&bytes(f[4]), 4);
        assert_eq!(state.len(), 23);
        assert_eq!(expected.len(), 103);
        let mut vm = Machine::from_image(&image).unwrap();
        for (name, value) in [
            ("command", command),
            ("length", message.len() as u32),
            ("countLo", state[5]),
            ("countHi", state[6]),
        ] {
            vm.write_scalar(image.symbol(name).unwrap(), value).unwrap();
        }
        vm.write_array(image.symbol("digest").unwrap(), &state[..5])
            .unwrap();
        vm.write_array(image.symbol("data").unwrap(), &state[7..])
            .unwrap();
        vm.write_array(image.symbol("w").unwrap(), &[0xcccccccc; 80])
            .unwrap();
        let mut input = vec![0xcc; 1025];
        for (slot, byte) in input.iter_mut().zip(&message) {
            *slot = u32::from(*byte);
        }
        vm.write_array(image.symbol("message").unwrap(), &input)
            .unwrap();
        let run = vm.run(2_000_000 * (message.len() as u64 / 64 + 2));
        assert!(
            matches!(run.outcome, actionc_vm68k_tests::Outcome::Completed),
            "{optimize}/{}: {run:#?}",
            f[0]
        );
        let mut actual = vm.read_array(image.symbol("digest").unwrap()).unwrap();
        for name in ["countLo", "countHi"] {
            actual.push(vm.read_scalar(image.symbol(name).unwrap()).unwrap());
        }
        actual.extend(vm.read_array(image.symbol("data").unwrap()).unwrap());
        actual.extend(vm.read_array(image.symbol("w").unwrap()).unwrap());
        assert_eq!(actual, expected, "{optimize}/{}/state and schedule", f[0]);
        assert_eq!(
            vm.read_array(image.symbol("message").unwrap()).unwrap(),
            input,
            "{optimize}/{}/message modified",
            f[0]
        );
        assert_eq!(
            vm.read_scalar(image.symbol("command").unwrap()).unwrap(),
            command
        );
        assert_eq!(
            vm.read_scalar(image.symbol("length").unwrap()).unwrap(),
            message.len() as u32
        );
        if matches!(f[0], "abc" | "fips-56") {
            let digest = actual[..5]
                .iter()
                .map(|w| format!("{w:08x}"))
                .collect::<String>();
            assert_eq!(
                digest,
                if f[0] == "abc" {
                    "0164b8a914cd2a5e74c4f7ff082c4d97f1edf880"
                } else {
                    "d2516ee1acfa5baf33dfc1c471e438449ef134c8"
                }
            );
        }
        executed += 1;
    }
    assert_eq!(executed, 155);
}
#[test]
fn sha_raw_matches_complete_reference_state_and_schedule() {
    execute(false);
}
#[test]
fn sha_optimized_matches_complete_reference_state_and_schedule() {
    execute(true);
}
