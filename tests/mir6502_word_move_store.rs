use actionc::{
    codegen,
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use std::collections::BTreeMap;

#[test]
fn word_move_store_keeps_typed_casts_and_eliminates_private_result_homes() {
    let lf = include_str!("../fixtures/runtime/word_move_store.act").replace("\r\n", "\n");
    let mut images = BTreeMap::new();
    for crlf in [false, true] {
        let source = if crlf {
            lf.replace('\n', "\r\n")
        } else {
            lf.clone()
        };
        let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &ast,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
        actionc::nir::verify_program(&raw).unwrap();
        // The optimization belongs after typed NIR lowering. The explicit
        // integer-to-pointer cast remains in the source-language contract.
        assert!(actionc::nir::format_program(&raw).contains("cast"));
        for opt in [false, true] {
            let nir = if opt {
                actionc::nir::optimize_program(&raw).unwrap()
            } else {
                raw.clone()
            };
            for rt in [Runtime::ActionCart, Runtime::Standalone] {
                for origin in [0x3000, 0x41C7] {
                    let config = Mir6502Config {
                        enable_small_leaf_inlining: false,
                        ..Mir6502Config::optimized()
                    };
                    let p = mir6502::materialize_program_with_origin_and_runtime(
                        mir6502::lower_program(&nir).unwrap(),
                        &config,
                        origin,
                        rt,
                    )
                    .unwrap();
                    mir6502::verify_program(&p, mir6502::MirPhase::PreEmission).unwrap();
                    for name in ["Add", "Subtract", "Chained"] {
                        let r = p.routines.iter().find(|r| r.name == name).unwrap();
                        assert!(r.frame.spills.is_empty(), "{name}: {:?}", r.frame);
                        assert!(
                            r.frame.virtual_zero_page.is_empty(),
                            "{name}: {:?}",
                            r.frame
                        );
                        assert_eq!(
                            r.blocks
                                .iter()
                                .flat_map(|b| &b.ops)
                                .filter(|op| matches!(op, mir6502::MirOp::Store { .. }))
                                .count(),
                            2,
                            "{name}"
                        );
                    }
                    let out =
                        mir6502::generate_output_with_config_and_runtime(&nir, origin, &config, rt)
                            .unwrap();
                    let key = format!("{opt}/{rt:?}/{origin:04X}");
                    let image = codegen::format_load_file(&out);
                    if crlf {
                        assert_eq!(images[&key], image);
                    } else {
                        images.insert(key, image);
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn word_move_store_mads_listings_reconstruct_and_relocate_with_lf_and_crlf() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/word_move_store.act");
    let dir = std::env::temp_dir().join(format!(
        "actionc-word-move-store-mads-{}",
        std::process::id()
    ));
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
