#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/kalscope_codegen_patterns.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/rev02.rom}"
expected="11 22 33 44 af 45 af 45 82 84 1f"

for required in "$source_path" "$vm_root/Cargo.toml" "$cart_rom" "$os_rom"; do
  if [[ ! -f "$required" ]]; then
    echo "Missing runtime dependency: $required" >&2
    exit 1
  fi
done

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-kalscope-patterns.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

for backend in classic mir6502; do
  object_path="$out_dir/$backend.com"
  memory_path="$out_dir/$backend.memory.bin"

  echo "==> KALSCOPE codegen patterns: compile modern/$backend"
  (
    cd "$repo_root"
    cargo run --quiet --bin actionc -- \
      --profile modern \
      --backend "$backend" \
      --output "$object_path" \
      "$source_path"
  )

  echo "==> KALSCOPE codegen patterns: execute modern/$backend"
  cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
    --cart "$cart_rom" \
    --os "$os_rom" \
    --load-object "$object_path" \
    --dump-memory-on-stop "$memory_path" \
    --max-steps 40000 \
    --history 8

  actual="$(od -An -tx1 -j "$((0x0600))" -N 11 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
  actual_signature="$(od -An -tx1 -j "$((0x0610))" -N 1 "$memory_path" | tr -d '[:space:]')"
  if [[ "$actual" != "$expected" || "$actual_signature" != "a5" ]]; then
    echo "FAILED: modern/$backend KALSCOPE codegen patterns" >&2
    echo "  expected:  $expected; signature a5" >&2
    echo "  actual:    $actual; signature $actual_signature" >&2
    exit 1
  fi

  echo "    results at \$0600-\$060A: $actual; signature $actual_signature"
done

echo "KALSCOPE codegen-pattern runtime gate passed"
