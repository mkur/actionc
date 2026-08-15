#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
harness_root="$repo_root/tools/vm-runtime-tests"
source_path="$runtime_dir/indirect_call_fields.act"
materialized_path="$(mktemp "${TMPDIR:-/tmp}/actionc-indirect-call-fields.XXXXXX")"
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
selection_count="$(grep -c 'copy_indirect_bytes_to_fixed_zp' "$materialized_path" || true)"
if [[ "$selection_count" != "2" ]]; then
  echo "FAILED: expected two indirect call-field transfers, got $selection_count" >&2
  exit 1
fi

cd "$harness_root"
cargo test --locked indirect_call_fields_execute_through_the_vm_library
