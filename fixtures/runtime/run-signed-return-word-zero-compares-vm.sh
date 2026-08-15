#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
harness_root="$repo_root/tools/vm-runtime-tests"
source_path="$runtime_dir/signed_return_word_zero_compares.act"
stats_path="$(mktemp "${TMPDIR:-/tmp}/actionc-signed-return-word-zero.XXXXXX")"
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
selected="$(sed -n 's/^[[:space:]]*signed-return-word-zero-compare-branch: //p' "$stats_path")"
if [[ "$selected" != "8" ]]; then
  echo "FAILED: expected eight signed return-word zero selections, got ${selected:-0}" >&2
  exit 1
fi

cd "$harness_root"
cargo test --locked signed_return_word_zero_compares_execute_through_the_vm_library
