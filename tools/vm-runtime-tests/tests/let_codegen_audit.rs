//! Reproducible LET/mutable-local code-quality audit, not a performance golden.
//! Set ACTIONC_LET_AUDIT_DIR to a fresh directory to retain sources, IR, listings
//! and measurements. Correctness is asserted; byte/cycle counts are reported.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::mir6502::{self, Mir6502Config, MirPhase};
use actionc::nir::{self, NirProgram, NirStorageId};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

const COMMON: &str = "BYTE input=$680,flag=$690,result=$600,side=$602,done=$67F\nCARD word=$684,wide=$604\nVOLATILE BYTE deviceValue=$691\n";

struct Case {
    name: &'static str,
    globals: &'static str,
    locals: &'static str,
    mutable: &'static str,
    binding: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "byte_once",
        globals: "",
        locals: "BYTE value",
        mutable: "value=input+3\nresult=value",
        binding: "LET value=input+3\nresult=value",
    },
    Case {
        name: "byte_chain",
        globals: "",
        locals: "BYTE value",
        mutable: "value=input\nvalue=value+3\nvalue=value XOR $5A\nresult=value",
        binding: "LET value=input\nLET value=value+3\nLET value=value XOR $5A\nresult=value",
    },
    Case {
        name: "word_chain",
        globals: "",
        locals: "CARD value",
        mutable: "value=word\nvalue=value+257\nvalue=value XOR $1234\nwide=value",
        binding: "LET value=word\nLET value=value+257\nLET value=value XOR $1234\nwide=value",
    },
    Case {
        name: "table_relay",
        globals: "BYTE ARRAY table(16)=[15 14 13 12 11 10 9 8 7 6 5 4 3 2 1 0]\nBYTE POINTER ptr=[$680]",
        locals: "BYTE value",
        mutable: "value=ptr(0)\nvalue=table(value)\nptr(0)=value",
        binding: "LET value=ptr(0)\nLET value=table(value)\nptr(0)=value",
    },
    Case {
        name: "indexed_table_relay",
        globals: "BYTE ARRAY table(16)=[15 14 13 12 11 10 9 8 7 6 5 4 3 2 1 0]\nBYTE ARRAY data=$680",
        locals: "BYTE value",
        mutable: "value=target(index)\nvalue=table(value)\ntarget(index)=value",
        binding: "LET value=target(index)\nLET value=table(value)\ntarget(index)=value",
    },
    Case {
        name: "branch_snapshot",
        globals: "",
        locals: "BYTE value",
        mutable: "value=input\nvalue=value+3\nIF flag THEN side=value+1 FI\nresult=value XOR $5A",
        binding: "LET value=input\nLET value=value+3\nIF flag THEN side=value+1 FI\nresult=value XOR $5A",
    },
    Case {
        name: "call_snapshot",
        // The opaque body prevents leaf inlining and changes observable RAM.
        globals: "PROC Touch()\n[ $EE $02 $06 ]\nRETURN",
        locals: "BYTE value",
        mutable: "value=input\nvalue=value+3\nTouch()\nresult=value XOR $5A",
        binding: "LET value=input\nLET value=value+3\nTouch()\nresult=value XOR $5A",
    },
    Case {
        name: "volatile_snapshot",
        globals: "",
        locals: "BYTE value",
        mutable: "value=input\nvalue=value+3\nside=deviceValue\nresult=value XOR $5A",
        binding: "LET value=input\nLET value=value+3\nside=deviceValue\nresult=value XOR $5A",
    },
    Case {
        name: "loop_chain",
        globals: "",
        locals: "BYTE i",
        mutable: "result=0\nFOR i=0 TO 15 DO\nBEGIN\nBYTE value\nvalue=input\nvalue=value+i\nresult==+value\nEND\nOD",
        binding: "result=0\nFOR i=0 TO 15 DO\nBEGIN\nLET value=input\nLET value=value+i\nresult==+value\nEND\nOD",
    },
];

#[derive(Clone, Copy)]
enum Form {
    Reused,
    Fresh,
    Let,
}

impl Form {
    fn name(self) -> &'static str {
        match self {
            Self::Reused => "mutable",
            Self::Fresh => "fresh",
            Self::Let => "let",
        }
    }
}

