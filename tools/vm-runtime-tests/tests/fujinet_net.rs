//! Execute the production NET and SIO modules against a stateful SIOV peripheral.
//! Wire contract: FujiNet v1.6.1, d7a5b2faac61a7889874849bf5db6f82718e42c1.
//! No live network or firmware emulation is claimed by this controlled service.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::includes::{ModuleLoadOptions, load_compilation};
use actionc_vm::{
    AddressRange, BusAccess, CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind,
    OS_ROM_BASE, RunRequest, StopReason, VmRunHooks, VmRunner,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const SIOV: u16 = 0xE459;
const DONE: u16 = 0x0700;
const SERVICE: u16 = 0x0720;
const SERVICE_RETURN: u16 = SERVICE + 19;
const ERROR_TRAP: u16 = 0x0780;
const BUFFER: u16 = 0x08FF;
const OUTPUT: u16 = 0x1000;
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Source(PathBuf);
impl Source {
    fn new(text: &str, crlf_copy: bool) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-fujinet-net-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let mut source = text.replace("\r\n", "\n");
        if crlf_copy {
            // Reserved ATARI modules cannot be overridden. Compile identical
            // bodies under a host namespace to exercise CRLF module loading.
            source = source.replace("ATARI.", "SIO_COPY.");
            for relative in [
                "sio.act",
                "sio/devices.act",
                "fujinet/net.act",
                "fujinet/net/commands.act",
            ] {
                let body =
                    std::fs::read_to_string(root().join("embedded/modules/atari").join(relative))
                        .unwrap()
                        .replace("\r\n", "\n")
                        .replace("ATARI.", "SIO_COPY.")
                        .replace('\n', "\r\n");
                let target = path.join("sio_copy").join(relative);
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::write(target, body).unwrap();
            }
            source = source.replace('\n', "\r\n");
        }
        std::fs::write(path.join("main.act"), source).unwrap();
        Self(path)
    }

    fn main(&self) -> PathBuf {
        self.0.join("main.act")
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn images(text: &str, include_raw: bool, crlf_copy: bool) -> Vec<(String, Runtime, Vec<u8>)> {
    let source = Source::new(text, crlf_copy);
    let loaded = load_compilation(&source.main(), &ModuleLoadOptions::default()).unwrap();
    assert_eq!(
        loaded
            .modules
            .iter()
            .any(|m| m.origin.to_string() == "<embedded:ATARI.SIO>"),
        !crlf_copy,
        "exercise the intended module provider"
    );
    let mut images = Vec::new();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let compiled = compile_file(
                source.main(),
                &CompileOptions::for_mode(mode)
                    .with_runtime(runtime)
                    .with_origin(0x3000),
            )
            .unwrap();
            images.push((
                format!("{mode:?}/{runtime:?}/CRLF={crlf_copy}"),
                runtime,
                compiled.object_bytes().to_vec(),
            ));
        }
        if include_raw {
            let model = actionc::semantic::analyze_compilation_with_options(
                &loaded,
                actionc::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            let semir = actionc::semantic::ir::lower_compilation(&loaded, &model);
            let raw = actionc::nir::lower_program(&semir);
            actionc::nir::verify_program(&raw).unwrap();
            for optimized in [false, true] {
                let nir = if optimized {
                    actionc::nir::optimize_program(&raw).unwrap()
                } else {
                    raw.clone()
                };
                let output = actionc::mir6502::generate_output_with_config_and_runtime(
                    &nir,
                    0x3000,
                    &actionc::mir6502::Mir6502Config::default(),
                    runtime,
                )
                .unwrap();
                images.push((
                    format!("MIR/{runtime:?}/optimized={optimized}"),
                    runtime,
                    actionc::codegen::format_load_file(&output),
                ));
            }
        }
    }
    images
}

// Step fixtures control external events; Endpoint tracks connection ownership,
// buffered data, sticky translation and bytes actually sent across later calls.
#[derive(Clone)]
struct Step {
    unit: u8,
    command: u8,
    timeout: u8,
    aux: u16,
    status: u8,
    sent: Vec<u8>,
    received: Vec<u8>,
}
impl Step {
    fn new(unit: u8, command: u8, aux: u16) -> Self {
        Self {
            unit,
            command,
            timeout: 15,
            aux,
            status: 1,
            sent: vec![],
            received: vec![],
        }
    }
    fn reply(mut self, bytes: &[u8]) -> Self {
        self.received = bytes.to_vec();
        self
    }
    fn send(mut self, bytes: &[u8]) -> Self {
        self.sent = bytes.to_vec();
        self
    }
    fn fail(mut self, status: u8) -> Self {
        self.status = status;
        self
    }
}
#[derive(Default)]
struct Endpoint {
    open: bool,
    translation: u8,
    remaining: usize,
    written: Vec<u8>,
}
struct Service {
    steps: Vec<Step>,
    next: usize,
    endpoints: [Endpoint; 8],
}
impl Service {
    fn new(steps: Vec<Step>) -> Self {
        Self {
            steps,
            next: 0,
            endpoints: std::array::from_fn(|i| Endpoint {
                translation: if i % 2 == 0 { 3 } else { 255 },
                ..Endpoint::default()
            }),
        }
    }
}
impl VmRunHooks for Service {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let pc = vm.cpu().registers().pc;
        if pc == SERVICE_RETURN {
            vm.bus_mut().clear_events();
        }
        if pc != SIOV {
            return Ok(());
        }
        let step = self
            .steps
            .get(self.next)
            .ok_or("unexpected extra SIO call")?;
        let dcb: Vec<_> = (0x300..=0x30B).map(|a| vm.bus().ram().read(a)).collect();
        let buffer = u16::from_le_bytes([dcb[4], dcb[5]]);
        let length = u16::from_le_bytes([dcb[8], dcb[9]]) as usize;
        let (direction, expected_length) = match step.command {
            b'T' | b'C' => (0, 0),
            b'O' => (0x80, 256),
            b'S' => (0x40, 4),
            b'R' => (0x40, usize::from(step.aux)),
            b'W' => (0x80, usize::from(step.aux)),
            _ => panic!("unmodeled command"),
        };
        let expected = [
            0x71,
            step.unit,
            step.command,
            direction,
            dcb[4],
            dcb[5],
            step.timeout,
            0,
            expected_length as u8,
            (expected_length >> 8) as u8,
            step.aux as u8,
            (step.aux >> 8) as u8,
        ];
        assert_eq!(dcb, expected, "transaction {}", self.next);
        if length == 0 {
            assert_eq!(buffer, 0);
        }
        for address in 0x300..=0x30B {
            assert_eq!(
                vm.bus()
                    .events()
                    .iter()
                    .filter(|e| e.address == address && e.access == BusAccess::Write)
                    .count(),
                1,
                "transaction {} DCB ${address:04X}",
                self.next
            );
        }
        if direction == 0x80 {
            let actual: Vec<_> = (0..length)
                .map(|i| vm.bus().ram().read(buffer.wrapping_add(i as u16)))
                .collect();
            assert_eq!(actual, step.sent, "transaction {} payload", self.next);
        }
        let endpoint = &mut self.endpoints[usize::from(step.unit - 1)];
        match step.command {
            b'T' if step.status == 1 => endpoint.translation = (step.aux >> 8) as u8,
            b'O' => {
                assert_eq!(
                    endpoint.translation, 0,
                    "Open must clear inherited translation"
                );
                assert_eq!(step.sent.len(), 256);
                let end = step.sent.iter().position(|b| *b == 0).unwrap();
                assert!(step.sent[end..].iter().all(|b| *b == 0));
                endpoint.open = step.status == 1;
                endpoint.translation = (step.aux >> 8) as u8;
                endpoint.remaining = 0;
            }
            b'C' if step.status == 1 => endpoint.open = false,
            b'S' if step.status == 1 => {
                assert_eq!(step.received.len(), 4);
                endpoint.remaining =
                    u16::from_le_bytes([step.received[0], step.received[1]]) as usize;
            }
            b'R' => {
                assert!(endpoint.open, "read requires the test's preceding Open");
                assert!(
                    length <= endpoint.remaining,
                    "never read past advertised availability"
                );
                if step.status == 1 {
                    assert_eq!(step.received.len(), length);
                    endpoint.remaining -= length;
                }
            }
            b'W' => {
                assert!(endpoint.open);
                endpoint.written.extend_from_slice(&step.sent);
            }
            _ => {}
        }
        assert!(step.received.len() <= length);
        if !step.received.is_empty() {
            vm.bus_mut().ram_mut().map(buffer, &step.received).unwrap();
        }
        vm.bus_mut().ram_mut().write(SERVICE + 11, step.status);
        vm.bus_mut()
            .ram_mut()
            .write(SERVICE + 16, step.status ^ 255);
        self.next += 1;
        Ok(())
    }
}

