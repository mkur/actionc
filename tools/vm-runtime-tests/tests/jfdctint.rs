//! TACLeBench integer JPEG DCT: complete input, row pass and output against C.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/jfdctint/jfdctint.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/jfdctint/vectors.txt");
const HOST_BASE: u16 = 0x0600;
const HOST_BYTES: usize = 0x0A00;
const INPUT: usize = 0x07FF - HOST_BASE as usize;
const INITIAL: usize = 0x09FF - HOST_BASE as usize;
const ROWS: usize = 0x0BFF - HOST_BASE as usize;
const OUTPUT: usize = 0x0DFF - HOST_BASE as usize;

fn replace_once(source: &mut String, old: &str, new: &str) {
    assert_eq!(source.matches(old).count(), 1, "source anchor: {old}");
    *source = source.replace(old, new);
}

fn instrument(source: &str) -> String {
    let mut source = source.replace("\r\n", "\n");
    // Keep every benchmark variable compiler-allocated. Only this 6502 adapter
    // owns the host mailboxes, serialization layout and page-crossing buffers.
    replace_once(
        &mut source,
        "INT result\n",
        "INT result\n\
         BYTE testCommand=$0600,testShift=$0601,done=$06FF\n\
         INT testResult=$0602\n\
         LONGINT testSum=$0604\n\
         LONGINT ARRAY testInput(64)=$07FF,testInitial(64)=$09FF\n\
         LONGINT ARRAY testRows(64)=$0BFF,testOutput(64)=$0DFF\n\
         PROC CaptureRows()\n\
           BYTE i\n\
           FOR i=0 TO 63 DO testRows(i)=block(i) OD\n\
         RETURN\n",
    );
    replace_once(
        &mut source,
        "  ; Pass 2: eight values per column, eight elements apart.",
        "  CaptureRows()\n  ; Pass 2: eight values per column, eight elements apart.",
    );
    replace_once(
        &mut source,
        "PROC Main()\n  Init()\n  Dct()\n  result=CheckResult()\nRETURN\n",
        "PROC Main()\n\
           BYTE i\n\
           IF testCommand=0 THEN Init()\n\
           ELSE FOR i=0 TO 63 DO block(i)=testInput(i) OD FI\n\
           FOR i=0 TO 63 DO testInitial(i)=block(i) OD\n\
           IF testCommand=2 THEN\n\
             FOR i=0 TO 63 DO block(i)=Descale(block(i),testShift) OD\n\
           ELSE Dct() FI\n\
           result=CheckResult()\n\
           FOR i=0 TO 63 DO testOutput(i)=block(i) OD\n\
           testSum=checksum testResult=result done=$A5\n\
         RETURN\n",
    );
    source
}

#[derive(Debug, PartialEq, Eq)]
struct Vector {
    label: String,
    command: u8,
    shift: u8,
    input: Vec<u8>,
    initial: Vec<u8>,
    rows: Vec<u8>,
    output: Vec<u8>,
    checksum: Vec<u8>,
    status: Vec<u8>,
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
            let vector = Vector {
                label: fields[0].into(),
                command: fields[1].parse().unwrap(),
                shift: fields[2].parse().unwrap(),
                input: hex(fields[3]),
                initial: hex(fields[4]),
                rows: hex(fields[5]),
                output: hex(fields[6]),
                checksum: hex(fields[7]),
                status: hex(fields[8]),
            };
            for array in [&vector.input, &vector.initial, &vector.rows, &vector.output] {
                assert_eq!(array.len(), 256);
            }
            assert_eq!(vector.checksum.len(), 4);
            assert_eq!(vector.status.len(), 2);
            assert!(vector.command <= 2);
            if vector.command == 2 {
                assert!([2, 11, 15].contains(&vector.shift));
                assert_eq!(vector.rows, vec![0xCC; 256]);
            } else {
                assert_eq!(vector.shift, 0);
            }
            if vector.command != 0 {
                assert_eq!(vector.initial, vector.input);
            }
            vector
        })
        .collect()
}

