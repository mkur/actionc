//! TACLeBench's nested state machines, checked against the pinned C reference.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/statemate-vectors.txt");
const DRIVER: &str = include_str!("../fixtures/statemate_driver.act");
const CORE: &str = include_str!("../../../fixtures/runtime/tacle/statemate-kernel.inc");
const STATE: u16 = 0x07A1;
const STATE_BYTES: usize = 201;
const SIGNATURE: u16 = 0x06FF;
const POISON: u8 = 0xCC;

#[derive(Debug, PartialEq, Eq)]
struct Vector {
    label: String,
    command: u8,
    input: Vec<u8>,
    expected: Vec<u8>,
}

fn parse_vectors(text: &str) -> Vec<Vector> {
    fn hex(text: &str) -> Vec<u8> {
        assert_eq!(text.len(), STATE_BYTES * 2);
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }
    // lines() accepts LF and CRLF; binary state bytes are decoded unchanged.
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 4, "invalid vector: {line}");
            let command = fields[1].parse().unwrap();
            assert!(command <= 6);
            Vector {
                label: fields[0].into(),
                command,
                input: hex(fields[2]),
                expected: hex(fields[3]),
            }
        })
        .collect()
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct SourceDirectory(PathBuf);
impl SourceDirectory {
    fn new(crlf: bool) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "actionc-statemate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        for (name, source) in [("driver.act", DRIVER), ("statemate-kernel.inc", CORE)] {
            let lf = source.replace("\r\n", "\n");
            std::fs::write(
                directory.join(name),
                if crlf { lf.replace('\n', "\r\n") } else { lf },
            )
            .unwrap();
        }
        Self(directory)
    }
}
impl Drop for SourceDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn state_field(address: u16) -> String {
    let offset = usize::from(address.saturating_sub(STATE));
    for line in include_str!("../../../fixtures/runtime/tacle/state.tsv").lines() {
        if line.starts_with('#') {
            continue;
        }
        let row: Vec<_> = line.split('\t').collect();
        let start: usize = row[3].parse().unwrap();
        let size: usize = row[4].parse().unwrap();
        if address >= STATE && (start..start + size).contains(&offset) {
            return format!("{} + {}", row[1], offset - start);
        }
    }
    "guard/command/completion".into()
}

#[test]
fn statemate_vectors_accept_lf_and_crlf_without_changing_state_bytes() {
    let lf = VECTORS.replace("\r\n", "\n");
    let vectors = parse_vectors(&lf);
    assert_eq!(vectors, parse_vectors(&lf.replace('\n', "\r\n")));
    assert!(vectors.len() >= 150);
    assert_eq!(vectors[0].label, "startup");
    assert_eq!(vectors[0].input, vec![0; STATE_BYTES]);
    assert_eq!(vectors[0].expected[..64].iter().sum::<u8>(), 1);
    assert_eq!(vectors[0].expected[5], 1);
}

#[test]
fn statemate_nested_cases_match_complete_c_state_in_both_backends_and_runtimes() {
    let lf = VECTORS.replace("\r\n", "\n");
    let mut failures = Vec::new();
    for mode in [CompileMode::Mir6502, CompileMode::Optimized] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            // Exercise both newline forms through the actual VM vector path.
            let text = match runtime {
                Runtime::ActionCart => lf.clone(),
                Runtime::Standalone => lf.replace('\n', "\r\n"),
            };
            let source = SourceDirectory::new(runtime == Runtime::Standalone);
            let compiled = compile_file(
                source.0.join("driver.act"),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
            let mut total_steps = 0;
            for vector in parse_vectors(&text) {
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
                                std::fs::read(root().join("roms").join(name)).unwrap(),
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
                        .all(|segment| { segment.end < 0x0600 || segment.start > 0x08FF }),
                    "host state overlaps object: {label}"
                );

                let mut expected = vec![POISON; 0x300];
                expected[0] = vector.command;
                expected[usize::from(STATE - 0x0600)..usize::from(STATE - 0x0600) + STATE_BYTES]
                    .copy_from_slice(&vector.input);
                vm.bus_mut().ram_mut().map(0x0600, &expected).unwrap();
                expected[0xFF] = 0xA5;
                expected[usize::from(STATE - 0x0600)..usize::from(STATE - 0x0600) + STATE_BYTES]
                    .copy_from_slice(&vector.expected);

                let limit = if matches!(vector.command, 0 | 6) {
                    1_000_000
                } else {
                    50_000
                };
                let mut steps = 0;
                while vm.bus().ram().read(SIGNATURE) != 0xA5 && steps < limit {
                    vm.step_cpu()
                        .unwrap_or_else(|error| panic!("{label}: {error:?}"));
                    steps += 1;
                }
                assert_eq!(
                    vm.bus().ram().read(SIGNATURE),
                    0xA5,
                    "{label} did not complete in {limit} steps; PC=${:04X}",
                    vm.cpu().registers().pc
                );
                total_steps += steps;
                for (offset, value) in expected.iter().enumerate() {
                    let address = 0x0600 + offset as u16;
                    let actual = vm.bus().ram().read(address);
                    if actual != *value {
                        failures.push(format!(
                            "{label}: ${address:04X} ({}): expected {value}, got {actual}",
                            state_field(address)
                        ));
                        break;
                    }
                }
            }
            eprintln!(
                "Statemate {mode:?}/{runtime:?}: {} object bytes, {total_steps} instructions",
                compiled.object_bytes().len()
            );
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
