//! Masked BYTE indexes preserve wide captures and the public result ABI.
use actionc::compiler::Runtime;
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

#[derive(Default)]
struct Reads {
    addresses: Vec<u16>,
}
impl VmRunHooks for Reads {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        let ram = vm.bus().ram();
        let pc = r.pc;
        let address = match ram.read(pc) {
            0xAD => Some(ram.read_word(pc.wrapping_add(1))),
            0xB1 => Some(
                ram.read_word(u16::from(ram.read(pc.wrapping_add(1))))
                    .wrapping_add(u16::from(r.y)),
            ),
            _ => None,
        };
        if let Some(address) = address {
            if (0x50FF..=0x5102).contains(&address) {
                self.addresses.push(address);
            }
        }
        Ok(())
    }
}

#[test]
fn masked_indexes_keep_source_reads_snapshots_and_wide_address_arithmetic() {
    let text = "BYTE ARRAY masks(8)=[1 2 4 8 16 32 64 128]\nBYTE FUNC Mask(CARD value)\nRETURN(masks(BYTE(value&7)))\nBYTE FUNC Snapshot(CARD POINTER cell)\n CARD captured\n captured=cell^ cell^=$FFFF\nRETURN(masks(BYTE(captured&7)))\nBYTE FUNC Wide(BYTE POINTER bits CARD ordinal)\n BYTE POINTER p\n p=bits+(ordinal RSH 3)\nRETURN(p^&masks(BYTE(ordinal&7)))\nPROC Main()\nRETURN\n";
    let values = [
        0u16, 1, 7, 8, 255, 256, 257, 511, 512, 1088, 0x7FFF, 0x8000, 0xFF00, 0xFFFE, 0xFFFF,
    ];
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for enabled in [false, true] {
            let mut lf = None;
            for crlf in [false, true] {
                let file = support::Source::new(&if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text.into()
                });
                let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
                let model = actionc::semantic::analyze_with_options(
                    &loaded.program,
                    actionc::semantic::SemanticOptions::modern(),
                )
                .unwrap();
                let sem = actionc::semantic::ir::lower_program(&loaded.program, &model);
                let nir =
                    actionc::nir::optimize_program(&actionc::nir::lower_program(&sem)).unwrap();
                let mut cfg = Mir6502Config::optimized();
                cfg.enable_dead_register_arithmetic = enabled;
                let output =
                    mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, &cfg, runtime)
                        .unwrap();
                let image = actionc::codegen::format_load_file(&output);
                if let Some(old) = &lf {
                    assert_eq!(&image, old);
                } else {
                    lf = Some(image.clone());
                }
                let entry = |name: &str| {
                    output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address
                };
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                let mut hooks = Reads::default();
                for value in values {
                    vm.bus_mut()
                        .ram_mut()
                        .map(0x9F, &[0xA5, 0x55, 0x5A])
                        .unwrap();
                    let cycles = vm.cpu().cycles();
                    vm = machine::call(vm, &mut hooks, entry("Mask"), &value.to_le_bytes());
                    assert_eq!(vm.bus().ram().read(0xA0), 1u8 << (value & 7));
                    assert_eq!(
                        machine::bytes(&vm, 0x9F, 3),
                        [0xA5, 1u8 << (value & 7), 0x5A]
                    );
                    if value == 1088 && !crlf {
                        println!(
                            "mask-index,{runtime:?},{enabled},{}",
                            vm.cpu().cycles() - cycles
                        );
                    }
                    vm.bus_mut()
                        .ram_mut()
                        .map(0x50FF, &[0xA5, value as u8, (value >> 8) as u8, 0x5A])
                        .unwrap();
                    hooks.addresses.clear();
                    vm = machine::call(vm, &mut hooks, entry("Snapshot"), &0x5100u16.to_le_bytes());
                    assert_eq!(vm.bus().ram().read(0xA0), 1u8 << (value & 7));
                    assert_eq!(machine::bytes(&vm, 0x50FF, 4), [0xA5, 0xFF, 0xFF, 0x5A]);
                    assert_eq!(
                        hooks.addresses,
                        vec![0x5100, 0x5101],
                        "full capture stays ordered"
                    );
                    let base = 0x6000u16;
                    vm.bus_mut()
                        .ram_mut()
                        .map(base + (value >> 3), &[0xA5])
                        .unwrap();
                    let mut args = base.to_le_bytes().to_vec();
                    args.extend(value.to_le_bytes());
                    vm = machine::call(vm, &mut hooks, entry("Wide"), &args);
                    assert_eq!(
                        vm.bus().ram().read(0xA0),
                        0xA5 & (1u8 << (value & 7)),
                        "wide ordinal {value}"
                    );
                }
            }
        }
    }
}
