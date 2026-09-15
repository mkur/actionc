//! Equivalent C and Action! run in the same VM against independent C vectors.
#[path = "../tests/common/dct.rs"]
mod dct;
#[path = "c_reference/image.rs"]
mod image;
#[path = "c_reference/vectors.rs"]
mod vectors;
use actionc::compiler::native::compile_file;
use image::Program;
use std::path::PathBuf;
#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod common;
mod measurement;

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
            .expect("usage: c_reference BUILD_DIR [BENCHMARK ...]"),
    );
    let (options, mut names) = measurement::options(args);
    if names.is_empty() {
        names = [
            "insertsort",
            "matrix1",
            "matrix1-multidimensional",
            "jfdctint",
            "jfdctint-multidimensional",
        ]
        .map(String::from)
        .to_vec();
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in &names {
        variant(name);
    }
    std::fs::write(
        directory.join("actionc-options.txt"),
        measurement::metadata(&root, &options, &names),
    )
    .unwrap();
    println!(
        "benchmark,compiler,mode,code_bytes,default_instructions,default_stack_read_bytes,default_stack_write_bytes,reference_cases,reference_instructions,reference_instrumented"
    );
    for name in names {
        let (family, file, shaped) = variant(&name);
        let path = root.join(format!("fixtures/runtime/tacle/{family}/{file}"));
        let compiled = compile_file(&path, &options).unwrap();
        // Intermediate captures are deliberately excluded from headline code
        // and default execution measurements for both compilers.
        let capture = if family == "jfdctint" {
            let source = dct::instrument(&std::fs::read_to_string(&path).unwrap(), shaped);
            let path = directory.join(format!("{name}-capture.act"));
            std::fs::write(&path, source).unwrap();
            Some(Program::action(compile_file(path, &options).unwrap().image))
        } else {
            None
        };
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
            (
                "actionc",
                if options.optimize { "optimized" } else { "raw" },
                Program::action(compiled.image),
            ),
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
            vectors::check_default(&name, &program, &vm);
            let c_capture = if family == "jfdctint" && compiler == "gcc" {
                Some(
                    Program::load(&directory.join(format!("{name}-{mode}-capture.image"))).unwrap(),
                )
            } else {
                None
            };
            let reference = if compiler == "actionc" {
                capture.as_ref()
            } else {
                c_capture.as_ref()
            };
            let (cases, instructions) = vectors::execute(&name, reference.unwrap_or(&program));
            println!(
                "{name},{compiler},{mode},{},{},{reads},{writes},{cases},{instructions},{}",
                program.code_bytes(),
                run.steps,
                reference.is_some()
            );
        }
    }
}

fn variant(name: &str) -> (&'static str, &'static str, bool) {
    match name {
        "insertsort" => ("insertsort", "insertsort.act", false),
        "matrix1" => ("matrix1", "matrix1.act", false),
        "matrix1-multidimensional" => ("matrix1", "multidimensional.act", true),
        "jfdctint" => ("jfdctint", "jfdctint.act", false),
        "jfdctint-multidimensional" => ("jfdctint", "multidimensional.act", true),
        _ => panic!("unknown benchmark: {name}"),
    }
}
