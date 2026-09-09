//! Fresh homes are poisoned before execution. Check actual byte writes and
//! source-time behavior on classic and all raw/optimized NIR/MIR paths.
#[path = "support/variant_values.rs"]
mod support;

use actionc::{
    codegen::{self, CodegenOutput},
    compiler::Runtime,
    nir,
    semantic::{self, ir::SemProgram},
};
use actionc_vm::{BusAccess, CompilerVm, ExecutionProfile};

fn lower(source: &str) -> SemProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, semantic::SemanticOptions::modern()).unwrap();
    semantic::ir::lower_program(&ast, &model)
}

fn each_output(semir: &SemProgram, check: impl Fn(&CodegenOutput, &str)) {
    let raw = nir::lower_program(semir);
    nir::verify_program(&raw).unwrap();
    let optimized = nir::optimize_program(&raw).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        check(
            &codegen::generate_semir_profile_at_origin_with_runtime(
                semir,
                0x3000,
                codegen::CodegenProfile::Modern,
                runtime,
            )
            .unwrap(),
            &format!("classic/{runtime:?}"),
        );
        for (lane, program, config) in [
            ("raw", &raw, actionc::mir6502::Mir6502Config::default()),
            (
                "optimized-nir",
                &optimized,
                actionc::mir6502::Mir6502Config::default(),
            ),
            (
                "optimized-mir",
                &optimized,
                actionc::mir6502::Mir6502Config::optimized(),
            ),
        ] {
            check(
                &actionc::mir6502::generate_output_with_config_and_runtime(
                    program, 0x3000, &config, runtime,
                )
                .unwrap(),
                &format!("{lane}/{runtime:?}"),
            );
        }
    }
}

fn check_poisoned_constructor(definition: &str, initializer: &str, expected: &[u8], minimal: bool) {
    let semir = lower(&format!(
        "{definition} Value current BYTE done=$600 \
        PROC Main() LET item={initializer} current=item done=$A5 DO OD RETURN"
    ));
    each_output(&semir, |output, lane| {
        let slot = output
            .map
            .storage_symbols
            .iter()
            .find(|s| s.name.to_ascii_lowercase().ends_with("::item"))
            .unwrap_or_else(|| panic!("{lane}: {:?}", output.map.storage_symbols));
        assert_eq!(usize::from(slot.size), expected.len());
        let mut vm = CompilerVm::default();
        vm.load_atari_object_for_execution(
            if output.map.runtime == Runtime::ActionCart {
                ExecutionProfile::CartridgeObject
            } else {
                ExecutionProfile::StandaloneObject
            },
            &codegen::format_load_file(output),
        )
        .unwrap();
        for address in slot.address..slot.address + slot.size {
            vm.bus_mut().ram_mut().write(address, 0xCC);
            vm.bus_mut().add_watchpoint(address);
        }
        for address in 0x600..=0x6FF {
            vm.bus_mut().ram_mut().write(address, 0xCC);
        }
        vm.bus_mut().clear_events();
        let start = vm.cpu().cycles();
        if minimal {
            let entry = output.run_address;
            let payload = slot.address + 1;
            let expected_code = [
                0xA9,
                42,
                0x8D,
                payload as u8,
                (payload >> 8) as u8,
                0xA9,
                2,
                0x8D,
                slot.address as u8,
                (slot.address >> 8) as u8,
            ];
            let code: Vec<_> = (entry..entry + 10)
                .map(|a| vm.bus().ram().read(a))
                .collect();
            assert_eq!(code, expected_code, "{lane}: four direct instructions");
            for _ in 0..4 {
                vm.step_cpu().unwrap();
            }
            assert_eq!(vm.cpu().cycles() - start, 12, "{lane}: initialization only");
            assert_eq!(vm.cpu().registers().pc, entry + 10);
        }
        for step in 0..300_000 {
            vm.step_cpu().unwrap();
            if vm.bus().ram().read(0x600) == 0xA5 {
                break;
            }
            assert!(step < 299_999, "{lane}: bounded completion");
        }
        let writes: Vec<_> = vm
            .bus()
            .events()
            .iter()
            .filter(|e| e.access == BusAccess::Write)
            .collect();
        assert_eq!(writes.len(), expected.len(), "{lane}: one write per byte");
        let mut counts = vec![0; expected.len()];
        for write in &writes {
            let offset = usize::from(write.address - slot.address);
            counts[offset] += 1;
            assert_eq!(write.value, expected[offset], "{lane}: byte {offset}");
        }
        assert!(counts.iter().all(|n| *n == 1), "{lane}: {counts:?}");
        let last = writes.last().unwrap();
        assert_eq!(
            (last.address, last.value),
            (slot.address, expected[0]),
            "{lane}: tag last"
        );
        let current = output
            .map
            .storage_symbols
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case("current"))
            .unwrap();
        let actual: Vec<_> = (current.address..current.address + current.size)
            .map(|a| vm.bus().ram().read(a))
            .collect();
        assert_eq!(actual, expected, "{lane}: later complete-value use");
        assert!((0x601..=0x6FF).all(|a| vm.bus().ram().read(a) == 0xCC));
    });
}

