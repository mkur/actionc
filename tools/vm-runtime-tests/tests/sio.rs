//! Execute the embedded transport against a controlled OS service boundary.
//! The host supplies peripheral effects only after generated code reaches SIOV.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::includes::{ModuleLoadOptions, load_compilation};
use actionc_vm::{
    AddressRange, BusAccess, CompilerVm, CpuStep, DEFAULT_CART_BASE, ExecutionProfile, ImageKind,
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
            "actionc-sio-{}-{}",
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
                "sio/disk/commands.act",
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

#[derive(Clone)]
struct Transaction {
    dcb: [u8; 12],
    status: u8,
    sent: Vec<u8>,
    received: Vec<u8>,
}

struct Service {
    transactions: Vec<Transaction>,
    next: usize,
}
impl VmRunHooks for Service {
    type Error = String;

    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let pc = vm.cpu().registers().pc;
        if pc == SERVICE_RETURN {
            // Ignore the service's DSTATS write when counting the next caller's
            // DCB stores. All remaining instructions execute normally.
            vm.bus_mut().clear_events();
        }
        if pc != SIOV {
            return Ok(());
        }
        let expected = self
            .transactions
            .get(self.next)
            .ok_or("unexpected extra SIO call")?;
        let actual: Vec<_> = (0x300..=0x30B).map(|a| vm.bus().ram().read(a)).collect();
        if actual != expected.dcb {
            return Err(format!(
                "request {}: DCB {actual:02X?}, expected {:02X?}",
                self.next, expected.dcb
            ));
        }
        for address in 0x300..=0x30B {
            let writes = vm
                .bus()
                .events()
                .iter()
                .filter(|event| event.address == address && event.access == BusAccess::Write)
                .count();
            if writes != 1 {
                return Err(format!(
                    "request {}: DCB ${address:04X} written {writes} times",
                    self.next
                ));
            }
        }
        let buffer = u16::from_le_bytes([actual[4], actual[5]]);
        let length = u16::from_le_bytes([actual[8], actual[9]]) as usize;
        // Snapshot-only transactions intentionally accept arbitrary pointer/count
        // bits without touching that memory. They test marshalling, not OS I/O.
        if !expected.sent.is_empty() {
            assert_eq!(expected.sent.len(), length);
            let sent: Vec<_> = (0..length)
                .map(|i| vm.bus().ram().read(buffer + i as u16))
                .collect();
            if sent != expected.sent {
                return Err(format!("request {}: send bytes differ", self.next));
            }
        }
        assert!(expected.received.len() <= length);
        if !expected.received.is_empty() {
            vm.bus_mut()
                .ram_mut()
                .map(buffer, &expected.received)
                .unwrap();
        }
        // The 6502 stub returns Y/DSTATS, a different A, and clobbered X.
        vm.bus_mut().ram_mut().write(SERVICE + 11, expected.status);
        vm.bus_mut()
            .ram_mut()
            .write(SERVICE + 16, expected.status ^ 0xFF);
        self.next += 1;
        Ok(())
    }

    fn after_step(&mut self, _: &CompilerVm, _: &CpuStep) -> Result<(), String> {
        Ok(())
    }
}