fn execute(
    image: &[u8],
    runtime: Runtime,
    steps: Vec<Step>,
    initial: &[u8],
    fault: bool,
) -> actionc_vm::RunOutcome {
    let mut vm = CompilerVm::default();
    let mut os = std::fs::read(root().join("roms/altirraos-xl.rom")).unwrap();
    let offset = usize::from(SIOV - OS_ROM_BASE);
    os[offset..offset + 3].copy_from_slice(&[0x4C, SERVICE as u8, (SERVICE >> 8) as u8]);
    vm.load_image_bytes(ImageKind::Rom, "SIO test OS vector", OS_ROM_BASE, os)
        .unwrap();
    let profile = if runtime == Runtime::ActionCart {
        vm.load_image_bytes(
            ImageKind::Cartridge,
            "action.rom",
            DEFAULT_CART_BASE,
            std::fs::read(root().join("roms/action.rom")).unwrap(),
        )
        .unwrap();
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|s| s.end < 0x600 || s.start > 0x17FF)
    );
    for address in 0x600..=0x17FF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    for address in 0x2FF..=0x30C {
        vm.bus_mut().ram_mut().write(address, 0xA5);
    }
    if !initial.is_empty() {
        vm.bus_mut().ram_mut().map(BUFFER, initial).unwrap();
    }
    vm.bus_mut().ram_mut().write(DONE, 0xEA);
    vm.bus_mut().ram_mut().write(ERROR_TRAP, 0xEA);
    vm.bus_mut()
        .ram_mut()
        .map(
            SERVICE,
            &[
                0xA2, 0x12, // LDX #$12
                0xA9, 0xA5, // LDA #$A5
                0x95, 0x30, // STA $30,X: OS SIO zero-page work area
                0xCA, 0x10, 0xFB, // DEX; BPL loop
                0xD8, // CLD
                0xA0, 0x00, // LDY #status (host-supplied)
                0x8C, 0x03, 0x03, // STY DSTATS
                0xA9, 0xFF, // LDA #different value
                0xA2, 0xD7, // LDX #$D7
                0x60, // RTS
            ],
        )
        .unwrap();
    match runtime {
        Runtime::ActionCart => vm
            .bus_mut()
            .ram_mut()
            .map(0x4CB, &[0x4C, 0x80, 0x07])
            .unwrap(),
        Runtime::Standalone => vm.bus_mut().ram_mut().write_word(0xA, ERROR_TRAP),
    }
    vm.bus_mut().add_watch_range(AddressRange {
        start: 0x2FF,
        end: 0x30C,
    });
    vm.bus_mut().clear_events();
    let count = steps.len();
    let mut service = Service::new(steps);
    let stop = if fault { ERROR_TRAP } else { DONE };
    let result = VmRunner::new(vm)
        .run_with_hooks(
            RunRequest {
                max_steps: 8_000_000,
                stop_after_pc: Some(stop),
                history_len: 16,
            },
            &mut service,
        )
        .unwrap();
    assert_eq!(
        result.stop_reason(),
        StopReason::PcReached { pc: stop },
        "{:?}",
        result.report
    );
    assert_eq!(service.next, count);
    assert_eq!(result.memory().read(0x2FF), 0xA5);
    assert_eq!(result.memory().read(0x30C), 0xA5);
    if fault {
        assert_eq!(result.report.registers.a, 105);
        assert!(
            !result
                .vm
                .bus()
                .events()
                .iter()
                .any(|e| e.access == BusAccess::Write && (0x300..=0x30B).contains(&e.address))
        );
    }
    result
}