#[test]
fn fresh_literal_is_ten_bytes_twelve_cycles_in_every_lane() {
    check_poisoned_constructor(
        "TYPE Value=VARIANT [NONE SOME [BYTE value]]",
        "Value.SOME(42)",
        &[2, 42],
        true,
    );
}

#[test]
fn maybe_byte_example_reports_cost_to_printbe_without_running_printing() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/support/fresh_maybe_byte.act");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let compiled = compile_file(
                &source,
                &CompileOptions::for_mode(mode)
                    .with_runtime(runtime)
                    .with_origin(0x3000),
            )
            .unwrap();
            let print_entry = if runtime == Runtime::ActionCart {
                0xA4EC
            } else {
                let listing = compiled.source_listing();
                let body = listing.split("proc_resident_printbe:").nth(1).unwrap();
                let address = body
                    .lines()
                    .find_map(|line| line.split("; $").nth(1).map(|part| &part[..4]))
                    .unwrap();
                u16::from_str_radix(address, 16).unwrap()
            };
            let mut vm = CompilerVm::default();
            vm.load_atari_object_for_execution(
                if runtime == Runtime::ActionCart {
                    ExecutionProfile::CartridgeObject
                } else {
                    ExecutionProfile::StandaloneObject
                },
                compiled.object_bytes(),
            )
            .unwrap();
            let start = vm.cpu().cycles();
            for step in 0..10_000 {
                if vm.cpu().registers().pc == print_entry {
                    break;
                }
                assert!(step < 9_999, "{mode:?}/{runtime:?}: did not reach PrintBE");
                vm.step_cpu().unwrap();
            }
            assert_eq!(vm.cpu().registers().a, 42);
            if mode == CompileMode::Mir6502 {
                assert!(
                    vm.cpu().cycles() - start <= 23,
                    "CASE must not regain its snapshot or preliminary validity check"
                );
                assert!(
                    compiled.object_bytes().len()
                        <= if runtime == Runtime::ActionCart {
                            44
                        } else {
                            280
                        }
                );
            }
            eprintln!(
                "maybe-byte,{mode:?},{runtime:?},xex_bytes={},cycles_to_PrintBE={}",
                compiled.object_bytes().len(),
                vm.cpu().cycles() - start
            );
        }
    }
}

#[test]
fn fresh_nullary_nested_and_page_crossing_values_define_every_poisoned_byte_once() {
    for (definition, initializer, expected) in [
        (
            "TYPE Value=VARIANT [NONE SOME [BYTE value]]",
            "Value.NONE",
            vec![1, 0],
        ),
        ("TYPE Value=VARIANT [ONLY]", "Value.ONLY", vec![1]),
        (
            "TYPE Inner=VARIANT [NONE SOME [BYTE value]] TYPE Value=VARIANT [NONE DATA [Inner inner]]",
            "Value.DATA(Inner.SOME(42))",
            vec![2, 2, 42],
        ),
        (
            "TYPE Payload=[BYTE ARRAY bytes(257)] TYPE Value=VARIANT [EMPTY DATA [Payload payload] SMALL [BYTE code]]",
            "Value.SMALL(42)",
            {
                let mut bytes = vec![0; 258];
                bytes[..2].copy_from_slice(&[3, 42]);
                bytes
            },
        ),
    ] {
        check_poisoned_constructor(definition, initializer, &expected, false);
    }
}

