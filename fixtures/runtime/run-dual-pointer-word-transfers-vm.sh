#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/dual_pointer_word_transfers.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/rev02.rom}"
expected="34 12 78 56 ef be fe ca 57 13 68 24 00 01 68 24 03 05 03 05 00 80 00 00"

require_file() {
  local path="$1"
  local label="$2"
  if [[ ! -f "$path" ]]; then
    echo "Missing $label: $path" >&2
    exit 1
  fi
}

require_file "$source_path" "runtime fixture"
require_file "$vm_root/Cargo.toml" "action-compiler-vm project"
require_file "$cart_rom" "Action! cartridge ROM"
require_file "$os_rom" "Atari OS ROM"

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-dual-pointer-word-transfers.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

materialized_path="$out_dir/mir6502.materialized"
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
if [[ "$copy_count" != "6" || "$scaled_copy_count" != "2" || "$compound_count" != "1" ]]; then
  echo "FAILED: MIR6502 did not select the expected dual-pointer transfers" >&2
  echo "  expected: 6 copies, including 2 scaled-source copies, and 1 compound update" >&2
  echo "  actual:   $copy_count copies, including $scaled_copy_count scaled-source copies, and $compound_count compound updates" >&2
  grep -E 'copy_indirect_word|indirect_word_compound' "$materialized_path" >&2 || true
  exit 1
fi

for backend in classic mir6502; do
  object_path="$out_dir/$backend.com"
  memory_path="$out_dir/$backend.memory.bin"

  echo "==> dual-pointer word transfers: compile modern/$backend"
  (
    cd "$repo_root"
    cargo run --quiet --bin actionc -- \
      --profile modern \
      --backend "$backend" \
      --output "$object_path" \
      "$source_path"
  )

  echo "==> dual-pointer word transfers: execute modern/$backend"
  cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
    --cart "$cart_rom" \
    --os "$os_rom" \
    --load-object "$object_path" \
    --dump-memory-on-stop "$memory_path" \
    --max-steps 1600 \
    --history 8

  actual="$(od -An -tx1 -j "$((0x0600))" -N 24 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
  if [[ "$actual" != "$expected" ]]; then
    echo "FAILED: modern/$backend dual-pointer word-transfer results" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    exit 1
  fi

  echo "    results at \$0600-\$0617: $actual"
done

echo "dual-pointer word-transfer runtime gate passed"
