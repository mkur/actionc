#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
harness_root="$repo_root/tools/vm-runtime-tests"
source_path="$runtime_dir/dual_pointer_word_transfers.act"
materialized_path="$(mktemp "${TMPDIR:-/tmp}/actionc-dual-pointer-word-transfers.XXXXXX")"
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

copy_count="$(grep -c 'copy_indirect_word' "$materialized_path" || true)"
scaled_copy_count="$(grep 'copy_indirect_word' "$materialized_path" | grep -c 'scaled_y' || true)"
compound_count="$(grep -c 'indirect_word_compound' "$materialized_path" || true)"
direct_copy_count="$(grep -c 'copy_direct_word_to_indirect' "$materialized_path" || true)"
if [[ "$copy_count" != "6" || "$scaled_copy_count" != "2" || "$compound_count" != "1" || "$direct_copy_count" != "1" ]]; then
  echo "FAILED: MIR6502 did not select the expected dual-pointer transfers" >&2
  echo "  expected: 6 copies, including 2 scaled-source copies, 1 compound update, and 1 direct-source copy" >&2
  echo "  actual:   $copy_count copies, including $scaled_copy_count scaled-source copies, $compound_count compound updates, and $direct_copy_count direct-source copies" >&2
  exit 1
fi

cd "$harness_root"
cargo test --locked dual_pointer_word_transfers_execute_through_the_vm_library
