//! TACLeBench matrix multiply: typed pointer traversal and complete C matrices.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/matrix1/matrix1.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/matrix1/vectors.txt");
const HOST_BASE: u16 = 0x0600;
const HOST_BYTES: usize = 0x1A00;
const RESULT: usize = 0x0601 - HOST_BASE as usize;
const CHECKSUM: usize = 0x0605 - HOST_BASE as usize;
const A: usize = 0x07FF - HOST_BASE as usize;
const B: usize = 0x0DFF - HOST_BASE as usize;
const C: usize = 0x13FF - HOST_BASE as usize;
const SHAPES: [[usize; 3]; 3] = [[10, 10, 10], [3, 7, 5], [2, 129, 1]];

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

    fn signed(self) -> bool {
        matches!(self, Self::Int | Self::LongInt)
    }
}

fn replace_once(source: &mut String, old: &str, new: &str) {
    assert_eq!(source.matches(old).count(), 1, "source anchor: {old}");
    *source = source.replace(old, new);
}

fn typed_source(source: &str, kind: Kind, shape: [usize; 3]) -> String {
    // Normalize before newline-sensitive instrumentation, including include_str!.
    let mut source = source.replace("\r\n", "\n");
    let [rows, inner, columns] = shape;
    replace_once(
        &mut source,
        "CONST Rows=10,Inner=10,Columns=10,ElementBytes=4",
        &format!(
            "CONST Rows={rows},Inner={inner},Columns={columns},ElementBytes={}",
            kind.width()
        ),
    );
    for old in [
        "LONGINT ARRAY matrixA(",
        "LONGINT ARRAY matrixB(",
        "LONGINT ARRAY matrixC(",
        "PROC PinDown(LONGINT ARRAY a,b,c)",
        "VOLATILE LONGINT one=[1]",
        "LONGINT POINTER pa,pb,pc",
    ] {
        replace_once(&mut source, old, &old.replace("LONGINT", kind.name()));
    }
    source
}

#[derive(Debug, PartialEq, Eq)]
struct Vector {
    kind: Kind,
    shape: [usize; 3],
    label: String,
    command: u8,
    inputs: [Vec<u8>; 3],
    expected: [Vec<u8>; 3],
    checksum: Vec<u8>,
    result: Vec<u8>,
}

fn hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn values(bytes: &[u8], kind: Kind) -> Vec<i64> {
    bytes
        .chunks_exact(kind.width())
        .map(|bytes| {
            let mut word = [0u8; 4];
            word[..bytes.len()].copy_from_slice(bytes);
            let raw = u32::from_le_bytes(word) as i64;
            let bits = 8 * kind.width();
            if kind.signed() && raw >= 1i64 << (bits - 1) {
                raw - (1i64 << bits)
            } else {
                raw
            }
        })
        .collect()
}

fn parse_vectors(text: &str) -> Vec<Vector> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 12);
            let kind = Kind::ALL
                .into_iter()
                .find(|kind| kind.name() == fields[0])
                .unwrap();
            let shape: Vec<usize> = fields[1].split('x').map(|v| v.parse().unwrap()).collect();
            let shape: [usize; 3] = shape.try_into().unwrap();
            assert!(SHAPES.contains(&shape));
            let vector = Vector {
                kind,
                shape,
                label: fields[2].into(),
                command: fields[3].parse().unwrap(),
                inputs: [hex(fields[4]), hex(fields[5]), hex(fields[6])],
                expected: [hex(fields[7]), hex(fields[8]), hex(fields[9])],
                checksum: hex(fields[10]),
                result: hex(fields[11]),
            };
            let [rows, inner, columns] = shape;
            for (i, count) in [rows * inner, inner * columns, rows * columns]
                .into_iter()
                .enumerate()
            {
                assert_eq!(vector.inputs[i].len(), count * kind.width());
                assert_eq!(vector.expected[i].len(), count * kind.width());
            }
            assert!(vector.command <= 1);
            assert_eq!(vector.checksum.len(), 4);
            assert_eq!(vector.result.len(), 2);
            assert!([0, -1].contains(&i16::from_le_bytes(
                vector.result.as_slice().try_into().unwrap()
            )));
            if vector.command == 1 {
                assert_eq!(vector.inputs[..2], vector.expected[..2]);
            }
            vector
        })
        .collect()
}

