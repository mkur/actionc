// Matched builds use the normal module/link/NIR pipeline and differ only in
// Mir6502Config::enable_small_leaf_inlining. Generated artifacts are opt-in.
fn q4_program(source: &std::path::Path) -> (crate::nir::NirProgram, String) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let loaded = crate::includes::load_compilation(
        source,
        &crate::includes::ModuleLoadOptions {
            project_root: None,
            module_paths: vec![
                root.join("samples/graphics/mandelbrot"),
                root.join("samples/vbxe"),
            ],
        },
    )
    .unwrap();
    let model = crate::semantic::analyze_compilation_with_options(
        &loaded,
        crate::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let semir = crate::semantic::ir::lower_compilation(&loaded, &model);
    let semir =
        crate::linker::select_semir(&semir, crate::linker::SemLinkPolicy::EntryReachable).unwrap();
    let nir = crate::nir::lower_program(&semir);
    (crate::nir::optimize_program(&nir).unwrap(), loaded.source)
}

#[test]
#[ignore = "writes matched benchmark artifacts under ignored build/"]
fn q4_inline_matched_artifacts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("build/inline-validation");
    std::fs::create_dir_all(&dir).unwrap();
    let mut times = String::from("round,inline,elapsed_ms,code_data_bytes\n");
    let mut previous = BTreeMap::new();
    for round in 0..5 {
        for inline in [false, true] {
            let start = std::time::Instant::now();
            let (nir, source) = q4_program(&root.join("samples/graphics/mandelbrot/mbfixed.act"));
            let config = Mir6502Config {
                enable_small_leaf_inlining: inline,
                ..Mir6502Config::optimized()
            };
            let output = crate::mir6502::generate_output_with_config_and_runtime(
                &nir,
                0x2000,
                &config,
                Runtime::Standalone,
            )
            .unwrap();
            let elapsed = start.elapsed();
            let name = if inline { "inline" } else { "control" };
            let object = crate::codegen::format_load_file(&output);
            if let Some(old) = previous.insert(inline, object.clone()) {
                assert_eq!(old, object);
            }
            eprintln!(
                "{name}: {} code/data bytes, build {elapsed:?}",
                output.bytes.len()
            );
            times.push_str(&format!(
                "{round},{inline},{:.3},{}\n",
                elapsed.as_secs_f64() * 1000.0,
                output.bytes.len()
            ));
            std::fs::write(dir.join(format!("{name}.xex")), object).unwrap();
            std::fs::write(
                dir.join(format!("{name}.lst")),
                crate::compiler::artifacts::format_listing_with_source(&output, &source),
            )
            .unwrap();
        }
    }
    std::fs::write(dir.join("build-times.csv"), times).unwrap();
}

#[test]
fn q4_mandelbrot_recurrence_inlines_squares_with_retained_bodies() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for sample in ["mbfixed.act", "mbfixed-vbxe.act"] {
        let (nir, _) = q4_program(&root.join("samples/graphics/mandelbrot").join(sample));
        for runtime in [Runtime::Standalone, Runtime::ActionCart] {
            let lowered = crate::mir6502::lower_program(&nir).unwrap();
            let wanted: Vec<_> = lowered
                .routines
                .iter()
                .filter(|r| r.inline.requested())
                .map(|r| r.id)
                .collect();
            assert_eq!(wanted.len(), 2);
            let before = crate::mir6502::materialize_program_with_origin_and_runtime(
                lowered.clone(),
                &Mir6502Config {
                    enable_small_leaf_inlining: false,
                    ..Mir6502Config::optimized()
                },
                0x2000,
                runtime,
            )
            .unwrap();
            let after = crate::mir6502::materialize_program_with_origin_and_runtime(
                lowered,
                &Mir6502Config::optimized(),
                0x2000,
                runtime,
            )
            .unwrap();
            assert_eq!(
                wanted
                    .iter()
                    .map(|id| calls_to(&before, *id))
                    .sum::<usize>(),
                3
            );
            for id in wanted {
                if runtime == Runtime::Standalone && calls_to(&before, id) == 2 {
                    assert_eq!(
                        calls_to(&after, id),
                        0,
                        "{sample}/{runtime:?}: retained {id:?}"
                    );
                } else {
                    // INLINE remains costed. Direct comparison branches change
                    // layout: the single MulFloor site can now decline even in
                    // standalone output. Both square sites must still expand;
                    // cartridge layouts may also retain their requested calls.
                    assert!(calls_to(&after, id) <= calls_to(&before, id));
                }
                assert_eq!(
                    before.routines.iter().find(|r| r.id == id),
                    after.routines.iter().find(|r| r.id == id)
                );
            }
        }
    }
}

