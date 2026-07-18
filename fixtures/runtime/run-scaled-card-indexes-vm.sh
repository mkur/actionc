#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/scaled_card_indexes.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/rev02.rom}"
expected="00 11 01 22 7f 33 80 44 ff 55 80 44 7f 33 ff 55"

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

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-scaled-card-indexes.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

object_path="$out_dir/classic.com"
memory_path="$out_dir/classic.memory.bin"

echo "==> scaled CARD indexes: compile modern/classic"
(
  cd "$repo_root"
  cargo run --quiet --bin actionc -- \
    --profile modern \
    --backend classic \
    --output "$object_path" \
    "$source_path"
)

echo "==> scaled CARD indexes: execute modern/classic"
cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
  --cart "$cart_rom" \
  --os "$os_rom" \
  --load-object "$object_path" \
  --dump-memory-on-stop "$memory_path" \
  --max-steps 800 \
  --history 8

actual="$(od -An -tx1 -j "$((0x0600))" -N 16 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
if [[ "$actual" != "$expected" ]]; then
  echo "FAILED: modern/classic scaled CARD index results" >&2
  echo "  expected: $expected" >&2
  echo "  actual:   $actual" >&2
  exit 1
fi

echo "    results at \$0600-\$060F: $actual"
echo "scaled CARD index runtime gate passed"
