//! Shared, explicit measurement settings. Unknown switches must never be ignored.
use actionc::compiler::native::NativeCompileOptions;
use std::{path::Path, process::Command};

pub fn options(args: impl Iterator<Item = String>) -> (NativeCompileOptions, Vec<String>) {
    let mut options = NativeCompileOptions::default();
    let mut names = Vec::new();
    let mut promotion = None;
    let mut allocation = None;
    for arg in args {
        match arg.as_str() {
            "--no-opt" => options.optimize = false,
            "--native-promotion" | "--conservative-promotion" => {
                let selected = if arg == "--native-promotion" {
                    actionc::nir::NirPromotionPolicy::NativeLoops
                } else {
                    actionc::nir::NirPromotionPolicy::Conservative
                };
                assert!(
                    promotion.is_none_or(|previous| previous == selected),
                    "conflicting promotion policies"
                );
                promotion = Some(selected);
                options.promotion = selected;
            }
            "--no-codegen-opt" => {
                options.codegen = actionc::mir68k::materialize::Options::conservative();
            }
            "--no-pointer-alignment" => options.codegen.pointer_alignment = false,
            "--no-control-flow" => options.codegen.control_flow = false,
            "--register-allocation" | "--no-register-allocation" => {
                let selected = arg == "--register-allocation";
                assert!(
                    allocation.is_none_or(|previous| previous == selected),
                    "conflicting register allocation options"
                );
                allocation = Some(selected);
                options.codegen.register_allocation = selected;
            }
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
    let directories = names
        .iter()
        .map(|name| name.strip_suffix("-multidimensional").unwrap_or(name))
        .collect::<std::collections::BTreeSet<_>>();
    for name in directories {
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
