mod common;
#[path = "common/compiler.rs"]
mod compiler;
use actionc::compiler::native::amiga;
use actionc_vm68k_tests::{STACK_BOTTOM, STACK_TOP, amiga::Os, hunk::File};
use std::{fs, path::Path, process::Command};

#[test]
fn public_amiga_samples_match_reports_and_fit_the_documented_command_stack() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["hello", "integer-array", "insertsort", "division-zero"] {
        let path = root.join(format!("samples/amiga/{name}.act"));
        // Host text checkout can be CRLF; guest bytes must remain exact LF.
        let expected = fs::read_to_string(root.join(format!("samples/amiga/expected/{name}.txt")))
            .unwrap()
            .replace("\r\n", "\n")
            .into_bytes();
        for raw in [false, true] {
            let temp = common::Source::new("");
            let output = temp.0.with_extension("amiga");
            let mut cmd = Command::new(compiler::executable());
            cmd.args(["--runtime", "amiga", "-o"])
                .arg(&output)
                .arg(&path);
            if raw {
                cmd.arg("--no-opt");
            }
            let result = cmd.output().unwrap();
            assert!(
                result.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let file = File::parse(&fs::read(&output).unwrap()).unwrap();
            // Only this metadata assertion uses the API, never execution/loading.
            let compiled = amiga::compile_file(
                &path,
                &amiga::Options {
                    optimize: !raw,
                    ..Default::default()
                },
            )
            .unwrap();
            for bases in [
                [0x10000, 0x40000, 0x60000],
                [0x30000, 0x70000, 0x50000],
                [0x70000, 0x50000, 0x20000],
            ] {
                let mut vm = file.load(&bases).unwrap();
                let mut os = Os::default();
                os.install(&mut vm).unwrap();
                vm.cpu.mem.trace_range(STACK_BOTTOM..STACK_TOP);
                let run = os.run(&mut vm, 1_000_000);
                run.assert_completed();
                assert_eq!(
                    run.registers[0],
                    if name == "division-zero" { 20 } else { 0 }
                );
                assert_eq!(os.output, expected, "{name}/raw={raw}");
                assert_eq!((os.open_count, os.close_count), (1, 1));
                let used = STACK_TOP
                    - vm.cpu
                        .mem
                        .take_trace()
                        .iter()
                        .filter(|(_, write)| *write)
                        .map(|(address, _)| *address)
                        .min()
                        .unwrap();
                assert!(used < 65536, "{name} stack: {used}");
                eprintln!(
                    "Amiga {name} raw={raw} stack={used} bytes, instructions={}",
                    run.steps
                );
                if name == "insertsort" {
                    let symbol = compiled
                        .executable
                        .object
                        .symbols
                        .iter()
                        .find(|s| s.name == "values")
                        .unwrap();
                    let backing = symbol
                        .array
                        .as_ref()
                        .unwrap()
                        .backing
                        .unwrap()
                        .resolve(&bases)
                        .unwrap();
                    let actual: Vec<_> = vm
                        .cpu
                        .mem
                        .bytes(backing, 44)
                        .unwrap()
                        .chunks_exact(4)
                        .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
                        .collect();
                    assert_eq!(actual, [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
                }
            }
        }
    }
}