const PRELUDE: &str = r#"MODULE NET_TEST
USE ATARI.FUJINET.NET AS NET
BYTE ARRAY output(256)=$1000
BYTE ARRAY buffer(1024)=$08FF
BYTE cursor
PROC Done=$0700()
PROC Capture(NET.Result outcome)
  CASE outcome OF
  WHEN NET.Result.OK THEN
    output(cursor)=1 output(cursor+1)=0
  WHEN NET.Result.SIO_ERROR(status) THEN
    output(cursor)=2 output(cursor+1)=status
  WHEN NET.Result.INVALID(reason) THEN
    output(cursor)=3 output(cursor+1)=BYTE(reason)
  ESAC
  cursor==+2
RETURN
PROC CaptureRead(NET.ReadResult outcome)
  CASE outcome OF
  WHEN NET.ReadResult.DATA(count) THEN
    output(cursor)=1 output(cursor+1)=BYTE(count) output(cursor+2)=BYTE(count RSH 8)
  WHEN NET.ReadResult.WAITING THEN
    output(cursor)=2
  WHEN NET.ReadResult.END_OF_STREAM THEN
    output(cursor)=3
  WHEN NET.ReadResult.SIO_ERROR(status) THEN
    output(cursor)=4 output(cursor+1)=status
  WHEN NET.ReadResult.NETWORK_ERROR(code) THEN
    output(cursor)=5 output(cursor+1)=code
  WHEN NET.ReadResult.INVALID(reason) THEN
    output(cursor)=6 output(cursor+1)=BYTE(reason)
  ESAC
  cursor==+3
