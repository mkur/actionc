#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/sort_runtime.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/rev02.rom}"

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

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-sort-runtime.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

expected="a5 0b d1 d2 c2 c1 c4 c3 d2 d1 d4 d3 e2 e1 e4 e3 \
80 00 ff \
00 00 01 7f 80 80 ff \
ff 80 80 7f 01 00 00 \
00 00 01 7f 80 80 ff \
00 00 ff 00 ff 00 00 01 ff 7f 00 80 ff ff \
ff ff 00 80 ff 7f 00 01 ff 00 ff 00 00 00 \
00 80 ff ff ff ff 00 00 01 00 ff 7f \
ff 7f 01 00 00 00 ff ff ff ff 00 80 \
01 41 02 41 02 41 03 41 01 42 \
01 42 03 41 02 41 02 41 01 41"

for backend in classic mir6502; do
  object_path="$out_dir/$backend.com"
  memory_path="$out_dir/$backend.memory.bin"

  echo "==> SORT runtime: compile modern/$backend"
  (
    cd "$repo_root"
    cargo run --quiet --bin actionc -- \
      --profile modern \
      --backend "$backend" \
      --output "$object_path" \
      "$source_path"
  )

  echo "==> SORT runtime: execute modern/$backend"
  cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
    --cart "$cart_rom" \
    --os "$os_rom" \
    --load-object "$object_path" \
    --dump-memory-on-stop "$memory_path" \
    --max-steps 200000 \
    --history 8

  actual="$(od -An -tx1 -j "$((0x0600))" -N 112 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
  if [[ "$actual" != "$expected" ]]; then
    echo "FAILED: modern/$backend SORT runtime results" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    exit 1
  fi

  echo "    results at \$0600-\$066F: $actual"
done

echo "SORT runtime gate passed"
