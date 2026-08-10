#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/circle_int_math.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/altirraos-xl.rom}"
expected_classic="40 9c ff 7f 00 00 80 7f 00 02 80 05 80 01 01 00 00 00 00 01 01 00 01 01 00 01 00 00 00 a5"
expected_mir6502="40 9c ff 7f 00 00 80 7f 00 02 80 05 80 01 01 00 00 00 00 01 01 01 00 01 00 01 00 01 00 a5"

for required in "$source_path" "$vm_root/Cargo.toml" "$cart_rom" "$os_rom"; do
  if [[ ! -f "$required" ]]; then
    echo "Missing runtime dependency: $required" >&2
    exit 1
  fi
done

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-circle-int-math.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

for backend in classic mir6502; do
  object_path="$out_dir/$backend.com"
  memory_path="$out_dir/$backend.memory.bin"

  echo "==> CIRCLE INT arithmetic: compile modern/$backend"
  (
    cd "$repo_root"
    cargo run --quiet --bin actionc -- \
      --profile modern \
      --backend "$backend" \
      --output "$object_path" \
      "$source_path"
  )

  echo "==> CIRCLE INT arithmetic: execute modern/$backend"
  cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
    --cart "$cart_rom" \
    --os "$os_rom" \
    --load-object "$object_path" \
    --dump-memory-on-stop "$memory_path" \
    --max-steps 20000 \
    --history 8

  actual="$(od -An -tx1 -j "$((0x0600))" -N 30 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
  if [[ "$backend" == "classic" ]]; then
    expected="$expected_classic"
  else
    expected="$expected_mir6502"
  fi
  if [[ "$actual" != "$expected" ]]; then
    echo "FAILED: modern/$backend CIRCLE INT arithmetic results" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    exit 1
  fi

  echo "    results at \$0600-\$061D: $actual"
done

echo "CIRCLE INT arithmetic runtime gate passed"
