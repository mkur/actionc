//! Real runtime qualification of wide record strides and field offsets.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile};
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
impl actionc_vm::VmRunHooks for Hooks {
    type Error = String;
}

#[test]
fn record_elements_preserve_wide_indexes_calls_and_guard_bytes() {
    for stride in [2u16, 3, 20, 255, 256] {
        let padding = if stride == 2 {
            String::new()
        } else {
            format!("BYTE ARRAY pad({})", stride - 2)
        };
        let source = format!(
            "TYPE Element=[CARD value {padding}]\nTYPE Table=[BYTE ARRAY prefix(255) Element ARRAY rows(2)]\nTable POINTER p=$0800\nCARD indexValue=$0802,input=$0804,observed=$0806\nBYTE calls=$0808\nCARD FUNC Index()\ncalls==+1\nRETURN(indexValue)\nPROC Main()\nElement POINTER entry\nentry=@p.rows(Index())\nobserved=entry.value\nentry=@p.rows(indexValue)\nentry.value=input\nRETURN\n"
        );
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                let file = Source(std::env::temp_dir().join(format!(
                    "actionc-record-consumers-{}-{stride}-{runtime:?}-{crlf}.act",
                    std::process::id()
                )));
                std::fs::write(
                    &file.0,
                    if crlf {
                        source.replace('\n', "\r\n")
                    } else {
                        source.clone()
                    },
                )
                .unwrap();
                let compiled = compile_file(
                    &file.0,
                    &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
                )
                .unwrap();
                let entry = symbols::routine_address(&compiled.source_listing(), "Main");
                let mut vm = if runtime == Runtime::ActionCart {
                    machine::load(&compiled)
                } else {
                    let mut vm = CompilerVm::default();
                    let loaded = vm
                        .load_atari_object_for_execution(
                            ExecutionProfile::StandaloneObject,
                            compiled.object_bytes(),
                        )
                        .unwrap();
                    assert!(
                        loaded
                            .segments
                            .iter()
                            .all(|s| s.end < 0x4500 || s.start >= 0x8000)
                    );
                    vm
                };
                for base in [0x5000u16, 0x50F1] {
                    for index in [0u16, 1, 127, 255, 257, 65535] {
                        let field = base
                            .wrapping_add(255)
                            .wrapping_add(index.wrapping_mul(stride));
                        // Preserve the ABI/code/ROM regions in this raw-index fixture.
                        if !(0x4500..0x7FFF).contains(&field) {
                            continue;
                        }
                        let mut expected = vec![0xA5; 0x3B00];
                        expected[usize::from(field - 0x4500)..usize::from(field - 0x4500) + 2]
                            .copy_from_slice(&0xB327u16.to_le_bytes());
                        let ram = vm.bus_mut().ram_mut();
                        ram.map(0x4500, &vec![0xA5; 0x3B00]).unwrap();
                        ram.map(0x800, &base.to_le_bytes()).unwrap();
                        ram.map(0x802, &index.to_le_bytes()).unwrap();
                        ram.map(0x804, &0xB327u16.to_le_bytes()).unwrap();
                        ram.write(0x808, 0);
                        ram.map(field, &0x73D9u16.to_le_bytes()).unwrap();
                        vm = machine::call(vm, &mut Hooks, entry, &[]);
                        let label = format!(
                            "stride={stride}/index={index}/base={base:04X}/{runtime:?}/crlf={crlf}"
                        );
                        assert_eq!(
                            machine::bytes(&vm, 0x806, 2),
                            0x73D9u16.to_le_bytes(),
                            "{label}"
                        );
                        assert_eq!(
                            vm.bus().ram().read(0x808),
                            1,
                            "{label}: single index evaluation"
                        );
                        assert_eq!(
                            machine::bytes(&vm, 0x4500, 0x3B00),
                            expected,
                            "{label}: guards"
                        );
                    }
                }
            }
        }
    }
}
