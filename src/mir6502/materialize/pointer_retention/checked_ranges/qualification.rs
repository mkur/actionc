//! Explicit benchmark bindings, separate from historical unchecked scorecards.
use super::*;
use crate::{
    codegen::{self, CodegenOutput, CodegenSymbolScope},
    compiler::{CompileMode, CompileOptions, Runtime, compile_file},
    nir,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}
fn source_nir(path: &Path) -> nir::NirProgram {
    let loaded = crate::includes::load_compilation(path, &Default::default()).unwrap();
    let model = crate::semantic::analyze_compilation_with_options(
        &loaded,
        crate::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let semir = crate::semantic::ir::lower_compilation(&loaded, &model);
    let linked =
        crate::linker::select_semir(&semir, crate::linker::SemLinkPolicy::EntryReachable).unwrap();
    nir::lower_program(&linked)
}
fn facts(n: &nir::NirProgram) -> Value {
    let started = Instant::now();
    let ranges = nir::analyze_unsigned_ranges(n).unwrap();
    let elapsed = started.elapsed().as_secs_f64() * 1000.;
    let strcat = n.routines.iter().find(|r| r.name == "Strcat").unwrap();
    let mut calls = Vec::new();
    for r in &n.routines {
        for b in &r.blocks {
            for (index, op) in b.ops.iter().enumerate() {
                if let nir::NirOp::Call {
                    callee: nir::NirCallee::User { id, .. },
                    args,
                    ..
                } = op
                {
                    if *id == strcat.id {
                        calls.push(json!({"caller":r.name,"block":b.id.0,"op":index,"args":args.iter().map(|v|ranges.prove_before(r.id,b.id,index,v).map(|p|[p.range().min(),p.range().max()])).collect::<Vec<_>>(),"admission":"original Strcat entry has no pointer bounds; no private clones"}));
                    }
                }
            }
        }
    }
    let returns = ["ArenaCursor", "Alloc"].map(|name| {
        let r = n.routines.iter().find(|r| r.name == name).unwrap();
        (
            name,
            ranges.normal_return_range(r.id).map(|v| [v.min(), v.max()]),
        )
    });
    json!({"evaluations":n.routines.iter().filter_map(|r|ranges.evaluations(r.id)).sum::<usize>(),"routines":n.routines.len(),"range_analysis_ms":elapsed,"return_ranges":returns,"strcat_entry_cells":ranges.storage_ranges_at_entry(strcat.id,strcat.blocks[0].id).len(),"strcat_calls":calls})
}
fn row(
    out: &CodegenOutput,
    name: &str,
    mode: &str,
    enabled: bool,
    elapsed: f64,
    dir: &Path,
) -> Value {
    let runtime = if out.map.runtime == Runtime::ActionCart {
        "cart"
    } else {
        "standalone"
    };
    let stem = format!("checked-{name}-{mode}-{runtime}-{enabled}");
    let image = codegen::format_load_file(out);
    std::fs::write(dir.join(format!("{stem}.xex")), &image).unwrap();
    std::fs::write(
        dir.join(format!("{stem}.asm")),
        crate::compiler::artifacts::format_listing_with_source(out, ""),
    )
    .unwrap();
    let mut frames = BTreeMap::<String, u32>::new();
    let mut zp = BTreeSet::new();
    for s in &out.map.storage_symbols {
        if let CodegenSymbolScope::Routine(r) = &s.scope {
            *frames.entry(r.clone()).or_default() += u32::from(s.size);
        }
        for a in u32::from(s.address)..u32::from(s.address) + u32::from(s.size) {
            if a < 256 {
                zp.insert(a);
            }
        }
    }
    json!({"stem":stem,"name":name,"mode":mode,"runtime":runtime,"range_enabled":enabled,"origin":out.origin,"emitted_bytes":out.bytes.len(),"xex_bytes":image.len(),"compile_ms":elapsed,"routine_bytes":out.map.routine_ranges.iter().map(|r|u32::from(r.end-r.start)).sum::<u32>(),"routines":out.map.routine_ranges.iter().map(|r|json!({"name":r.name,"start":r.start,"end":r.end})).collect::<Vec<_>>(),"frame_bytes":frames,"reservations":out.skipped_ranges.iter().map(|s|[s.start,s.len]).collect::<Vec<_>>(),"source_storage_zero_page":zp})
}
#[test]
#[ignore = "run tools/compare6502/tn/checked_ranges.py to generate/check fresh bindings"]
fn fresh_checked_range_measurements() {
    let dir = PathBuf::from(
        std::env::var_os("ACTIONC_CHECKED_RANGE_OUTPUT").expect("measurement output directory"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let mut rows = Vec::new();
    let mut analyses = Vec::new();
    for name in ["TN", "TNDBG"] {
        let path = root().join(format!("samples/tn/modern/{name}.ACT"));
        let raw = source_nir(&path);
        let n = nir::optimize_program(&raw).unwrap();
        analyses.push(json!({"name":name,"raw":facts(&raw),"optimized":facts(&n)}));
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let started = Instant::now();
            let classic = compile_file(
                &path,
                &CompileOptions::for_mode(CompileMode::Optimized).with_runtime(runtime),
            )
            .unwrap();
            rows.push(row(
                &classic.output,
                name,
                "optimized",
                false,
                started.elapsed().as_secs_f64() * 1000.,
                &dir,
            ));
            let mut baseline = None;
            for enabled in [false, true] {
                let config = Mir6502Config {
                    enable_checked_range_pointer_retention: enabled,
                    ..Mir6502Config::optimized()
                };
                let started = Instant::now();
                let out = crate::mir6502::generate_output_with_config_and_runtime(
                    &n, 0x2C00, &config, runtime,
                )
                .unwrap();
                let elapsed = started.elapsed().as_secs_f64() * 1000.;
                if let Some(old) = &baseline {
                    assert_eq!(old, &out, "TN original entries must remain unrestricted");
                } else {
                    let public = compile_file(
                        &path,
                        &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
                    )
                    .unwrap();
                    assert_eq!(codegen::format_load_file(&out), public.object_bytes());
                    baseline = Some(out.clone());
                }
                rows.push(row(&out, name, "mir6502", enabled, elapsed, &dir));
            }
        }
    }
    std::fs::write(
        dir.join("measurements.json"),
        serde_json::to_vec_pretty(&json!({"configurations":rows,"analyses":analyses})).unwrap(),
    )
    .unwrap();
}

fn neutral(rt: Runtime, origin: u16) -> CodegenOutput {
    let text = include_str!("../../../../../fixtures/runtime/checked_pointer_ranges.act");
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let n = nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let n = nir::optimize_program(&n).unwrap();
    crate::mir6502::generate_output_with_config_and_runtime(
        &n,
        origin,
        &Mir6502Config {
            enable_checked_range_pointer_retention: true,
            ..Mir6502Config::optimized()
        },
        rt,
    )
    .unwrap()
}
#[test]
#[ignore = "requires MADS; assembles actual LF/CRLF listings and relocates enabled images"]
fn checked_range_mads_relocation() {
    let dir = root().join(format!("build/checked-arena/mads-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for rt in [Runtime::ActionCart, Runtime::Standalone] {
        let original = neutral(rt, 0x3000);
        for origin in [0x3000, 0x41C7] {
            let expected = neutral(rt, origin);
            let listing = crate::compiler::artifacts::format_listing_with_source(&original, "")
                .replace("\r\n", "\n")
                .replace(
                    "ACTIONC_ORIGIN = $3000",
                    &format!("ACTIONC_ORIGIN = ${origin:04X}"),
                );
            for crlf in [false, true] {
                let stem = format!("{rt:?}-{origin:04X}-{crlf}");
                let asm = dir.join(format!("{stem}.asm"));
                let xex = dir.join(format!("{stem}.xex"));
                std::fs::write(
                    &asm,
                    if crlf {
                        listing.replace('\n', "\r\n")
                    } else {
                        listing.clone()
                    },
                )
                .unwrap();
                let result = std::process::Command::new(
                    std::env::var_os("ACTIONC_MADS").unwrap_or_else(|| "mads".into()),
                )
                .arg(&asm)
                .arg(format!("-o:{}", xex.display()))
                .arg("-s")
                .output()
                .unwrap();
                assert!(
                    result.status.success(),
                    "{} {}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    std::fs::read(xex).unwrap(),
                    codegen::format_load_file(&expected)
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn whole_image_cost_rejects_pair_savings_cancelled_by_data() {
    use crate::mir6502::ir::{MirDataImage, MirGlobal, MirGlobalBacking, MirGlobalInit};
    let text = include_str!("../../../../../fixtures/runtime/checked_pointer_ranges.act");
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let n = nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let lower = crate::mir6502::lower_program(&n).unwrap();
    let baseline = crate::mir6502::materialize_program_with_origin_and_runtime(
        lower.clone(),
        &Mir6502Config::optimized(),
        0x3000,
        Runtime::ActionCart,
    )
    .unwrap();
    let ranges = MirAddressRanges::project(&n, &lower, &baseline).unwrap();
    let map = memory(&baseline, &baseline, 0x3000).unwrap();
    let (mut proposed, affected) = apply_ranges(&baseline, &baseline, &map, &ranges).unwrap();
    assert_eq!(affected.len(), 2);
    assert!(profitable(&baseline, &proposed, &affected, 0x3000));
    assert!(!profitable(&baseline, &baseline, &affected, 0x3000));
    let offset = proposed
        .globals
        .iter()
        .filter_map(|g| {
            if let MirGlobalBacking::Ordinary { offset } = g.backing {
                Some(offset + g.storage_size)
            } else {
                None
            }
        })
        .max()
        .unwrap();
    let id = crate::nir::SymbolId(proposed.globals.iter().map(|g| g.id.0).max().unwrap() + 1);
    proposed.globals.push(MirGlobal {
        id,
        name: "cost-padding".into(),
        kind: "BYTE ARRAY".into(),
        width: None,
        storage_size: 40,
        backing: MirGlobalBacking::Ordinary { offset },
        init: Some(MirGlobalInit::Bytes {
            image: MirDataImage {
                bytes: vec![0xEA; 40],
                relocations: vec![],
            },
            zero_fill: 0,
            mutable: true,
            section: "data".into(),
            array: None,
        }),
    });
    assert_eq!(
        address_values::Cost::measure(&baseline, 0x3000)
            .unwrap()
            .bytes,
        address_values::Cost::measure(&proposed, 0x3000)
            .unwrap()
            .bytes
    );
    assert!(!profitable(&baseline, &proposed, &affected, 0x3000));
}
