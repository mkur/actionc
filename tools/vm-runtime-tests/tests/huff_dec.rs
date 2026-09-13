//! TACLeBench Huffman decoder: bit streams, record arrays, trees, and C state.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/huff_dec/huff_dec.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/huff_dec/vectors.txt");
const HOST_BASE: u16 = 0x0600;
const HOST_BYTES: usize = 0x7A00;
const INPUT: usize = 0x0801 - HOST_BASE as usize;
const INPUT_BYTES: usize = 12288;
const OUTPUT: usize = 0x3901 - HOST_BASE as usize;
const TABLE: usize = 0x4001 - HOST_BASE as usize;
const TABLE_BYTES: usize = 257 * 35;
const POOL: usize = 0x6501 - HOST_BASE as usize;
const POOL_BYTES: usize = 514 * 6;

#[derive(Debug, PartialEq, Eq)]
struct Vector {
    label: String,
    input_header: Vec<u8>,
    input: Vec<u8>,
    expected_header: Vec<u8>,
    expected_output: Vec<u8>,
    expected_table: Vec<u8>,
    expected_pool: Vec<u8>,
}

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

fn filled(text: &str, length: usize, fill: u8) -> Vec<u8> {
    let mut bytes = hex(text);
    assert!(bytes.len() <= length);
    bytes.resize(length, fill);
    bytes
}

fn parse_vectors(text: &str) -> Vec<Vector> {
    // lines() accepts host LF/CRLF; decoded binary bytes remain exact.
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 7);
            let vector = Vector {
                label: fields[0].into(),
                input_header: hex(fields[1]),
                input: hex(fields[2]),
                expected_header: hex(fields[3]),
                expected_output: filled(fields[4], 1024, 0xCC),
                expected_table: filled(fields[5], TABLE_BYTES, 0),
                expected_pool: filled(fields[6], POOL_BYTES, 0),
            };
            assert_eq!(vector.input_header.len(), 24);
            assert_eq!(vector.expected_header.len(), 24);
            assert!(vector.input_header[0] <= 2);
            assert_eq!(
                u16::from_le_bytes([vector.input_header[2], vector.input_header[3]]) as usize,
                vector.input.len()
            );
            assert!(vector.input.len() <= INPUT_BYTES);
            for entry in vector.expected_pool.chunks_exact(6) {
                for field in [2, 4] {
                    let address = u16::from_le_bytes([entry[field], entry[field + 1]]);
                    assert!(
                        address == 0
                            || (address >= 0x6501
                                && (address - 0x6501) % 6 == 0
                                && (address - 0x6501) / 6 < 514)
                    );
                }
            }
            vector
        })
        .collect()
}

#[test]
fn huff_dec_vectors_accept_lf_and_crlf_and_preserve_upstream_results() {
    let text = VECTORS.replace("\r\n", "\n");
    let vectors = parse_vectors(&text);
    assert_eq!(vectors, parse_vectors(&text.replace('\n', "\r\n")));
    assert_eq!(vectors.len(), 185);
    let original = &vectors[0];
    assert_eq!(original.label, "upstream");
    assert_eq!(original.input.len(), 419);
    assert_eq!(&original.expected_header[6..8], &600u16.to_le_bytes());
    assert!(
        original
            .expected_output
            .starts_with(b"You are doubtless asking")
    );
    let deepest = vectors.iter().find(|v| v.label == "depth-256").unwrap();
    assert_eq!(&deepest.expected_header[22..24], &513u16.to_le_bytes());
    assert_eq!(&deepest.expected_output[..4], &[255, 0, 128, 255]);
    assert_eq!(
        &deepest.expected_table[256 * 35 + 32..256 * 35 + 34],
        &256u16.to_le_bytes()
    );
}