RETURN
PROC CaptureStatus(NET.StatusResult outcome)
  BYTE i
  CASE outcome OF
  WHEN NET.StatusResult.OK(value) THEN
    output(cursor)=1
    FOR i=0 TO 3 DO output(cursor+1+i)=value.bytes(i) OD
  WHEN NET.StatusResult.SIO_ERROR(status) THEN
    output(cursor)=2 output(cursor+1)=status
  WHEN NET.StatusResult.INVALID(reason) THEN
    output(cursor)=3 output(cursor+1)=BYTE(reason)
  ESAC
  cursor==+5
RETURN
"#;

fn payload(uri: &[u8]) -> Vec<u8> {
    assert!(uri.len() <= 255);
    let mut result = vec![0; 256];
    result[..uri.len()].copy_from_slice(uri);
    result
}
fn open(unit: u8, options: u16, uri: &[u8]) -> Vec<Step> {
    vec![
        Step::new(unit, b'T', 0),
        Step::new(unit, b'O', options).send(&payload(uri)),
    ]
}
fn source(main: &str) -> String {
    format!("{PRELUDE}\nPROC Main()\n{main}\n  Done()\nRETURN\nENDMODULE\n")
}

#[test]
fn network_stream_sequences_preserve_channels_counts_strings_and_eof() {
    let text = source(
        r#"
  NET.Channel first,second
  STRING uri="N:HTTP://example.test/",echo="N8:TCP://echo.test:1234/",empty="",greeting="hello"
  first=NET.DefaultChannel(1)
  second=NET.DefaultChannel(8)
  second.access=NET.READ_WRITE second.translation=NET.TRANSLATE_LF second.timeout=255
  cursor=0
  Capture(NET.Open(first,uri))
  Capture(NET.Open(second,echo))
  Capture(NET.WriteString(second,empty))
  Capture(NET.WriteString(second,greeting))
  Capture(NET.Write(second,@buffer(0),258))
  CaptureStatus(NET.Status(first))
  CaptureRead(NET.ReadAvailable(first,@buffer(0),512))
  CaptureRead(NET.ReadAvailable(second,@buffer(0),512))
  CaptureRead(NET.ReadAvailable(first,@buffer(0),512))
  output(200)=buffer(0) output(201)=buffer(255)
  CaptureRead(NET.ReadAvailable(first,@buffer(0),100))
  output(202)=buffer(0) output(203)=buffer(99)
  CaptureRead(NET.ReadAvailable(first,@buffer(0),512))
  output(204)=buffer(0) output(205)=buffer(243)
  CaptureRead(NET.ReadAvailable(first,@buffer(0),512))
  Capture(NET.Close(first))
  CaptureStatus(NET.Status(second))
  Capture(NET.Read(second,@buffer(0),258))
  output(206)=buffer(0) output(207)=buffer(257)
  Capture(NET.Close(second))
"#,
    );
    let initial: Vec<_> = (0..258).map(|i| (i % 251) as u8).collect();
    let mut steps = open(1, 4, b"N:HTTP://example.test/");
    steps.extend(open(8, 0x020C, b"N8:TCP://echo.test:1234/"));
    steps.extend([
        Step::new(8, b'W', 5).send(b"hello"),
        Step::new(8, b'W', 258).send(&initial),
        Step::new(1, b'S', 0).reply(&[0x34, 0x12, 7, 233]), // raw snapshot retains unusual fields
        Step::new(1, b'S', 0).reply(&[0, 0, 1, 1]),
        Step::new(8, b'S', 0).reply(&[0, 0, 0, 1]), // disconnected alone is not EOF
        Step::new(1, b'S', 0).reply(&[88, 2, 0, 136]), // 600 buffered after remote close
        Step::new(1, b'R', 256).reply(&vec![0xA1; 256]),
        Step::new(1, b'S', 0).reply(&[88, 1, 0, 136]),
        Step::new(1, b'R', 100).reply(&vec![0xB2; 100]),
        Step::new(1, b'S', 0).reply(&[244, 0, 0, 136]),
        Step::new(1, b'R', 244).reply(&vec![0xC3; 244]),
        Step::new(1, b'S', 0).reply(&[0, 0, 0, 136]),
        Step::new(1, b'C', 0),
        Step::new(8, b'S', 0).reply(&[2, 1, 1, 1]),
        Step::new(8, b'R', 258).reply(&vec![0xD4; 258]),
        Step::new(8, b'C', 0),
    ]);
    for step in &mut steps {
        if step.unit == 8 {
            step.timeout = 255;
        }
    }
    let expected = [
        1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0x34, 0x12, 7, 233, 2, 0xCC, 0xCC, 2, 0xCC, 0xCC, 1, 0, 1,
        1, 100, 0, 1, 244, 0, 3, 0xCC, 0xCC, 1, 0, 1, 2, 1, 1, 1, 1, 0, 1, 0,
    ];
    let mut all = images(&text, true, false);
    all.extend(images(&text, false, true));
    for (label, runtime, image) in all {
        let result = execute(&image, runtime, steps.clone(), &initial, false);
        let actual: Vec<_> = (0..expected.len())
            .map(|i| result.memory().read(OUTPUT + i as u16))
            .collect();
        assert_eq!(actual, expected, "{label}");
        let markers: Vec<_> = (200..208)
            .map(|i| result.memory().read(OUTPUT + i))
            .collect();
        assert_eq!(
            markers,
            [0xA1, 0xA1, 0xB2, 0xB2, 0xC3, 0xC3, 0xD4, 0xD4],
            "{label}"
        );
    }
}

