//! Complete-program ADT versus handwritten tagged-record costs. Correctness
//! and bounded completion are gates; measured costs are reported, not hidden
//! behind an assertion that every ADT must be zero-cost.
use actionc::{
    compiler::{CompileMode, CompileOptions, Runtime, compile_file},
    nir::{self, NirLocalPurpose, NirOp, NirPlaceKind, NirValue},
};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};

struct WorkDir {
    path: PathBuf,
    retain: bool,
}
impl WorkDir {
    fn new() -> Self {
        let retained = std::env::var_os("ACTIONC_ADT_AUDIT_DIR").map(PathBuf::from);
        let path = retained.clone().unwrap_or_else(|| {
            std::env::temp_dir().join(format!("actionc-adt-audit-{}", std::process::id()))
        });
        if retained.is_some() {
            assert!(
                path.is_dir() && std::fs::read_dir(&path).unwrap().next().is_none(),
                "create a fresh empty audit directory first"
            );
        } else {
            std::fs::create_dir(&path).unwrap();
        }
        Self {
            path,
            retain: retained.is_some(),
        }
    }
    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        use std::io::Write;
        let path = self.path.join(name);
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

fn source(case: &str, adt: bool) -> String {
    let common = "BYTE ARRAY output=$600\nBYTE done=$67F,seed=$680,flag=$681,calls=$602\n";
    let event = if adt {
        "TYPE Event=VARIANT [NONE KEY [BYTE code] MOVE [INT x,y]] Event current\n"
    } else {
        "TYPE Event=[BYTE tag INT x,y] Event current\n"
    };
    let validate_event = "IF saved.tag<1 OR saved.tag>3 THEN Error(105,0,105) DO OD FI\n";
    let (types, routines, body): (&str, &str, String) = match (case, adt) {
        ("event_dispatch", true) => (
            event,
            "",
            r#"
 IF flag THEN current=Event.KEY(seed) ELSE current=Event.MOVE(seed,3) FI
 CASE current OF
 WHEN Event.KEY(code) IF code<32 THEN
   output(0)=code+1
 WHEN Event.KEY(code) THEN
   output(0)=code+2
 WHEN Event.MOVE(x,y) THEN
   output(0)=BYTE(x+y)
 WHEN Event.NONE THEN
   output(0)=0
 ESAC
"#
            .into(),
        ),
        ("event_dispatch", false) => (
            event,
            "",
            format!(
                r#"
 current.x=seed
 IF flag THEN current.tag=2 current.y=0 ELSE current.tag=3 current.y=3 FI
 LET saved=current
 {validate_event}
 CASE saved.tag OF
 WHEN 2 IF BYTE(saved.x)<32 THEN
   output(0)=BYTE(saved.x)+1
 WHEN 2 THEN
   output(0)=BYTE(saved.x)+2
 WHEN 3 THEN
   output(0)=BYTE(saved.x+saved.y)
 WHEN 1 THEN
   output(0)=0
 ESAC
"#
            ),
        ),
        ("guard_snapshot", true) => (
            event,
            "BYTE FUNC Reject() calls==+1 current=Event.NONE RETURN(0)\n",
            r#"
 current=Event.KEY(seed)
 CASE current OF
 WHEN Event.KEY(code) IF Reject() THEN
   output(0)=255
 WHEN Event.KEY(code) THEN
   output(0)=code
 ELSE
   output(0)=254
 ESAC
"#
            .into(),
        ),
        ("guard_snapshot", false) => (
            event,
            "BYTE FUNC Reject() calls==+1 current.tag=1 current.x=0 current.y=0 RETURN(0)\n",
            format!(
                r#"
 current.tag=2 current.x=seed current.y=0
 LET saved=current
 {validate_event}
 CASE saved.tag OF
 WHEN 2 IF Reject() THEN
   output(0)=255
 WHEN 2 THEN
   output(0)=BYTE(saved.x)
 ELSE
   output(0)=254
 ESAC
"#
            ),
        ),
        ("nested_result", true) => (
            "TYPE Option<T>=VARIANT [NONE SOME [T value]] TYPE Result<T,E>=VARIANT [OK [T value] ERROR [E error]]\n",
            "Result<Option<BYTE>,BYTE> FUNC Make(BYTE value,mode) IF mode THEN RETURN(Result<Option<BYTE>,BYTE>.OK(Option<BYTE>.SOME(value))) FI RETURN(Result<Option<BYTE>,BYTE>.ERROR(value))\n",
            r#"
 CASE Make(seed,flag) OF
 WHEN Result<Option<BYTE>,BYTE>.OK(Option<BYTE>.SOME(n)) IF n<32 THEN
   output(0)=n+1
 WHEN Result<Option<BYTE>,BYTE>.OK(Option<BYTE>.SOME(n)) THEN
   output(0)=n+2
 WHEN Result<Option<BYTE>,BYTE>.OK(Option<BYTE>.NONE) THEN
   output(0)=0
 WHEN Result<Option<BYTE>,BYTE>.ERROR(code) THEN
   output(0)=code XOR 5
 ESAC
"#
            .into(),
        ),
        ("nested_result", false) => (
            "TYPE Packet=[BYTE tag,inner_tag,value]\n",
            // The same packed three-byte layout: ERROR's byte overlaps the
            // nested Option tag, and its remaining byte is zero padding.
            "Packet FUNC Make(BYTE value,mode) Packet result IF mode THEN result.tag=1 result.inner_tag=2 result.value=value ELSE result.tag=2 result.inner_tag=value result.value=0 FI RETURN(result)\n",
            r#"
 LET saved=Make(seed,flag)
 IF saved.tag<1 OR saved.tag>2 THEN Error(105,0,105) DO OD FI
 IF saved.tag=1 THEN
   IF saved.inner_tag<1 OR saved.inner_tag>2 THEN Error(105,0,105) DO OD FI
 FI
 CASE saved.tag OF
 WHEN 1 THEN
   CASE saved.inner_tag OF
   WHEN 2 IF saved.value<32 THEN
     output(0)=saved.value+1
   WHEN 2 THEN
     output(0)=saved.value+2
   WHEN 1 THEN
     output(0)=0
   ESAC
 WHEN 2 THEN
   output(0)=saved.inner_tag XOR 5
 ESAC
"#
            .into(),
        ),
        _ => unreachable!(),
    };
    format!("{types}{common}{routines}PROC Main()\ncalls=0\n{body}\ndone=$A5\nDO OD\nRETURN\n")
}

#[derive(Debug)]
struct Metrics {
    local_bytes: u64,
    capture_bytes: u64,
    copy_sites: usize,
    copy_bytes: u64,
    tag_compare_sites: usize,
    compares: usize,
    fault_sites: usize,
}

fn metrics(program: &nir::NirProgram, nested_manual: bool) -> Metrics {
    let stats = nir::collect_program_stats(program);
    let mut result = Metrics {
        local_bytes: stats.storage.locals.bytes,
        capture_bytes: 0,
        copy_sites: 0,
        copy_bytes: 0,
        tag_compare_sites: 0,
        compares: 0,
        fault_sites: 0,
    };
    for routine in &program.routines {
        result.capture_bytes += routine
            .locals
            .iter()
            .filter(|l| l.purpose == NirLocalPurpose::AggregateCapture)
            .map(|l| u64::from(l.layout.size))
            .sum::<u64>();
        // This audit's types reserve byte offset 0 for tags. The handwritten
        // nested Packet additionally reserves offset 1 for its inline tag.
        // Count direct tag-load comparisons, not loop-iteration/dynamic checks.
        let tag_temps: std::collections::BTreeSet<_> = routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|op| {
                if let NirOp::Load { dest, place, .. } = op {
                    if let NirPlaceKind::Field { offset, ty, .. } = &place.kind {
                        if ty.kind == nir::NirTypeKind::U8
                            && (offset.get() == 0 || (nested_manual && offset.get() == 1))
                        {
                            return Some(*dest);
                        }
                    }
                }
                None
            })
            .collect();
        for op in routine.blocks.iter().flat_map(|b| &b.ops) {
            match op {
                NirOp::CopyBytes { size, .. } => {
                    result.copy_sites += 1;
                    result.copy_bytes += u64::from(*size);
                }
                NirOp::Compare { left, right, .. } => {
                    result.compares += 1;
                    if [left, right]
                        .iter()
                        .any(|v| matches!(v, NirValue::Temp { id, .. } if tag_temps.contains(id)))
                    {
                        result.tag_compare_sites += 1;
                    }
                }
                NirOp::Call { callee, .. } => {
                    if matches!(callee, nir::NirCallee::Fault(_))
                        || matches!(callee, nir::NirCallee::Builtin(name) | nir::NirCallee::Runtime { name, .. } | nir::NirCallee::User { name, .. } if name.eq_ignore_ascii_case("Error"))
                    {
                        result.fault_sites += 1;
                    }
                }
                _ => {}
            }
        }
    }
    result
}

