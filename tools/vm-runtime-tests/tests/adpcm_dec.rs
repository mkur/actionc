//! Stateful TACLeBench ADPCM decoding: every sample and state against pinned C.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/adpcm_dec/adpcm_dec.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/adpcm_dec/vectors.txt");
const BASE: u16 = 0x0600;
const BYTES: usize = 0x0A00;
const INPUT: usize = 0x07FF - BASE as usize;
const RESETS: usize = 0x09FF - BASE as usize;
const STATE: usize = 0x0BFF - BASE as usize;
const READY: u16 = 0x0603;
const DONE: u16 = 0x06FF;

#[derive(Debug, PartialEq, Eq)]
struct Event {
    kind: u8,
    state: Vec<u8>,
}
#[derive(Debug, PartialEq, Eq)]
struct Vector {
    label: String,
    original: bool,
    codes: Vec<u8>,
    resets: Vec<u8>,
    events: Vec<Event>,
    report: Vec<u8>,
}

fn hex(text: &str) -> Vec<u8> {
    if text == "-" {
        return vec![];
    }
    assert_eq!(text.len() % 2, 0);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn state_layout(text: &str) -> Vec<(&str, usize)> {
    text.lines()
        .find_map(|line| line.strip_prefix("# state "))
        .unwrap()
        .split_whitespace()
        .map(|field| {
            let (name, length) = field.split_once(':').unwrap();
            assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
            let length = length.parse::<usize>().unwrap();
            assert!((1..=11).contains(&length));
            (name, length)
        })
        .collect()
}

fn parse_vectors(text: &str) -> Vec<Vector> {
    let bytes: usize = state_layout(text).iter().map(|(_, n)| n * 4).sum();
    assert_eq!(bytes, 332);
    assert!(STATE + bytes <= BYTES);
    let mut vectors = Vec::new();
    let mut current: Option<Vector> = None;
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        match fields[0] {
            "case" => {
                assert!(current.is_none());
                assert_eq!(fields.len(), 5);
                assert!(["0", "1"].contains(&fields[2]));
                current = Some(Vector {
                    label: fields[1].into(),
                    original: fields[2] == "1",
                    codes: hex(fields[3]),
                    resets: hex(fields[4]),
                    events: vec![],
                    report: vec![],
                });
            }
            "event" => {
                assert_eq!(fields.len(), 3);
                let event = Event {
                    kind: fields[1].parse().unwrap(),
                    state: hex(fields[2]),
                };
                assert!([1, 2].contains(&event.kind));
                assert_eq!(event.state.len(), bytes);
                current.as_mut().unwrap().events.push(event);
            }
            "end" => {
                assert_eq!(fields.len(), 2);
                let mut vector = current.take().unwrap();
                vector.report = hex(fields[1]);
                assert_eq!(vector.report.len(), 24);
                assert_eq!(vector.codes.len(), vector.resets.len());
                assert!(vector.codes.len() <= 256);
                assert!(vector.resets.iter().all(|&r| r <= 1));
                let mut kinds = vec![1];
                for &reset in &vector.resets {
                    if reset != 0 {
                        kinds.push(1);
                    }
                    kinds.push(2);
                }
                assert_eq!(
                    vector.events.iter().map(|e| e.kind).collect::<Vec<_>>(),
                    kinds
                );
                vectors.push(vector);
            }
            other => panic!("unexpected vector record {other}"),
        }
    }
    assert!(current.is_none());
    vectors
}

fn replace_once(source: &mut String, old: &str, new: &str) {
    assert_eq!(source.matches(old).count(), 1, "source anchor: {old}");
    *source = source.replace(old, new);
}