#[test]
fn invalid_helper_inputs_do_not_touch_the_dcb_or_enter_sio() {
    let text = source(
        r#"
  NET.Channel channel
  STRING empty=""
  channel=NET.DefaultChannel(0)
  cursor=0
  Capture(NET.Close(channel))
  Capture(NET.WriteString(channel,empty))
  CaptureStatus(NET.Status(channel))
  CaptureRead(NET.ReadAvailable(channel,@buffer(0),1))
  channel.unit=9 Capture(NET.Close(channel))
  channel.unit=255 Capture(NET.Close(channel))
  channel.unit=1 channel.timeout=0 Capture(NET.Close(channel))
  channel.timeout=15 channel.chunkLimit=0 Capture(NET.Close(channel))
  channel.chunkLimit=256 channel.access=0 Capture(NET.Close(channel))
  channel.access=6 Capture(NET.Close(channel))
  channel.access=4 channel.translation=4 Capture(NET.Close(channel))
  channel.translation=255 Capture(NET.Close(channel))
  channel.translation=0
  Capture(NET.Read(channel,@buffer(0),0))
  Capture(NET.Write(channel,@buffer(0),0))
  CaptureRead(NET.ReadAvailable(channel,@buffer(0),0))
  Capture(NET.WriteString(channel,empty))
  Capture(NET.Open(channel,empty))
  Capture(NET.OpenBuffer(channel,BYTE POINTER(0),0))
  Capture(NET.OpenBuffer(channel,BYTE POINTER(0),256))
  Capture(NET.OpenBuffer(channel,BYTE POINTER(0),65535))
  buffer(0)='N buffer(1)=': buffer(2)=0
  Capture(NET.OpenBuffer(channel,@buffer(0),3))
  buffer(2)=$9B Capture(NET.OpenBuffer(channel,@buffer(0),3))
  buffer(1)='8 buffer(2)=':
  Capture(NET.OpenBuffer(channel,@buffer(0),3))
  buffer(1)='0 Capture(NET.OpenBuffer(channel,@buffer(0),3))
  buffer(1)='1 buffer(2)='0 buffer(3)=':
  Capture(NET.OpenBuffer(channel,@buffer(0),4))
"#,
    );
    let expected = [
        3, 1, 3, 1, 3, 1, 0xCC, 0xCC, 0xCC, 6, 1, 0xCC, 3, 1, 3, 1, 3, 2, 3, 3, 3, 4, 3, 4, 3, 5,
        3, 5, 3, 6, 3, 6, 6, 6, 0xCC, 1, 0, 3, 7, 3, 7, 3, 7, 3, 7, 3, 8, 3, 8, 3, 9, 3, 9, 3, 9,
    ];
    for (label, runtime, image) in images(&text, false, false) {
        let result = execute(&image, runtime, vec![], &[], false);
        let actual: Vec<_> = (0..expected.len())
            .map(|i| result.memory().read(OUTPUT + i as u16))
            .collect();
        assert_eq!(actual, expected, "{label}");
        assert!(
            !result
                .vm
                .bus()
                .events()
                .iter()
                .any(|e| e.access == BusAccess::Write && (0x300..=0x30B).contains(&e.address)),
            "{label}: invalid input touched DCB"
        );
    }
}

