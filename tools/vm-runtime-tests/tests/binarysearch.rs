//! TACLeBench binary search across byte, signed/unsigned word, and long records.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/binarysearch/binarysearch.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/binarysearch/vectors.txt");
const HOST_BASE: u16 = 0x0600;
const HOST_BYTES: usize = 0x0400;
const QUERY: usize = 0x0601 - HOST_BASE as usize;
const RESULT: usize = 0x0609 - HOST_BASE as usize;
const SEED: usize = 0x0611 - HOST_BASE as usize;
const DATA: usize = 0x07FF - HOST_BASE as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    LongInt,
    Byte,
    Int,
    Card,
}
impl Kind {
    const ALL: [Self; 4] = [Self::LongInt, Self::Byte, Self::Int, Self::Card];

    fn name(self) -> &'static str {
        match self {
            Self::LongInt => "LONGINT",
            Self::Byte => "BYTE",
            Self::Int => "INT",
            Self::Card => "CARD",
        }
    }

    fn width(self) -> usize {
        match self {
            Self::Byte => 1,
            Self::Int | Self::Card => 2,
            Self::LongInt => 4,
        }
    }

    fn result_type(self) -> &'static str {
        match self {
            Self::Byte | Self::Int => "INT",
            Self::Card | Self::LongInt => "LONGINT",
        }
    }

    fn result_width(self) -> usize {
        match self {
            Self::Byte | Self::Int => 2,
            Self::Card | Self::LongInt => 4,
        }
    }
}

fn typed_source(source: &str, kind: Kind) -> String {
    // Instrument normalized host text, including when include_str! read CRLF.
    let mut source = source.replace("\r\n", "\n");
    if kind == Kind::LongInt {
        return source;
    }
    let element = kind.name();
    let result = kind.result_type();
    for (old, new) in [
        (
            "TYPE Entry=[LONGINT key,value]",
            format!("TYPE Entry=[{element} key,value]"),
        ),
        ("LONGINT query=$0601", format!("{element} query=$0601")),
        ("LONGINT result=$0609", format!("{result} result=$0609")),
        (
            "LONGINT FUNC Search(LONGINT x)",
            format!("{result} FUNC Search({element} x)"),
        ),
        ("  LONGINT fvalue\n", format!("  {result} fvalue\n")),
        (
            "data(i).key=RandomInteger()",
            format!("data(i).key={element}(RandomInteger())"),
        ),
        (
            "data(i).value=RandomInteger()",
            format!("data(i).value={element}(RandomInteger())"),
        ),
    ] {
        assert_eq!(source.matches(old).count(), 1, "{kind:?}: {old}");
        source = source.replace(old, &new);
    }
    source
}

#[derive(Debug, PartialEq, Eq)]
struct Vector {
    kind: Kind,
    label: String,
    command: u8,
    query: Vec<u8>,
    seed: Vec<u8>,
    data: Vec<u8>,
    expected_seed: Vec<u8>,
    expected_data: Vec<u8>,
    result: Vec<u8>,
}

fn hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn parse_vectors(text: &str) -> Vec<Vector> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 9);
            let kind = Kind::ALL
                .into_iter()
                .find(|kind| kind.name() == fields[0])
                .unwrap();
            let vector = Vector {
                kind,
                label: fields[1].into(),
                command: fields[2].parse().unwrap(),
                query: hex(fields[3]),
                seed: hex(fields[4]),
                data: hex(fields[5]),
                expected_seed: hex(fields[6]),
                expected_data: hex(fields[7]),
                result: hex(fields[8]),
            };
            assert!(vector.command <= 1);
            assert_eq!(vector.query.len(), kind.width());
            assert_eq!(vector.seed.len(), 4);
            assert_eq!(vector.expected_seed.len(), 4);
            assert_eq!(vector.data.len(), 30 * kind.width());
            assert_eq!(vector.expected_data.len(), 30 * kind.width());
            assert_eq!(vector.result.len(), kind.result_width());
            if vector.command == 1 {
                assert_eq!(vector.data, vector.expected_data);
                assert_eq!(vector.seed, vector.expected_seed);
            }
            vector
        })
        .collect()
}

fn result_value(vector: &Vector) -> i32 {
    match vector.kind.result_width() {
        2 => i16::from_le_bytes(vector.result.as_slice().try_into().unwrap()) as i32,
        4 => i32::from_le_bytes(vector.result.as_slice().try_into().unwrap()),
        _ => unreachable!(),
    }
}

