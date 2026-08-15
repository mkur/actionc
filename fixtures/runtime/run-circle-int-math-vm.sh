#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
harness_root="$repo_root/tools/vm-runtime-tests"

cd "$harness_root"
cargo test --locked circle_int_math_executes_through_the_vm_library
