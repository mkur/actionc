//! TACLeBench SHA-0: full 32-bit state and schedule versus a fixed-width C oracle.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/sha/sha.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/sha/vectors.txt");
const STATE_BYTES: usize = 92;
const SCHEDULE_BYTES: usize = 320;
const STATE: usize = 0x101; // $0701, relative to the guarded region at $0600
const SCHEDULE: usize = 0x1FD; // $07FD: unaligned and crosses two pages
const MESSAGE: usize = 0x401; // $0A01

#[derive(Debug, PartialEq, Eq)]
struct Vector {
    label: String,
    command: u8,
    state: Vec<u8>,
    message: Vec<u8>,
    expected: Vec<u8>,
}

fn parse_vectors(text: &str) -> Vec<Vector> {
    fn hex(text: &str) -> Vec<u8> {
        if text == "-" {
            return Vec::new();
        }
        assert_eq!(text.len() % 2, 0);
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }
    // lines() accepts host LF and CRLF without changing any binary input bytes.
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 5, "invalid SHA vector: {line}");
            let vector = Vector {
                label: fields[0].into(),
                command: fields[1].parse().unwrap(),
                state: hex(fields[2]),
                message: hex(fields[3]),
                expected: hex(fields[4]),
            };
            assert!(vector.command <= 2);
            assert_eq!(vector.state.len(), STATE_BYTES);
            assert_eq!(vector.expected.len(), STATE_BYTES + SCHEDULE_BYTES);
            assert!(vector.message.len() <= 1025);
            vector
        })
        .collect()
}

#[test]
fn sha_vectors_accept_lf_and_crlf_and_include_published_digests() {
    let lf = VECTORS.replace("\r\n", "\n");
    let vectors = parse_vectors(&lf);
    assert_eq!(vectors, parse_vectors(&lf.replace('\n', "\r\n")));
    assert_eq!(vectors.len(), 155);
    // SHA-0 answers also published in OpenSSL 1.0.2 crypto/sha/shatest.c.
    // This independent check prevents accidentally accepting SHA-1 vectors.
    for (name, digest) in [
        ("abc", "0164b8a914cd2a5e74c4f7ff082c4d97f1edf880"),
        ("fips-56", "d2516ee1acfa5baf33dfc1c471e438449ef134c8"),
    ] {
        let vector = vectors.iter().find(|vector| vector.label == name).unwrap();
        let actual = vector.expected[..20]
            .chunks_exact(4)
            .map(|word| format!("{:08x}", u32::from_le_bytes(word.try_into().unwrap())))
            .collect::<String>();
        assert_eq!(actual, digest, "{name}");
    }
    for length in 0..=65 {
        assert!(
            vectors
                .iter()
                .any(|v| v.command == 0 && v.message.len() == length)
        );
    }
}

struct TemporarySource(PathBuf);

impl TemporarySource {
    fn new() -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("actionc-sha-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TemporarySource {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sha_full_state_and_schedule_match_c_in_both_backends_and_runtimes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temporary = TemporarySource::new();
    let source_lf = SOURCE.replace("\r\n", "\n");
    let vectors_lf = VECTORS.replace("\r\n", "\n");
    let mut failures = Vec::new();
    for mode in [CompileMode::Mir6502, CompileMode::Optimized] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            // Both newline conventions pass through the actual source compiler,
            // vector parser, and VM execution, without touching the checkout.
            let (source, vectors) = if runtime == Runtime::ActionCart {
                (source_lf.clone(), vectors_lf.clone())
            } else {
                (
                    source_lf.replace('\n', "\r\n"),
                    vectors_lf.replace('\n', "\r\n"),
                )
            };
            let path = temporary.0.join("sha.act");
            std::fs::write(&path, source).unwrap();
            let compiled =
                compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime))
                    .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
            let mut total_steps = 0u64;
            for vector in parse_vectors(&vectors) {
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
                        .all(|s| s.end < 0x0600 || s.start > 0x0EFF),
                    "host data overlaps object: {label}"
                );
                let mut expected = vec![0xCC; 0x900];
                expected[0] = vector.command;
                expected[2..4].copy_from_slice(&(vector.message.len() as u16).to_le_bytes());
                expected[STATE..STATE + STATE_BYTES].copy_from_slice(&vector.state);
                expected[MESSAGE..MESSAGE + vector.message.len()].copy_from_slice(&vector.message);
                vm.bus_mut().ram_mut().map(0x0600, &expected).unwrap();
                expected[0xFF] = 0xA5;
                expected[STATE..STATE + STATE_BYTES]
                    .copy_from_slice(&vector.expected[..STATE_BYTES]);
                expected[SCHEDULE..SCHEDULE + SCHEDULE_BYTES]
                    .copy_from_slice(&vector.expected[STATE_BYTES..]);
                let limit = 2_000_000 * (vector.message.len() / 64 + 2);
                let mut steps = 0;
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
                total_steps += steps as u64;
                for (offset, &value) in expected.iter().enumerate() {
                    let address = 0x0600 + offset as u16;
                    let actual = vm.bus().ram().read(address);
                    if actual != value {
                        failures.push(format!(
                            "{label}: ${address:04X}: expected {value:02X}, got {actual:02X}"
                        ));
                        break;
                    }
                }
            }
            eprintln!(
                "SHA-0 {mode:?}/{runtime:?}: {} object bytes, {total_steps} instructions",
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
