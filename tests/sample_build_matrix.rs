use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, CompiledProgram, Runtime, compile_file};

const RUNAD: u16 = 0x02E2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildTier {
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BuildCase {
    tier: BuildTier,
    mode: CompileMode,
    runtime: Runtime,
    project_root: Option<&'static str>,
    module_paths: Vec<&'static str>,
    required_segment_start: Option<u16>,
    forbidden_ranges: Vec<(u16, u16)>,
    program_range: Option<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SampleRole {
    Executable { builds: Vec<BuildCase> },
    Dependency { used_by: &'static [&'static str] },
    SourceOnly { reason: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SampleSpec {
    path: &'static str,
    role: SampleRole,
}

fn release(mode: CompileMode, runtime: Runtime) -> BuildCase {
    BuildCase {
        tier: BuildTier::Release,
        mode,
        runtime,
        project_root: None,
        module_paths: Vec::new(),
        required_segment_start: None,
        forbidden_ranges: Vec::new(),
        program_range: None,
    }
}

impl BuildCase {
    fn requiring_segment_start(mut self, address: u16) -> Self {
        self.required_segment_start = Some(address);
        self
    }

    fn avoiding(mut self, start: u16, end: u16) -> Self {
        self.forbidden_ranges.push((start, end));
        self
    }

    fn fitting_in(mut self, start: u16, end: u16) -> Self {
        self.program_range = Some((start, end));
        self
    }
}

fn release_with_project_root(
    mode: CompileMode,
    runtime: Runtime,
    project_root: &'static str,
) -> BuildCase {
    BuildCase {
        project_root: Some(project_root),
        ..release(mode, runtime)
    }
}

fn release_with_module_path(
    mode: CompileMode,
    runtime: Runtime,
    module_path: &'static str,
) -> BuildCase {
    BuildCase {
        module_paths: vec![module_path],
        ..release(mode, runtime)
    }
}

fn executable(path: &'static str, builds: Vec<BuildCase>) -> SampleSpec {
    SampleSpec {
        path,
        role: SampleRole::Executable { builds },
    }
}

fn dependency(path: &'static str, used_by: &'static [&'static str]) -> SampleSpec {
    SampleSpec {
        path,
        role: SampleRole::Dependency { used_by },
    }
}

fn source_only(path: &'static str, reason: &'static str) -> SampleSpec {
    SampleSpec {
        path,
        role: SampleRole::SourceOnly { reason },
    }
}

fn sample_catalog() -> Vec<SampleSpec> {
    use CompileMode::{Compatibility, Optimized};
    use Runtime::{ActionCart, Standalone};

    vec![
        source_only(
            "samples/action-runtime/modern/ST.ACT",
            "library source fragment whose SYSBLK.ACT and SYSSTR.ACT inputs live outside the maintained sample tree",
        ),
        executable(
            "samples/atari-fuji-logo.act",
            vec![release(Compatibility, ActionCart)],
        ),
        dependency(
            "samples/benchmarks/bench/bsort.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/chessboard.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/countdown_for.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/countdown_while.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/flames_array.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/flames_display.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/flames_pointer.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/floating_real.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/guessing.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/landscape.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/lipsum.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/ludolphian.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/montecarlo.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/qr_1d.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/sieve1028.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/sieve1899.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/support.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        dependency(
            "samples/benchmarks/bench/yoshplus.act",
            &[
                "samples/benchmarks/suite.act",
                "samples/benchmarks/suite-compat.act",
                "samples/benchmarks/suite-nongraphics.act",
            ],
        ),
        executable(
            "samples/benchmarks/suite-compat.act",
            vec![
                release_with_project_root(Compatibility, Standalone, "samples/benchmarks")
                    .requiring_segment_start(0x2000)
                    .avoiding(0x8000, 0x9FFF),
            ],
        ),
        executable(
            "samples/benchmarks/suite-nongraphics.act",
            vec![
                release_with_project_root(Optimized, Standalone, "samples/benchmarks")
                    .requiring_segment_start(0x2000)
                    .avoiding(0x8000, 0x9FFF),
            ],
        ),
        executable(
            "samples/benchmarks/suite.act",
            vec![
                release_with_project_root(Optimized, Standalone, "samples/benchmarks")
                    .requiring_segment_start(0x2000)
                    .avoiding(0x8000, 0x9FFF),
            ],
        ),
        executable(
            "samples/demoscene/plasma.act",
            vec![release(Optimized, ActionCart)],
        ),
        executable(
            "samples/demoscene/unlimited-bobs.act",
            vec![
                release(Optimized, Standalone)
                    .requiring_segment_start(0x8000)
                    .fitting_in(0x8000, 0xBFFF),
            ],
        ),
        executable(
            "samples/graphics/fedora.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/graphics/landscape.act",
            vec![
                release(Compatibility, ActionCart),
                release(Optimized, Standalone),
            ],
        ),
        dependency(
            "samples/graphics/unknown-pleasures/unknown-pleasure-vbxe-data.inc",
            &["samples/graphics/unknown-pleasures/unknown-pleasure-vbxe.act"],
        ),
        executable(
            "samples/graphics/unknown-pleasures/unknown-pleasure-vbxe.act",
            vec![release(Optimized, Standalone).avoiding(0xA000, 0xBFFF)],
        ),
        dependency(
            "samples/graphics/unknown-pleasures/unknown-pleasures-data.inc",
            &["samples/graphics/unknown-pleasures/unknown-pleasures.act"],
        ),
        executable(
            "samples/graphics/unknown-pleasures/unknown-pleasures.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/hello-world.act",
            vec![release(Compatibility, ActionCart)],
        ),
        executable(
            "samples/inline-asm-fine-scroll.act",
            vec![release(Compatibility, ActionCart)],
        ),
        executable(
            "samples/lexical-blocks.act",
            vec![release(Optimized, ActionCart)],
        ),
        executable("samples/logo.act", vec![release(Compatibility, ActionCart)]),
        executable(
            "samples/modules/hello.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/modules/local-runtime-override.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/modules/native-real-library.act",
            vec![release(Optimized, Standalone)],
        ),
        dependency(
            "samples/modules/project/demo/color.act",
            &["samples/modules/project/main.act"],
        ),
        executable(
            "samples/modules/project/main.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/modules/rainbow.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/modules/sys-memory-open.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/modules/sys-memory-qualified.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/rainbow.act",
            vec![release(Compatibility, ActionCart)],
        ),
        executable(
            "samples/rainbow_asm.act",
            vec![release(Compatibility, ActionCart)],
        ),
        executable(
            "samples/real-basics.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/standalone/standalone-runtime.act",
            vec![release(Optimized, Standalone)],
        ),
        dependency(
            "samples/tn/modern/LIB.ACT",
            &["samples/tn/modern/TN.ACT", "samples/tn/modern/TNDBG.ACT"],
        ),
        executable(
            "samples/tn/modern/TN.ACT",
            vec![release(Optimized, ActionCart)],
        ),
        executable(
            "samples/tn/modern/TNDBG.ACT",
            vec![release(Optimized, ActionCart)],
        ),
        source_only(
            "samples/toolkit/modern/.PMG_trace_default_no_appmhi.DM1",
            "retained compiler trace input, not a user-facing Toolkit program",
        ),
        source_only(
            "samples/toolkit/modern/ALLOCATE.ACT",
            "Toolkit library module retained as readable source; no maintained executable root currently includes it",
        ),
        executable(
            "samples/toolkit/modern/KALSCOPE.DEM",
            vec![release(Optimized, ActionCart)],
        ),
        dependency(
            "samples/toolkit/modern/IO.ACT",
            &["samples/toolkit/modern/MUSIC.DEM"],
        ),
        executable(
            "samples/toolkit/modern/MUSIC.DEM",
            vec![release(Optimized, ActionCart)],
        ),
        dependency(
            "samples/toolkit/modern/PMG.ACT",
            &[
                "samples/toolkit/modern/MUSIC.DEM",
                "samples/toolkit/modern/PMG.DM1",
                "samples/toolkit/modern/PMG.DM2",
            ],
        ),
        executable(
            "samples/toolkit/modern/PMG.DM1",
            vec![release(Optimized, ActionCart)],
        ),
        executable(
            "samples/toolkit/modern/PMG.DM2",
            vec![release(Optimized, ActionCart)],
        ),
        dependency(
            "samples/toolkit/modern/PRINTF.ACT",
            &["samples/toolkit/modern/PRINTF.DM1"],
        ),
        executable(
            "samples/toolkit/modern/PRINTF.DM1",
            vec![release(Optimized, ActionCart)],
        ),
        dependency(
            "samples/toolkit/modern/REAL.ACT",
            &["samples/toolkit/modern/REAL.DM1"],
        ),
        executable(
            "samples/toolkit/modern/REAL.DM1",
            vec![release(Optimized, ActionCart)],
        ),
        dependency(
            "samples/toolkit/modern/SORT.ACT",
            &[
                "samples/toolkit/modern/SORT.DM1",
                "samples/toolkit/modern/SORT.DM2",
            ],
        ),
        executable(
            "samples/toolkit/modern/SORT.DM1",
            vec![release(Optimized, ActionCart)],
        ),
        executable(
            "samples/toolkit/modern/SORT.DM2",
            vec![release(Optimized, ActionCart)],
        ),
        executable(
            "samples/vbxe/detect.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/vbxe/gradient.act",
            vec![
                release_with_module_path(Optimized, Standalone, "samples/vbxe")
                    .avoiding(0xA000, 0xBFFF),
            ],
        ),
        dependency(
            "samples/vbxe/raytracer/fuji/fuji_palette.act",
            &["samples/vbxe/raytracer/fuji/fuji_raytracer.act"],
        ),
        executable(
            "samples/vbxe/raytracer/fuji/fuji_raytracer.act",
            vec![
                release_with_module_path(Optimized, Standalone, "samples/vbxe")
                    .avoiding(0xA000, 0xBFFF),
            ],
        ),
        dependency(
            "samples/vbxe/raytracer/fuji/fuji_scene.act",
            &[
                "samples/vbxe/raytracer/fuji/fuji_raytracer.act",
                "samples/vbxe/raytracer/fuji/fuji_scene_probe.act",
            ],
        ),
        executable(
            "samples/vbxe/raytracer/fuji/fuji_scene_probe.act",
            vec![release(Optimized, Standalone)],
        ),
        dependency(
            "samples/vbxe/raytracer/neon/neon_palette.act",
            &["samples/vbxe/raytracer/neon/neon_raytracer.act"],
        ),
        executable(
            "samples/vbxe/raytracer/neon/neon_raytracer.act",
            vec![
                release_with_module_path(Optimized, Standalone, "samples/vbxe")
                    .avoiding(0xA000, 0xBFFF),
            ],
        ),
        dependency(
            "samples/vbxe/raytracer/neon/neon_scene.act",
            &[
                "samples/vbxe/raytracer/neon/neon_raytracer.act",
                "samples/vbxe/raytracer/neon/neon_scene_probe.act",
            ],
        ),
        executable(
            "samples/vbxe/raytracer/neon/neon_scene_probe.act",
            vec![release(Optimized, Standalone)],
        ),
        executable(
            "samples/vbxe/raytracer/spheres/spheres_raytracer.act",
            vec![
                release_with_module_path(Optimized, Standalone, "samples/vbxe")
                    .avoiding(0xA000, 0xBFFF),
            ],
        ),
        dependency(
            "samples/vbxe/raytracer/spheres/spheres_scene.act",
            &[
                "samples/vbxe/raytracer/spheres/spheres_raytracer.act",
                "samples/vbxe/raytracer/spheres/spheres_scene_probe.act",
            ],
        ),
        executable(
            "samples/vbxe/raytracer/spheres/spheres_scene_probe.act",
            vec![release(Optimized, Standalone)],
        ),
        dependency(
            "samples/vbxe/shared/screen.act",
            &[
                "samples/vbxe/gradient.act",
                "samples/vbxe/raytracer/fuji/fuji_raytracer.act",
                "samples/vbxe/raytracer/neon/neon_raytracer.act",
                "samples/vbxe/raytracer/spheres/spheres_raytracer.act",
            ],
        ),
    ]
}

#[test]
fn sample_catalog_classifies_every_action_source() {
    let root = repository_root();
    let discovered = discover_action_sources(&root.join("samples"));
    let catalog = sample_catalog();
    let mut registered = BTreeMap::new();

    for spec in &catalog {
        assert!(
            root.join(spec.path).is_file(),
            "sample catalog path does not exist: {}",
            spec.path
        );
        assert!(
            registered.insert(spec.path, spec).is_none(),
            "sample catalog contains duplicate path: {}",
            spec.path
        );
    }

    let registered = registered
        .keys()
        .map(|path| (*path).to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        discovered, registered,
        "every Action-family source under samples/ must have an explicit catalog role"
    );
}

#[test]
fn sample_catalog_roles_are_complete_and_consistent() {
    let root = repository_root();
    let catalog = sample_catalog();
    let roles = catalog
        .iter()
        .map(|spec| (spec.path, &spec.role))
        .collect::<BTreeMap<_, _>>();

    for spec in &catalog {
        match &spec.role {
            SampleRole::Executable { builds } => {
                assert!(
                    !builds.is_empty(),
                    "executable sample has no declared build: {}",
                    spec.path
                );
                let mut distinct = BTreeSet::new();
                for build in builds {
                    let identity = format!(
                        "{:?}/{:?}/{:?}/{:?}/{:?}",
                        build.tier,
                        build.mode,
                        build.runtime,
                        build.project_root,
                        build.module_paths
                    );
                    assert!(
                        distinct.insert(identity),
                        "sample has a duplicate build declaration: {}",
                        spec.path
                    );
                    if let Some(project_root) = build.project_root {
                        assert!(
                            root.join(project_root).is_dir(),
                            "sample build project root does not exist: {project_root}"
                        );
                    }
                    for module_path in &build.module_paths {
                        assert!(
                            root.join(module_path).is_dir(),
                            "sample build module path does not exist: {module_path}"
                        );
                    }
                }
            }
            SampleRole::Dependency { used_by } => {
                assert!(
                    !used_by.is_empty(),
                    "sample dependency has no executable owner: {}",
                    spec.path
                );
                for owner in *used_by {
                    assert!(
                        matches!(roles.get(owner), Some(SampleRole::Executable { .. })),
                        "sample dependency {} names a non-executable owner: {owner}",
                        spec.path
                    );
                }
            }
            SampleRole::SourceOnly { reason } => {
                assert!(
                    !reason.trim().is_empty(),
                    "source-only sample needs an explanation: {}",
                    spec.path
                );
            }
        }
    }
}

#[test]
fn release_sample_builds_produce_valid_load_files() {
    let root = repository_root();
    let mut failures = Vec::new();

    for spec in sample_catalog() {
        let SampleRole::Executable { builds } = spec.role else {
            continue;
        };
        for build in builds
            .iter()
            .filter(|build| build.tier == BuildTier::Release)
        {
            let description = build_description(spec.path, build);
            let mut options = CompileOptions::for_mode(build.mode).with_runtime(build.runtime);
            if let Some(project_root) = build.project_root {
                options = options.with_project_root(root.join(project_root));
            }
            for module_path in &build.module_paths {
                options = options.with_module_path(root.join(module_path));
            }

            let source = root.join(spec.path);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                compile_file(&source, &options)
            }));
            match result {
                Ok(Ok(compiled)) => {
                    if let Err(error) = validate_compiled_program(&compiled, build) {
                        failures.push(format!("{description}: {error}"));
                    }
                }
                Ok(Err(error)) => failures.push(format!("{description}: {error}")),
                Err(payload) => failures.push(format!(
                    "{description}: compiler panicked: {}",
                    panic_payload(payload)
                )),
            }
        }
    }

    assert!(
        failures.is_empty(),
        "release sample build matrix failed:\n{}",
        failures.join("\n")
    );
}

fn build_description(path: &str, build: &BuildCase) -> String {
    format!("{path} ({:?}/{:?})", build.mode, build.runtime)
}

fn panic_payload(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else {
        "non-string panic payload".to_string()
    }
}

fn validate_compiled_program(compiled: &CompiledProgram, build: &BuildCase) -> Result<(), String> {
    if compiled.runtime() != build.runtime {
        return Err(format!(
            "selected runtime {:?}, expected {:?}",
            compiled.runtime(),
            build.runtime
        ));
    }

    let segments = parse_load_file(compiled.object_bytes())?;
    let program_segments = segments
        .iter()
        .filter(|segment| !(segment.start == RUNAD && segment.end == RUNAD + 1))
        .collect::<Vec<_>>();
    if program_segments.is_empty() {
        return Err("load file has no program segment".to_string());
    }

    let run_address = read_loaded_word(&segments, RUNAD)
        .ok_or_else(|| "load file does not initialize RUNAD".to_string())?;
    if run_address != compiled.run_address() {
        return Err(format!(
            "RUNAD is ${run_address:04X}, compiler reports ${:04X}",
            compiled.run_address()
        ));
    }
    if !program_segments
        .iter()
        .any(|segment| segment.contains(run_address))
    {
        return Err(format!(
            "RUNAD ${run_address:04X} is outside every program segment"
        ));
    }

    if let Some(required_start) = build.required_segment_start
        && !program_segments
            .iter()
            .any(|segment| segment.start == required_start)
    {
        return Err(format!(
            "no program segment starts at required address ${required_start:04X}"
        ));
    }
    for &(forbidden_start, forbidden_end) in &build.forbidden_ranges {
        if let Some(segment) = program_segments
            .iter()
            .find(|segment| segment.overlaps(forbidden_start, forbidden_end))
        {
            return Err(format!(
                "segment ${:04X}-${:04X} overlaps forbidden range ${forbidden_start:04X}-${forbidden_end:04X}",
                segment.start, segment.end
            ));
        }
    }
    if let Some((allowed_start, allowed_end)) = build.program_range
        && let Some(segment) = program_segments
            .iter()
            .find(|segment| segment.start < allowed_start || segment.end > allowed_end)
    {
        return Err(format!(
            "segment ${:04X}-${:04X} is outside required program range ${allowed_start:04X}-${allowed_end:04X}",
            segment.start, segment.end
        ));
    }

    Ok(())
}

#[derive(Debug)]
struct LoadSegment<'a> {
    start: u16,
    end: u16,
    data: &'a [u8],
}

