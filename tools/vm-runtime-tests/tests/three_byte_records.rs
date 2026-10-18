//! Full BYTE range, captured inputs and aliases for three-byte record addresses.
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
struct Accesses(Vec<(bool, u16)>);
impl VmRunHooks for Accesses {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        let ram = vm.bus().ram();
        let opcode = ram.read(r.pc);
        let operand = r.pc.wrapping_add(1);
        let address = match opcode {
            0xB1 | 0x91 => Some(
                ram.read_word(u16::from(ram.read(operand)))
                    .wrapping_add(u16::from(r.y)),
            ),
            0xAD | 0xAE | 0xAC | 0x8D | 0x8E | 0x8C => Some(ram.read_word(operand)),
            0xBD | 0x9D => Some(ram.read_word(operand).wrapping_add(u16::from(r.x))),
            0xB9 | 0x99 => Some(ram.read_word(operand).wrapping_add(u16::from(r.y))),
            _ => None,
        };
        if let Some(address) = address.filter(|a| (0x4F00..0x5500).contains(a)) {
            self.0.push((
                matches!(opcode, 0x91 | 0x8D | 0x8E | 0x8C | 0x9D | 0x99),
                address,
            ));
        }
        Ok(())
    }
}

#[test]
fn three_byte_records_keep_full_indexes_access_order_and_aliases() {
    let text = "TYPE Entry=[BYTE tag CARD length]
Entry ARRAY entries(16)=$5000,crossing(16)=$50FF
BYTE indexValue=$0800,inputTag=$0801,calls=$0806,aliasIndex=$5000
CARD inputLength=$0802,observed=$0804
BYTE FUNC Index()
 calls==+1
RETURN(indexValue)
PROC StoreRead()
 entries(Index()).tag=inputTag
 entries(indexValue).length=inputLength
 observed=entries(indexValue).length
RETURN
PROC PageCrossing()
 crossing(Index()).tag=inputTag
 crossing(indexValue).length=inputLength
 observed=crossing(indexValue).length
RETURN
PROC Alias()
 entries(aliasIndex).tag=inputTag
 entries(aliasIndex).length=inputLength
RETURN
PROC Wide(CARD index)
 entries(index).length=inputLength
 observed=entries(index).length
RETURN
PROC Local(BYTE index)
 Entry ARRAY owned(16)
 owned(index).tag=inputTag
 owned(index).length=inputLength
 observed=owned(index).length
RETURN
PROC Main()
RETURN
";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for raw_nir in [false, true] {
            for optimized in [false, true] {
                let mut lf_image = None;
                for crlf in [false, true] {
                    let file = support::Source::new(&if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.into()
                    });
                    let loaded =
                        actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
                    let model = actionc::semantic::analyze_with_options(
                        &loaded.program,
                        actionc::semantic::SemanticOptions::modern(),
                    )
                    .unwrap();
                    let sem = actionc::semantic::ir::lower_program(&loaded.program, &model);
                    let raw = actionc::nir::lower_program(&sem);
                    let nir = if raw_nir {
                        raw
                    } else {
                        actionc::nir::optimize_program(&raw).unwrap()
                    };
                    let config = if optimized {
                        Mir6502Config::optimized()
                    } else {
                        Mir6502Config::default()
                    };
                    let output = mir6502::generate_output_with_config_and_runtime(
                        &nir, 0x3000, &config, runtime,
                    )
                    .unwrap();
                    let image = actionc::codegen::format_load_file(&output);
                    if let Some(lf) = &lf_image {
                        assert_eq!(&image, lf);
                    } else {
                        lf_image = Some(image.clone());
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
                    for (name, base) in [("StoreRead", 0x5000u16), ("PageCrossing", 0x50FF)] {
                        // Declared capacity 16 deliberately does not bound raw indexing.
                        for index in 0u16..=255 {
                            let field = base + 3 * index;
                            let mut expected = vec![0xA5; 0x600];
                            let at = usize::from(field - 0x4F00);
                            expected[at..at + 3].copy_from_slice(&[0x37, 0x27, 0xB3]);
                            let ram = vm.bus_mut().ram_mut();
                            ram.map(0x4F00, &vec![0xA5; 0x600]).unwrap();
                            ram.map(0x800, &[index as u8, 0x37, 0x27, 0xB3, 0, 0, 0])
                                .unwrap();
                            let mut accesses = Accesses::default();
                            vm = machine::call(vm, &mut accesses, entry(name), &[]);
                            let label = format!(
                                "{name}/{index}/{runtime:?}/raw={raw_nir}/opt={optimized}/crlf={crlf}"
                            );
                            assert_eq!(
                                machine::bytes(&vm, 0x804, 3),
                                [0x27, 0xB3, 1],
                                "{label}: result and one index call"
                            );
                            assert_eq!(
                                machine::bytes(&vm, 0x4F00, 0x600),
                                expected,
                                "{label}: guards"
                            );
                            assert_eq!(
                                accesses.0,
                                [
                                    (true, field),
                                    (true, field + 1),
                                    (true, field + 2),
                                    (false, field + 1),
                                    (false, field + 2)
                                ],
                                "{label}: access order"
                            );
                        }
                    }
                    for index in [256u16, 257, 0xFFFF] {
                        let field = 0x5000u16
                            .wrapping_add(3u16.wrapping_mul(index))
                            .wrapping_add(1);
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x802, &[0x27, 0xB3, 0, 0])
                            .unwrap();
                        vm = machine::call(
                            vm,
                            &mut Accesses::default(),
                            entry("Wide"),
                            &index.to_le_bytes(),
                        );
                        assert_eq!(machine::bytes(&vm, field, 2), [0x27, 0xB3]);
                        assert_eq!(machine::bytes(&vm, 0x804, 2), [0x27, 0xB3]);
                    }
                    // The first store changes the index for the second store.
                    vm.bus_mut().ram_mut().write(0x5000, 0);
                    vm.bus_mut()
                        .ram_mut()
                        .map(0x801, &[255, 0xD9, 0x73])
                        .unwrap();
                    let mut accesses = Accesses::default();
                    vm = machine::call(vm, &mut accesses, entry("Alias"), &[]);
                    assert_eq!(
                        accesses.0,
                        [
                            (false, 0x5000),
                            (true, 0x5000),
                            (false, 0x5000),
                            (true, 0x52FE),
                            (true, 0x52FF)
                        ]
                    );
                    assert_eq!(machine::bytes(&vm, 0x52FE, 2), [0xD9, 0x73]);
                    // Compiler-placed local storage exercises relocation constants.
                    for index in [0, 15] {
                        vm.bus_mut().ram_mut().map(0x804, &[0, 0]).unwrap();
                        vm = machine::call(vm, &mut Accesses::default(), entry("Local"), &[index]);
                        assert_eq!(machine::bytes(&vm, 0x804, 2), [0xD9, 0x73]);
                    }
                }
            }
        }
    }
}
