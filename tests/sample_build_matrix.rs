use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, Runtime};

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
            vec![release_with_project_root(
                Compatibility,
                Standalone,
                "samples/benchmarks",
            )],
        ),
        executable(
            "samples/benchmarks/suite-nongraphics.act",
            vec![release_with_project_root(
                Optimized,
                Standalone,
                "samples/benchmarks",
            )],
        ),
        executable(
            "samples/benchmarks/suite.act",
            vec![release_with_project_root(
                Optimized,
                Standalone,
                "samples/benchmarks",
            )],
        ),
        executable(
            "samples/demoscene/plasma.act",
            vec![release(Optimized, ActionCart)],
        ),
        executable(
            "samples/demoscene/unlimited-bobs.act",
            vec![release(Optimized, Standalone)],
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
            vec![release(Optimized, Standalone)],
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
        dependency(
            "samples/toolkit/modern/ALLOCATE.ACT",
            &["samples/toolkit/modern/KALSCOPE.DEM"],
        ),
        executable(
            "samples/toolkit/modern/KALSCOPE.DEM",
            vec![release(Optimized, ActionCart)],
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
            vec![release_with_module_path(
                Optimized,
                Standalone,
                "samples/vbxe",
            )],
        ),
        dependency(
            "samples/vbxe/raytracer/fuji/fuji_palette.act",
            &["samples/vbxe/raytracer/fuji/fuji_raytracer.act"],
        ),
        executable(
            "samples/vbxe/raytracer/fuji/fuji_raytracer.act",
            vec![release_with_module_path(
                Optimized,
                Standalone,
                "samples/vbxe",
            )],
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
            vec![release_with_module_path(
                Optimized,
                Standalone,
                "samples/vbxe",
            )],
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
            vec![release_with_module_path(
                Optimized,
                Standalone,
                "samples/vbxe",
            )],
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
