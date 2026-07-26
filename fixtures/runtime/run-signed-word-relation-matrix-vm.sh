#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/signed_word_relation_matrix.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/rev02.rom}"
expected="\
00 01 00 01 01 01 00 00 01 01 00 00 01 01 00 00 01 01 00 00 \
00 00 01 01 00 01 00 01 01 01 00 00 01 01 00 00 01 01 00 00 \
00 00 01 01 00 00 01 01 00 01 00 01 01 01 00 00 01 01 00 00 \
00 00 01 01 00 00 01 01 00 00 01 01 00 01 00 01 01 01 00 00 \
00 00 01 01 00 00 01 01 00 00 01 01 00 00 01 01 00 01 00 01 \
a5"

for required in "$source_path" "$vm_root/Cargo.toml" "$cart_rom" "$os_rom"; do
  if [[ ! -f "$required" ]]; then
    echo "Missing runtime dependency: $required" >&2
    exit 1
  fi
done

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-signed-word-matrix.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

object_path="$out_dir/mir6502.com"
memory_path="$out_dir/mir6502.memory.bin"

echo "==> signed word relation matrix: compile modern/mir6502"
(
  cd "$repo_root"
  cargo run --quiet --bin actionc -- \
    --profile modern \
    --backend mir6502 \
    --output "$object_path" \
    "$source_path"
)

echo "==> signed word relation matrix: execute modern/mir6502"
cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
  --cart "$cart_rom" \
  --os "$os_rom" \
  --load-object "$object_path" \
  --dump-memory-on-stop "$memory_path" \
  --max-steps 12000 \
  --history 8

actual="$(od -An -tx1 -j "$((0x0600))" -N 101 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
expected="$(printf '%s' "$expected" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
if [[ "$actual" != "$expected" ]]; then
  echo "FAILED: modern/mir6502 signed word relation matrix" >&2
  echo "  expected: $expected" >&2
  echo "  actual:   $actual" >&2
  exit 1
fi

echo "    results at \$0600-\$0664: $actual"
echo "signed word relation matrix runtime gate passed"
