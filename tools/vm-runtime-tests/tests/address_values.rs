//! Address values, retained source pointer stores, and public result lanes.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile, VmRunHooks};
use std::path::{Path, PathBuf};
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[path = "support/tn_symbols.rs"]
mod symbols;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}
struct Source(PathBuf);
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn named_address(listing: &str, label: &str) -> u16 {
    if let Some(address) = listing
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{label} = $")))
    {
        return u16::from_str_radix(address, 16).unwrap();
    }
    listing
        .lines()
        .skip_while(|line| *line != format!("{label}:"))
        .find_map(|line| {
            line.split_once("; $")
                .and_then(|(_, rest)| rest.split_once(':'))
                .and_then(|(address, _)| u16::from_str_radix(address, 16).ok())
        })
        .unwrap_or_else(|| panic!("missing {label}"))
}

#[test]
fn known_record_addresses_preserve_locals_fields_and_neighbors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text =
        std::fs::read_to_string(root.join("surveys/tn/reproducers/known-record-address.act"))
            .unwrap()
            .replace("\r\n", "\n");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                let source = Source(std::env::temp_dir().join(format!(
                    "actionc-known-address-{}-{mode:?}-{runtime:?}-{crlf}.act",
                    std::process::id()
                )));
                std::fs::write(
                    &source.0,
                    if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.clone()
                    },
                )
                .unwrap();
                let compiled = compile_file(
                    &source.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
                )
                .unwrap();
                let listing = compiled.source_listing();
                let mut vm = if runtime == Runtime::ActionCart {
                    machine::load(&compiled)
                } else {
                    let mut vm = CompilerVm::default();
                    vm.load_atari_object_for_execution(
                        ExecutionProfile::StandaloneObject,
                        compiled.object_bytes(),
                    )
                    .unwrap();
                    vm
                };
                let ordinary = symbols::global_address(&listing, "ordinary");
                for value in [0u16, 1, 0x00FF, 0x0100, 0xFEDC] {
                    let mut guarded = vec![0xA5; 35];
                    guarded[17..19].copy_from_slice(&value.to_le_bytes());
                    vm.bus_mut().ram_mut().map(0x5FF0, &guarded).unwrap();
                    vm.bus_mut()
                        .ram_mut()
                        .map(ordinary, &[0xA5, value as u8, (value >> 8) as u8])
                        .unwrap();
                    for (name, expected) in [
                        ("ReadFixed", 0xA5),
                        ("ReadFixedWord", value),
                        ("ReadLocal", value),
                        ("ReadOrdinary", value),
                        ("FieldAddress", 0x6001),
                    ] {
                        vm = machine::call(
                            vm,
                            &mut Hooks,
                            symbols::routine_address(&listing, name),
                            &[],
                        );
                        if name == "ReadFixed" {
                            assert_eq!(vm.bus().ram().read(0xA0), expected as u8);
                        } else {
                            assert_eq!(
                                vm.bus().ram().read_word(0xA0),
                                expected,
                                "{mode:?}/{runtime:?}/{crlf}/{name}"
                            );
                        }
                        if matches!(name, "ReadLocal" | "ReadOrdinary") {
                            let p = named_address(&listing, &format!("local_{name}_p"));
                            assert_eq!(
                                vm.bus().ram().read_word(p),
                                if name == "ReadLocal" {
                                    0x6000
                                } else {
                                    ordinary
                                }
                            );
                        }
                        assert_eq!(machine::bytes(&vm, 0x5FF0, 35), guarded);
                    }
                    let written = value ^ 0x55AA;
                    guarded[17..19].copy_from_slice(&written.to_le_bytes());
                    vm = machine::call(
                        vm,
                        &mut Hooks,
                        symbols::routine_address(&listing, "WriteLocal"),
                        &written.to_le_bytes(),
                    );
                    assert_eq!(
                        vm.bus()
                            .ram()
                            .read_word(named_address(&listing, "local_WriteLocal_p")),
                        0x6000
                    );
                    assert_eq!(machine::bytes(&vm, 0x5FF0, 35), guarded);
                }
            }
        }
    }
}