fn source(case: &Case, form: Form) -> String {
    let auxiliary_locals = if case.name == "loop_chain" {
        "BYTE i\n"
    } else {
        ""
    };
    let (locals, body) = match form {
        Form::Reused => (case.locals.to_string(), case.mutable.to_string()),
        Form::Let => (auxiliary_locals.to_string(), case.binding.to_string()),
        Form::Fresh => {
            // A third control uses distinct ordinary mutable locals, isolating
            // storage identity from LET's source immutability. These tiny fixed
            // inputs use only the bare, lower-case identifier `value`.
            let ty = if case.name == "word_chain" {
                "CARD"
            } else {
                "BYTE"
            };
            let mut locals = auxiliary_locals.to_string();
            let mut body = String::new();
            let mut previous = "value".to_string();
            let mut index = 0;
            for line in case.binding.lines() {
                if let Some(initializer) = line.strip_prefix("LET value=") {
                    let next = format!("fresh{index}");
                    locals.push_str(&format!("{ty} {next}\n"));
                    body.push_str(&format!(
                        "{next}={}\n",
                        initializer.replace("value", &previous)
                    ));
                    previous = next;
                    index += 1;
                } else {
                    body.push_str(&line.replace("value", &previous));
                    body.push('\n');
                }
            }
            (locals, body)
        }
    };
    if case.name == "indexed_table_relay" {
        return format!(
            "{COMMON}{}\nPROC Map(BYTE ARRAY target,BYTE index)\n{locals}\n{body}\nRETURN\nPROC Main()\nMap(data,flag)\ndone=$A5\nDO OD\nRETURN\n",
            case.globals
        );
    }
    format!(
        "{COMMON}{}\nPROC Main()\n{locals}\n{body}\ndone=$A5\nDO OD\nRETURN\n",
        case.globals
    )
}

struct WorkDir {
    path: PathBuf,
    retain: bool,
}