struct TemporarySource(PathBuf);
impl TemporarySource {
    fn new(mode: CompileMode, runtime: Runtime) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "actionc-huff_dec-{}-{unique}-{mode:?}-{runtime:?}",
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

fn check_huff_dec(mode: CompileMode, runtime: Runtime) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temporary = TemporarySource::new(mode, runtime);
    let source_lf = SOURCE.replace("\r\n", "\n");
    let vectors_lf = VECTORS.replace("\r\n", "\n");
    let mut failures = Vec::new();
    // Exercise each newline convention through compilation and actual VM input.
    let text = |lf: &str| {
        if runtime == Runtime::Standalone {
            lf.replace('\n', "\r\n")
        } else {
            lf.into()
        }
    };
    let path = temporary.0.join("huff_dec.act");
    std::fs::write(&path, text(&source_lf)).unwrap();
    let compiled = compile_file(
        &path,
        &CompileOptions::for_mode(mode)
            .with_runtime(runtime)
            .with_origin(0x8000),
    )
    .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
    let mut total_steps = 0u64;
    let mut vectors = parse_vectors(&text(&vectors_lf));
    vectors.sort_by_key(|v| v.input_header[0] == 0);
    for vector in vectors {
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
                .all(|s| s.end < HOST_BASE || s.start >= 0x8000),
            "host data overlaps object: {label}"
        );
        assert!(
            loaded
                .segments
                .iter()
                .all(|s| s.start < 0x8000 || s.end < 0xA000),
            "object exceeds RAM below cartridge: {label}"
        );
        let mut expected = vec![0xCC; HOST_BYTES];
        expected[..24].copy_from_slice(&vector.input_header);
        expected[INPUT..INPUT + vector.input.len()].copy_from_slice(&vector.input);
        expected[TABLE..TABLE + TABLE_BYTES].fill(0);
        expected[POOL..POOL + POOL_BYTES].fill(0);
        vm.bus_mut().ram_mut().map(HOST_BASE, &expected).unwrap();
        expected[..24].copy_from_slice(&vector.expected_header);
        expected[0xFF] = 0xA5;
        expected[OUTPUT..OUTPUT + 1024].copy_from_slice(&vector.expected_output);
        expected[TABLE..TABLE + TABLE_BYTES].copy_from_slice(&vector.expected_table);
        expected[POOL..POOL + POOL_BYTES].copy_from_slice(&vector.expected_pool);
        let limit = 60_000_000;
        let mut steps = 0u64;
        while vm.bus().ram().read(0x06FF) != 0xA5 && steps < limit {
            vm.step_cpu()
                .unwrap_or_else(|error| panic!("{label}: {error:?}"));
            steps += 1;
        }
        if vm.bus().ram().read(0x06FF) != 0xA5 {
            for (name, start, length) in [("table", TABLE, TABLE_BYTES), ("pool", POOL, POOL_BYTES)]
            {
                let differences = (start..start + length)
                    .filter_map(|offset| {
                        let actual = vm.bus().ram().read(HOST_BASE + offset as u16);
                        (actual != expected[offset]).then_some((
                            HOST_BASE + offset as u16,
                            expected[offset],
                            actual,
                        ))
                    })
                    .take(16)
                    .collect::<Vec<_>>();
                eprintln!(
                    "{label} {name} differences (address, expected, actual): {differences:04X?}"
                );
            }
        }
        assert_eq!(
            vm.bus().ram().read(0x06FF),
            0xA5,
            "{label} did not complete in {limit} steps; PC=${:04X}; header={:02X?}",
            vm.cpu().registers().pc,
            (0..24)
                .map(|offset| vm.bus().ram().read(HOST_BASE + offset))
                .collect::<Vec<_>>()
        );
        total_steps += steps;
        for (offset, &value) in expected.iter().enumerate() {
            let address = HOST_BASE + offset as u16;
            let actual = vm.bus().ram().read(address);
            if actual != value {
                let failure =
                    format!("{label}: ${address:04X}: expected {value:02X}, got {actual:02X}");
                eprintln!("{failure}");
                failures.push(failure);
                break;
            }
        }
        if matches!(vector.label.as_str(), "upstream" | "depth-256") {
            eprintln!("{label}: {steps} instructions");
        }
    }
    eprintln!(
        "Huffman decoder {mode:?}/{runtime:?}: {} object bytes, {total_steps} instructions",
        compiled.object_bytes().len()
    );
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn huff_dec_mir6502_cart_matches_complete_c_state() {
    check_huff_dec(CompileMode::Mir6502, Runtime::ActionCart);
}

#[test]
fn huff_dec_mir6502_standalone_matches_complete_c_state() {
    check_huff_dec(CompileMode::Mir6502, Runtime::Standalone);
}

#[test]
fn huff_dec_classic_cart_matches_complete_c_state() {
    check_huff_dec(CompileMode::Optimized, Runtime::ActionCart);
}

#[test]
fn huff_dec_classic_standalone_matches_complete_c_state() {
    check_huff_dec(CompileMode::Optimized, Runtime::Standalone);
}
