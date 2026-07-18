use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "runs the original-compiler probe sweep; use cargo test --test compatibility -- --ignored"]
fn original_compiler_probe_sweep() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("surveys")
        .join("probes")
        .join("original-compiler")
        .join("sweep.sh");

    let output = Command::new(&script)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("run {}: {err}", script.display()));

    if !output.status.success() {
        panic!(
            "{} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "compiles the full TN source in legacy and modern profiles; use cargo test --test compatibility -- --ignored"]
fn tn_stability_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("surveys")
        .join("tn")
        .join("check-stability.sh");

    let output = Command::new(&script)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("run {}: {err}", script.display()));

    if !output.status.success() {
        panic!(
            "{} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "executes generated code with action-compiler-vm; use cargo test --test compatibility -- --ignored"]
fn initialized_array_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-initialized-arrays-vm.sh");

    let output = Command::new(&script)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("run {}: {err}", script.display()));

    if !output.status.success() {
        panic!(
            "{} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "executes generated code with action-compiler-vm; use cargo test --test compatibility -- --ignored"]
fn scaled_card_index_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-scaled-card-indexes-vm.sh");

    let output = Command::new(&script)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("run {}: {err}", script.display()));

    if !output.status.success() {
        panic!(
            "{} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "executes generated code with action-compiler-vm; use cargo test --test compatibility -- --ignored"]
fn kalscope_backend_contract_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-kalscope-contracts-vm.sh");

    let output = Command::new(&script)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("run {}: {err}", script.display()));

    if !output.status.success() {
        panic!(
            "{} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
