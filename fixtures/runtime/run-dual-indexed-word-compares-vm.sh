#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
harness_root="$repo_root/tools/vm-runtime-tests"
source_path="$runtime_dir/dual_indexed_word_compares.act"
materialized_path="$(mktemp "${TMPDIR:-/tmp}/actionc-dual-indexed-word-compares.XXXXXX")"
trap 'rm -f "$materialized_path"' EXIT

(
  cd "$repo_root"
  cargo run --quiet --bin actionc-emit -- \
    --profile modern \
    --backend mir6502 \
    --emit-materialized-mir6502 \
    "$source_path" \
    > "$materialized_path"
)
selected="$(grep -c 'cmp_indirect\.w' "$materialized_path" || true)"
if [[ "$selected" != "5" ]]; then
  echo "FAILED: expected five direct indexed word compares, got $selected" >&2
  exit 1
fi

cd "$harness_root"
cargo test --locked dual_indexed_word_compares_execute_through_the_vm_library