impl LoadSegment<'_> {
    fn contains(&self, address: u16) -> bool {
        self.start <= address && address <= self.end
    }

    fn overlaps(&self, start: u16, end: u16) -> bool {
        self.start <= end && start <= self.end
    }
}

fn parse_load_file(bytes: &[u8]) -> Result<Vec<LoadSegment<'_>>, String> {
    if !bytes.starts_with(&[0xFF, 0xFF]) {
        return Err("load file does not start with the $FFFF marker".to_string());
    }

    let mut cursor = 0usize;
    let mut segments = Vec::new();
    while cursor < bytes.len() {
        while bytes.get(cursor..cursor + 2) == Some(&[0xFF, 0xFF]) {
            cursor += 2;
        }
        if cursor == bytes.len() {
            return Err("load file ends after a $FFFF marker".to_string());
        }
        if cursor + 4 > bytes.len() {
            return Err(format!("truncated segment header at byte {cursor}"));
        }

        let start = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]);
        let end = u16::from_le_bytes([bytes[cursor + 2], bytes[cursor + 3]]);
        cursor += 4;
        if end < start {
            return Err(format!("invalid segment range ${start:04X}-${end:04X}"));
        }
        let size = usize::from(end - start) + 1;
        let data_end = cursor
            .checked_add(size)
            .ok_or_else(|| "segment size overflow".to_string())?;
        let data = bytes
            .get(cursor..data_end)
            .ok_or_else(|| format!("truncated segment data for ${start:04X}-${end:04X}"))?;
        segments.push(LoadSegment { start, end, data });
        cursor = data_end;
    }

    if segments.is_empty() {
        return Err("load file has no segments".to_string());
    }
    Ok(segments)
}