#[test]
fn dynamic_local_addresses_preserve_pointer_bytes_and_byte_word_fields() {
    for (stride, prefix, displacement, ty) in [
        (1u16, 38u16, 0u16, "BYTE"),
        (2, 0, 254, "CARD"),
        (20, 38, 255, "CARD"),
        (20, 0, 38, "CARD"),
        (20, 256, 256, "BYTE"),
        (255, 254, 38, "CARD"),
        (256, 255, 0, "CARD"),
    ] {
        let pad = if displacement == 0 {
            String::new()
        } else {
            format!("BYTE ARRAY pad({displacement})")
        };
        let prefix_declaration = if prefix == 0 {
            String::new()
        } else {
            format!("BYTE ARRAY prefix({prefix})")
        };
        let source = format!(
            "TYPE E=[BYTE ARRAY space({stride})]\nTYPE R=[{pad} {ty} value]\nTYPE T=[{prefix_declaration} E ARRAY rows(2)]\nT POINTER base=$0800\nCARD indexValue=$0802,input=$0804,observed=$0806\nBYTE calls=$0808\nCARD FUNC Index() calls==+1 RETURN(indexValue)\nCARD FUNC Read(T POINTER table CARD index) R POINTER item item=R POINTER(@table.rows(index)) RETURN(item.value)\nPROC Write(T POINTER table CARD index) R POINTER item item=R POINTER(@table.rows(index)) item.value=input RETURN\nPROC Main() observed=Read(base,Index()) Write(base,indexValue) RETURN\n"
        );
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                let path = Source(std::env::temp_dir().join(format!(
                    "actionc-address-fields-{}-{stride}-{displacement}-{runtime:?}-{crlf}.act",
                    std::process::id()
                )));
                // Exercise the actual LF and CRLF compiler input paths.
                std::fs::write(
                    &path.0,
                    if crlf {
                        source.replace('\n', "\r\n")
                    } else {
                        source.clone()
                    },
                )
                .unwrap();
                let options = CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime);

                let compiled = compile_file(&path.0, &options).unwrap();
                let listing = compiled.source_listing();
                let entry = symbols::routine_address(&listing, "Main");
                let mut vm = if runtime == Runtime::ActionCart {
                    machine::load(&compiled)
                } else {
                    let mut vm = CompilerVm::default();
                    vm.load_atari_object_for_execution(
                        ExecutionProfile::StandaloneObject,
                        compiled.object_bytes(),
                    )
                    .unwrap();
                    vm
                };
                for base in [0x5000u16, 0x50F1] {
                    for index in [0u16, 1, 127, 255, 256, 257, 65535] {
                        let pointer = base
                            .wrapping_add(prefix)
                            .wrapping_add(index.wrapping_mul(stride));
                        let field = pointer.wrapping_add(displacement);
                        if !(0x4500..0x7FFE).contains(&field) {
                            continue;
                        }
                        let mut expected = vec![0xA5; 0x3B00];
                        let location = usize::from(field - 0x4500);
                        let width = if ty == "BYTE" { 1 } else { 2 };
                        let initial = 0x73D9u16.to_le_bytes();
                        expected[location..location + width].copy_from_slice(&initial[..width]);
                        vm.bus_mut().ram_mut().map(0x4500, &expected).unwrap();
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x800, &base.to_le_bytes())
                            .unwrap();
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x802, &index.to_le_bytes())
                            .unwrap();
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x804, &0xB327u16.to_le_bytes())
                            .unwrap();
                        vm.bus_mut().ram_mut().write(0x808, 0);
                        vm = machine::call(vm, &mut Hooks, entry, &[]);
                        let label = format!(
                            "{stride}/{displacement}/{ty}/{runtime:?}/{crlf}/{base:04X}/{index}"
                        );
                        assert_eq!(
                            vm.bus().ram().read_word(0x806),
                            if width == 1 { 0xD9 } else { 0x73D9 },
                            "{label}"
                        );
                        assert_eq!(
                            vm.bus().ram().read(0x808),
                            1,
                            "{label}: effectful index once"
                        );
                        for routine in ["Read", "Write"] {
                            assert_eq!(
                                vm.bus().ram().read_word(named_address(
                                    &listing,
                                    &format!("local_{routine}_item")
                                )),
                                pointer,
                                "{label}: retained pointer"
                            );
                        }
                        expected[location..location + width]
                            .copy_from_slice(&0xB327u16.to_le_bytes()[..width]);
                        assert_eq!(
                            machine::bytes(&vm, 0x4500, 0x3B00),
                            expected,
                            "{label}: fields and guards"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn pointer_mutations_calls_and_self_aliases_end_address_reuse() {
    let text = r#"
TYPE E=[CARD value BYTE ARRAY pad(18)]
TYPE T=[BYTE ARRAY prefix(38) E ARRAY rows(2)]
CARD POINTER wordAlias
BYTE POINTER byteAlias
PROC Change() wordAlias(0)=$5100 RETURN
CARD FUNC Called(T POINTER table CARD index)
  E POINTER item
  item=@table.rows(index)
  wordAlias=CARD POINTER(@item)
  Change()
RETURN(item.value)
CARD FUNC Aliased(T POINTER table CARD index)
  E POINTER item
  item=@table.rows(index)
  wordAlias=CARD POINTER(@item)
  wordAlias(0)=$5100
RETURN(item.value)
CARD FUNC HighLane(T POINTER table CARD index)
  E POINTER item
  item=@table.rows(index)
  byteAlias=BYTE POINTER(@item)
  byteAlias(1)=$51
RETURN(item.value)
CARD FUNC OwnCell(CARD index)
  E POINTER item
  T POINTER table
  table=T POINTER(CARD(@item)-38)
  item=@table.rows(index)
  item.value=$5100
RETURN(item.value)
PROC Main()
  CARD value
  value=Called($5000,0) value=Aliased($5000,0)
  value=HighLane($5000,0) value=OwnCell(0)
RETURN
"#;
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let path = Source(std::env::temp_dir().join(format!(
            "actionc-address-aliases-{}-{runtime:?}.act",
            std::process::id()
        )));
        std::fs::write(&path.0, text).unwrap();
        let compiled = compile_file(
            &path.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        let listing = compiled.source_listing();
        let mut vm = if runtime == Runtime::ActionCart {
            machine::load(&compiled)
        } else {
            let mut vm = CompilerVm::default();
            vm.load_atari_object_for_execution(
                ExecutionProfile::StandaloneObject,
                compiled.object_bytes(),
            )
            .unwrap();
            vm
        };
        vm.bus_mut()
            .ram_mut()
            .map(0x5026, &0x1234u16.to_le_bytes())
            .unwrap();
        vm.bus_mut()
            .ram_mut()
            .map(0x5100, &0xABCDu16.to_le_bytes())
            .unwrap();
        vm.bus_mut()
            .ram_mut()
            .map(0x5126, &0x5678u16.to_le_bytes())
            .unwrap();
        for (name, expected, pointer) in [
            ("Called", 0xABCD, 0x5100),
            ("Aliased", 0xABCD, 0x5100),
            ("HighLane", 0x5678, 0x5126),
            ("OwnCell", 0xABCD, 0x5100),
        ] {
            let args: &[u8] = if name == "OwnCell" {
                &[0, 0]
            } else {
                &[0, 0x50, 0, 0]
            };
            vm = machine::call(
                vm,
                &mut Hooks,
                symbols::routine_address(&listing, name),
                args,
            );
            assert_eq!(
                vm.bus().ram().read_word(0xA0),
                expected,
                "{runtime:?}/{name}"
            );
            assert_eq!(
                vm.bus()
                    .ram()
                    .read_word(named_address(&listing, &format!("local_{name}_item"))),
                pointer,
                "{runtime:?}/{name}: mutated pointer bytes"
            );
        }
    }
}
