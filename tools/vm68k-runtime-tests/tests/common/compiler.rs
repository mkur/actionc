//! Invoke the public compiler, never an unrelated executable found on PATH.
#![allow(dead_code)]
use actionc::compiler::native::artifacts::NativeArtifact;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

pub fn executable() -> &'static Path {
    static COMPILER: OnceLock<PathBuf> = OnceLock::new();
    COMPILER.get_or_init(|| {
        if let Some(path) = std::env::var_os("ACTIONC_TEST_COMPILER") {
            return PathBuf::from(path).canonicalize().expect(
                "ACTIONC_TEST_COMPILER must name this checkout's built actionc executable",
            );
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        // A separate Cargo target directory avoids acquiring the lock held by
        // the workspace that is currently running these tests. Cargo's artifact
        // message supplies the exact path, including target triples and .exe.
        let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .current_dir(&root)
            .args([
                "build",
                "--locked",
                "--bin",
                "actionc",
                "--message-format=json",
                "--target-dir",
            ])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("target/public-compiler"))
            .output()
            .expect("build public compiler");
        assert!(
            output.status.success(),
            "compiler build failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find_map(|message| {
                if message["reason"] == "compiler-artifact"
                    && message["target"]["name"] == "actionc"
                {
                    message["executable"].as_str().map(PathBuf::from)
                } else {
                    None
                }
            })
            .expect("Cargo did not report the actionc executable")
    })
}

pub fn compile(source: &Path, output: &Path, options: &[&str]) -> NativeArtifact {
    let result = Command::new(executable())
        .args(["--target", "motorola-68000"])
        .args(options)
        .arg("--output")
        .arg(output)
        .arg(source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}: {}",
        source.display(),
        String::from_utf8_lossy(&result.stderr)
    );
    NativeArtifact::load(output).unwrap()
}
