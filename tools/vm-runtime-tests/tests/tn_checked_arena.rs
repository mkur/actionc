//! Calls the maintained library, including released-byte and window checkpoints.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, VmRunHooks};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
#[path = "support/tn_symbols.rs"]
mod symbols;

struct Observe {
    high_water: u16,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        self.high_water = self.high_water.max(vm.bus().ram().read_word(0xE8));
        Ok(())
    }
}

struct Sources(PathBuf);
impl Sources {
    fn new(crlf: bool) -> Self {
        let path =
            std::env::temp_dir().join(format!("actionc-arena-{}-{crlf}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        for name in ["TN.ACT", "TNDBG.ACT", "LIB.ACT"] {
            let text = std::fs::read_to_string(root().join("samples/tn/modern").join(name))
                .unwrap()
                .replace("\r\n", "\n");
            std::fs::write(
                path.join(name),
                if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text
                },
            )
            .unwrap();
        }
        Self(path)
    }
}
impl Drop for Sources {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn compile(
    sources: &Sources,
    name: &str,
    mode: CompileMode,
    runtime: Runtime,
) -> (Vec<u8>, String) {
    // An explicitly selected freshly built release compiler avoids spending
    // minutes in debug materialization. CI normally exercises the linked crate.
    if let Some(compiler) = std::env::var_os("ACTIONC_ARENA_COMPILER") {
        let output = std::process::Command::new(compiler)
            .args([
                "--mode",
                if mode == CompileMode::Mir6502 {
                    "mir6502"
                } else {
                    "optimized"
                },
                "--runtime",
                if runtime == Runtime::ActionCart {
                    "cart"
                } else {
                    "standalone"
                },
            ])
            .arg("-o")
            .arg(sources.0.join("image.xex"))
            .arg("--listing")
            .arg(sources.0.join("image.asm"))
            .arg(sources.0.join(name))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        (
            std::fs::read(sources.0.join("image.xex")).unwrap(),
            std::fs::read_to_string(sources.0.join("image.asm")).unwrap(),
        )
    } else {
        let compiled = compile_file(
            sources.0.join(name),
            &CompileOptions::for_mode(mode).with_runtime(runtime),
        )
        .unwrap();
        (compiled.object_bytes().to_vec(), compiled.source_listing())
    }
}

#[test]
fn real_arena_success_and_window_checkpoints_with_lf_and_crlf() {
    let mut calls = 0;
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for name in ["TN.ACT", "TNDBG.ACT"] {
                let mut image = None;
                for crlf in [false, true] {
                    let sources = Sources::new(crlf);
                    eprintln!("{name}/{mode:?}/{runtime:?}/crlf={crlf}");
                    let (bytes, listing) = compile(&sources, name, mode, runtime);
                    if let Some(image) = &image {
                        assert_eq!(image, &bytes);
                    } else {
                        image = Some(bytes.clone());
                    }
                    let routine = |n| symbols::routine_address(&listing, n);
                    let global = |n| symbols::global_address(&listing, n);
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &bytes).unwrap();
                    vm.bus_mut().ram_mut().write(machine::RETURN, 0x60);
                    let mut hooks = Observe { high_water: 0 };
                    vm = machine::call(vm, &mut hooks, routine("Init"), &[]);
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2000);
                    for n in [0u16, 1, 255, 256, 0x800 - 512] {
                        let before = vm.bus().ram().read_word(0xE8);
                        vm = machine::call(vm, &mut hooks, routine("Alloc"), &n.to_le_bytes());
                        assert_eq!(vm.bus().ram().read_word(0xA0), before);
                        assert_eq!(vm.bus().ram().read_word(0xE8), before + n);
                        calls += 1;
                    }
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2800);
                    vm = machine::call(vm, &mut hooks, routine("Alloc"), &[0, 0]);
                    assert_eq!(vm.bus().ram().read_word(0xA0), 0x2800);
                    vm = machine::call(vm, &mut hooks, routine("Free"), &0x2000u16.to_le_bytes());
                    vm = machine::call(vm, &mut hooks, routine("Push"), &0xBEEFu16.to_le_bytes());
                    assert_eq!(vm.bus().ram().read_word(0x2000), 0xBEEF);
                    vm = machine::call(vm, &mut hooks, routine("Pull"), &[]);
                    assert_eq!(vm.bus().ram().read_word(0xA0), 0xBEEF);
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2000);
                    // MkDir's released allocation remains writable scratch.
                    vm = machine::call(vm, &mut hooks, routine("Alloc"), &20u16.to_le_bytes());
                    vm = machine::call(vm, &mut hooks, routine("Free"), &0x2000u16.to_le_bytes());
                    vm.bus_mut()
                        .ram_mut()
                        .map(0x2000, &[3, b'D', b':', b'X'])
                        .unwrap();
                    assert_eq!(machine::bytes(&vm, 0x2000, 4), [3, b'D', b':', b'X']);
                    // Exercise real drawing and image restoration without CIO.
                    vm.bus_mut().ram_mut().write_word(0x58, 0x7000);
                    let screen = (0..960)
                        .map(|i| (i as u8).wrapping_mul(17))
                        .collect::<Vec<_>>();
                    vm.bus_mut().ram_mut().map(0x7000, &screen).unwrap();
                    let position = [0, 0, 39, 23];
                    vm.bus_mut().ram_mut().map(0x5A, &position).unwrap();
                    vm.bus_mut().ram_mut().write(0x1FFF, 0xA5);
                    vm.bus_mut().ram_mut().write(0x2800, 0x5A);
                    vm = machine::call(vm, &mut hooks, routine("Window"), &[1, 1, 10, 4]);
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2000 + 4 + 55 + 2 + 4);
                    assert_eq!(vm.bus().ram().read(global("openwindows")), 1);
                    vm = machine::call(vm, &mut hooks, routine("CloseWindow"), &[]);
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2000);
                    assert_eq!(vm.bus().ram().read(global("openwindows")), 0);
                    assert_eq!(machine::bytes(&vm, 0x5A, 4), position);
                    assert_eq!(machine::bytes(&vm, 0x7000, 960), screen);
                    assert_eq!(vm.bus().ram().read(0x1FFF), 0xA5);
                    assert_eq!(vm.bus().ram().read(0x2800), 0x5A);
                    assert_eq!(hooks.high_water, 0x2800);
                    calls += 10;
                }
            }
        }
    }
    eprintln!(
        "checked-arena successful controls: {calls} calls, both backends/runtimes and LF/CRLF"
    );
}