#[test]
#[ignore = "exhaustive signed-16 arithmetic audit"]
fn q4_exhaustive_squares_and_deterministic_floor_products_match_both_configurations() {
    let path = std::env::temp_dir().join(format!("actionc-inline-q4-{}.act", std::process::id()));
    std::fs::write(&path, "MODULE INLINE_NUMERIC USE MATH.Q4_12 AS Q INT a=$600,b=$602 LONGCARD square=$610 INT product=$614 PROC Main() square=Q.SqrWide(a) product=Q.MulFloor(a,b) RETURN ENDMODULE").unwrap();
    let (nir, _) = q4_program(&path);
    std::fs::remove_file(path).unwrap();
    let lowered = crate::mir6502::lower_program(&nir).unwrap();
    let entry = lowered
        .routines
        .iter()
        .find(|r| r.name.contains("_MAIN_"))
        .unwrap()
        .id;
    let images: Vec<_> = [0, 1, 2]
        .into_iter()
        .map(|mode| {
            let mut lowered = lowered.clone();
            if mode == 2 {
                // Exercise every legal clone even if this small probe's code
                // layout causes the costed configuration to retain a call.
                let census = analyze(&lowered);
                let index = lowered.routines.iter().position(|r| r.id == entry).unwrap();
                for leaf in census
                    .leaves
                    .values()
                    .filter(|leaf| leaf.routine.inline.requested())
                {
                    assert!(
                        expand_group(&mut lowered.routines[index], leaf)
                            .unwrap()
                            .helper_inputs_proven
                    );
                }
                crate::mir6502::verify_program(&lowered, MirPhase::PreMaterialization).unwrap();
            }
            let inline = mode == 1;
            let config = Mir6502Config {
                enable_small_leaf_inlining: inline,
                ..Mir6502Config::optimized()
            };
            let program = crate::mir6502::materialize_program_with_origin_and_runtime(
                lowered.clone(),
                &config,
                0x2000,
                Runtime::ActionCart,
            )
            .unwrap();
            Image::build(&program, 0x2000).unwrap()
        })
        .collect();
    let mut memories = vec![[0u8; 65536]; images.len()];
    let entries: Vec<_> = images
        .iter()
        .zip(&mut memories)
        .map(|(image, memory)| {
            memory[0x2000..0x2000 + image.bytes.len()].copy_from_slice(&image.bytes);
            0x2000
                + image
                    .blocks
                    .iter()
                    .find(|(r, _, _)| *r == entry)
                    .unwrap()
                    .2
                    .start
        })
        .collect();
    let mut seed = 0x51441221u32;
    let mut pairs: Vec<_> = (i16::MIN..=i16::MAX)
        .map(|a| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (a, (seed >> 16) as i16)
        })
        .collect();
    let boundaries = [
        i16::MIN,
        -32767,
        -8192,
        -4097,
        -4096,
        -2049,
        -2048,
        -1,
        0,
        1,
        2047,
        2048,
        4095,
        4096,
        4097,
        8192,
        i16::MAX,
    ];
    pairs.extend(
        boundaries
            .into_iter()
            .flat_map(|a| boundaries.into_iter().map(move |b| (a, b))),
    );
    for (a, b) in pairs {
        for (memory, &entry) in memories.iter_mut().zip(&entries) {
            memory[0x600..0x602].copy_from_slice(&a.to_le_bytes());
            memory[0x602..0x604].copy_from_slice(&b.to_le_bytes());
            super::super::leaf_test_cpu::run_memory(memory, entry);
            assert_eq!(
                u32::from_le_bytes(memory[0x610..0x614].try_into().unwrap()),
                (i32::from(a) * i32::from(a)) as u32,
                "square {a}"
            );
            assert_eq!(
                i16::from_le_bytes(memory[0x614..0x616].try_into().unwrap()),
                (i64::from(a) * i64::from(b)).div_euclid(4096) as i16,
                "floor {a},{b}"
            );
        }
        assert_eq!(&memories[0][0x610..0x616], &memories[1][0x610..0x616]);
    }
}