fn instrument(source: &str, vectors: &str) -> String {
    let mut source = source.replace("\r\n", "\n");
    let layout = state_layout(vectors);
    let words: usize = layout.iter().map(|(_, n)| n).sum();
    let mut capture = format!(
        "INT result\n\
         BYTE testOriginal=$0600,testReady=$0603,testDone=$06FF\n\
         CARD testCount=$0601\n\
         BYTE ARRAY testInput(256)=$07FF,testResets(256)=$09FF\n\
         LONGINT ARRAY testState({words})=$0BFF,testReport(6)=$0608\n\
         PROC Capture(BYTE kind)\n  BYTE i\n"
    );
    let mut offset = 0;
    for (name, size) in layout {
        if size == 1 {
            capture.push_str(&format!("  testState({offset})={name}\n"));
        } else {
            capture.push_str(&format!(
                "  FOR i=0 TO {} DO testState({offset}+i)={name}(i) OD\n",
                size - 1
            ));
        }
        offset += size;
    }
    // The VM inspects this checkpoint before acknowledging it. Absolute-memory
    // reads keep the wait observable through all optimizer paths.
    capture.push_str("  testReady=kind\n  DO UNTIL testReady=0 OD\nRETURN\n");
    replace_once(&mut source, "INT result\n", &capture);
    replace_once(
        &mut source,
        "  accumd(0)=xs\nRETURN",
        "  accumd(0)=xs\n  Capture(2)\nRETURN",
    );
    replace_once(
        &mut source,
        "  FOR i=0 TO 10 DO accumc(i)=0 accumd(i)=0 OD\nRETURN",
        "  FOR i=0 TO 10 DO accumc(i)=0 accumd(i)=0 OD\n  Capture(1)\nRETURN",
    );
    replace_once(&mut source, "PROC Main()\n", "PROC Benchmark()\n");
    replace_once(
        &mut source,
        "ENDMODULE\n",
        "\
        PROC Main()\n\
          CARD i\n\
          IF testOriginal#0 THEN\n\
            Benchmark()\n\
            testReport(0)=result testReport(1)=checksum\n\
            FOR i=0 TO 3 DO testReport(2+i)=samples(i) OD\n\
          ELSE\n\
            Reset()\n\
            IF testCount>0 THEN\n\
              FOR i=0 TO testCount-1 DO\n\
                IF testResets(i)#0 THEN Reset() FI\n\
                Decode(testInput(i))\n\
              OD\n\
            FI\n\
          FI\n\
          testDone=$A5\n\
        RETURN\nENDMODULE\n",
    );
    source
}

#[test]
fn adpcm_dec_lf_and_crlf_preserve_instrumentation_and_reference_vectors() {
    let source = SOURCE.replace("\r\n", "\n");
    let vectors = VECTORS.replace("\r\n", "\n");
    let crlf = vectors.replace('\n', "\r\n");
    assert_eq!(
        instrument(&source, &vectors),
        instrument(&source.replace('\n', "\r\n"), &crlf)
    );
    let parsed = parse_vectors(&vectors);
    assert_eq!(parsed, parse_vectors(&crlf));
    assert_eq!(parsed.len(), 15);
    let original = parsed.iter().find(|v| v.label == "upstream").unwrap();
    assert!(original.original);
    assert_eq!(original.codes, [0, 253]);
    assert_eq!(original.report[0..8], [0, 0, 0, 0, 254, 255, 255, 255]);
    let cold = parsed
        .iter()
        .find(|v| v.label == "each-code-from-reset")
        .unwrap();
    assert_eq!(cold.codes, (0..=255).collect::<Vec<u8>>());
    assert_eq!(cold.resets, vec![1; 256]);
}

