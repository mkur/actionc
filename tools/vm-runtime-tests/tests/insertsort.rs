//! TACLeBench insertion sort: unsigned wide arrays, nested loops, and C state.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/insertsort/insertsort.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/insertsort/vectors.txt");
const HOST_BASE: u16 = 0x0600;
const HOST_BYTES: usize = 0x0400;
const STATS: usize = 0x0701 - HOST_BASE as usize;
const VALUES: usize = 0x07FF - HOST_BASE as usize;
const INPUT: usize = 0x08FF - HOST_BASE as usize;

#[derive(Debug, PartialEq, Eq)]
struct Vector {
    label: String,
    command: u8,
    input_stats: Vec<u8>,
    input_values: Vec<u8>,
    expected_stats: Vec<u8>,
    expected_values: Vec<u8>,
    result: u8,
}

fn hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect()
}

fn parse_vectors(text: &str) -> Vec<Vector> {
    // Host LF/CRLF are interchangeable; decoded memory bytes remain exact.
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 7);
            let vector = Vector {
                label: fields[0].into(),
                command: fields[1].parse().unwrap(),
                input_stats: hex(fields[2]),
                input_values: hex(fields[3]),
                expected_stats: hex(fields[4]),
                expected_values: hex(fields[5]),
                result: fields[6].parse().unwrap(),
            };
            assert!(vector.command <= 1 && vector.result <= 1);
            assert_eq!(vector.input_stats.len(), 24);
            assert_eq!(vector.expected_stats.len(), 24);
            assert_eq!(vector.input_values.len(), 44);
            assert_eq!(vector.expected_values.len(), 44);
            if vector.command == 1 {
                let input = words(&vector.input_values);
                assert!(input[1..].iter().all(|value| *value >= input[0]));
            }
            vector
        })
        .collect()
}

#[test]
fn insertsort_vectors_accept_lf_and_crlf_and_preserve_upstream_results() {
    let text = VECTORS.replace("\r\n", "\n");
    let vectors = parse_vectors(&text);
    assert_eq!(vectors, parse_vectors(&text.replace('\n', "\r\n")));
    assert_eq!(vectors.len(), 209);
    let original = &vectors[0];
    assert_eq!(original.label, "upstream");
    assert_eq!(original.command, 0);
    assert_eq!(original.result, 0);
    assert_eq!(
        words(&original.expected_values),
        [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    assert_eq!(words(&original.expected_stats), [9, 9, 9, 9, 1, 9]);
    let wrap = vectors
        .iter()
        .find(|v| v.label == "checksum-wrap-65")
        .unwrap();
    assert_eq!(wrap.result, 0);
    assert_eq!(words(&wrap.expected_values)[10], u32::MAX);
}

struct TemporarySource(PathBuf);
impl TemporarySource {
    fn new(mode: CompileMode, runtime: Runtime) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "actionc-insertsort-{}-{unique}-{mode:?}-{runtime:?}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TemporarySource {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check_insertsort(mode: CompileMode, runtime: Runtime) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temporary = TemporarySource::new(mode, runtime);
    // Both newline conventions pass through compilation and actual VM input.
    let text = |source: &str| {
        let lf = source.replace("\r\n", "\n");
        if runtime == Runtime::Standalone {
            lf.replace('\n', "\r\n")
        } else {
            lf
        }
    };
    let path = temporary.0.join("insertsort.act");
    std::fs::write(&path, text(SOURCE)).unwrap();
    let compiled = compile_file(
        &path,
        &CompileOptions::for_mode(mode)
            .with_runtime(runtime)
            .with_origin(0x3000),
    )
    .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
    let mut total_steps = 0u64;
    for vector in parse_vectors(&text(VECTORS)) {
        let label = format!("{mode:?}/{runtime:?}/{}", vector.label);
        let mut vm = CompilerVm::default();
        let profile = match runtime {
            Runtime::Standalone => ExecutionProfile::StandaloneObject,
            Runtime::ActionCart => {
                for (kind, name, base) in [
                    (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
                    (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
                ] {
                    vm.load_image_bytes(
                        kind,
                        name,
                        base,
                        std::fs::read(root.join("roms").join(name)).unwrap(),
                    )
                    .unwrap();
                }
                ExecutionProfile::CartridgeObject
            }
        };
        let loaded = vm
            .load_atari_object_for_execution(profile, compiled.object_bytes())
            .unwrap();
        assert!(
            loaded
                .segments
                .iter()
                .all(|s| s.end < HOST_BASE || s.start >= 0x0A00),
            "host data overlaps object: {label}"
        );
        let mut expected = vec![0xCC; HOST_BYTES];
        expected[0] = vector.command;
        expected[STATS..STATS + 24].copy_from_slice(&vector.input_stats);
        expected[VALUES..VALUES + 44].fill(0xA5);
        expected[INPUT..INPUT + 44].copy_from_slice(&vector.input_values);
        vm.bus_mut().ram_mut().map(HOST_BASE, &expected).unwrap();
        expected[1] = vector.result;
        expected[0xFF] = 0xA5;
        expected[STATS..STATS + 24].copy_from_slice(&vector.expected_stats);
        expected[VALUES..VALUES + 44].copy_from_slice(&vector.expected_values);
        let limit = 1_000_000;
        let mut steps = 0u64;
        while vm.bus().ram().read(0x06FF) != 0xA5 && steps < limit {
            vm.step_cpu()
                .unwrap_or_else(|error| panic!("{label}: {error:?}"));
            steps += 1;
        }
        assert_eq!(
            vm.bus().ram().read(0x06FF),
            0xA5,
            "{label} did not complete in {limit} steps; PC=${:04X}",
            vm.cpu().registers().pc
        );
        for (offset, &value) in expected.iter().enumerate() {
            let address = HOST_BASE + offset as u16;
            assert_eq!(
                vm.bus().ram().read(address),
                value,
                "{label}: ${address:04X}"
            );
        }
        total_steps += steps;
        if vector.label == "upstream" {
            eprintln!("{label}: {steps} instructions");
        }
    }
    eprintln!(
        "Insertion sort {mode:?}/{runtime:?}: {} object bytes, {total_steps} instructions",
        compiled.object_bytes().len()
    );
}

#[test]
fn insertsort_mir6502_cart_matches_complete_c_state() {
    check_insertsort(CompileMode::Mir6502, Runtime::ActionCart);
}

#[test]
fn insertsort_mir6502_standalone_matches_complete_c_state() {
    check_insertsort(CompileMode::Mir6502, Runtime::Standalone);
}

#[test]
fn insertsort_classic_cart_matches_complete_c_state() {
    check_insertsort(CompileMode::Optimized, Runtime::ActionCart);
}

#[test]
fn insertsort_classic_standalone_matches_complete_c_state() {
    check_insertsort(CompileMode::Optimized, Runtime::Standalone);
}
