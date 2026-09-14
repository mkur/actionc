mod common;
#[path = "common/compiler.rs"]
mod compiler;
use actionc_vm68k_tests::{amiga::Os, hunk::File};
use std::{fs, process::Command};

#[test]
fn public_compiler_output_runs_after_moving_without_sources_or_sidecars() {
    for newline in ["\n", "\r\n"] {
        for raw in [false, true] {
            let temp = common::Source::new("");
            let root = temp.0.parent().unwrap();
            let source_dir = root.join("source files");
            let modules = root.join("host modules");
            let run_dir = root.join("working directory");
            for dir in [&source_dir, &modules, &run_dir] {
                fs::create_dir(dir).unwrap();
            }
            let source = source_dir.join("input file.act");
            fs::write(&source,";@actionc target motorola-68000\n;@actionc backend mir68k\nMODULE APP\nUSE SYS\nUSE VALUES\nINCLUDE \"local.inc\"\nPROC Main()\nSYS.PrintE(\"Amiga\")\nSYS.PrintIE(VALUES.Number())\nSYS.PrintCE(answer)\nRETURN\nENDMODULE\n".replace('\n',newline)).unwrap();
            fs::write(
                source_dir.join("local.inc"),
                "CARD answer=[65]\n".replace('\n', newline),
            )
            .unwrap();
            fs::write(
                modules.join("values.act"),
                "MODULE VALUES\nPUBLIC INT FUNC Number() RETURN(-32768)\nENDMODULE\n"
                    .replace('\n', newline),
            )
            .unwrap();
            let mut cmd = Command::new(compiler::executable());
            cmd.current_dir(&run_dir)
                .args([
                    "--runtime",
                    "amiga",
                    "--module-path",
                    "unused",
                    "--module-path",
                ])
                .arg(&modules)
                .args(["--listing", "output listing.txt", "-o", "output file.amiga"]);
            if raw {
                cmd.arg("--no-opt");
            }
            let result = cmd.arg(&source).output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let moved = root.join("moved.amiga");
            fs::rename(run_dir.join("output file.amiga"), &moved).unwrap();
            fs::remove_dir_all(&source_dir).unwrap();
            fs::remove_dir_all(&modules).unwrap();
            fs::remove_dir_all(&run_dir).unwrap();
            let file = File::parse(&fs::read(moved).unwrap()).unwrap();
            for bases in [
                [0x10000, 0x40000, 0x60000],
                [0x30000, 0x70000, 0x50000],
                [0x70000, 0x50000, 0x20000],
            ] {
                let mut vm = file.load(&bases).unwrap();
                let mut os = Os::default();
                os.install(&mut vm).unwrap();
                let run = os.run(&mut vm, 100000);
                run.assert_completed();
                assert_eq!(run.registers[0], 0);
                assert_eq!(os.output, b"Amiga\n-32768\n65\n");
                assert_eq!((os.open_count, os.close_count), (1, 1));
            }
        }
    }
}