#[test]
fn matrix1_lf_and_crlf_variants_preserve_original_results_and_layout() {
    let source = SOURCE.replace("\r\n", "\n");
    let text = VECTORS.replace("\r\n", "\n");
    let vectors = parse_vectors(&text);
    assert_eq!(vectors, parse_vectors(&text.replace('\n', "\r\n")));
    assert_eq!(vectors.len(), 252);
    for kind in Kind::ALL {
        for shape in SHAPES {
            assert_eq!(
                typed_source(&source, kind, shape),
                typed_source(&source.replace('\n', "\r\n"), kind, shape)
            );
            let cases: Vec<_> = vectors
                .iter()
                .filter(|v| v.kind == kind && v.shape == shape)
                .collect();
            assert_eq!(cases.len(), if kind.signed() { 22 } else { 20 });
            let original = cases.iter().find(|v| v.label == "upstream").unwrap();
            let [rows, inner, columns] = shape;
            assert_eq!(original.command, 0);
            assert_eq!(values(&original.expected[0], kind), vec![1; rows * inner]);
            assert_eq!(
                values(&original.expected[1], kind),
                vec![1; inner * columns]
            );
            assert_eq!(
                values(&original.expected[2], kind),
                vec![inner as i64; rows * columns]
            );
            assert_eq!(
                original.checksum,
                ((rows * inner * columns) as i32).to_le_bytes()
            );
            assert_eq!(
                original.result,
                (if shape == [10, 10, 10] { 0i16 } else { -1i16 }).to_le_bytes()
            );
            let last = cases.iter().find(|v| v.label == "last-element").unwrap();
            let output = values(&last.expected[2], kind);
            assert!(output[..output.len() - 1].iter().all(|value| *value == 0));
            assert_eq!(
                *output.last().unwrap(),
                (1i64 << (8 * kind.width() - usize::from(kind.signed()))) - 1
            );
        }
    }
    assert_eq!(typed_source(&source, Kind::LongInt, [10, 10, 10]), source);
    let asymmetric = vectors
        .iter()
        .find(|v| v.kind == Kind::LongInt && v.shape == [3, 7, 5] && v.label == "asymmetric")
        .unwrap();
    let output = values(&asymmetric.expected[2], Kind::LongInt);
    assert_ne!(output[1], output[5]); // Distinguish column-major from row-major storage.
}

struct TemporarySource(PathBuf);
impl TemporarySource {
    fn new(mode: CompileMode, runtime: Runtime) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "actionc-matrix1-{}-{unique}-{mode:?}-{runtime:?}",
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

fn check_matrix1(mode: CompileMode, runtime: Runtime) {
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
        for shape in SHAPES {
            let [rows, inner, columns] = shape;
            let shape_name = format!("{rows}x{inner}x{columns}");
            let path = temporary
                .0
                .join(format!("{}-{shape_name}.act", kind.name()));
            // Each newline convention reaches instrumentation and compilation.
            let source = typed_source(&text(SOURCE), kind, shape);
            std::fs::write(&path, text(&source)).unwrap();
            let compiled = compile_file(
                &path,
                &CompileOptions::for_mode(mode)
                    .with_runtime(runtime)
                    .with_origin(0x3000),
            )
            .unwrap_or_else(|error| {
                panic!("compile {kind:?}/{shape_name}/{mode:?}/{runtime:?}: {error}")
            });
            let mut total_steps = 0u64;
            for vector in vectors
                .iter()
                .filter(|v| v.kind == kind && v.shape == shape)
            {
                let label = format!(
                    "{kind:?}/{shape_name}/{mode:?}/{runtime:?}/{}",
                    vector.label
                );
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
                        .all(|s| s.end < HOST_BASE || s.start >= 0x2000),
                    "host data overlaps object: {label}"
                );
                let mut expected = vec![0xCC; HOST_BYTES];
                expected[0] = vector.command;
                for (i, offset) in [A, B, C].into_iter().enumerate() {
                    expected[offset..offset + vector.inputs[i].len()]
                        .copy_from_slice(&vector.inputs[i]);
                }
                vm.bus_mut().ram_mut().map(HOST_BASE, &expected).unwrap();
                expected[RESULT..RESULT + 2].copy_from_slice(&vector.result);
                expected[CHECKSUM..CHECKSUM + 4].copy_from_slice(&vector.checksum);
                for (i, offset) in [A, B, C].into_iter().enumerate() {
                    expected[offset..offset + vector.expected[i].len()]
                        .copy_from_slice(&vector.expected[i]);
                }
                expected[0xFF] = 0xA5;
                let limit = 5_000_000;
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
                if expected
                    .iter()
                    .enumerate()
                    .any(|(offset, value)| vm.bus().ram().read(HOST_BASE + offset as u16) != *value)
                {
                    for (name, start, length) in [
                        ("control", 0, 16),
                        ("A", A, vector.expected[0].len()),
                        ("B", B, vector.expected[1].len()),
                        ("C", C, vector.expected[2].len()),
                    ] {
                        let differences: Vec<_> = (start..start + length)
                            .filter_map(|offset| {
                                let address = HOST_BASE + offset as u16;
                                let actual = vm.bus().ram().read(address);
                                (actual != expected[offset]).then_some((
                                    address,
                                    expected[offset],
                                    actual,
                                ))
                            })
                            .take(8)
                            .collect();
                        eprintln!(
                            "{label} {name} differences (address, expected, actual): {differences:04X?}"
                        );
                    }
                }
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
                "Matrix1 {kind:?}/{shape_name}/{mode:?}/{runtime:?}: {} object bytes, {total_steps} instructions",
                compiled.object_bytes().len()
            );
        }
    }
}

#[test]
fn matrix1_mir6502_cart_matches_c_reference() {
    check_matrix1(CompileMode::Mir6502, Runtime::ActionCart);
}

#[test]
fn matrix1_mir6502_standalone_matches_c_reference() {
    check_matrix1(CompileMode::Mir6502, Runtime::Standalone);
}

#[test]
fn matrix1_classic_cart_matches_c_reference() {
    check_matrix1(CompileMode::Optimized, Runtime::ActionCart);
}

#[test]
fn matrix1_classic_standalone_matches_c_reference() {
    check_matrix1(CompileMode::Optimized, Runtime::Standalone);
}