#[test]
fn jfdctint_lf_and_crlf_preserve_instrumentation_and_reference_vectors() {
    let source = SOURCE.replace("\r\n", "\n");
    assert_eq!(
        instrument(&source),
        instrument(&source.replace('\n', "\r\n"))
    );
    let text = VECTORS.replace("\r\n", "\n");
    let vectors = parse_vectors(&text);
    assert_eq!(vectors, parse_vectors(&text.replace('\n', "\r\n")));
    assert_eq!(vectors.len(), 181);
    let upstream = vectors.iter().find(|v| v.label == "upstream").unwrap();
    assert_eq!(upstream.command, 0);
    assert_eq!(upstream.checksum, 1668124i32.to_le_bytes());
    assert_eq!(upstream.status, [0, 0]);
    let zero = vectors.iter().find(|v| v.label == "constant-0").unwrap();
    assert_eq!(zero.rows, vec![0; 256]);
    assert_eq!(zero.output, vec![0; 256]);
    assert_eq!(vectors.iter().filter(|v| v.command == 2).count(), 3);
}

struct TemporarySource(PathBuf);
impl TemporarySource {
    fn new(mode: CompileMode, runtime: Runtime) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "actionc-jfdctint-{}-{unique}-{mode:?}-{runtime:?}",
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

fn check_jfdctint(mode: CompileMode, runtime: Runtime) {
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
    let path = temporary.0.join("jfdctint.act");
    // Both newline conventions reach actual instrumentation and compilation.
    std::fs::write(&path, text(&instrument(&text(SOURCE)))).unwrap();
    let compiled = compile_file(
        &path,
        &CompileOptions::for_mode(mode)
            .with_runtime(runtime)
            .with_origin(0x3000),
    )
    .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
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
    let vectors = parse_vectors(&text(VECTORS));
    let mut total_steps = 0u64;
    for vector in vectors {
        let label = format!("{mode:?}/{runtime:?}/{}", vector.label);
        let mut vm = CompilerVm::default();
        for (kind, name, base, bytes) in &roms {
            vm.load_image_bytes(*kind, *name, *base, bytes.clone())
                .unwrap();
        }
        let loaded = vm
            .load_atari_object_for_execution(profile, compiled.object_bytes())
            .unwrap();
        assert!(
            loaded
                .segments
                .iter()
                .all(|s| s.end < HOST_BASE || s.start >= 0x1000),
            "host data overlaps object: {label}"
        );
        let mut expected = vec![0xCC; HOST_BYTES];
        expected[0] = vector.command;
        expected[1] = vector.shift;
        expected[INPUT..INPUT + 256].copy_from_slice(&vector.input);
        vm.bus_mut().ram_mut().map(HOST_BASE, &expected).unwrap();
        expected[2..4].copy_from_slice(&vector.status);
        expected[4..8].copy_from_slice(&vector.checksum);
        expected[0xFF] = 0xA5;
        for (offset, array) in [
            (INITIAL, &vector.initial),
            (ROWS, &vector.rows),
            (OUTPUT, &vector.output),
        ] {
            expected[offset..offset + 256].copy_from_slice(array);
        }
        let limit = 3_000_000;
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
        for (name, start, length) in [
            ("control", 0, 8),
            ("initial", INITIAL, 256),
            ("rows", ROWS, 256),
            ("output", OUTPUT, 256),
        ] {
            let differences: Vec<_> = (start..start + length)
                .filter_map(|offset| {
                    let address = HOST_BASE + offset as u16;
                    let actual = vm.bus().ram().read(address);
                    (actual != expected[offset]).then_some((address, expected[offset], actual))
                })
                .take(12)
                .collect();
            if !differences.is_empty() {
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
        "Jfdctint {mode:?}/{runtime:?}: {} object bytes, {total_steps} instructions",
        compiled.object_bytes().len()
    );
}

#[test]
fn jfdctint_mir6502_cart_matches_c_reference() {
    check_jfdctint(CompileMode::Mir6502, Runtime::ActionCart);
}

#[test]
fn jfdctint_mir6502_standalone_matches_c_reference() {
    check_jfdctint(CompileMode::Mir6502, Runtime::Standalone);
}

#[test]
fn jfdctint_classic_cart_matches_c_reference() {
    check_jfdctint(CompileMode::Optimized, Runtime::ActionCart);
}

#[test]
fn jfdctint_classic_standalone_matches_c_reference() {
    check_jfdctint(CompileMode::Optimized, Runtime::Standalone);
}
