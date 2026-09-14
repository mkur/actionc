//! Reproducible native execution measurements; no test-only source instrumentation.
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

fn main() {
    // Match the compiler tooling's explicit stack on Windows as well as Unix.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap();
}
fn run() {
    let requested: Vec<_> = std::env::args().skip(1).collect();
    let conservative = requested.iter().any(|arg| arg == "--no-codegen-opt");
    let requested: Vec<_> = requested
        .iter()
        .filter(|arg| !arg.starts_with("--"))
        .collect();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    println!("benchmark,nir,code_bytes,instructions,max_frame_bytes");
    for name in [
        "insertsort",
        "matrix1",
        "binarysearch",
        "sha",
        "jfdctint",
        "adpcm_dec",
        "adpcm_enc",
    ] {
        if !requested.is_empty() && !requested.iter().any(|arg| arg.as_str() == name) {
            continue;
        }
        for optimize in [false, true] {
            let program = compile_file(
                root.join(format!("fixtures/runtime/tacle/{name}/{name}.act")),
                &NativeCompileOptions {
                    optimize,
                    codegen: actionc::mir68k::materialize::Options {
                        forward_temporaries: !conservative,
                        select_instructions: !conservative,
                    },
                    ..Default::default()
                },
            )
            .unwrap();
            let mut vm = Machine::from_image(&program.image).unwrap();
            if name == "sha" {
                let mut message = vec![0; 1025];
                message[..3].copy_from_slice(&[97, 98, 99]);
                vm.write_array(program.image.symbol("message").unwrap(), &message)
                    .unwrap();
                vm.write_scalar(program.image.symbol("length").unwrap(), 3)
                    .unwrap();
            }
            let run = vm.run(100_000_000);
            run.assert_completed();
            if name == "sha" {
                assert_eq!(
                    vm.read_array(program.image.symbol("digest").unwrap())
                        .unwrap(),
                    [0x0164b8a9, 0x14cd2a5e, 0x74c4f7ff, 0x082c4d97, 0xf1edf880]
                );
            } else {
                let result = match name {
                    "adpcm_dec" => "ADPCM_DEC.result",
                    "adpcm_enc" => "ADPCM_ENC.result",
                    _ => "result",
                };
                assert_eq!(
                    vm.read_scalar(program.image.symbol(result).unwrap())
                        .unwrap(),
                    if name == "binarysearch" { u32::MAX } else { 0 },
                    "{name}/{optimize}"
                );
            }
            let bytes: usize = program
                .image
                .segments
                .iter()
                .filter(|s| s.executable)
                .map(|s| s.bytes.len())
                .sum();
            let frame = program
                .machine
                .routines
                .iter()
                .map(|r| r.frame.extent.get())
                .max()
                .unwrap_or(0);
            println!(
                "{name},{},{bytes},{},{frame}",
                if optimize { "optimized" } else { "raw" },
                run.steps
            );
        }
    }
}