#[test]
fn network_failures_preserve_primary_results_without_retry_or_invented_counts() {
    let text = source(
        r#"
  NET.Channel channel
  NET.Result primary
  NET.ReadResult readFailure
  STRING uri="N:TCP://echo.test:1234/"
  CHAR ARRAY binaryText(4)
  CARD i
  channel=NET.DefaultChannel(2)
  channel.access=NET.HTTP_POST
  cursor=0
  Capture(NET.Open(channel,uri))
  Capture(NET.Open(channel,uri))
  channel.access=NET.HTTP_PUT
  Capture(NET.Open(channel,uri))
  channel.chunkLimit=17
  CaptureRead(NET.ReadAvailable(channel,@buffer(0),128))
  CaptureRead(NET.ReadAvailable(channel,@buffer(0),128))
  CaptureRead(NET.ReadAvailable(channel,@buffer(0),128))
  CaptureRead(NET.ReadAvailable(channel,@buffer(0),128))
  readFailure=NET.ReadAvailable(channel,@buffer(0),128)
  CaptureRead(readFailure)
  output(200)=buffer(0) output(201)=buffer(1) output(202)=buffer(2)
  CaptureStatus(NET.Status(channel))
  Capture(NET.Close(channel))
  CaptureRead(readFailure)
  ; A full 255-byte span crossing a page needs only one zero terminator.
  FOR i=0 TO 254 DO buffer(i)='A OD
  buffer(0)='N buffer(1)='2 buffer(2)=':
  Capture(NET.OpenBuffer(channel,@buffer(0),255))
  ; These access/translation changes affect validation, not the open connection.
  channel.access=NET.WRITE_ONLY channel.translation=NET.TRANSLATE_CRLF
  binaryText(0)=3 binaryText(1)='A binaryText(2)=0 binaryText(3)='B
  primary=NET.WriteString(channel,binaryText)
  Capture(primary)
  Capture(NET.Close(channel))
  Capture(primary)
"#,
    );
    let mut steps = vec![Step::new(2, b'T', 0).fail(0x8A)];
    steps.extend(open(2, 13, b"N:TCP://echo.test:1234/"));
    steps.last_mut().unwrap().status = 0x90;
    steps.extend(open(2, 14, b"N:TCP://echo.test:1234/"));
    steps.extend([
        Step::new(2, b'S', 0).fail(0x8B),
        Step::new(2, b'S', 0).reply(&[0, 0, 0, 233]),
        Step::new(2, b'S', 0).reply(&[10, 0, 0, 204]), // non-EOF error is not hidden by available data
        Step::new(2, b'S', 0).reply(&[255, 255, 1, 1]),
        Step::new(2, b'R', 17).reply(&vec![0x41; 17]),
        Step::new(2, b'S', 0).reply(&[20, 0, 1, 1]),
        Step::new(2, b'R', 17).reply(&[0xDE, 0xAD]).fail(0x90), // partial buffer write, no DATA count
        Step::new(2, b'S', 0).reply(&[99, 88]).fail(0x8F), // incomplete status must not become OK
        Step::new(2, b'C', 0).fail(0x80),
    ]);
    let mut uri = vec![b'A'; 255];
    uri[..3].copy_from_slice(b"N2:");
    steps.extend(open(2, 14, &uri));
    steps.extend([
        Step::new(2, b'W', 3).send(b"A\0B").fail(0x8F),
        Step::new(2, b'C', 0),
    ]);
    let expected = [
        2, 0x8A, 2, 0x90, 1, 0, 4, 0x8B, 0xCC, 5, 233, 0xCC, 5, 204, 0xCC, 1, 17, 0, 4, 0x90, 0xCC,
        2, 0x8F, 0xCC, 0xCC, 0xCC, 2, 0x80, 4, 0x90, 0xCC, 1, 0, 2, 0x8F, 1, 0, 2, 0x8F,
    ];
    for (label, runtime, image) in images(&text, false, false) {
        let result = execute(&image, runtime, steps.clone(), &[], false);
        let actual: Vec<_> = (0..expected.len())
            .map(|i| result.memory().read(OUTPUT + i as u16))
            .collect();
        assert_eq!(actual, expected, "{label}");
        assert_eq!(
            [
                result.memory().read(OUTPUT + 200),
                result.memory().read(OUTPUT + 201),
                result.memory().read(OUTPUT + 202)
            ],
            [0xDE, 0xAD, 0x41],
            "{label}"
        );
    }
}

