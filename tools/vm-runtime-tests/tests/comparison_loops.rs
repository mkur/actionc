//! Host-oracle comparisons and measurements of generic record loops.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{CompilerVm, CpuStep, VmRunHooks};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
#[path = "support/tn_symbols.rs"]
mod symbols;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile(
    text: &str,
    config: &Mir6502Config,
    runtime: Runtime,
) -> actionc::codegen::CodegenOutput {
    let source = support::Source::new(text);
    let loaded = actionc::includes::load_program_with_expanded_source(&source.0).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let sem = actionc::semantic::ir::lower_program(&loaded.program, &model);
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(&sem)).unwrap();
    let mir = mir6502::lower_program(&nir).unwrap();
    if let Some(dir) = std::env::var_os("ACTIONC_COMPARISON_DUMP") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("typed-mir.txt"), mir6502::format_program(&mir)).unwrap();
        let selected =
            mir6502::materialize_program_with_origin_and_runtime(mir, config, 0x3000, runtime)
                .unwrap();
        std::fs::write(
            dir.join(format!("selected-{runtime:?}.txt")),
            mir6502::format_program(&selected),
        )
        .unwrap();
    }
    mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, config, runtime).unwrap()
}

#[derive(Default)]
struct Reads {
    records: Vec<(u16, u16)>,
    sequence: Vec<u16>,
    pair_writes: usize,
    indirect_compares: usize,
    y_advances: usize,
    steps: usize,
}
impl VmRunHooks for Reads {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        self.steps += 1;
        let r = vm.cpu().registers();
        let ram = vm.bus().ram();
        let op = ram.read(r.pc);
        let arg = ram.read(r.pc.wrapping_add(1));
        if op == 0xC8 {
            self.y_advances += 1;
        }
        if matches!(op, 0xB1 | 0xD1) {
            let address = u16::from_le_bytes([
                ram.read(u16::from(arg)),
                ram.read(u16::from(arg.wrapping_add(1))),
            ])
            .wrapping_add(u16::from(r.y));
            if self
                .records
                .iter()
                .any(|&(base, len)| address.wrapping_sub(base) < len)
            {
                self.sequence.push(address);
                if op == 0xD1 {
                    self.indirect_compares += 1;
                }
            }
        }
        if matches!(op, 0x85 | 0x84 | 0x86) && (0xAA..=0xAD).contains(&arg) {
            self.pair_writes += 1;
        }
        Ok(())
    }
    fn after_step(&mut self, _: &CompilerVm, _: &CpuStep) -> Result<(), String> {
        Ok(())
    }
}