fn read_loaded_word(segments: &[LoadSegment<'_>], address: u16) -> Option<u16> {
    let mut bytes = [None, None];
    for segment in segments {
        for (offset, byte) in bytes.iter_mut().enumerate() {
            let target = address.checked_add(offset as u16)?;
            if segment.contains(target) {
                *byte = Some(segment.data[usize::from(target - segment.start)]);
            }
        }
    }
    Some(u16::from_le_bytes([bytes[0]?, bytes[1]?]))
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn discover_action_sources(samples: &Path) -> BTreeSet<String> {
    let root = repository_root();
    let mut discovered = BTreeSet::new();
    discover_action_sources_into(samples, &root, &mut discovered);
    discovered
}

fn discover_action_sources_into(dir: &Path, root: &Path, discovered: &mut BTreeSet<String>) {
    let mut entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read sample directory {}: {error}", dir.display()))
        .map(|entry| entry.expect("read sample directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            discover_action_sources_into(&path, root, discovered);
        } else if is_action_source(&path) {
            let relative = path
                .strip_prefix(root)
                .expect("sample source is inside repository")
                .to_string_lossy()
                .replace('\\', "/");
            discovered.insert(relative);
        }
    }
}

fn is_action_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "act" | "lib" | "dem" | "dm1" | "dm2" | "inc"
            )
        })
}
