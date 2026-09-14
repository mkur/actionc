//! TACLeBench Dijkstra: records, pointer links, row arrays, and complete C state.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SOURCE: &str = include_str!("../fixtures/dijkstra_driver.act");
const TYPES: &str = include_str!("../../../fixtures/runtime/tacle/dijkstra/types.inc");
const CORE: &str = include_str!("../../../fixtures/runtime/tacle/dijkstra/kernel.inc");
const GRAPHS: &str = include_str!("../../../fixtures/runtime/tacle/dijkstra/graphs.txt");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/dijkstra/vectors.txt");
const HOST_BASE: u16 = 0x0600;
const HOST_BYTES: usize = 0x7A00;
const NODES: usize = 0x0801 - HOST_BASE as usize;
const MATRIX: usize = 0x1001 - HOST_BASE as usize;
const POOL: usize = 0x4001 - HOST_BASE as usize;
const POOL_BYTES: usize = 8000;

#[derive(Debug, PartialEq, Eq)]
struct Vector {
    label: String,
    graph: String,
    input_header: Vec<u8>,
    input_pool: Vec<u8>,
    expected_header: Vec<u8>,
    expected_nodes: Vec<u8>,
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

fn pool(text: &str) -> Vec<u8> {
    let mut bytes = hex(text);
    assert!(bytes.len() <= POOL_BYTES);
    bytes.resize(POOL_BYTES, 0); // the wire format omits trailing zero bytes
    for entry in bytes.chunks_exact(8) {
        let link = u16::from_le_bytes([entry[6], entry[7]]);
        assert!(
            link == 0 || (link >= 0x4001 && (link - 0x4001) % 8 == 0 && (link - 0x4001) / 8 < 1000)
        );
    }
    bytes
}

fn parse_graphs(text: &str) -> BTreeMap<String, Vec<u8>> {
    let mut graphs = BTreeMap::new();
    for line in text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 2);
        let graph = hex(fields[1]);
        assert_eq!(graph.len(), 10000);
        assert!(graphs.insert(fields[0].into(), graph).is_none());
    }
    graphs
}

fn parse_vectors(text: &str) -> Vec<Vector> {
    // lines() handles LF and CRLF while preserving exact decoded bytes.
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 7);
            let vector = Vector {
                label: fields[0].into(),
                graph: fields[1].into(),
                input_header: hex(fields[2]),
                input_pool: pool(fields[3]),
                expected_header: hex(fields[4]),
                expected_nodes: hex(fields[5]),
                expected_pool: pool(fields[6]),
            };
            assert_eq!(vector.input_header.len(), 32);
            assert_eq!(vector.expected_header.len(), 32);
            assert_eq!(vector.expected_nodes.len(), 400);
            assert!(vector.input_header[0] <= 3);
            vector
        })
        .collect()
}

#[test]
fn dijkstra_vectors_accept_lf_and_crlf_and_preserve_upstream_results() {
    let graphs = GRAPHS.replace("\r\n", "\n");
    assert_eq!(
        parse_graphs(&graphs),
        parse_graphs(&graphs.replace('\n', "\r\n"))
    );
    let text = VECTORS.replace("\r\n", "\n");
    let vectors = parse_vectors(&text);
    assert_eq!(vectors, parse_vectors(&text.replace('\n', "\r\n")));
    assert_eq!(vectors.len(), 33);
    let original = &vectors[0];
    assert_eq!(original.label, "benchmark-original");
    assert_eq!(&original.expected_header[6..10], &[0, 0, 25, 0]);
    for label in [
        "benchmark-exhaust",
        "find-pool-998",
        "find-pool-999",
        "enqueue-empty-999",
    ] {
        let vector = vectors.iter().find(|v| v.label == label).unwrap();
        assert_eq!(&vector.expected_header[6..8], &[0xFF, 0xFF], "{label}");
    }
    for label in ["original-0-0", "original-99-99"] {
        let vector = vectors.iter().find(|v| v.label == label).unwrap();
        assert!(
            vector
                .expected_nodes
                .chunks_exact(2)
                .all(|v| u16::from_le_bytes([v[0], v[1]]) == 9999)
        );
    }
}

struct TemporarySource(PathBuf);
impl TemporarySource {
    fn new(mode: CompileMode, runtime: Runtime) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "actionc-dijkstra-{}-{unique}-{mode:?}-{runtime:?}",
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

fn check_dijkstra(mode: CompileMode, runtime: Runtime) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temporary = TemporarySource::new(mode, runtime);
    let source_lf = SOURCE.replace("\r\n", "\n");
    let graphs_lf = GRAPHS.replace("\r\n", "\n");
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
    let path = temporary.0.join("dijkstra.act");
    std::fs::write(&path, text(&source_lf)).unwrap();
    for (name, source) in [("types.inc", TYPES), ("kernel.inc", CORE)] {
        std::fs::write(temporary.0.join(name), text(&source.replace("\r\n", "\n"))).unwrap();
    }
    let compiled = compile_file(
        &path,
        &CompileOptions::for_mode(mode)
            .with_runtime(runtime)
            .with_origin(0x8000),
    )
    .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
    let graphs = parse_graphs(&text(&graphs_lf));
    let mut total_steps = 0u64;
    for vector in parse_vectors(&text(&vectors_lf)) {
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
        expected[..32].copy_from_slice(&vector.input_header);
        expected[NODES..NODES + 400].fill(0);
        expected[MATRIX..MATRIX + 10000].copy_from_slice(&graphs[&vector.graph]);
        expected[POOL..POOL + POOL_BYTES].copy_from_slice(&vector.input_pool);
        vm.bus_mut().ram_mut().map(HOST_BASE, &expected).unwrap();
        expected[..32].copy_from_slice(&vector.expected_header);
        expected[0xFF] = 0xA5;
        expected[NODES..NODES + 400].copy_from_slice(&vector.expected_nodes);
        expected[POOL..POOL + POOL_BYTES].copy_from_slice(&vector.expected_pool);
        let limit = match vector.input_header[0] {
            0 => 1_000_000_000,
            1 => 60_000_000,
            _ => 1_000_000,
        };
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
        if vector.input_header[0] == 0 {
            eprintln!("{label}: {steps} instructions");
        }
    }
    eprintln!(
        "Dijkstra {mode:?}/{runtime:?}: {} object bytes, {total_steps} instructions",
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
fn dijkstra_mir6502_cart_matches_complete_c_state() {
    check_dijkstra(CompileMode::Mir6502, Runtime::ActionCart);
}

#[test]
fn dijkstra_mir6502_standalone_matches_complete_c_state() {
    check_dijkstra(CompileMode::Mir6502, Runtime::Standalone);
}

#[test]
fn dijkstra_classic_cart_matches_complete_c_state() {
    check_dijkstra(CompileMode::Optimized, Runtime::ActionCart);
}

#[test]
fn dijkstra_classic_standalone_matches_complete_c_state() {
    check_dijkstra(CompileMode::Optimized, Runtime::Standalone);
}