fn run(text: &str, config: &Mir6502Config, runtime: Runtime, crlf: bool) {
    let output = compile(text, config, runtime);
    let (mut vm, profile) = support::vm_for(runtime);
    vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output))
        .unwrap();
    let mut initial = vec![0xA5; 520];
    for n in 0..255 {
        initial[1 + n] = ((n * 37 + 127) & 255) as u8;
    }
    let cases = [
        ("After", 6usize, 14usize),
        ("Equal", 6, 11),
        ("Observed", 6, 11),
        ("WideAfter", 239, 16),
    ];
    for (name, offset, len) in cases {
        let entry = output
            .routine_addresses
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(name))
            .unwrap()
            .address;
        for (left, right) in [(0x5001u16, 0x5201u16), (0x50F9, 0x52FE), (0x5001, 0x5001)] {
            for mismatch in 0..=len {
                for (a, b) in [(0u8, 255u8), (127, 128), (128, 127), (255, 0)] {
                    let mut lhs = initial.clone();
                    let mut rhs = initial.clone();
                    if mismatch < len && left != right {
                        lhs[1 + offset + mismatch] = a;
                        rhs[1 + offset + mismatch] = b;
                    }
                    vm.bus_mut().ram_mut().map(left - 1, &lhs[..257]).unwrap();
                    vm.bus_mut().ram_mut().map(right - 1, &rhs[..257]).unwrap();
                    let expected = if name == "Equal" {
                        u8::from(
                            left == right
                                || lhs[1 + offset..1 + offset + len]
                                    == rhs[1 + offset..1 + offset + len],
                        )
                    } else {
                        u8::from(
                            left != right
                                && lhs[1 + offset..1 + offset + len]
                                    > rhs[1 + offset..1 + offset + len],
                        )
                    };
                    let args: Vec<_> = [left, right]
                        .into_iter()
                        .flat_map(u16::to_le_bytes)
                        .collect();
                    let mut reads = Reads {
                        records: vec![(left, 255), (right, 255)],
                        ..Reads::default()
                    };
                    let cycles = vm.cpu().cycles();
                    vm = machine::call(vm, &mut reads, entry, &args);
                    assert_eq!(
                        vm.bus().ram().read(0xA0),
                        expected,
                        "{runtime:?}/{crlf}/{name}/{left:04X}/{right:04X}/{mismatch}/{a}/{b}"
                    );
                    assert_eq!(machine::bytes(&vm, left - 1, 257), lhs[..257]);
                    assert_eq!(machine::bytes(&vm, right - 1, 257), rhs[..257]);
                    let visited = if mismatch < len && left != right {
                        mismatch + 1
                    } else {
                        len
                    };
                    let expected_reads: Vec<_> = (0..visited)
                        .flat_map(|n| {
                            [
                                left + offset as u16 + n as u16,
                                right + offset as u16 + n as u16,
                            ]
                        })
                        .collect();
                    assert_eq!(
                        reads.sequence, expected_reads,
                        "{name}: read order and multiplicity"
                    );
                    if config.enable_comparison_loop_y
                        && matches!(name, "After" | "Equal" | "WideAfter")
                    {
                        assert_eq!(
                            reads.y_advances,
                            if mismatch < len && left != right {
                                visited - 1
                            } else {
                                visited
                            },
                            "Y carrier selected for {name}"
                        );
                    }
                    if config.enable_comparison_loop_fusion
                        && matches!(name, "After" | "Equal" | "WideAfter")
                    {
                        assert_eq!(
                            reads.indirect_compares, visited,
                            "fused comparison selected for {name}"
                        );
                    }
                    if config.enable_comparison_loop_pointers && name == "Equal" {
                        assert!(
                            reads.pair_writes <= 4,
                            "retained bases: {} writes",
                            reads.pair_writes
                        );
                    }
                    if name == "Observed" {
                        let n = if mismatch < len && left != right {
                            mismatch
                        } else {
                            len - 1
                        };
                        assert_eq!(
                            vm.bus().ram().read(
                                output
                                    .map
                                    .storage_symbols
                                    .iter()
                                    .find(|s| s.name.eq_ignore_ascii_case("savedLeft"))
                                    .unwrap()
                                    .address
                            ),
                            lhs[1 + offset + n]
                        );
                        assert_eq!(
                            vm.bus().ram().read(
                                output
                                    .map
                                    .storage_symbols
                                    .iter()
                                    .find(|s| s.name.eq_ignore_ascii_case("savedRight"))
                                    .unwrap()
                                    .address
                            ),
                            rhs[1 + offset + n]
                        );
                        assert_eq!(
                            vm.bus().ram().read(
                                output
                                    .map
                                    .storage_symbols
                                    .iter()
                                    .find(|s| s.name.eq_ignore_ascii_case("savedIndex"))
                                    .unwrap()
                                    .address
                            ),
                            if mismatch < len && left != right {
                                mismatch as u8
                            } else {
                                len as u8
                            }
                        );
                    }
                    if left == 0x5001
                        && right == 0x5201
                        && a == 0
                        && [0, len / 2, len - 1, len].contains(&mismatch)
                        && !crlf
                    {
                        println!(
                            "comparison,{runtime:?},{name},{mismatch},{},{},{}",
                            vm.cpu().cycles() - cycles,
                            reads.pair_writes,
                            reads.steps
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn record_comparison_loops_preserve_values_read_order_and_guards() {
    let text = std::fs::read_to_string(root().join("surveys/tn/reproducers/comparison-loops.act"))
        .unwrap()
        .replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for crlf in [false, true] {
            for (pointers, fusion, carry_y) in [
                (false, false, false),
                (true, false, false),
                (true, true, false),
                (true, true, true),
            ] {
                let mut config = Mir6502Config::optimized();
                config.enable_comparison_loop_pointers = pointers;
                config.enable_comparison_loop_fusion = fusion;
                config.enable_comparison_loop_y = carry_y;
                println!("variant pointers={pointers},fusion={fusion},y={carry_y}");
                run(
                    &if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.clone()
                    },
                    &config,
                    runtime,
                    crlf,
                );
            }
        }
    }
}

#[test]
fn classic_record_comparisons_remain_an_independent_source_control() {
    let text = std::fs::read_to_string(root().join("surveys/tn/reproducers/comparison-loops.act"))
        .unwrap()
        .replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for crlf in [false, true] {
            let source = support::Source::new(&if crlf {
                text.replace('\n', "\r\n")
            } else {
                text.clone()
            });
            let compiled = compile_file(
                &source.0,
                &CompileOptions::for_mode(CompileMode::Optimized).with_runtime(runtime),
            )
            .unwrap();
            let listing = compiled.source_listing();
            let (mut vm, profile) = support::vm_for(runtime);
            vm.load_atari_object_for_execution(profile, compiled.object_bytes())
                .unwrap();
            vm.bus_mut().ram_mut().map(0x5000, &[0; 256]).unwrap();
            vm.bus_mut().ram_mut().map(0x5200, &[0; 256]).unwrap();
            for (left, right, want) in [
                (0u8, 255u8, 0u8),
                (255, 0, 1),
                (128, 127, 1),
                (127, 128, 0),
                (255, 255, 0),
            ] {
                vm.bus_mut().ram_mut().write(0x5006, left);
                vm.bus_mut().ram_mut().write(0x5206, right);
                vm = machine::call(
                    vm,
                    &mut Reads::default(),
                    symbols::routine_address(&listing, "After"),
                    &[0x00, 0x50, 0x00, 0x52],
                );
                assert_eq!(vm.bus().ram().read(0xA0), want);
            }
        }
    }
}

#[test]
fn comparison_boundaries_preserve_wrap_overlap_zero_trips_and_exposed_captures() {
    let source = r#"
TYPE Probe=[BYTE ARRAY prefix(6),name(255)]
CARD exposed
BYTE FUNC Order(Probe POINTER left,right)
  BYTE n,a,b
  FOR n=0 TO 10 DO
    a=left.name(n) b=right.name(n)
    IF a#b THEN RETURN(a>b) FI
  OD
RETURN(0)
BYTE FUNC Broad(Probe POINTER left,right)
  BYTE n,a,b
  FOR n=0 TO 254 DO
    a=left.name(n) b=right.name(n)
    IF a#b THEN RETURN(a>b) FI
  OD
RETURN(0)
BYTE FUNC Empty(Probe POINTER left,right)
  BYTE n,a,b
  FOR n=1 TO 0 DO
    a=left.name(n) b=right.name(n)
    IF a#b THEN RETURN(a>b) FI
  OD
RETURN(0)
BYTE FUNC Escaped(Probe POINTER left,right)
  BYTE n,a,b
  exposed=@a
  FOR n=0 TO 0 DO
    a=left.name(n) b=right.name(n)
    IF a#b THEN RETURN(a>b) FI
  OD
RETURN(0)
PROC Start()
RETURN
"#;
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for crlf in [false, true] {
            for op in [">", "<", ">=", "<="] {
                let source = source.replace("a>b", &format!("a{op}b"));
                let source = if crlf {
                    source.replace('\n', "\r\n")
                } else {
                    source
                };
                for enabled in [false, true] {
                    let mut config = Mir6502Config::optimized();
                    config.enable_comparison_loop_pointers = enabled;
                    config.enable_comparison_loop_fusion = enabled;
                    config.enable_comparison_loop_y = enabled;
                    let output = compile(&source, &config, runtime);
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(
                        profile,
                        &actionc::codegen::format_load_file(&output),
                    )
                    .unwrap();
                    let entry = |name: &str| {
                        output
                            .routine_addresses
                            .iter()
                            .find(|r| r.name.eq_ignore_ascii_case(name))
                            .unwrap()
                            .address
                    };
                    for (name, len, left, right) in [
                        ("Order", 11usize, 0x5000u16, 0x5200u16),
                        ("Order", 11, 0x5000, 0x5003),
                        ("Broad", 255, 0x5000, 0x5200),
                    ] {
                        for mismatch in [0, len - 1, len] {
                            for (a, b) in [(0u8, 255u8), (127, 128), (128, 127), (255, 0)] {
                                vm.bus_mut().ram_mut().map(0x4FFF, &[0x80; 1024]).unwrap();
                                if mismatch < len {
                                    vm.bus_mut().ram_mut().write(left + 6 + mismatch as u16, a);
                                    vm.bus_mut().ram_mut().write(right + 6 + mismatch as u16, b);
                                }
                                let lhs = machine::bytes(&vm, left + 6, len);
                                let rhs = machine::bytes(&vm, right + 6, len);
                                let snapshot = machine::bytes(&vm, 0x4FFF, 1024);
                                let first = lhs.iter().zip(&rhs).position(|(a, b)| a != b);
                                let visited = first.map_or(len, |n| n + 1);
                                let expected = first.is_some_and(|n| {
                                    if matches!(op, ">" | ">=") {
                                        lhs[n] > rhs[n]
                                    } else {
                                        lhs[n] < rhs[n]
                                    }
                                });
                                let args = [left, right]
                                    .into_iter()
                                    .flat_map(u16::to_le_bytes)
                                    .collect::<Vec<_>>();
                                let mut reads = Reads {
                                    records: vec![(left, 261), (right, 261)],
                                    ..Reads::default()
                                };
                                vm = machine::call(vm, &mut reads, entry(name), &args);
                                assert_eq!(
                                    vm.bus().ram().read(0xA0),
                                    u8::from(expected),
                                    "{name}/{op}/{enabled}/{crlf}"
                                );
                                assert_eq!(machine::bytes(&vm, 0x4FFF, 1024), snapshot);
                                assert_eq!(
                                    reads.sequence,
                                    (0..visited)
                                        .flat_map(|n| [left + 6 + n as u16, right + 6 + n as u16])
                                        .collect::<Vec<_>>()
                                );
                                if enabled {
                                    assert_eq!(
                                        reads.indirect_compares, visited,
                                        "selected {name}/{op}"
                                    );
                                }
                            }
                        }
                    }
                    let args = [0x5000u16, 0x5200]
                        .into_iter()
                        .flat_map(u16::to_le_bytes)
                        .collect::<Vec<_>>();
                    let mut reads = Reads {
                        records: vec![(0x5000, 261), (0x5200, 261)],
                        ..Reads::default()
                    };
                    vm = machine::call(vm, &mut reads, entry("Empty"), &args);
                    assert_eq!(vm.bus().ram().read(0xA0), 0);
                    assert!(reads.sequence.is_empty());
                    vm.bus_mut().ram_mut().write(0x5006, 0x80);
                    vm.bus_mut().ram_mut().write(0x5206, 0x80);
                    vm = machine::call(vm, &mut Reads::default(), entry("Escaped"), &args);
                    let public = output
                        .map
                        .storage_symbols
                        .iter()
                        .find(|s| s.name.eq_ignore_ascii_case("exposed"))
                        .unwrap()
                        .address;
                    let home = u16::from_le_bytes([
                        vm.bus().ram().read(public),
                        vm.bus().ram().read(public + 1),
                    ]);
                    let left = if matches!(op, ">" | ">=") { 255 } else { 0 };
                    vm.bus_mut().ram_mut().write(0x5006, left);
                    vm.bus_mut().ram_mut().write(home, 255 - left);
                    let alias = [0x5000u16, home.wrapping_sub(6)]
                        .into_iter()
                        .flat_map(u16::to_le_bytes)
                        .collect::<Vec<_>>();
                    vm = machine::call(vm, &mut Reads::default(), entry("Escaped"), &alias);
                    assert_eq!(
                        vm.bus().ram().read(0xA0),
                        0,
                        "capture must precede aliased right read"
                    );
                    assert_eq!(vm.bus().ram().read(home), left);
                    if runtime == Runtime::Standalone {
                        // Only the eleven observed bytes and their guards are
                        // mapped; the wrapped window avoids ABI/pointer scratch.
                        let left = 0xFFF8u16;
                        let right = 0x5000u16;
                        for n in 0..13u16 {
                            vm.bus_mut().ram_mut().write(left.wrapping_add(5 + n), 0x80);
                            vm.bus_mut().ram_mut().write(right + 5 + n, 0x80);
                        }
                        let mut reads = Reads {
                            records: vec![(left, 261), (right, 261)],
                            ..Reads::default()
                        };
                        let args = [left, right]
                            .into_iter()
                            .flat_map(u16::to_le_bytes)
                            .collect::<Vec<_>>();
                        vm = machine::call(vm, &mut reads, entry("Order"), &args);
                        assert_eq!(vm.bus().ram().read(0xA0), 0);
                        assert_eq!(
                            reads.sequence,
                            (0..11u16)
                                .flat_map(|n| [left.wrapping_add(6 + n), right + 6 + n])
                                .collect::<Vec<_>>()
                        );
                        assert_eq!(vm.bus().ram().read(left.wrapping_add(5)), 0x80);
                        assert_eq!(vm.bus().ram().read(left.wrapping_add(17)), 0x80);
                    }
                }
            }
        }
    }
}

#[test]
fn comparison_pointer_ownership_conflict_keeps_valid_fallback() {
    let text = std::fs::read_to_string(root().join("surveys/tn/reproducers/comparison-loops.act"))
        .unwrap()
        .replace("\r\n", "\n");
    let text = format!("BYTE owned=$AA\n{text}");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = compile(&text, &Mir6502Config::optimized(), runtime);
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output))
            .unwrap();
        vm.bus_mut().ram_mut().map(0x5000, &[0x7F; 512]).unwrap();
        vm.bus_mut().ram_mut().map(0x5200, &[0x7F; 512]).unwrap();
        vm.bus_mut().ram_mut().write(0xAA, 0xD5);
        let entry = output
            .routine_addresses
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case("After"))
            .unwrap()
            .address;
        let args = [0x5000u16, 0x5200]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        vm = machine::call(vm, &mut Reads::default(), entry, &args);
        assert_eq!(vm.bus().ram().read(0xA0), 0);
        assert_eq!(vm.bus().ram().read(0xAA), 0xD5);
    }
}
