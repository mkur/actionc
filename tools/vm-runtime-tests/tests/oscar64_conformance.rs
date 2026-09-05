//! Behavioral ports of Oscar64's first eight selected autotests.
//! Oracles are computed here, independently of the Action! code under test.
use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};

const SIGNATURE: u16 = 0x06FF;
const POISON: u8 = 0xCC;
const ALL_MODES: &[CompileMode] = &[
    CompileMode::Compatibility,
    CompileMode::Optimized,
    CompileMode::Mir6502,
];
const CLASSIC_MODES: &[CompileMode] = &[CompileMode::Compatibility, CompileMode::Optimized];
const MIR_MODE: &[CompileMode] = &[CompileMode::Mir6502];

#[derive(Default)]
struct Case {
    label: String,
    setup: Vec<(u16, Vec<u8>)>,
    expected: Vec<(u16, Vec<u8>)>,
}

impl Case {
    fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    fn input(&mut self, address: u16, value: u16) {
        self.setup.push((address, value.to_le_bytes().to_vec()));
    }

    fn word(&mut self, address: u16, value: u16) {
        self.expected.push((address, value.to_le_bytes().to_vec()));
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_cases(name: &str, max_steps: u64, cases: &[Case]) {
    run_cases_in_modes(name, max_steps, cases, ALL_MODES);
}

fn run_cases_in_modes(name: &str, max_steps: u64, cases: &[Case], modes: &[CompileMode]) {
    let root = repository_root();
    let mut failures = Vec::new();
    for &mode in modes {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                root.join("fixtures/runtime/oscar64")
                    .join(format!("{name}.act")),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|error| panic!("compile {name}/{mode:?}/{runtime:?}: {error}"));
            for case in cases {
                let label = format!("{name}/{mode:?}/{runtime:?}/{}", case.label);
                let mut vm = CompilerVm::default();
                let profile = match runtime {
                    Runtime::Standalone => ExecutionProfile::StandaloneObject,
                    Runtime::ActionCart => {
                        for (kind, file, base) in [
                            (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
                            (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
                        ] {
                            vm.load_image_bytes(
                                kind,
                                file,
                                base,
                                std::fs::read(root.join("roms").join(file)).expect("read ROM"),
                            )
                            .expect("load ROM");
                        }
                        ExecutionProfile::CartridgeObject
                    }
                };
                let loaded = vm
                    .load_atari_object_for_execution(profile, compiled.object_bytes())
                    .unwrap_or_else(|error| panic!("load {label}: {error}"));
                // Host-owned result/configuration and guarded test buffers
                // must never overwrite generated code, data, or helpers.
                for (start, len) in std::iter::once((0x0600, 0x100)).chain(
                    case.setup
                        .iter()
                        .map(|(start, bytes)| (*start, bytes.len())),
                ) {
                    let end = usize::from(start) + len;
                    assert!(end <= 0x10000, "host buffer wraps for {label}");
                    for segment in &loaded.segments {
                        assert!(
                            end <= usize::from(segment.start) || start > segment.end,
                            "host buffer overlaps loaded segment {segment:?} for {label}"
                        );
                    }
                }
                // Unwritten result bytes, including the completion marker,
                // must not accidentally equal an expected zero.
                for address in 0x0600..=SIGNATURE {
                    vm.bus_mut().ram_mut().write(address, POISON);
                }
                for (start, bytes) in &case.setup {
                    for (offset, value) in bytes.iter().enumerate() {
                        vm.bus_mut().ram_mut().write(start + offset as u16, *value);
                    }
                }
                let outcome = VmRunner::new(vm).run(RunRequest {
                    max_steps,
                    history_len: 16,
                    ..RunRequest::default()
                });
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected stop for {label}: {:?}",
                    outcome.report
                );
                assert_eq!(
                    outcome.memory().read(SIGNATURE),
                    0xA5,
                    "{label} did not complete within {max_steps} steps: {:?}",
                    outcome.report
                );
                'oracle: for (start, bytes) in &case.expected {
                    for (offset, expected) in bytes.iter().enumerate() {
                        let address = start + offset as u16;
                        let actual = outcome.memory().read(address);
                        if actual != *expected {
                            let result = u16::from_le_bytes([
                                outcome.memory().read(0x0600),
                                outcome.memory().read(0x0601),
                            ]);
                            failures.push(format!(
                                "{label}: memory ${address:04X}: expected ${expected:02X}, \
                                 got ${actual:02X}; result word $0600=${result:04X}"
                            ));
                            break 'oracle;
                        }
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn write_word(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn oscar64_byte_indexes_cover_inline_fixed_and_descriptor_storage() {
    let cases = [0u16, 1, 20, 127, 128, 255, 256, 257].map(|count| {
        let mut case = Case::new(format!("count={count}"));
        case.input(0x06F0, count);
        let mut expected = vec![0xA5; 0x300];
        case.setup.push((0x4F00, expected.clone()));
        for i in 0..usize::from(count) {
            expected[0x101 + i] = i as u8;
        }
        case.expected.push((0x4F00, expected));
        case.expected.push((0x0600, vec![190]));
        let sum = (0..count).map(|i| u16::from(i as u8)).sum();
        case.word(0x0602, sum);
        case.word(0x0604, sum);
        case
    });
    run_cases("byteindextest", 100_000, &cases);
}

#[test]
fn oscar64_word_indexes_survive_get_put_calls() {
    let cases = [0u16, 1, 127, 128, 255, 256].map(|index| {
        let mut case = Case::new(format!("index={index}"));
        let value = 0xFFF3u16.wrapping_add(index * 113);
        case.input(0x06F0, index);
        case.input(0x06F2, value);
        let mut expected = vec![0xA5; 0x500];
        case.setup.push((0x4F00, expected.clone()));
        write_word(&mut expected, 0x101 + usize::from(index) * 2, value);
        case.expected.push((0x4F00, expected));
        case.word(0x0600, 0);
        case.word(0x0602, value);
        case
    });
    run_cases("arrayindexintrangecheck", 50_000, &cases);
}

fn offset_cases(indexes: &[u16]) -> Vec<Case> {
    let mut cases = Vec::new();
    for base in [0x5000u16, 0x50F1] {
        for &index in indexes {
            let mut case = Case::new(format!("base={base:04X}, index={index}"));
            case.input(0x06F0, base);
            case.input(0x06F2, index);
            let mut expected = vec![0xA5; 0x500];
            case.setup.push((0x4F00, expected.clone()));
            for i in 0..4 {
                let offset = usize::from(base - 0x4F00) + (usize::from(index) + 3 + i) * 2;
                write_word(&mut expected, offset, (i + 1) as u16);
            }
            case.expected.push((0x4F00, expected));
            case.word(0x0600, 0);
            case.word(0x0602, 10);
            cases.push(case);
        }
    }
    cases
}

#[test]
fn oscar64_offsets_compose_with_word_pointer_indexing() {
    run_cases("arrayoffsetindex", 50_000, &offset_cases(&[4]));
}

#[test]
fn oscar64_mir_word_pointer_offsets_cross_index_128() {
    run_cases_in_modes(
        "arrayoffsetindex",
        50_000,
        &offset_cases(&[123, 124, 252]),
        MIR_MODE,
    );
}

#[test]
fn oscar64_classic_word_pointer_offsets_cross_index_128() {
    run_cases_in_modes(
        "arrayoffsetindex",
        50_000,
        &offset_cases(&[123, 124, 252]),
        CLASSIC_MODES,
    );
}

fn vector_cases(copy: bool, counts: &[u16]) -> Vec<Case> {
    counts
        .iter()
        .copied()
        .map(|count| {
            let mut case = Case::new(format!("count={count}"));
            let mut initial = vec![0xA5; 0x900];
            let mut expected = initial.clone();
            let words = [0u16, 0xFF, 0xFFFF, 0x7FFF, 0x8000, 0xFFF3, 0x1234];
            for i in 0..usize::from(count) {
                let value = words[i % words.len()];
                let source = 0x101 + i * 2;
                write_word(&mut initial, source, value);
                write_word(
                    &mut expected,
                    source,
                    if copy { value } else { value.wrapping_add(1) },
                );
                if copy {
                    write_word(&mut expected, 0x503 + i * 2, value);
                }
            }
            case.setup.push((0x4F00, initial));
            case.expected.push((0x4F00, expected));
            case.input(0x06F0, 0x5001);
            if copy {
                case.input(0x06F2, 0x5403);
                case.input(0x06F4, count);
            } else {
                case.input(0x06F2, count);
            }
            case.word(0x0600, 0);
            case
        })
        .collect()
}

#[test]
fn oscar64_word_vector_copy_preserves_source_and_guards() {
    run_cases_in_modes(
        "copyintvec",
        200_000,
        &vector_cases(true, &[0, 1, 100, 127, 128]),
        CLASSIC_MODES,
    );
}

#[test]
fn oscar64_word_vector_increment_preserves_carry_and_guards() {
    run_cases_in_modes(
        "incvector",
        200_000,
        &vector_cases(false, &[0, 1, 100, 127, 128]),
        CLASSIC_MODES,
    );
}

#[test]
fn oscar64_classic_word_vector_copy_crosses_index_128() {
    run_cases_in_modes(
        "copyintvec",
        200_000,
        &vector_cases(true, &[129, 255, 256, 257]),
        CLASSIC_MODES,
    );
}

#[test]
fn oscar64_classic_word_vector_increment_crosses_index_128() {
    run_cases_in_modes(
        "incvector",
        200_000,
        &vector_cases(false, &[129, 255, 256, 257]),
        CLASSIC_MODES,
    );
}

#[test]
fn oscar64_mir_word_vector_copy_preserves_source_and_guards() {
    run_cases_in_modes(
        "copyintvec",
        200_000,
        &vector_cases(true, &[0, 1, 100, 127, 128, 129, 255, 256, 257]),
        MIR_MODE,
    );
}

#[test]
fn oscar64_mir_word_vector_increment_preserves_carry_and_guards() {
    run_cases_in_modes(
        "incvector",
        200_000,
        &vector_cases(false, &[0, 1, 100, 127, 128, 129, 255, 256, 257]),
        MIR_MODE,
    );
}

#[test]
fn oscar64_loop_bounds_exclude_sentinel_elements() {
    let mut case = Case::new("strict/inclusive ascending/descending, INT/CARD sums");
    let mut results = Vec::new();
    for count in [50u16, 100] {
        let sum: u16 = (0..count).map(|i| (i & 15) + 3).sum();
        for _ in 0..8 {
            results.extend(sum.to_le_bytes());
        }
    }
    case.expected.push((0x0600, results));
    let mut table = vec![0xA5; 0x300];
    case.setup.push((0x4F00, table.clone()));
    for i in 0..100usize {
        write_word(&mut table, 0x101 + 2 * i, (i as u16 & 15) + 3);
    }
    write_word(&mut table, 0x101 + 200, 1000);
    case.expected.push((0x4F00, table));
    run_cases("loopboundtest", 300_000, &[case]);
}

#[test]
fn oscar64_range_comparisons_retain_exact_counts() {
    let mut case = Case::new("six predicates, five thresholds, signed/unsigned induction");
    let mut expected = Vec::new();
    for _ in 0..2 {
        for op in 0..6 {
            for threshold in [4, 5, 10, 14, 15] {
                let count = (5..15)
                    .filter(|i| match op {
                        0 => *i < threshold,
                        1 => *i <= threshold,
                        2 => *i > threshold,
                        3 => *i >= threshold,
                        4 => *i == threshold,
                        _ => *i != threshold,
                    })
                    .count() as u16;
                expected.extend(count.to_le_bytes());
            }
        }
    }
    case.expected.push((0x0600, expected));
    run_cases("cmprangeshortcuttest", 100_000, &[case]);
}

#[test]
fn oscar64_constant_masks_cover_every_byte_and_bit() {
    let mut case = Case::new("256 byte inputs, eight literal masks, four predicates");
    let mut expected = vec![0xA5; 8194];
    case.setup.push((0x4FFF, expected.clone()));
    for value in 0..256usize {
        for bit in 0..8 {
            let set = value & (1 << bit) != 0;
            for (group, answer) in [set, !set, !set, set].into_iter().enumerate() {
                expected[1 + value * 32 + group * 8 + bit] = u8::from(answer);
            }
        }
    }
    case.expected.push((0x4FFF, expected));
    run_cases("maskcheck", 1_000_000, &[case]);
}
