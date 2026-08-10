#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/dual_indexed_word_compares.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/altirraos-xl.rom}"
expected="01 01 01 01 00 00 00 00 01 a5"

for required in "$source_path" "$vm_root/Cargo.toml" "$cart_rom" "$os_rom"; do
  if [[ ! -f "$required" ]]; then
    echo "Missing runtime dependency: $required" >&2
    exit 1
  fi
done

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-dual-indexed-word-compares.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

for backend in classic mir6502; do
  object_path="$out_dir/$backend.com"
  memory_path="$out_dir/$backend.memory.bin"

  echo "==> dual indexed word compares: compile modern/$backend"
  (
    cd "$repo_root"
    cargo run --quiet --bin actionc -- \
      --profile modern \
      --backend "$backend" \
      --output "$object_path" \
      "$source_path"
  )

  if [[ "$backend" == "mir6502" ]]; then
    materialized_path="$out_dir/$backend.mir"
    (
      cd "$repo_root"
      cargo run --quiet --bin actionc-emit -- \
        --profile modern \
        --backend mir6502 \
        --emit-materialized-mir6502 \
        "$source_path" \
        > "$materialized_path"
    )
    selected="$(grep -c 'cmp_indirect\.w' "$materialized_path" || true)"
    if [[ "$selected" != "5" ]]; then
      echo "FAILED: expected five direct indexed word compares, got $selected" >&2
      exit 1
    fi
  fi

  echo "==> dual indexed word compares: execute modern/$backend"
  cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
    --cart "$cart_rom" \
    --os "$os_rom" \
    --load-object "$object_path" \
    --dump-memory-on-stop "$memory_path" \
    --max-steps 2500 \
    --history 8

  actual="$(od -An -tx1 -j "$((0x0600))" -N 10 "$memory_path" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
  if [[ "$actual" != "$expected" ]]; then
    echo "FAILED: modern/$backend dual indexed word compare results" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    exit 1
  fi

  echo "    results at \$0600-\$0609: $actual"
done

echo "dual indexed word compare runtime gate passed"
