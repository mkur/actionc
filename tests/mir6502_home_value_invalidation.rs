use actionc::{
    codegen,
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use std::{collections::BTreeMap, path::PathBuf};

#[test]
fn home_value_invalidation_source_controls_have_stable_images_and_real_word_snapshots() {
    let lf = include_str!("../fixtures/runtime/home_value_invalidation.act").replace("\r\n", "\n");
    let output = std::env::var_os("ACTIONC_HOME_VALUE_OUTPUT").map(PathBuf::from);
    if let Some(path) = &output {
        std::fs::create_dir_all(path).unwrap();
    }
    let mut images = BTreeMap::new();
    for crlf in [false, true] {
        let text = if crlf {
            lf.replace('\n', "\r\n")
        } else {
            lf.clone()
        };
        let ast = actionc::parser::parse(&actionc::lexer::tokenize(&text).unwrap()).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &ast,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
        actionc::nir::verify_program(&raw).unwrap();
        for opt in [false, true] {
            let nir = if opt {
                actionc::nir::optimize_program(&raw).unwrap()
            } else {
                raw.clone()
            };
            let lower = mir6502::lower_program(&nir).unwrap();
            let check = lower
                .routines
                .iter()
                .find(|r| r.name == "CheckRoom")
                .unwrap();
            assert!(check.blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(
                op,
                mir6502::MirOp::Load {
                    width: mir6502::MirWidth::Word,
                    ..
                }
            )));
            for rt in [Runtime::ActionCart, Runtime::Standalone] {
                for origin in [0x3000, 0x41C7] {
                    let out = mir6502::generate_output_with_config_and_runtime(
                        &nir,
                        origin,
                        &Mir6502Config::optimized(),
                        rt,
                    )
                    .unwrap();
                    let stem = format!("neutral-{opt}-{rt:?}-{origin:04X}");
                    let image = codegen::format_load_file(&out);
                    // The fixture was frozen before selective invalidation:
                    // its ordinary saved-word comparison needs no second copy.
                    for (name, bytes) in [
                        ("CheckRoom", 89),
                        ("ChangedSource", 68),
                        ("ChangedHigh", 68),
                    ] {
                        let range = out
                            .map
                            .routine_ranges
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap();
                        assert_eq!(range.end - range.start, bytes, "{stem}: {name}");
                    }
                    assert_eq!(image.len(), if opt { 411 } else { 402 }, "{stem}");
                    if crlf {
                        assert_eq!(images[&stem], image);
                    } else {
                        images.insert(stem.clone(), image.clone());
                        if let Some(dir) = &output {
                            std::fs::write(dir.join(format!("{stem}.xex")), &image).unwrap();
                            let data = serde_json::json!({"emitted_bytes":out.bytes.len(),"xex_bytes":image.len(),"routines":out.map.routine_ranges.iter().map(|r|(&r.name,r.start,r.end)).collect::<Vec<_>>(),"storage":out.map.storage_symbols.iter().map(|s|(format!("{:?}",s.scope),&s.name,s.address,s.size)).collect::<Vec<_>>(),"reservations":out.skipped_ranges.iter().map(|s|(s.start,s.len)).collect::<Vec<_>>()});
                            std::fs::write(
                                dir.join(format!("{stem}.json")),
                                serde_json::to_vec_pretty(&data).unwrap(),
                            )
                            .unwrap();
                        }
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn home_value_invalidation_mads_listings_reconstruct_and_relocate_with_lf_and_crlf() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/home_value_invalidation.act");
    let dir = std::env::temp_dir().join(format!("actionc-home-value-mads-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let outputs = [0x3000, 0x41C7].map(|origin| {
            compile_file(
                &source,
                &CompileOptions::for_mode(CompileMode::Mir6502)
                    .with_origin(origin)
                    .with_runtime(runtime),
            )
            .unwrap()
        });
        for (i, origin) in [0x3000, 0x41C7].into_iter().enumerate() {
            let listing = outputs[0].source_listing().replace("\r\n", "\n").replace(
                "ACTIONC_ORIGIN = $3000",
                &format!("ACTIONC_ORIGIN = ${origin:04X}"),
            );
            for crlf in [false, true] {
                let stem = format!("{runtime:?}-{origin:04X}-{crlf}");
                let asm = dir.join(format!("{stem}.asm"));
                let image = dir.join(format!("{stem}.xex"));
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
                .arg(format!("-o:{}", image.display()))
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
                    std::fs::read(image).unwrap(),
                    outputs[i].object_bytes(),
                    "{stem}"
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