impl WorkDir {
    fn new() -> Self {
        let retained = std::env::var_os("ACTIONC_LET_AUDIT_DIR").map(PathBuf::from);
        let path = retained.clone().unwrap_or_else(|| {
            std::env::temp_dir().join(format!("actionc-let-codegen-audit-{}", std::process::id()))
        });
        if retained.is_some() {
            assert!(path.is_dir(), "create a fresh audit directory first");
            assert!(
                std::fs::read_dir(&path).unwrap().next().is_none(),
                "audit directory must be empty"
            );
        } else {
            // Only directories created by this run may be removed on drop.
            std::fs::create_dir(&path).unwrap();
        }
        Self {
            path,
            retain: retained.is_some(),
        }
    }

    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.path.join(name);
        // Never silently overwrite another run's artifacts.
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(contents.as_ref()).unwrap();
        path
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        if !self.retain {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn lower(source: &str) -> NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = actionc::semantic::ir::lower_program(&ast, &model);
    let nir = nir::lower_program(&semir);
    nir::verify_program(&nir).unwrap();
    nir
}

fn local_stats(program: &NirProgram) -> (usize, usize, usize) {
    let analysis = nir::analyze_program_storage(program);
    analysis
        .routines
        .iter()
        .flat_map(|r| r.homes.values())
        .filter(|f| matches!(f.id, NirStorageId::Local(_)))
        .fold((0, 0, 0), |(homes, loads, stores), f| {
            (homes + 1, loads + f.direct_loads, stores + f.direct_stores)
        })
}

fn new_vm(image: &[u8], runtime: Runtime) -> CompilerVm {
    let mut vm = CompilerVm::default();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let profile = if runtime == Runtime::ActionCart {
        for (kind, name, base) in [
            (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
            (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
        ] {
            vm.load_image_bytes(
                kind,
                name,
                base,
                std::fs::read(root.join("roms").join(name)).unwrap(),
            )
            .unwrap();
        }
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|s| s.end < 0x600 || s.start > 0x6FF)
    );
    vm
}

fn execute(image: &[u8], runtime: Runtime, case: &Case, seed: u16, flag: u8) -> u64 {
    let mut vm = new_vm(image, runtime);
    let input = if case.name.ends_with("table_relay") {
        seed as u8 & 15
    } else {
        seed as u8
    };
    for address in 0x600..=0x6FF {
        vm.bus_mut().ram_mut().write(address, 0);
    }
    vm.bus_mut().ram_mut().write(0x680, input);
    vm.bus_mut().ram_mut().write(0x681, input);
    vm.bus_mut().ram_mut().write_word(0x684, seed);
    vm.bus_mut().ram_mut().write(0x690, flag);
    vm.bus_mut().ram_mut().write(0x691, 0x37);
    let start = vm.cpu().cycles();
    let mut finished = false;
    for _ in 0..100_000 {
        vm.step_cpu().unwrap();
        if vm.bus().ram().read(0x67F) == 0xA5 {
            finished = true;
            break;
        }
    }
    assert!(finished, "{} did not finish", case.name);
    let cycles = vm.cpu().cycles() - start;
    let ram = vm.bus().ram();
    let chain = input.wrapping_add(3) ^ 0x5A;
    match case.name {
        "byte_once" => assert_eq!(ram.read(0x600), input.wrapping_add(3)),
        "byte_chain" => assert_eq!(ram.read(0x600), chain),
        "word_chain" => assert_eq!(ram.read_word(0x604), seed.wrapping_add(257) ^ 0x1234),
        "table_relay" => assert_eq!(ram.read(0x680), 15 - input),
        "indexed_table_relay" => {
            assert_eq!(ram.read(0x680 + u16::from(flag)), 15 - input);
            assert_eq!(ram.read(0x680 + u16::from(1 - flag)), input);
        }
        "branch_snapshot" => {
            assert_eq!(ram.read(0x600), chain);
            assert_eq!(
                ram.read(0x602),
                if flag == 0 { 0 } else { input.wrapping_add(4) }
            );
        }
        "call_snapshot" => {
            assert_eq!(ram.read(0x600), chain);
            assert_eq!(ram.read(0x602), 1);
        }
        "volatile_snapshot" => {
            assert_eq!(ram.read(0x600), chain);
            assert_eq!(ram.read(0x602), 0x37);
        }
        "loop_chain" => assert_eq!(ram.read(0x600), input.wrapping_mul(16).wrapping_add(120)),
        _ => unreachable!(),
    }
    cycles
}

#[test]
fn let_and_mutable_codegen_audit() {
    let work = WorkDir::new();
    let mut report = String::from(
        "case,form,backend,runtime,xex_bytes,min_cycles,max_cycles,raw_homes,raw_loads,raw_stores,opt_homes,opt_loads,opt_stores\n",
    );
    let mut executions = String::from("case,form,backend,runtime,seed,flag,cycles\n");
    for case in CASES {
        for variant in [Form::Reused, Form::Fresh, Form::Let] {
            let form = variant.name();
            let stem = format!("{}-{form}", case.name);
            let source = source(case, variant);
            let path = work.write(&format!("{stem}.act"), &source);
            let raw = lower(&source);
            let optimized = nir::optimize_program(&raw).unwrap();
            let before = local_stats(&raw);
            let after = local_stats(&optimized);
            work.write(&format!("{stem}.nir"), nir::format_program(&raw));
            work.write(
                &format!("{stem}.optimized.nir"),
                nir::format_program(&optimized),
            );
            work.write(
                &format!("{stem}.storage.txt"),
                format!(
                    "RAW\n{:#?}\nOPTIMIZED\n{:#?}",
                    nir::analyze_program_storage(&raw),
                    nir::analyze_program_storage(&optimized)
                ),
            );
            let mir = mir6502::lower_program(&optimized).unwrap();
            mir6502::verify_program(&mir, MirPhase::PreMaterialization).unwrap();
            work.write(&format!("{stem}.mir6502"), mir6502::format_program(&mir));
            let mir = mir6502::materialize_program_with_origin_and_runtime(
                mir,
                &Mir6502Config::optimized(),
                0x3000,
                Runtime::ActionCart,
            )
            .unwrap();
            mir6502::verify_program(&mir, MirPhase::PreEmission).unwrap();
            work.write(
                &format!("{stem}.materialized.mir6502"),
                mir6502::format_program(&mir),
            );
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let backend = if mode == CompileMode::Mir6502 {
                    "mir6502"
                } else {
                    "classic"
                };
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let runtime_name = if runtime == Runtime::ActionCart {
                        "cart"
                    } else {
                        "standalone"
                    };
                    let compiled =
                        compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime))
                            .unwrap_or_else(|e| panic!("{stem}/{backend}/{runtime_name}: {e}"));
                    let stem = format!("{stem}-{backend}-{runtime_name}");
                    work.write(&format!("{stem}.asm"), compiled.source_listing());
                    work.write(&format!("{stem}.xex"), compiled.object_bytes());
                    let mut cycles = Vec::new();
                    for seed in [0u16, 1, 7, 15, 0x7FFF, 0xFFFF] {
                        for flag in [0u8, 1] {
                            let cost = execute(compiled.object_bytes(), runtime, case, seed, flag);
                            cycles.push(cost);
                            executions.push_str(&format!(
                                "{},{form},{backend},{runtime_name},{seed},{flag},{cost}\n",
                                case.name
                            ));
                        }
                    }
                    let line = format!(
                        "{},{form},{backend},{runtime_name},{},{},{},{},{},{},{},{},{}\n",
                        case.name,
                        compiled.object_bytes().len(),
                        cycles.iter().min().unwrap(),
                        cycles.iter().max().unwrap(),
                        before.0,
                        before.1,
                        before.2,
                        after.0,
                        after.1,
                        after.2
                    );
                    eprint!("{line}");
                    report.push_str(&line);
                }
            }
        }
    }
    work.write("measurements.csv", report);
    work.write("executions.csv", executions);
}
