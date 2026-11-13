//! Complete-image and interface checks for the opt-in TN size audit.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, VmRunHooks};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn build() -> PathBuf {
    std::env::var_os("ACTIONC_TN_SIZE_BUILD").unwrap().into()
}
#[allow(dead_code)]
#[path = "../../vm-runtime-tests/tests/support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "../../vm-runtime-tests/tests/support/fixed_point.rs"]
mod support;
#[path = "../../vm-runtime-tests/tests/support/tn_symbols.rs"]
mod symbols;
use symbols::{global_address, routine_address};

fn loaded(image: &[u8], runtime: Runtime) -> CompilerVm {
    let (mut vm, profile) = support::vm_for(runtime);
    vm.load_atari_object_for_execution(profile, image).unwrap();
    vm.bus_mut().ram_mut().write(machine::RETURN, 0x60);
    vm
}
fn ranges(path: &Path) {
    let loaded = actionc::includes::load_program_with_expanded_source(path).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
        &loaded.program,
        &model,
    ));
    for nir in [&raw, &actionc::nir::optimize_program(&raw).unwrap()] {
        let facts = actionc::nir::analyze_unsigned_ranges(nir).unwrap();
        for name in ["ArenaCursor", "Alloc"] {
            let r = nir.routines.iter().find(|r| r.name == name).unwrap();
            let v = facts.normal_return_range(r.id).unwrap();
            assert_eq!((v.min(), v.max()), (0x2000, 0x2800));
        }
    }
}
fn image_memory(image: &[u8]) -> (Vec<u8>, u16) {
    let mut ram = vec![0; 65536];
    let mut cursor = 0;
    let mut end_main = 0;
    while cursor < image.len() {
        let start = u16::from_le_bytes(image[cursor..cursor + 2].try_into().unwrap());
        cursor += 2;
        if start == 0xffff {
            continue;
        }
        let end = u16::from_le_bytes(image[cursor..cursor + 2].try_into().unwrap());
        cursor += 2;
        let n = usize::from(end - start) + 1;
        ram[usize::from(start)..=usize::from(end)].copy_from_slice(&image[cursor..cursor + n]);
        cursor += n;
        if start >= 0x2c00 {
            end_main = end_main.max(end + 1);
        }
    }
    (ram, end_main)
}
fn buffer_home(listing: &str) -> u16 {
    let mut lines = listing.lines().skip_while(|l| *l != "global_buffer:");
    lines
        .find_map(|l| {
            l.split_once("; $")
                .and_then(|(_, t)| t.split_once(':'))
                .and_then(|(a, _)| u16::from_str_radix(a, 16).ok())
        })
        .unwrap()
}

#[test]
fn compile_images() {
    let mut rows = Vec::new();
    for variant in ["before", "after"] {
        for program in ["TN", "TNDBG"] {
            let source = build().join(variant).join(format!("{program}.ACT"));
            ranges(&source);
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    eprintln!("SIZE {variant}/{program}/{mode:?}/{runtime:?}");
                    let o = compile_file(
                        &source,
                        &CompileOptions::for_mode(mode).with_runtime(runtime),
                    )
                    .unwrap();
                    let stem = build()
                        .join(variant)
                        .join(format!("{program}-{mode:?}-{runtime:?}"));
                    let l = o.source_listing().replace("\r\n", "\n");
                    std::fs::write(stem.with_extension("xex"), o.object_bytes()).unwrap();
                    std::fs::write(stem.with_extension("asm"), &l).unwrap();
                    let routines = l
                        .lines()
                        .filter_map(|l| {
                            let t = l.strip_prefix("; ===== PROC ")?;
                            let (n, t) = t.split_once(" $")?;
                            let (a, b) = t.split_once("..$")?;
                            Some((
                                n.to_string(),
                                u16::from_str_radix(b.split_whitespace().next()?, 16).ok()?
                                    - u16::from_str_radix(a, 16).ok()?,
                            ))
                        })
                        .collect::<BTreeMap<_, _>>();
                    let (ram, end) = image_memory(o.object_bytes());
                    let home = usize::from(buffer_home(&l));
                    let buffer = u16::from_le_bytes(ram[home..home + 2].try_into().unwrap());
                    assert!(buffer >= end && buffer < 0xa000);
                    rows.push(serde_json::json!({"variant":variant,"program":program,"mode":format!("{mode:?}"),"runtime":format!("{runtime:?}"),"xex_bytes":o.object_bytes().len(),"copy_buffer":buffer,"reserved_bytes":buffer-0x2c00,"routine_bytes":routines}));
                }
            }
        }
    }
    std::fs::write(
        build().join("images.json"),
        serde_json::to_string_pretty(&rows).unwrap() + "\n",
    )
    .unwrap();
}