#[test]
fn fresh_initialization_stays_inside_branches_loops_and_shadowing_scopes() {
    let source = r#"
TYPE Value=VARIANT [NONE SOME [BYTE number]]
BYTE ARRAY output=$600
BYTE done=$63F
PROC Main()
BYTE i,seed
seed=40
IF seed=0 THEN
BEGIN
  LET skipped=Value.SOME(99)
  output(10)=99
END
FI
FOR i=0 TO 2 DO
BEGIN
  LET item=Value.SOME(seed+i)
  CASE item OF
  WHEN Value.SOME(number) THEN
    output(i)=number
  ELSE
    output(i)=255
  ESAC
END
OD
LET item=Value.SOME(17)
LET item=item
CASE item OF
WHEN Value.SOME(number) THEN
  output(3)=number
ELSE
  output(3)=255
ESAC
done=$A5 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..4].copy_from_slice(&[40, 41, 42, 17]);
    expected[0x3F] = 0xA5;
    support::check_semir(&lower(source), &expected, false);
}

#[test]
fn fresh_payloads_snapshot_complete_union_and_record_images_before_mutation() {
    let source = r#"
TYPE Row=[BYTE head BYTE ARRAY bytes(257)]
TYPE Bits=UNION [CARD word BYTE ARRAY bytes(3)]
TYPE Value=VARIANT [NONE DATA [Row row Bits bits]]
Row original Bits sourceBits
BYTE ARRAY output=$600
BYTE done=$63F
PROC Main()
CARD i
original.head=11
FOR i=0 TO 256 DO original.bytes(i)=BYTE(i) OD
sourceBits.bytes(2)=$A5 sourceBits.word=$2345
LET item=Value.DATA(original,sourceBits)
original.head=99 original.bytes(256)=99 sourceBits.bytes(2)=99
CASE item OF
WHEN Value.DATA(row,data) THEN
  output(0)=row.head output(1)=row.bytes(256)
  output(2)=data.bytes(0) output(3)=data.bytes(1) output(4)=data.bytes(2)
ELSE
  output(0)=255
ESAC
done=$A5 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..5].copy_from_slice(&[11, 0, 0x45, 0x23, 0xA5]);
    expected[0x3F] = 0xA5;
    support::check_semir(&lower(source), &expected, false);
}

#[test]
fn effectful_fields_remain_ordered_and_observe_the_old_replacement_destination() {
    let source = r#"
TYPE Value=VARIANT [NONE DATA [BYTE first,second]]
Value current
BYTE ARRAY output=$600
BYTE calls=$63E,done=$63F
BYTE FUNC Next()
calls==+1
CASE current OF
WHEN Value.DATA(first,_) THEN
  output(calls)=first
ELSE
  output(calls)=255
ESAC
RETURN(calls)
PROC Main()
calls=0 current=Value.DATA(41,42)
LET saved=Value.DATA(Next(),Next())
current=Value.DATA(Next(),Next())
CASE saved OF
WHEN Value.DATA(first,second) THEN
  output(5)=first output(6)=second
ELSE
  output(5)=255
ESAC
done=$A5 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[1..7].copy_from_slice(&[41, 41, 41, 41, 1, 2]);
    expected[0x3E] = 4;
    expected[0x3F] = 0xA5;
    support::check_semir(&lower(source), &expected, false);
}

#[test]
fn invalid_snapshot_and_nested_payload_do_not_publish_a_value() {
    for consumer in ["LET item=source", "LET item=Outer.DATA(41,source)"] {
        let source = format!(
            "TYPE Value=VARIANT [NONE SOME [BYTE value]] \
            TYPE Outer=VARIANT [NONE DATA [BYTE marker Value inner]] \
            Value source BYTE ARRAY output=$600 PROC Main() \
            output(0)=41 {consumer} output(1)=99 DO OD RETURN"
        );
        let mut expected = vec![0xCC; 0x500];
        expected[0] = 41;
        support::check_semir(&lower(&source), &expected, true);
    }
}

