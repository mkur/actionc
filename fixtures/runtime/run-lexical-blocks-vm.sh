#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
harness_root="$repo_root/tools/vm-runtime-tests"

cd "$harness_root"
cargo test --locked --test lexical_blocks lexical_blocks_preserve_bindings_in_every_backend_and_runtime
