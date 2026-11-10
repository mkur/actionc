//! A store to a fixed address may target RAM or read-only cartridge memory.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile, ImageKind, VmRunHooks};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[path = "support/tn_symbols.rs"]
mod symbols;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Source(PathBuf);
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

const SOURCE: &str = "BYTE FUNC Probe()\n\
 BYTE cell=@ADDRESS@, original\n\
 original=cell\n\
 cell==!$FF\n\
 IF cell=original THEN\n\
  RETURN(1)\n\
 FI\n\
 RETURN(0)\n\
PROC Main()\n\
 Probe()\n";

#[test]
fn absolute_write_probes_read_back_ram_and_rom_through_actual_emission() {
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for readonly in [false, true] {
            let address = if readonly { 0xBFFA } else { 0x6000 };
            for origin in [0x3000, 0x41C7] {
                let mut previous = None;
                for crlf in [false, true] {
                    let source = Source(std::env::temp_dir().join(format!(
                        "actionc-readback-{}-{runtime:?}-{readonly}-{origin}-{crlf}.act",
                        std::process::id()
                    )));
                    let text = SOURCE.replace("@ADDRESS@", &format!("${address:04X}"));
                    std::fs::write(
                        &source.0,
                        if crlf {
                            text.replace('\n', "\r\n")
                        } else {
                            text
                        },
                    )
                    .unwrap();
                    let compiled = compile_file(
                        &source.0,
                        &CompileOptions::for_mode(CompileMode::Mir6502)
                            .with_runtime(runtime)
                            .with_origin(origin),
                    )
                    .unwrap();
                    if let Some(previous) = previous {
                        assert_eq!(compiled.object_bytes(), previous);
                    }
                    previous = Some(compiled.object_bytes().to_vec());
                    let entry = symbols::routine_address(&compiled.source_listing(), "Probe");
                    let mut vm = if runtime == Runtime::ActionCart {
                        machine::load(&compiled)
                    } else {
                        let mut vm = CompilerVm::default();
                        vm.load_atari_object_for_execution(
                            ExecutionProfile::StandaloneObject,
                            compiled.object_bytes(),
                        )
                        .unwrap();
                        if readonly {
                            vm.load_image_bytes(ImageKind::Rom, "probe-rom", address, vec![0x7F])
                                .unwrap();
                        }
                        vm
                    };
                    for value in [0, 127, 128, 254, 255] {
                        vm.bus_mut().ram_mut().write(address - 1, 0xA5);
                        vm.bus_mut().ram_mut().write(address, value);
                        vm.bus_mut().ram_mut().write(address + 1, 0x5A);
                        let original = vm.bus_mut().read(address);
                        vm = machine::call(vm, &mut Hooks, entry, &[]);
                        assert_eq!(
                            vm.bus().ram().read(0xA0),
                            u8::from(readonly),
                            "{runtime:?}/readonly={readonly}/origin={origin:04X}/crlf={crlf}/value={value}"
                        );
                        assert_eq!(
                            vm.bus_mut().read(address),
                            if readonly { original } else { !original }
                        );
                        assert_eq!(vm.bus().ram().read(address - 1), 0xA5);
                        assert_eq!(vm.bus().ram().read(address + 1), 0x5A);
                    }
                }
            }
        }
    }
}