#[test]
fn binarysearch_lf_and_crlf_variants_preserve_upstream_and_typed_results() {
    let source = SOURCE.replace("\r\n", "\n");
    let text = VECTORS.replace("\r\n", "\n");
    let vectors = parse_vectors(&text);
    assert_eq!(vectors, parse_vectors(&text.replace('\n', "\r\n")));
    assert_eq!(vectors.len(), 1153);
    for (kind, count) in Kind::ALL.into_iter().zip([233, 460, 231, 229]) {
        assert_eq!(
            typed_source(&source, kind),
            typed_source(&source.replace('\n', "\r\n"), kind)
        );
        assert_eq!(vectors.iter().filter(|v| v.kind == kind).count(), count);
        let original = vectors
            .iter()
            .find(|v| v.kind == kind && v.label == "upstream")
            .unwrap();
        assert_eq!(original.command, 0);
        assert_eq!(result_value(original), -1);
        let duplicate = vectors
            .iter()
            .find(|v| v.kind == kind && v.label == "all-equal-30")
            .unwrap();
        assert_eq!(result_value(duplicate), 17); // The first midpoint is index 7.
    }
    let original = &vectors[0];
    assert_eq!(&original.expected_data[..8], &[81, 0, 0, 0, 199, 10, 0, 0]);
    for (kind, label, expected) in [
        (Kind::Byte, "boundary-0", 255),
        (Kind::Card, "boundary-0", 65535),
        (Kind::Int, "boundary-32767", -32768),
        (Kind::LongInt, "boundary-2147483647", i32::MIN),
    ] {
        let vector = vectors
            .iter()
            .find(|v| v.kind == kind && v.label == label)
            .unwrap();
        assert_eq!(result_value(vector), expected);
    }
    let queries: Vec<_> = vectors
        .iter()
        .filter(|v| v.kind == Kind::Byte && v.label.starts_with("boundary-"))
        .map(|v| v.query[0])
        .collect();
    assert_eq!(queries, (0..=255).collect::<Vec<u8>>());
}

struct TemporarySource(PathBuf);
impl TemporarySource {
    fn new(mode: CompileMode, runtime: Runtime) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "actionc-binarysearch-{}-{unique}-{mode:?}-{runtime:?}",
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

fn check_binarysearch(mode: CompileMode, runtime: Runtime) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temporary = TemporarySource::new(mode, runtime);
    let text = |source: &str| {
        let lf = source.replace("\r\n", "\n");
        if runtime == Runtime::Standalone {
            lf.replace('\n', "\r\n")
        } else {
            lf
        }
    };
    let vectors = parse_vectors(&text(VECTORS));
    let mut roms = Vec::new();
    let profile = match runtime {
        Runtime::Standalone => ExecutionProfile::StandaloneObject,
        Runtime::ActionCart => {
            for (kind, name, base) in [
                (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
                (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
            ] {
                roms.push((
                    kind,
                    name,
                    base,
                    std::fs::read(root.join("roms").join(name)).unwrap(),
                ));
            }
            ExecutionProfile::CartridgeObject
        }
    };
    for kind in Kind::ALL {
        let path = temporary.0.join(format!("{}.act", kind.name()));
        // CRLF passes through both instrumentation and actual compilation.
        let source = typed_source(&text(SOURCE), kind);
        std::fs::write(&path, text(&source)).unwrap();
        let compiled = compile_file(
            &path,
            &CompileOptions::for_mode(mode)
                .with_runtime(runtime)
                .with_origin(0x3000),
        )
        .unwrap_or_else(|error| panic!("compile {kind:?}/{mode:?}/{runtime:?}: {error}"));
        let mut total_steps = 0u64;
        for vector in vectors.iter().filter(|v| v.kind == kind) {
            let label = format!("{kind:?}/{mode:?}/{runtime:?}/{}", vector.label);
            let mut vm = CompilerVm::default();
            for (image_kind, name, base, bytes) in &roms {
                vm.load_image_bytes(*image_kind, *name, *base, bytes.clone())
                    .unwrap();
            }
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
            expected[QUERY..QUERY + kind.width()].copy_from_slice(&vector.query);
            expected[SEED..SEED + 4].copy_from_slice(&vector.seed);
            expected[DATA..DATA + vector.data.len()].copy_from_slice(&vector.data);
            vm.bus_mut().ram_mut().map(HOST_BASE, &expected).unwrap();
            expected[RESULT..RESULT + kind.result_width()].copy_from_slice(&vector.result);
            expected[SEED..SEED + 4].copy_from_slice(&vector.expected_seed);
            expected[DATA..DATA + vector.expected_data.len()]
                .copy_from_slice(&vector.expected_data);
            expected[0xFF] = 0xA5;
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
        }
        eprintln!(
            "Binary search {kind:?}/{mode:?}/{runtime:?}: {} object bytes, {total_steps} instructions",
            compiled.object_bytes().len()
        );
    }
}

#[test]
fn binarysearch_mir6502_cart_matches_c_reference() {
    check_binarysearch(CompileMode::Mir6502, Runtime::ActionCart);
}

#[test]
fn binarysearch_mir6502_standalone_matches_c_reference() {
    check_binarysearch(CompileMode::Mir6502, Runtime::Standalone);
}

#[test]
fn binarysearch_classic_cart_matches_c_reference() {
    check_binarysearch(CompileMode::Optimized, Runtime::ActionCart);
}

#[test]
fn binarysearch_classic_standalone_matches_c_reference() {
    check_binarysearch(CompileMode::Optimized, Runtime::Standalone);
}
