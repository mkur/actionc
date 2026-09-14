//! Equivalent C and Action! run in the same VM against independent C vectors.
#[path = "c_reference/image.rs"]
mod image;
#[path = "c_reference/vectors.rs"]
mod vectors;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use image::Program;
use std::path::PathBuf;

fn main() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap();
}
fn run() {
    let mut args = std::env::args().skip(1);
    let directory = PathBuf::from(
        args.next()
            .expect("usage: c_reference BUILD_DIR [insertsort matrix1]"),
    );
    let mut names: Vec<_> = args.collect();
    if names.is_empty() {
        names = vec!["insertsort".into(), "matrix1".into()];
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    println!(
        "benchmark,compiler,mode,code_bytes,default_instructions,default_stack_read_bytes,default_stack_write_bytes,reference_cases,reference_instructions"
    );
    for name in names {
        assert!(
            matches!(name.as_str(), "insertsort" | "matrix1"),
            "unknown benchmark {name}"
        );
        let compiled = compile_file(
            root.join(format!("fixtures/runtime/tacle/{name}/{name}.act")),
            &NativeCompileOptions::default(),
        )
        .unwrap();
        actionc_vm68k_tests::artifacts::dump(&compiled, &directory.join(format!("{name}-actionc")))
            .unwrap();
        for segment in compiled.image.segments.iter().filter(|s| s.executable) {
            std::fs::write(
                directory.join(format!("{name}-actionc-{:x}.bin", segment.address)),
                &segment.bytes,
            )
            .unwrap();
        }
        let programs = [
            ("actionc", "optimized", Program::action(compiled.image)),
            (
                "gcc",
                "O2",
                Program::load(&directory.join(format!("{name}-O2.image"))).unwrap(),
            ),
            (
                "gcc",
                "Os",
                Program::load(&directory.join(format!("{name}-Os.image"))).unwrap(),
            ),
        ];
        for (compiler, mode, program) in programs {
            eprintln!("Checking {name}/{compiler}/{mode} against pinned reference states");
            let mut vm = program.machine();
            vm.cpu
                .mem
                .trace_range(actionc_vm68k_tests::STACK_BOTTOM..actionc_vm68k_tests::STACK_TOP);
            let run = vm.run(2_000_000);
            run.assert_completed();
            let trace = vm.cpu.mem.take_trace();
            let reads = trace.iter().filter(|(_, write)| !write).count();
            let writes = trace.len() - reads;
            program.check(
                &vm,
                "result",
                if name == "insertsort" { 1 } else { 2 },
                &[0],
            );
            let (cases, instructions) = vectors::execute(&name, &program);
            println!(
                "{name},{compiler},{mode},{},{},{reads},{writes},{cases},{instructions}",
                program.code_bytes(),
                run.steps
            );
        }
    }
}
