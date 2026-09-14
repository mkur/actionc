//! Reproducible native execution measurements; no test-only source instrumentation.
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;
mod measurement;

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
    let (options, requested) = measurement::options(std::env::args().skip(1));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let names: Vec<String> = [
        "insertsort",
        "matrix1",
        "matrix1-multidimensional",
        "binarysearch",
        "sha",
        "jfdctint",
        "jfdctint-multidimensional",
        "adpcm_dec",
        "adpcm_enc",
    ]
    .into_iter()
    .filter(|name| requested.is_empty() || requested.iter().any(|arg| arg == name))
    .map(str::to_owned)
    .collect();
    assert!(
        requested.iter().all(|name| names.contains(name)),
        "unknown benchmark"
    );
    eprintln!("{}", measurement::metadata(&root, &options, &names));
    println!(
        "benchmark,nir,code_bytes,instructions,max_frame_bytes,stack_read_bytes,stack_write_bytes"
    );
    for name in names {
        for optimize in [false, true]
            .into_iter()
            .filter(|optimized| !optimized || options.optimize)
        {
            let (directory, file) = name
                .strip_suffix("-multidimensional")
                .map_or((name.as_str(), name.as_str()), |base| {
                    (base, "multidimensional")
                });
            let program = compile_file(
                root.join(format!("fixtures/runtime/tacle/{directory}/{file}.act")),
                &NativeCompileOptions {
                    optimize,
                    ..options.clone()
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
            vm.cpu
                .mem
                .trace_range(actionc_vm68k_tests::STACK_BOTTOM..actionc_vm68k_tests::STACK_TOP);
            let run = vm.run(100_000_000);
            run.assert_completed();
            let trace = vm.cpu.mem.take_trace();
            let reads = trace.iter().filter(|(_, write)| !write).count();
            let writes = trace.len() - reads;
            if name == "sha" {
                assert_eq!(
                    vm.read_array(program.image.symbol("digest").unwrap())
                        .unwrap(),
                    [0x0164b8a9, 0x14cd2a5e, 0x74c4f7ff, 0x082c4d97, 0xf1edf880]
                );
            } else {
                let result = match name.as_str() {
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
                "{name},{},{bytes},{},{frame},{reads},{writes}",
                if optimize { "optimized" } else { "raw" },
                run.steps
            );
        }
    }
}