struct TemporarySource(PathBuf);
impl TemporarySource {
    fn new(mode: CompileMode, runtime: Runtime) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "actionc-adpcm-dec-{}-{unique}-{mode:?}-{runtime:?}",
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

fn assert_memory(vm: &CompilerVm, expected: &[u8], label: &str) {
    for (offset, &value) in expected.iter().enumerate() {
        let address = BASE + offset as u16;
        assert_eq!(
            vm.bus().ram().read(address),
            value,
            "{label}: ${address:04X}"
        );
    }
}

fn advance(vm: &mut CompilerVm, label: &str) -> u64 {
    let limit = 1_000_000;
    for steps in 0..limit {
        if vm.bus().ram().read(READY) != 0 || vm.bus().ram().read(DONE) == 0xA5 {
            return steps;
        }
        vm.step_cpu()
            .unwrap_or_else(|error| panic!("{label}: {error:?}"));
    }
    panic!(
        "{label}: no checkpoint in {limit} steps; PC=${:04X}",
        vm.cpu().registers().pc
    );
}

fn check_adpcm_dec(mode: CompileMode, runtime: Runtime) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temporary = TemporarySource::new(mode, runtime);
    let text = |s: &str| {
        let lf = s.replace("\r\n", "\n");
        if runtime == Runtime::Standalone {
            lf.replace('\n', "\r\n")
        } else {
            lf
        }
    };
    let vectors = text(VECTORS);
    let path = temporary.0.join("adpcm_dec.act");
    std::fs::write(&path, text(&instrument(&text(SOURCE), &vectors))).unwrap();
    let compiled = compile_file(
        &path,
        &CompileOptions::for_mode(mode)
            .with_runtime(runtime)
            .with_origin(0x2000),
    )
    .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
    let mut roms = Vec::new();
    let profile = if runtime == Runtime::Standalone {
        ExecutionProfile::StandaloneObject
    } else {
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
    };
    let mut total_steps = 0;
    for vector in parse_vectors(&vectors) {
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
                .all(|s| s.end < BASE || s.start >= 0x1000),
            "host overlap: {label}"
        );
        let mut expected = vec![0xCC; BYTES];
        expected[0] = u8::from(vector.original);
        expected[1..3].copy_from_slice(&(vector.codes.len() as u16).to_le_bytes());
        expected[3] = 0;
        expected[0xFF] = 0;
        expected[INPUT..INPUT + vector.codes.len()].copy_from_slice(&vector.codes);
        expected[RESETS..RESETS + vector.resets.len()].copy_from_slice(&vector.resets);
        vm.bus_mut().ram_mut().map(BASE, &expected).unwrap();
        for (i, event) in vector.events.iter().enumerate() {
            let event_label = format!("{label}/checkpoint {i}");
            total_steps += advance(&mut vm, &event_label);
            assert_ne!(vm.bus().ram().read(DONE), 0xA5, "early end: {event_label}");
            expected[3] = event.kind;
            expected[STATE..STATE + event.state.len()].copy_from_slice(&event.state);
            assert_memory(&vm, &expected, &event_label);
            vm.bus_mut().ram_mut().map(READY, &[0]).unwrap();
            expected[3] = 0;
        }
        total_steps += advance(&mut vm, &label);
        assert_eq!(
            vm.bus().ram().read(READY),
            0,
            "unexpected extra checkpoint: {label}"
        );
        expected[0xFF] = 0xA5;
        expected[8..32].copy_from_slice(&vector.report);
        assert_memory(&vm, &expected, &label);
    }
    eprintln!(
        "ADPCM {mode:?}/{runtime:?}: {} object bytes, {total_steps} instructions",
        compiled.object_bytes().len()
    );
}

#[test]
fn adpcm_dec_mir6502_cart_matches_c_reference() {
    check_adpcm_dec(CompileMode::Mir6502, Runtime::ActionCart);
}
#[test]
fn adpcm_dec_mir6502_standalone_matches_c_reference() {
    check_adpcm_dec(CompileMode::Mir6502, Runtime::Standalone);
}
#[test]
fn adpcm_dec_classic_cart_matches_c_reference() {
    check_adpcm_dec(CompileMode::Optimized, Runtime::ActionCart);
}
#[test]
fn adpcm_dec_classic_standalone_matches_c_reference() {
    check_adpcm_dec(CompileMode::Optimized, Runtime::Standalone);
}