struct Hooks {
    cio: u16,
    expected: Option<(u8, u16, u16, u8)>,
    handlers: BTreeMap<u16, usize>,
    dispatched: Vec<usize>,
}
impl VmRunHooks for Hooks {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.cio {
            let (ch, buf, len, cmd) = self.expected.take().expect("unexpected CIO");
            assert_eq!((r.a, r.x, r.y), (cmd, ch << 4, (buf >> 8) as u8));
            assert_eq!(r.status & 0x83, 0, "C/N/Z at CIO entry");
            let base = 0x0340 + u16::from(ch) * 16;
            for i in 0..16u16 {
                let expected = match i {
                    2 => cmd,
                    4 => buf as u8,
                    5 => (buf >> 8) as u8,
                    8 => len as u8,
                    9 => (len >> 8) as u8,
                    _ => 0xCC,
                };
                assert_eq!(vm.bus().ram().read(base + i), expected, "ICB offset {i}");
            }
            vm.set_pc(machine::RETURN);
        } else if let Some(&command) = self.handlers.get(&r.pc) {
            self.dispatched.push(command);
            vm.set_pc(machine::RETURN);
        }
        Ok(())
    }
}

#[test]
fn check_machine_interfaces() {
    let (mut io_calls, mut dispatch_calls, mut decimal_calls, mut hex_calls) = (0, 0, 0, 0);
    for variant in ["before", "after"] {
        for program in ["TN", "TNDBG"] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let stem = build()
                        .join(variant)
                        .join(format!("{program}-{mode:?}-{runtime:?}"));
                    let image = std::fs::read(stem.with_extension("xex")).unwrap();
                    let listing = std::fs::read_to_string(stem.with_extension("asm"))
                        .unwrap()
                        .replace("\r\n", "\n");
                    let routine = |n| routine_address(&listing, n);
                    let mut hooks = Hooks {
                        cio: routine("_Cio"),
                        expected: None,
                        handlers: BTreeMap::new(),
                        dispatched: Vec::new(),
                    };
                    let mut vm = loaded(&image, runtime);
                    for (name, cmd) in [("Bget", 7), ("Bput", 11)] {
                        for ch in 0..8u8 {
                            for buf in [0u16, 0xFF, 0x7000, 0xFFFF] {
                                for len in [0u16, 1, 0xFF, 0x100, 0xFFFF] {
                                    vm.bus_mut().ram_mut().map(0x0340, &[0xCC; 128]).unwrap();
                                    hooks.expected = Some((ch, buf, len, cmd));
                                    vm = machine::call(
                                        vm,
                                        &mut hooks,
                                        routine(name),
                                        &[
                                            ch,
                                            buf as u8,
                                            (buf >> 8) as u8,
                                            len as u8,
                                            (len >> 8) as u8,
                                        ],
                                    );
                                    assert!(hooks.expected.is_none());
                                    io_calls += 1;
                                }
                            }
                        }
                    }
                    let mut names = vec![
                        "View", "Attrib", "Delete", "Rename", "TagAll", "Copy", "MkDir", "Format",
                    ];
                    if program == "TNDBG" {
                        names.push("Dbg");
                    }
                    names.push("Quit");
                    hooks.handlers = names
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (routine(n), i + 1))
                        .collect();
                    for command in 0..=255u8 {
                        hooks.dispatched.clear();
                        vm = machine::call(vm, &mut hooks, routine("MakeJmp"), &[command]);
                        let expected = if command > 0 && usize::from(command) <= names.len() {
                            vec![usize::from(command)]
                        } else {
                            vec![]
                        };
                        assert_eq!(
                            hooks.dispatched, expected,
                            "{variant}/{program}/{mode:?}/{runtime:?}/{command}"
                        );
                        dispatch_calls += 1;
                    }
                    hooks.handlers.clear();
                    vm = machine::call(vm, &mut hooks, routine("Init"), &[]);
                    vm.bus_mut().ram_mut().write_word(0x58, 0x9000);
                    let internal =
                        |c: u8| (c & 0x9F) | [0x40, 0, 0x20, 0x60][usize::from((c >> 5) & 3)];
                    for value in 0..=255u16 {
                        vm.bus_mut().ram_mut().map(0x9000, &[0xA6; 6]).unwrap();
                        vm = machine::call(vm, &mut hooks, routine("Position"), &[0, 0]);
                        vm = machine::call(vm, &mut hooks, routine("PrintB"), &[value as u8]);
                        // Preserve the existing suppression of a zero tens digit,
                        // even with a nonzero hundreds digit (100 prints "10").
                        let mut expected = Vec::new();
                        for digit in [value / 100, value % 100 / 10] {
                            if digit != 0 {
                                expected.push(internal(b'0' + digit as u8));
                            }
                        }
                        expected.push(internal(b'0' + (value % 10) as u8));
                        let len = expected.len();
                        assert_eq!(machine::bytes(&vm, 0x9000, len), expected);
                        assert_eq!(vm.bus().ram().read(0x9000 + len as u16), 0xA6);
                        assert_eq!(vm.bus().ram().read(0xFF), len as u8);
                        assert_eq!(vm.bus().ram().read(0xE0), (value % 100 / 10) as u8);
                        decimal_calls += 1;
                    }
                    if program == "TNDBG" {
                        for value in 0..=255u16 {
                            vm.bus_mut().ram_mut().map(0x9000, &[0xA6; 6]).unwrap();
                            vm = machine::call(vm, &mut hooks, routine("Position"), &[0, 0]);
                            vm =
                                machine::call(vm, &mut hooks, routine("PrintHexB"), &[value as u8]);
                            let expected = format!("{value:02X}")
                                .bytes()
                                .map(internal)
                                .collect::<Vec<_>>();
                            assert_eq!(machine::bytes(&vm, 0x9000, 2), expected);
                            assert_eq!(vm.bus().ram().read(0x9002), 0xA6);
                            assert_eq!(vm.bus().ram().read(0xFF), 2);
                            let h = global_address(&listing, "h");
                            assert_eq!(
                                vm.bus().ram().read(h),
                                format!("{value:02X}").as_bytes()[1]
                            );
                            hex_calls += 1;
                        }
                        for value in [0u16, 1, 0xFF, 0x100, 0x7FFF, 0x8000, 0xFFFF] {
                            vm.bus_mut().ram_mut().map(0x9000, &[0xA6; 6]).unwrap();
                            vm = machine::call(vm, &mut hooks, routine("Position"), &[0, 0]);
                            vm = machine::call(
                                vm,
                                &mut hooks,
                                routine("PrintHexC"),
                                &value.to_le_bytes(),
                            );
                            assert_eq!(
                                machine::bytes(&vm, 0x9000, 4),
                                format!("{value:04X}")
                                    .bytes()
                                    .map(internal)
                                    .collect::<Vec<_>>()
                            );
                            assert_eq!(vm.bus().ram().read(0x9004), 0xA6);
                            hex_calls += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(
        (io_calls, dispatch_calls, decimal_calls, hex_calls),
        (5120, 4096, 4096, 2104)
    );
    eprintln!(
        "Interfaces: {io_calls} CIO, {dispatch_calls} dispatch, {decimal_calls} decimal, {hex_calls} hex calls"
    );
}