fn execute(
    image: &[u8],
    runtime: Runtime,
    transactions: Vec<Transaction>,
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
    let count = transactions.len();
    let mut service = Service {
        transactions,
        next: 0,
    };
    let stop = if fault { ERROR_TRAP } else { DONE };
    let result = VmRunner::new(vm)
        .run_with_hooks(
            RunRequest {
                max_steps: 2_000_000,
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

const PRELUDE: &str = r#"MODULE SIO_TEST
USE ATARI.SIO AS SIO
USE ATARI.SIO.DEVICES AS DEV
USE ATARI.FUJINET.NET.COMMANDS AS NETCMD
BYTE ARRAY output=$1000
BYTE ARRAY bytes=$08FF
SIO.Request request
BYTE calls
PROC Done=$0700()

PROC CaptureResult(CARD index SIO.Result result)
  CASE result OF
  WHEN SIO.Result.OK THEN
    output(index*2)=1 output(index*2+1)=1
  WHEN SIO.Result.ERROR(status) THEN
    output(index*2)=2 output(index*2+1)=status
  ESAC
RETURN

SIO.Request FUNC NextRequest()
  calls==+1
RETURN(request)
"#;

fn mixed_source() -> String {
    format!(
        "{PRELUDE}{}",
        r#"
PROC Main()
  calls=0
  request.device=DEV.FUJINET_NETWORK request.unit=2
  request.command=NETCMD.OPEN request.timeout=15 request.aux.word=12
  request.phase=SIO.Transfer.WRITE(@bytes(0),256)
  LET saved=SIO.Execute(NextRequest())
  CaptureResult(0,saved)

  request.command=NETCMD.STATUS request.aux.word=0
  request.phase=SIO.Transfer.READ(@bytes(0),4)
  CaptureResult(1,SIO.Execute(request))
  output(48)=bytes(0) output(49)=bytes(1)

  request.command=NETCMD.READ request.aux.word=258
  request.phase=SIO.Transfer.READ(@bytes(0),258)
  CaptureResult(2,SIO.Execute(request))
  output(50)=bytes(0) output(51)=bytes(1) output(52)=bytes(2)

  request.command=NETCMD.WRITE
  request.phase=SIO.Transfer.WRITE(@bytes(0),258)
  CaptureResult(3,SIO.Execute(request))

  request.device=$7F request.unit=5 request.command=$E1 request.timeout=0
  request.aux.bytes.low=$34 request.aux.bytes.high=$12
  request.phase=SIO.Transfer.WRITE_READ(@bytes(0),257)
  CaptureResult(4,SIO.Execute(request))
  output(53)=bytes(0)

  request.device=0 request.unit=0 request.command=0 request.aux.word=0
  request.phase=SIO.Transfer.NO_DATA
  CaptureResult(5,SIO.Execute(request))

  request.device=$FF request.unit=$FF request.command=$FF request.timeout=$FF
  request.aux.word=$FFFF request.phase=SIO.Transfer.READ(BYTE POINTER(0),0)
  CaptureResult(6,SIO.Execute(request))
  request.phase=SIO.Transfer.READ(BYTE POINTER($0300),$FFFF)
  CaptureResult(7,SIO.Execute(request))
  request.phase=SIO.Transfer.READ(BYTE POINTER($FFFF),2)
  CaptureResult(8,SIO.Execute(request))
  CaptureResult(9,saved)
  output(54)=calls
  Done()
RETURN
ENDMODULE
"#
    )
}

#[test]
fn sio_marshals_all_directions_and_preserves_effects_and_result_values() {
    let mut initial: Vec<u8> = (0..512).map(|i| (i as u8).wrapping_mul(37)).collect();
    initial[..256].fill(0);
    let uri = b"N:http://example.invalid/test\0";
    initial[..uri.len()].copy_from_slice(uri);
    let mut after_read = initial.clone();
    after_read[..4].copy_from_slice(&[2, 1, 1, 0]);
    after_read[..3].copy_from_slice(&[0, 0x9B, 0xFF]);
    let exchanged: Vec<u8> = (0..257).map(|i| (i as u8) ^ 0x5A).collect();
    // Literal DCB bytes are an independent OS-layout oracle, not values read
    // from the implementation's record offsets or named command constants.
    let dcbs = [
        [0x71, 2, 0x4F, 0x80, 0xFF, 8, 15, 0, 0, 1, 12, 0],
        [0x71, 2, 0x53, 0x40, 0xFF, 8, 15, 0, 4, 0, 0, 0],
        [0x71, 2, 0x52, 0x40, 0xFF, 8, 15, 0, 2, 1, 2, 1],
        [0x71, 2, 0x57, 0x80, 0xFF, 8, 15, 0, 2, 1, 2, 1],
        [0x7F, 5, 0xE1, 0xC0, 0xFF, 8, 0, 0, 1, 1, 0x34, 0x12],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0xFF, 0xFF, 0xFF, 0x40, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0xFF],
        [
            0xFF, 0xFF, 0xFF, 0x40, 0, 3, 0xFF, 0, 0xFF, 0xFF, 0xFF, 0xFF,
        ],
        [
            0xFF, 0xFF, 0xFF, 0x40, 0xFF, 0xFF, 0xFF, 0, 2, 0, 0xFF, 0xFF,
        ],
    ];
    let statuses = [1, 1, 138, 139, 254, 0, 7, 143, 144];
    let mut transactions: Vec<_> = dcbs
        .into_iter()
        .zip(statuses)
        .map(|(dcb, status)| Transaction {
            dcb,
            status,
            sent: vec![],
            received: vec![],
        })
        .collect();
    transactions[0].sent = initial[..256].to_vec();
    transactions[1].received = vec![2, 1, 1, 0];
    transactions[2].received = vec![0, 0x9B, 0xFF];
    transactions[3].sent = after_read[..258].to_vec();
    transactions[4].sent = after_read[..257].to_vec();
    transactions[4].received = exchanged.clone();
    let mut final_buffer = after_read;
    final_buffer[..257].copy_from_slice(&exchanged);
    for (label, runtime, image) in images(&mixed_source(), true, false)
        .into_iter()
        .chain(images(&mixed_source(), false, true))
    {
        let result = execute(&image, runtime, transactions.clone(), &initial, false);
        let expected = [
            1, 1, 1, 1, 2, 138, 2, 139, 2, 254, 2, 0, 2, 7, 2, 143, 2, 144, 1, 1,
        ];
        assert_eq!(
            (0..20)
                .map(|i| result.memory().read(OUTPUT + i))
                .collect::<Vec<_>>(),
            expected,
            "{label}"
        );
        assert_eq!(
            (48..55)
                .map(|i| result.memory().read(OUTPUT + i))
                .collect::<Vec<_>>(),
            [2, 1, 0, 0x9B, 0xFF, 0x5A, 1],
            "{label}: aliases/capture"
        );
        assert_eq!(
            (0..512)
                .map(|i| result.memory().read(BUFFER + i))
                .collect::<Vec<_>>(),
            final_buffer,
            "{label}: partial reads and buffer tails"
        );
        assert_eq!(result.memory().read(BUFFER - 1), 0xCC);
        assert_eq!(result.memory().read(BUFFER + 512), 0xCC);
        assert_eq!(
            result.memory().read(0x303),
            144,
            "{label}: leave post-call DCB visible"
        );
    }
}

#[test]
fn sio_preserves_every_status_byte_and_only_one_is_success() {
    let source = format!(
        "{PRELUDE}{}",
        r#"
PROC Main()
  CARD code
  request.device=DEV.PRINTER request.unit=1 request.command=$E7 request.timeout=9
  request.phase=SIO.Transfer.NO_DATA
  FOR code=0 TO 255 DO
    request.aux.word=code
    CaptureResult(code,SIO.Execute(request))
  OD
  Done()
RETURN
ENDMODULE
"#
    );
    let transactions: Vec<_> = (0..=255)
        .map(|status| Transaction {
            dcb: [0x40, 1, 0xE7, 0, 0, 0, 9, 0, 0, 0, status, 0],
            status,
            sent: vec![],
            received: vec![],
        })
        .collect();
    for (label, runtime, image) in images(&source, false, false) {
        let result = execute(&image, runtime, transactions.clone(), &[], false);
        for status in 0..=255u16 {
            assert_eq!(
                result.memory().read(OUTPUT + status * 2),
                if status == 1 { 1 } else { 2 },
                "{label}/{status}"
            );
            assert_eq!(
                result.memory().read(OUTPUT + status * 2 + 1),
                status as u8,
                "{label}/{status}"
            );
        }
    }
}

#[test]
fn invalid_transfer_tags_fault_before_touching_the_dcb_or_calling_sio() {
    for tag in [0, 255] {
        let source = format!(
            "{PRELUDE}\nPROC Main()\n BYTE POINTER tag\n request.phase=SIO.Transfer.NO_DATA\n tag=BYTE POINTER(@request.phase)\n tag^={tag}\n CaptureResult(0,SIO.Execute(request))\n Done()\nRETURN\nENDMODULE\n"
        );
        for (label, runtime, image) in images(&source, false, false) {
            let result = execute(&image, runtime, vec![], &[], true);
            assert_eq!(
                result.memory().read(OUTPUT),
                0xCC,
                "{label}: caller must not resume"
            );
        }
    }
}
