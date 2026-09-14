//! Shared, explicit measurement settings. Unknown switches must never be ignored.
use actionc::compiler::native::NativeCompileOptions;
use std::{path::Path, process::Command};

pub fn options(args: impl Iterator<Item = String>) -> (NativeCompileOptions, Vec<String>) {
    let mut options = NativeCompileOptions::default();
    let mut names = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--no-opt" => options.optimize = false,
            "--no-codegen-opt" => {
                options.codegen = actionc::mir68k::materialize::Options::conservative();
            }
            "--no-pointer-alignment" => options.codegen.pointer_alignment = false,
            "--no-control-flow" => options.codegen.control_flow = false,
            "--no-forward-temporaries" => options.codegen.forward_temporaries = false,
            "--no-select-instructions" => options.codegen.select_instructions = false,
            "--no-relax-branches" => options.codegen.relax_branches = false,
            _ if arg.starts_with('-') => panic!("unknown measurement option: {arg}"),
            _ => names.push(arg),
        }
    }
    (options, names)
}

pub fn metadata(root: &Path, options: &NativeCompileOptions, names: &[String]) -> String {
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git metadata failed: {args:?}");
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n")
    };
    let mut result = format!(
        "actionc_commit: {}compiler_worktree_status:\n{}options: {options:#?}\n",
        git(&["rev-parse", "HEAD"]),
        git(&[
            "status",
            "--porcelain",
            "--",
            "src",
            "Cargo.toml",
            "Cargo.lock"
        ]),
    );
    for name in names {
        let mut files: Vec<_> =
            std::fs::read_dir(root.join(format!("fixtures/runtime/tacle/{name}")))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .filter(|file| {
                    matches!(
                        Path::new(file).extension().and_then(|ext| ext.to_str()),
                        Some("act" | "inc" | "txt")
                    )
                })
                .collect();
        files.sort();
        for file in files {
            let path = format!("fixtures/runtime/tacle/{name}/{file}");
            result.push_str(&format!(
                "input {path}: {}",
                git(&["hash-object", "--", &path])
            ));
        }
    }
    result
}