fn execute(image: &[u8], runtime: Runtime, case: &str, seed: u8, flag: u8) -> u64 {
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
    for address in 0x600..=0x6FF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    vm.bus_mut().ram_mut().write(0x680, seed);
    vm.bus_mut().ram_mut().write(0x681, flag);
    let start = vm.cpu().cycles();
    let mut finished = false;
    for _ in 0..100_000 {
        vm.step_cpu().unwrap();
        if vm.bus().ram().read(0x67F) == 0xA5 {
            finished = true;
            break;
        }
    }
    assert!(finished, "{case}/{runtime:?}/{seed}/{flag} did not finish");
    let mut expected = vec![0xCC; 0x100];
    expected[0] = match case {
        "event_dispatch" if flag == 0 => seed.wrapping_add(3),
        "event_dispatch" | "nested_result" if flag != 0 => {
            seed.wrapping_add(if seed < 32 { 1 } else { 2 })
        }
        "nested_result" => seed ^ 5,
        "guard_snapshot" => seed,
        _ => unreachable!(),
    };
    expected[2] = u8::from(case == "guard_snapshot");
    expected[0x7F] = 0xA5;
    expected[0x80] = seed;
    expected[0x81] = flag;
    let actual: Vec<_> = (0x600..=0x6FF).map(|a| vm.bus().ram().read(a)).collect();
    assert_eq!(actual, expected, "{case}/{runtime:?}/{seed}/{flag}");
    vm.cpu().cycles() - start
}

