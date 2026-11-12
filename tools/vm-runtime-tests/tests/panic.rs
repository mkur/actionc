use actionc::compiler::{CompileOptions, Runtime, compile_file};
use actionc_vm::{RunRequest, StopReason, VmRunner};

#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

#[test]
fn panic_preserves_dynamic_codes_and_stops_after_a_returning_handler() {
    let fixture = include_str!("../../../fixtures/runtime/panic.act").replace("\r\n", "\n");
    for crlf in [false, true] {
        // Exercise both legacy unqualified calls and the explicit module API,
        // through actual LF/CRLF files and relocated runtime bodies.
        let source = if crlf {
            format!(
                "MODULE PROBE\nUSE SYS\n{}\nENDMODULE\n",
                fixture.replace("Panic(code)", "SYS.Panic(code)")
            )
            .replace('\n', "\r\n")
        } else {
            // Runtime preflight must honor the program's fixed allocation
            // controls, just as final emission does (the TN library uses this).
            format!(
                "ORG $3000\nSET $E=$E6\nSET $F=0\nSET $491=$E6\nSET $492=0\n\
                 BYTE POINTER scratch\nSET $E=$3000\nSET $491=$3000\nMODULE\n\
                 BYTE diagnostic=[0]\n{fixture}"
            )
        };
        let source = support::Source::new(&source);
        for (mode, runtime) in support::lanes() {
            let compiled = compile_file(
                &source.0,
                &CompileOptions::for_mode(mode)
                    .with_runtime(runtime)
                    .with_origin(if crlf { 0x5000 } else { 0x3000 }),
            )
            .unwrap_or_else(|e| panic!("{mode:?}/{runtime:?}/CRLF={crlf}: {e}"));
            let listing = compiled.source_listing();
            let routines = listing
                .lines()
                .filter_map(|line| line.strip_prefix("; ===== PROC "))
                .filter_map(|header| header.split_whitespace().next())
                .filter_map(|name| name.rsplit("::").next())
                .collect::<Vec<_>>();
            let implementation = if runtime == Runtime::ActionCart {
                "CartLongError"
            } else {
                "SysLongError"
            };
            assert!(
                routines
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(implementation))
            );
            for unrelated in ["SysParseLong", "SysStrLC", "SysPrintLC", "CartPrintLC"] {
                assert!(
                    !routines
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(unrelated)),
                    "Panic must not import unrelated LONG services: {unrelated}"
                );
            }
            for code in [0, 1, 100, 105, 107, 108, 109, 110, 111, 128, 255] {
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, compiled.object_bytes())
                    .unwrap();
                let ram = vm.bus_mut().ram_mut();
                ram.write(0x604, code);
                ram.write(0x703, 0);
                // Capture A/X/Y and the number of calls. Deliberately overwrite
                // the caller's code as well as registers and decimal mode.
                ram.map(
                    0x780,
                    &[
                        0x8D, 0x00, 0x07, 0x8E, 0x01, 0x07, 0x8C, 0x02, 0x07, 0xEE, 0x03, 0x07,
                        0xA9, 17, 0x8D, 0x04, 0x06, 0xA2, 34, 0xA0, 51, 0xF8, 0x60,
                    ],
                )
                .unwrap();
                match runtime {
                    Runtime::ActionCart => ram.map(0x04CB, &[0x4C, 0x80, 0x07]).unwrap(),
                    Runtime::Standalone => ram.write_word(0x000A, 0x780),
                }
                let outcome = VmRunner::new(vm).run(RunRequest {
                    max_steps: 2_000,
                    history_len: 8,
                    ..RunRequest::default()
                });
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps: 2_000 },
                    "{:?}",
                    outcome.report
                );
                assert_eq!(
                    (0x700..=0x703)
                        .map(|a| outcome.memory().read(a))
                        .collect::<Vec<_>>(),
                    [code, 0, code, 1],
                    "{mode:?}/{runtime:?}/{code}: {:?}",
                    outcome.report
                );
                assert_eq!(
                    outcome.memory().read(0x605),
                    41,
                    "source continued after Panic"
                );
                assert_eq!(outcome.memory().read(0x604), 17);
                let r = &outcome.report.registers;
                assert_eq!([r.a, r.x, r.y], [code, 0, code]);
                assert_eq!(r.status & 9, 1, "decimal clear, carry set");
                assert_eq!(
                    [outcome.memory().read(r.pc), outcome.memory().read(r.pc + 1)],
                    [0xB0, 0xFE]
                );
            }
        }
    }
}
