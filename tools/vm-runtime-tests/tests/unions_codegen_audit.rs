//! Compare representation-only views with explicit aliases, and account for
//! whole-value copies separately. Every measurement has a guarded host oracle.
use actionc::{
    codegen,
    compiler::Runtime,
    nir,
    semantic::{self, SemanticOptions},
};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind};

fn source(copy: bool, union: bool) -> String {
    let common = "CARD result=$600 BYTE done=$67F,seed=$690\n";
    if copy {
        let kind = if union { "UNION " } else { "" };
        format!(
            "TYPE View={kind}[BYTE ARRAY bytes(33)] View original=$700,destination=$740 BYTE index {common}\
            PROC Main() FOR index=0 TO 32 DO original.bytes(index)=seed+index OD \
            LET saved=original original.bytes(0)=99 destination=saved result=SIZEOF(View) done=$A5 DO OD RETURN"
        )
    } else {
        let (declarations, word, low) = if union {
            (
                "TYPE View=UNION [CARD word BYTE low BYTE ARRAY bytes(3)] View value=$680",
                "value.word",
                "value.low",
            )
        } else {
            ("CARD word=$680 BYTE low=$680", "word", "low")
        };
        format!(
            "{declarations} {common} PROC Main() {word}=$1234 {low}=seed result={word} done=$A5 DO OD RETURN"
        )
    }
}

fn execute(image: &[u8], runtime: Runtime, copy: bool, seed: u8) -> u64 {
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
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|s| s.end < 0x600 || s.start > 0x7FF)
    );
    for addr in 0x600..=0x7FF {
        vm.bus_mut().ram_mut().write(addr, 0xCC);
    }
    vm.bus_mut().ram_mut().write(0x690, seed);
    let start = vm.cpu().cycles();
    for step in 0..100_000 {
        vm.step_cpu().unwrap();
        if vm.bus().ram().read(0x67F) == 0xA5 {
            break;
        }
        assert!(step < 99_999, "audit program did not finish");
    }
    let mut expected = vec![0xCC; 0x200];
    expected[0x7F] = 0xA5;
    expected[0x90] = seed;
    if copy {
        expected[..2].copy_from_slice(&[33, 0]);
        for i in 0..33 {
            expected[0x100 + i] = seed.wrapping_add(i as u8);
            expected[0x140 + i] = seed.wrapping_add(i as u8);
        }
        expected[0x100] = 99;
    } else {
        expected[..2].copy_from_slice(&[seed, 0x12]);
        expected[0x80..0x82].copy_from_slice(&[seed, 0x12]);
    }
    let actual: Vec<_> = (0x600..=0x7FF).map(|a| vm.bus().ram().read(a)).collect();
    assert_eq!(actual, expected, "{runtime:?}/copy={copy}/seed={seed}");
    vm.cpu().cycles() - start
}

#[test]
fn union_views_have_no_tag_or_clear_overhead_and_copy_costs_are_explicit() {
    eprintln!(
        "case,form,backend,runtime,xex_bytes,min_cycles,max_cycles,raw_copy_sites,opt_copy_sites,opt_capture_bytes"
    );
    for copy in [false, true] {
        let mut baseline = Vec::new();
        for union in [false, true] {
            let source = source(copy, union);
            let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
            let options = SemanticOptions::modern();
            let model = semantic::analyze_with_options(&ast, options).unwrap();
            let semir = semantic::ir::lower_program(&ast, &model);
            let raw = nir::lower_program(&semir);
            nir::verify_program(&raw).unwrap();
            let optimized = nir::optimize_program(&raw).unwrap();
            nir::verify_program(&optimized).unwrap();
            let copy_sites = |p: &nir::NirProgram| {
                p.routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .filter(|op| matches!(op, nir::NirOp::CopyBytes { .. }))
                    .count()
            };
            let capture_bytes: u32 = optimized
                .routines
                .iter()
                .flat_map(|r| &r.locals)
                .filter(|l| l.purpose == nir::NirLocalPurpose::AggregateCapture)
                .map(|l| l.layout.size.get())
                .sum();
            for p in [&raw, &optimized] {
                let ops: Vec<_> = p
                    .routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .collect();
                assert!(
                    !ops.iter().any(|op| matches!(op, nir::NirOp::Call { .. })),
                    "no union helper or tag fault"
                );
                if !copy {
                    assert_eq!(copy_sites(p), 0);
                    assert!(
                        !ops.iter()
                            .any(|op| matches!(op, nir::NirOp::Compare { .. })),
                        "no tag tests"
                    );
                    assert_eq!(
                        ops.iter()
                            .filter(|op| matches!(op, nir::NirOp::Store { .. }))
                            .count(),
                        4,
                        "only source-requested stores"
                    );
                }
            }
            let mut lane = 0;
            for mir in [false, true] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let output = if mir {
                        actionc::mir6502::generate_output_with_config_and_runtime(
                            &optimized,
                            0x3000,
                            &actionc::mir6502::Mir6502Config::optimized(),
                            runtime,
                        )
                        .unwrap()
                    } else {
                        codegen::generate_semir_profile_at_origin_with_runtime(
                            &semir,
                            0x3000,
                            codegen::CodegenProfile::Modern,
                            runtime,
                        )
                        .unwrap()
                    };
                    let image = codegen::format_load_file(&output);
                    let cycles: Vec<_> = [0, 7, 127, 255]
                        .into_iter()
                        .map(|seed| execute(&image, runtime, copy, seed))
                        .collect();
                    let measured = (
                        image.len(),
                        *cycles.iter().min().unwrap(),
                        *cycles.iter().max().unwrap(),
                    );
                    if union {
                        assert_eq!(
                            measured, baseline[lane],
                            "representation-only overhead: copy={copy}, mir={mir}, {runtime:?}"
                        );
                    } else {
                        baseline.push(measured);
                    }
                    eprintln!(
                        "{},{},{},{},{},{},{},{},{},{}",
                        if copy { "copy33" } else { "direct" },
                        if union { "union" } else { "manual" },
                        if mir { "mir6502" } else { "classic" },
                        if runtime == Runtime::ActionCart {
                            "cart"
                        } else {
                            "standalone"
                        },
                        measured.0,
                        measured.1,
                        measured.2,
                        copy_sites(&raw),
                        copy_sites(&optimized),
                        capture_bytes
                    );
                    lane += 1;
                }
            }
        }
    }
}