#[test]
fn static_reentry_finishes_before_the_fresh_binding_is_written() {
    let semir = lower(
        r#"
TYPE Value=VARIANT [DATA [BYTE first,second]]
Value current
BYTE phase=$680,done=$600
BYTE FUNC Build(BYTE recurse)
  IF recurse=0 THEN phase=1 RETURN(42) FI
  LET item=Value.DATA(17,Build(0))
  phase=2
  current=item
RETURN(42)
PROC Main()
  BYTE result
  result=Build(1)
  done=$A5 DO OD
RETURN
"#,
    );
    each_output(&semir, |output, lane| {
        let slot = output
            .map
            .storage_symbols
            .iter()
            .find(|s| s.name.to_ascii_lowercase().ends_with("::item"))
            .unwrap();
        let mut vm = CompilerVm::default();
        vm.load_atari_object_for_execution(
            if output.map.runtime == Runtime::ActionCart {
                ExecutionProfile::CartridgeObject
            } else {
                ExecutionProfile::StandaloneObject
            },
            &codegen::format_load_file(output),
        )
        .unwrap();
        for address in slot.address..slot.address + slot.size {
            vm.bus_mut().ram_mut().write(address, 0xCC);
            vm.bus_mut().add_watchpoint(address);
        }
        vm.bus_mut().ram_mut().write(0x600, 0);
        vm.bus_mut().ram_mut().write(0x680, 0);
        vm.bus_mut().add_watchpoint(0x680);
        vm.bus_mut().clear_events();
        for step in 0..100_000 {
            vm.step_cpu().unwrap();
            if vm.bus().ram().read(0x600) == 0xA5 {
                break;
            }
            assert!(step < 99_999, "{lane}: bounded reentry");
        }
        let writes: Vec<_> = vm
            .bus()
            .events()
            .iter()
            .filter(|e| e.access == BusAccess::Write)
            .collect();
        assert_eq!(writes.len(), 5);
        assert_eq!((writes[0].address, writes[0].value), (0x680, 1));
        assert_eq!((writes[4].address, writes[4].value), (0x680, 2));
        let bytes: Vec<_> = (slot.address..slot.address + slot.size)
            .map(|a| vm.bus().ram().read(a))
            .collect();
        assert_eq!(bytes, [1, 17, 42], "{lane}: staged result after reentry");
    });
}

#[test]
fn skipped_initialization_never_writes_the_binding_home() {
    let semir = lower(
        r#"
TYPE Value=VARIANT [SOME [BYTE number]]
Value current
BYTE gate=$680,done=$600
PROC Main()
  IF gate=1 THEN
  BEGIN
    LET skipped=Value.SOME(99)
    current=skipped
  END
  FI
  done=$A5 DO OD
RETURN
"#,
    );
    each_output(&semir, |output, lane| {
        let slot = output
            .map
            .storage_symbols
            .iter()
            .find(|s| s.name.to_ascii_lowercase().ends_with("::skipped"))
            .unwrap();
        let mut vm = CompilerVm::default();
        vm.load_atari_object_for_execution(
            if output.map.runtime == Runtime::ActionCart {
                ExecutionProfile::CartridgeObject
            } else {
                ExecutionProfile::StandaloneObject
            },
            &codegen::format_load_file(output),
        )
        .unwrap();
        for address in slot.address..slot.address + slot.size {
            vm.bus_mut().ram_mut().write(address, 0xCC);
            vm.bus_mut().add_watchpoint(address);
        }
        vm.bus_mut().ram_mut().write(0x600, 0);
        vm.bus_mut().ram_mut().write(0x680, 0);
        vm.bus_mut().clear_events();
        for step in 0..100_000 {
            vm.step_cpu().unwrap();
            if vm.bus().ram().read(0x600) == 0xA5 {
                break;
            }
            assert!(step < 99_999, "{lane}: bounded completion");
        }
        assert!(
            !vm.bus()
                .events()
                .iter()
                .any(|e| e.access == BusAccess::Write),
            "{lane}"
        );
        assert!((slot.address..slot.address + slot.size).all(|a| vm.bus().ram().read(a) == 0xCC));
    });
}