#[test]
fn adt_and_handwritten_tagged_record_codegen_audit() {
    let work = WorkDir::new();
    let mut report = String::from(
        "case,form,backend,runtime,xex_bytes,min_cycles,max_cycles,raw_local_bytes,opt_local_bytes,raw_capture_bytes,opt_capture_bytes,raw_copy_sites,opt_copy_sites,raw_copy_bytes,opt_copy_bytes,raw_tag_compares,opt_tag_compares,raw_compares,opt_compares,raw_fault_sites,opt_fault_sites\n",
    );
    for case in ["event_dispatch", "guard_snapshot", "nested_result"] {
        for adt in [false, true] {
            let form = if adt { "adt" } else { "manual" };
            let source = source(case, adt);
            let stem = format!("{case}-{form}");
            let path = work.write(&format!("{stem}.act"), &source);
            let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
            let model = actionc::semantic::analyze_with_options(
                &ast,
                actionc::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            let raw = nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
            nir::verify_program(&raw).unwrap();
            let optimized = nir::optimize_program(&raw).unwrap();
            let before = metrics(&raw, case == "nested_result" && !adt);
            let after = metrics(&optimized, case == "nested_result" && !adt);
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
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let compiled = compile_file(
                        &path,
                        &CompileOptions::for_mode(mode)
                            .with_runtime(runtime)
                            .with_origin(0x3000),
                    )
                    .unwrap();
                    let backend = if mode == CompileMode::Mir6502 {
                        "mir6502"
                    } else {
                        "classic"
                    };
                    let runtime_name = if runtime == Runtime::ActionCart {
                        "cart"
                    } else {
                        "standalone"
                    };
                    let output_stem = format!("{stem}-{backend}-{runtime_name}");
                    work.write(&format!("{output_stem}.xex"), compiled.object_bytes());
                    work.write(&format!("{output_stem}.asm"), compiled.source_listing());
                    let mut cycles = Vec::new();
                    for seed in [0, 7, 31, 32, 127, 255] {
                        for flag in [0, 1] {
                            cycles.push(execute(
                                compiled.object_bytes(),
                                runtime,
                                case,
                                seed,
                                flag,
                            ));
                        }
                    }
                    let line = format!(
                        "{case},{form},{backend},{runtime_name},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                        compiled.object_bytes().len(),
                        cycles.iter().min().unwrap(),
                        cycles.iter().max().unwrap(),
                        before.local_bytes,
                        after.local_bytes,
                        before.capture_bytes,
                        after.capture_bytes,
                        before.copy_sites,
                        after.copy_sites,
                        before.copy_bytes,
                        after.copy_bytes,
                        before.tag_compare_sites,
                        after.tag_compare_sites,
                        before.compares,
                        after.compares,
                        before.fault_sites,
                        after.fault_sites
                    );
                    eprint!("{line}");
                    report.push_str(&line);
                }
            }
        }
    }
    work.write("measurements.csv", report);
}
