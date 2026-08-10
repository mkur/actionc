#!/usr/bin/env bash
set -euo pipefail

runtime_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$runtime_dir/../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
source_path="$runtime_dir/indexed_byte_fixed_action_args.act"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/altirraos-xl.rom}"
expected="34 12 55 00 cd ab 11 00 22 00 33 00 44 00 55 00 a5"

for required in "$source_path" "$vm_root/Cargo.toml" "$cart_rom" "$os_rom"; do
  if [[ ! -f "$required" ]]; then
    echo "Missing runtime dependency: $required" >&2
    exit 1
  fi
done

out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-indexed-byte-fixed-args.XXXXXX")"
cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

for backend in classic mir6502; do
  object_path="$out_dir/$backend.com"
  memory_path="$out_dir/$backend.memory.bin"

  echo "==> indexed BYTE fixed Action args: compile modern/$backend"
  (
    cd "$repo_root"
    cargo run --quiet --bin actionc -- \
      --profile modern \
      --backend "$backend" \
      --output "$object_path" \
      "$source_path"
  )

  if [[ "$backend" == "mir6502" ]]; then
    stats_path="$out_dir/$backend.stats"
    (
      cd "$repo_root"
      ACTIONC_MIR6502_PEEPHOLES=summary \
        cargo run --quiet --bin actionc-emit -- \
          --profile modern \
          --backend mir6502 \
          --emit-load \
          "$source_path" \
          > /dev/null \
          2> "$stats_path"
    )
    selected="$(
      sed -n \
        's/^[[:space:]]*indexed-byte-direct-fixed-action-call-arg: //p' \
        "$stats_path"
    )"
    if [[ "$selected" != "6" ]]; then
      echo "FAILED: expected six direct indexed BYTE arguments, got ${selected:-0}" >&2
      exit 1
    fi
  fi

  echo "==> indexed BYTE fixed Action args: execute modern/$backend"
  cargo run --quiet --manifest-path "$vm_root/Cargo.toml" -- run \
    --cart "$cart_rom" \
    --os "$os_rom" \
    --load-object "$object_path" \
    --dump-memory-on-stop "$memory_path" \
    --max-steps 12000 \
    --history 8

  actual="$(
    od -An -tx1 -j "$((0x0600))" -N 17 "$memory_path" |
      tr -s '[:space:]' ' ' |
      sed 's/^ //; s/ $//'
  )"
  if [[ "$actual" != "$expected" ]]; then
    echo "FAILED: modern/$backend indexed BYTE fixed Action arguments" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    exit 1
  fi

  echo "    results at \$0600-\$0610: $actual"
done

echo "indexed BYTE fixed Action argument runtime gate passed"
