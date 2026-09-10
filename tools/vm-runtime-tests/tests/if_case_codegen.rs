//! Statement/value selection comparison. Functional oracles are asserted;
//! emitted image bytes and cycles to completion are reproducible measurements.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::nir::{self, NirCallee, NirOp};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const COMMON: &str = "BYTE input=$680,flag=$681,result=$600,done=$67F\n";
const VARIANT: &str = "TYPE MaybeByte=VARIANT [NONE SOME [BYTE value]]\nMaybeByte current\n";

fn source(case: &str, expression: bool) -> String {
    let selection = match (case, expression) {
        ("if", false) => "IF n<128 THEN RETURN(n) ELSE RETURN(BYTE(n+1)) FI",
        ("if", true) => "RETURN(IF n<128 THEN n ELSE BYTE(n+1) FI)",
        ("case", false) => {
            "CASE n OF\nWHEN 0 THEN\nRETURN(7)\nWHEN 1 TO 127 THEN\nRETURN(BYTE(n+1))\nELSE\nRETURN(n)\nESAC"
        }
        ("case", true) => {
            "RETURN(CASE n OF\nWHEN 0 THEN\n7\nWHEN 1 TO 127 THEN\nBYTE(n+1)\nELSE\nn\nESAC)"
        }
        ("known_some", false) => {
            "BYTE value\nLET item=MaybeByte.SOME(42)\nCASE item OF\nWHEN MaybeByte.NONE THEN\nvalue=0\nWHEN MaybeByte.SOME(n) THEN\nvalue=n\nESAC\nresult=value"
        }
        ("known_some", true) => {
            "LET item=MaybeByte.SOME(42)\nLET value=CASE item OF\nWHEN MaybeByte.NONE THEN\n0\nWHEN MaybeByte.SOME(n) THEN\nn\nESAC\nresult=value"
        }
        ("dynamic_variant", false) => {
            "CASE current OF\nWHEN MaybeByte.NONE THEN\nRETURN(0)\nWHEN MaybeByte.SOME(n) THEN\nRETURN(n)\nESAC"
        }
        ("dynamic_variant", true) => {
            "RETURN(CASE current OF\nWHEN MaybeByte.NONE THEN\n0\nWHEN MaybeByte.SOME(n) THEN\nn\nESAC)"
        }
        _ => unreachable!(),
    };
    match case {
        "known_some" => {
            format!("{COMMON}{VARIANT}PROC Main()\n{selection}\ndone=$A5\nDO OD\nRETURN")
        }
        "dynamic_variant" => format!(
            "{COMMON}{VARIANT}BYTE FUNC Pick()\n{selection}\nPROC Main()\nIF flag THEN current=MaybeByte.SOME(input) ELSE current=MaybeByte.NONE FI\nresult=Pick()\ndone=$A5\nDO OD\nRETURN"
        ),
        _ => format!(
            "{COMMON}BYTE FUNC Pick(BYTE n)\n{selection}\nPROC Main()\nresult=Pick(input)\ndone=$A5\nDO OD\nRETURN"
        ),
    }
}

struct WorkDir(PathBuf);
impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn execute(image: &[u8], runtime: Runtime, case: &str, input: u8, flag: u8) -> u64 {
    let mut vm = CompilerVm::default();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let profile = if runtime == Runtime::ActionCart {
        for (kind, file, base) in [
            (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
            (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
        ] {
            vm.load_image_bytes(
                kind,
                file,
                base,
                std::fs::read(root.join("roms").join(file)).unwrap(),
            )
            .unwrap();
        }
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    let loaded = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        loaded
            .segments
            .iter()
            .all(|s| s.end < 0x600 || s.start > 0x6FF)
    );
    let mut expected = vec![0xCC; 0x100];
    expected[0x80] = input;
    expected[0x81] = flag;
    vm.bus_mut().ram_mut().map(0x600, &expected).unwrap();
    expected[0] = match case {
        "if" => {
            if input < 128 {
                input
            } else {
                input.wrapping_add(1)
            }
        }
        "case" => {
            if input == 0 {
                7
            } else if input < 128 {
                input + 1
            } else {
                input
            }
        }
        "known_some" => 42,
        "dynamic_variant" => {
            if flag == 0 {
                0
            } else {
                input
            }
        }
        _ => unreachable!(),
    };
    expected[0x7F] = 0xA5;
    let start = vm.cpu().cycles();
    for _ in 0..10_000 {
        vm.step_cpu().unwrap();
        if vm.bus().ram().read(0x67F) == 0xA5 {
            let cycles = vm.cpu().cycles() - start;
            let actual = (0x600..=0x6FF)
                .map(|a| vm.bus().ram().read(a))
                .collect::<Vec<_>>();
            assert_eq!(
                actual, expected,
                "{case}/{runtime:?}/input={input}/flag={flag}"
            );
            return cycles;
        }
    }
    panic!("{case}/{runtime:?} failed to complete");
}

#[test]
fn statement_and_expression_selection_codegen() {
    let work = WorkDir(
        std::env::temp_dir().join(format!("actionc-selection-audit-{}", std::process::id())),
    );
    std::fs::create_dir(&work.0).unwrap();
    println!("case,form,backend,runtime,xex_bytes,min_cycles,max_cycles");
    for case in ["if", "case", "known_some", "dynamic_variant"] {
        for expression in [false, true] {
            let form = if expression {
                "expression"
            } else {
                "statement"
            };
            let source = source(case, expression);
            let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
            let model = actionc::semantic::analyze_with_options(
                &ast,
                actionc::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            let raw = nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
            nir::verify_program(&raw).unwrap();
            let optimized = nir::optimize_program(&raw).unwrap();
            let ops = optimized
                .routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .collect::<Vec<_>>();
            if case == "known_some" {
                assert!(
                    !ops.iter().any(|op| matches!(
                        op,
                        NirOp::Compare { .. }
                            | NirOp::Call {
                                callee: NirCallee::Fault(_),
                                ..
                            }
                    )),
                    "known tag must eliminate matching and fault paths"
                );
            } else if case == "dynamic_variant" {
                assert!(
                    ops.iter().any(|op| matches!(
                        op,
                        NirOp::Call {
                            callee: NirCallee::Fault(_),
                            ..
                        }
                    )),
                    "dynamic selector must retain tag validation"
                );
            }
            let path = work.0.join(format!("{case}-{form}.act"));
            std::fs::write(&path, source).unwrap();
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let compiled = compile_file(
                        &path,
                        &CompileOptions::for_mode(mode)
                            .with_runtime(runtime)
                            .with_origin(0x3000),
                    )
                    .unwrap();
                    let mut cycles = Vec::new();
                    for input in [0, 1, 127, 128, 254, 255] {
                        for flag in [0, 1] {
                            cycles.push(execute(
                                compiled.object_bytes(),
                                runtime,
                                case,
                                input,
                                flag,
                            ));
                        }
                    }
                    println!(
                        "{case},{form},{mode:?},{runtime:?},{},{},{}",
                        compiled.object_bytes().len(),
                        cycles.iter().min().unwrap(),
                        cycles.iter().max().unwrap()
                    );
                }
            }
        }
    }
}
