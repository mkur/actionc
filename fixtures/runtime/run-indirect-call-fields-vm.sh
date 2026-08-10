#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/indirect_call_fields.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/altirraos-xl.rom}"
expected="c8 02 11 11 fe 70 34 12 cd ab 22 22 a3 00 78 56 bc 9a"

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

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-indirect-call-fields.XXXXXX")"
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

selection_count="$(grep -c 'copy_indirect_bytes_to_fixed_zp' "$materialized_path" || true)"
if [[ "$selection_count" != "2" ]]; then
  echo "FAILED: MIR6502 did not select both indirect call-field transfers" >&2
  echo "  expected: 2 selections" >&2
  echo "  actual:   $selection_count selections" >&2
  grep 'copy_indirect_bytes_to_fixed_zp' "$materialized_path" >&2 || true
  exit 1
fi

object_path="$out_dir/mir6502.com"
memory_path="$out_dir/mir6502.memory.bin"

echo "==> indirect call fields: compile modern/mir6502"
(
  cd "$repo_root"
  cargo run --quiet --bin actionc -- \
    --profile modern \
    --backend mir6502 \
    --output "$object_path" \
    "$source_path"
)

echo "==> indirect call fields: execute modern/mir6502"
cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
  --cart "$cart_rom" \
  --os "$os_rom" \
  --load-object "$object_path" \
  --dump-memory-on-stop "$memory_path" \
  --max-steps 3000 \
  --history 8

actual="$(od -An -tx1 -j "$((0x0600))" -N 18 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
if [[ "$actual" != "$expected" ]]; then
  echo "FAILED: modern/mir6502 indirect call-field results" >&2
  echo "  expected: $expected" >&2
  echo "  actual:   $actual" >&2
  exit 1
fi

echo "    results at \$0600-\$0611: $actual"
echo "indirect call-field runtime gate passed"
