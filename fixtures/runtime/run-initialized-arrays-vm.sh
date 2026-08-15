#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
harness_root="$repo_root/tools/vm-runtime-tests"

if [[ ! -f "$harness_root/Cargo.toml" ]]; then
  echo "Missing VM runtime test harness: $harness_root/Cargo.toml" >&2
  exit 1
fi

cd "$harness_root"
cargo test --locked initialized_arrays_execute_through_the_vm_library
