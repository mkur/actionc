#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/ordered_absolute_sub_runtime.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/altirraos-xl.rom}"
expected="a9 4e 0f"

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

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-ordered-absolute-sub.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

object_path="$out_dir/mir6502.com"
memory_path="$out_dir/mir6502.memory.bin"

echo "==> ordered absolute subtraction: compile modern/mir6502"
(
  cd "$repo_root"
  cargo run --quiet --bin actionc -- \
    --profile modern \
    --backend mir6502 \
    --output "$object_path" \
    "$source_path"
)

echo "==> ordered absolute subtraction: execute modern/mir6502"
cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
  --cart "$cart_rom" \
  --os "$os_rom" \
  --load-object "$object_path" \
  --dump-memory-on-stop "$memory_path" \
  --max-steps 1000 \
  --history 8

actual="$(od -An -tx1 -j "$((0x0600))" -N 3 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
if [[ "$actual" != "$expected" ]]; then
  echo "FAILED: modern/mir6502 ordered absolute subtraction" >&2
  echo "  expected: $expected" >&2
  echo "  actual:   $actual" >&2
  exit 1
fi

echo "    results at \$0600-\$0602: $actual"
echo "Ordered absolute subtraction runtime gate passed"
