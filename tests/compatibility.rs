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
fn dual_indexed_word_compare_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-dual-indexed-word-compares-vm.sh");

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
fn dual_pointer_word_transfer_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-dual-pointer-word-transfers-vm.sh");

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
fn allocate_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-allocate-vm.sh");

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
fn sort_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-sort-vm.sh");

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
fn ordered_absolute_sub_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-ordered-absolute-sub-vm.sh");

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
fn indirect_call_fields_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-indirect-call-fields-vm.sh");

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
fn direct_word_compare_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-direct-word-compares-vm.sh");

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
fn signed_return_word_zero_compare_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-signed-return-word-zero-compares-vm.sh");

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
fn signed_word_relation_matrix_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-signed-word-relation-matrix-vm.sh");

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
fn direct_action_word_arithmetic_args_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-direct-action-word-arithmetic-args-vm.sh");

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
fn indexed_byte_fixed_action_args_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-indexed-byte-fixed-action-args-vm.sh");

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
fn paired_word_arithmetic_compare_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-paired-word-arithmetic-compare-vm.sh");

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
fn circle_int_math_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-circle-int-math-vm.sh");

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

#[test]
#[ignore = "executes generated code with action-compiler-vm; use cargo test --test compatibility -- --ignored"]
fn kalscope_codegen_patterns_runtime_check() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root
        .join("fixtures")
        .join("runtime")
        .join("run-kalscope-codegen-patterns-vm.sh");

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