#[test]
fn uri_string_and_span_paths_agree_and_stream_bytes_remain_binary() {
    let text = source(
        r#"
  NET.Channel channel
  STRING uri="N3:TCP://echo.test:1234/",one="N"
  CHAR ARRAY binaryText(6)
  channel=NET.DefaultChannel(3)
  channel.access=NET.READ_WRITE
  cursor=0
  Capture(NET.Open(channel,one))
  Capture(NET.Open(channel,uri))
  Capture(NET.Close(channel))
  Capture(NET.OpenBuffer(channel,BYTE POINTER(@uri(1)),uri(0)))
  binaryText(0)=5 binaryText(1)=0 binaryText(2)=10
  binaryText(3)=13 binaryText(4)=$9B binaryText(5)=$FF
  Capture(NET.WriteString(channel,binaryText))
  CaptureRead(NET.ReadAvailable(channel,@buffer(0),5))
  CaptureRead(NET.ReadAvailable(channel,@buffer(0),5))
  Capture(NET.Close(channel))
"#,
    );
    let mut steps = open(3, 12, b"N");
    steps.last_mut().unwrap().status = 0x90; // URI syntax belongs to the device
    steps.extend(open(3, 12, b"N3:TCP://echo.test:1234/"));
    steps.push(Step::new(3, b'C', 0));
    steps.extend(open(3, 12, b"N3:TCP://echo.test:1234/"));
    let bytes = [0, 10, 13, 0x9B, 0xFF];
    steps.extend([
        Step::new(3, b'W', 5).send(&bytes),
        Step::new(3, b'S', 0).reply(&[5, 0, 1, 1]),
        Step::new(3, b'R', 5).reply(&bytes),
        Step::new(3, b'S', 0).reply(&[0, 0, 0, 136]),
        Step::new(3, b'C', 0),
    ]);
    let expected = [
        2, 0x90, 1, 0, 1, 0, 1, 0, 1, 0, 1, 5, 0, 3, 0xCC, 0xCC, 1, 0,
    ];
    for (label, runtime, image) in images(&text, false, false) {
        let result = execute(&image, runtime, steps.clone(), &[], false);
        let actual: Vec<_> = (0..expected.len())
            .map(|i| result.memory().read(OUTPUT + i as u16))
            .collect();
        assert_eq!(actual, expected, "{label}");
        assert_eq!(
            (0..5)
                .map(|i| result.memory().read(BUFFER + i))
                .collect::<Vec<_>>(),
            bytes,
            "{label}"
        );
        assert_eq!(result.memory().read(BUFFER - 1), 0xCC, "{label}");
        assert_eq!(result.memory().read(BUFFER + 5), 0xCC, "{label}");
    }
}
