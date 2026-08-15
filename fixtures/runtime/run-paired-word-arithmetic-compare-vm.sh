#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
harness_root="$repo_root/tools/vm-runtime-tests"
source_path="$runtime_dir/paired_word_arithmetic_compare.act"
stats_path="$(mktemp "${TMPDIR:-/tmp}/actionc-paired-word-compare.XXXXXX")"
trap 'rm -f "$stats_path"' EXIT

(
  cd "$repo_root"
  ACTIONC_MIR6502_PEEPHOLES=summary \
    cargo run --quiet --bin actionc-emit -- \
      --profile modern \
      --backend mir6502 \
      --emit-load \
      "$source_path" \
      > /dev/null \
      2> "$stats_path"
)
selected="$(sed -n 's/^[[:space:]]*word-arithmetic-compare-branch: //p' "$stats_path")"
if [[ "$selected" != "7" ]]; then
  echo "FAILED: expected seven paired word arithmetic compare selections, got ${selected:-0}" >&2
  exit 1
fi

cd "$harness_root"
cargo test --locked paired_word_arithmetic_compare_executes_through_the_vm_library
