#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/signed_return_word_zero_compares.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/rev02.rom}"
expected="01 01 00 00 00 00 01 01 01 01 00 00 00 00 01 01 00 01 00 01 00 01 00 01 00 00 01 01 01 01 00 00 00 00 01 01 01 01 00 00 a5"

for required in "$source_path" "$vm_root/Cargo.toml" "$cart_rom" "$os_rom"; do
  if [[ ! -f "$required" ]]; then
    echo "Missing runtime dependency: $required" >&2
    exit 1
  fi
done

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-signed-return-word-zero.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

object_path="$out_dir/mir6502.com"
memory_path="$out_dir/mir6502.memory.bin"
stats_path="$out_dir/mir6502.stats"

echo "==> signed return-word zero compares: compile modern/mir6502"
(
  cd "$repo_root"
  cargo run --quiet --bin actionc -- \
    --profile modern \
    --backend mir6502 \
    --output "$object_path" \
    "$source_path"
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

echo "==> signed return-word zero compares: execute modern/mir6502"
cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
  --cart "$cart_rom" \
  --os "$os_rom" \
  --load-object "$object_path" \
  --dump-memory-on-stop "$memory_path" \
  --max-steps 5000 \
  --history 8

actual="$(od -An -tx1 -j "$((0x0600))" -N 41 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
if [[ "$actual" != "$expected" ]]; then
  echo "FAILED: modern/mir6502 signed return-word zero results" >&2
  echo "  expected: $expected" >&2
  echo "  actual:   $actual" >&2
  exit 1
fi

echo "    results at \$0600-\$0628: $actual"

echo "signed return-word zero compare runtime gate passed"
