//! Read-only slice-1 cost baseline. Whole watched-memory oracle; raw/optimized
//! MIR plus classic, both runtimes. Logical and ABI-expanded counts are separate.
#[path = "../../../tests/support/aggregate_forwarding_cases.rs"]
mod cases;

use actionc::{
    codegen,
    compiler::Runtime,
    nir,
    semantic::{self, SemanticOptions},
};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind};

fn execute(image: &[u8], runtime: Runtime, case: &str, seed: u8) -> u64 {
    let mut vm = CompilerVm::default();
    let profile = if runtime == Runtime::ActionCart {
        vm.load_image_bytes(
            ImageKind::Cartridge,
            "action.rom",
            DEFAULT_CART_BASE,
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roms/action.rom"),
            )
            .unwrap(),
        )
        .unwrap();
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
    for address in 0x600..=0x6FF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    vm.bus_mut().ram_mut().write(0x690, seed);
    let start = vm.cpu().cycles();
    let mut completed = false;
    for _ in 0..100_000 {
        vm.step_cpu().unwrap();
        if vm.bus().ram().read(0x67F) == 0xA5 {
            completed = true;
            break;
        }
    }
    assert!(completed, "{case}/{runtime:?}/{seed}: bounded completion");
    let mut expected = vec![0xCC; 0x100];
    expected[..4].copy_from_slice(&cases::expected(case, seed));
    expected[0x7F] = 0xA5;
    expected[0x90] = seed;
    let actual: Vec<_> = (0x600..=0x6FF).map(|a| vm.bus().ram().read(a)).collect();
    assert_eq!(actual, expected, "{case}/{runtime:?}/{seed}");
    vm.cpu().cycles() - start
}

fn logical_counts(program: &nir::NirProgram) -> (usize, u64, u64) {
    let copies: Vec<_> = program
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .filter_map(|op| {
            if let nir::NirOp::CopyBytes { size, .. } = op {
                Some(u64::from(size.get()))
            } else {
                None
            }
        })
        .collect();
    let capture_bytes = program
        .routines
        .iter()
        .flat_map(|r| &r.locals)
        .filter(|l| l.purpose == nir::NirLocalPurpose::AggregateCapture)
        .map(|l| u64::from(l.layout.size.get()))
        .sum();
    (copies.len(), copies.iter().sum(), capture_bytes)
}

#[test]
fn private_aggregate_baseline_preserves_snapshots_and_ordered_arguments() {
    eprintln!(
        "shape,case,backend,runtime,xex_bytes,min_cycles,max_cycles,raw_copy_sites,opt_copy_sites,raw_copied_bytes,opt_copied_bytes,raw_capture_bytes,opt_capture_bytes,initial_mir_copy_sites"
    );
    for shape in cases::SHAPES {
        for case in cases::CASES {
            let source = cases::source(shape, case);
            let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
            let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
            let semir = semantic::ir::lower_program(&ast, &model);
            let raw = nir::lower_program(&semir);
            nir::verify_program(&raw).unwrap();
            let optimized = nir::optimize_program(&raw).unwrap();
            let (raw_sites, raw_bytes, raw_captures) = logical_counts(&raw);
            let (opt_sites, opt_bytes, opt_captures) = logical_counts(&optimized);
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                for lane in ["classic", "mir-raw", "mir-optimized"] {
                    let (output, physical) = if lane == "classic" {
                        (
                            codegen::generate_semir_profile_at_origin_with_runtime(
                                &semir,
                                0x3000,
                                codegen::CodegenProfile::Modern,
                                runtime,
                            )
                            .unwrap(),
                            "n/a".to_owned(),
                        )
                    } else {
                        let program = if lane == "mir-raw" { &raw } else { &optimized };
                        let initial = actionc::mir6502::lower_program(program).unwrap();
                        let physical = initial
                            .routines
                            .iter()
                            .flat_map(|r| &r.blocks)
                            .flat_map(|b| &b.ops)
                            .filter(|op| matches!(op, actionc::mir6502::MirOp::CopyBytes { .. }))
                            .count();
                        (
                            actionc::mir6502::generate_output_with_config_and_runtime(
                                program,
                                0x3000,
                                &actionc::mir6502::Mir6502Config::optimized(),
                                runtime,
                            )
                            .unwrap(),
                            physical.to_string(),
                        )
                    };
                    let image = codegen::format_load_file(&output);
                    let cycles: Vec<_> = [0, 1, 127, 255]
                        .into_iter()
                        .map(|seed| execute(&image, runtime, case, seed))
                        .collect();
                    eprintln!(
                        "{shape},{case},{lane},{runtime:?},{},{},{},{raw_sites},{opt_sites},{raw_bytes},{opt_bytes},{raw_captures},{opt_captures},{physical}",
                        image.len(),
                        cycles.iter().min().unwrap(),
                        cycles.iter().max().unwrap()
                    );
                }
            }
        }
    }
}
