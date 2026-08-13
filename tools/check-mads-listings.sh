#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

usage() {
  cat <<'EOF'
Usage: tools/check-mads-listings.sh

Generate plain and source-annotated actionc listings, assemble them with MADS,
and compare the complete Atari load files byte for byte.

Set ACTIONC_MADS to select a MADS executable that is not on PATH.
EOF
}

if [[ $# -gt 0 ]]; then
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unexpected argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
fi

mads_request="${ACTIONC_MADS:-mads}"
if [[ "$mads_request" == */* ]]; then
  if [[ ! -f "$mads_request" || ! -x "$mads_request" ]]; then
    echo "MADS executable is not an executable regular file: $mads_request" >&2
    exit 2
  fi
  mads_bin="$mads_request"
else
  mads_bin="$(command -v "$mads_request" || true)"
  if [[ -z "$mads_bin" ]]; then
    echo "MADS executable not found: $mads_request" >&2
    echo "install MADS or set ACTIONC_MADS=/path/to/mads" >&2
    exit 2
  fi
fi

mads_version="$("$mads_bin" 2>&1 | sed -n '1p' || true)"
if [[ -z "$mads_version" ]]; then
  mads_version="version unavailable"
fi
echo "==> MADS: $mads_version"

oracle_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-mads-listings.XXXXXX")"
oracle_parent="$(cd "$(dirname "$oracle_dir")" && pwd -P)"
oracle_name="$(basename "$oracle_dir")"

cleanup() {
  local exit_code=$?
  if [[ $exit_code -ne 0 ]]; then
    echo "MADS oracle artifacts retained in $oracle_dir" >&2
    return "$exit_code"
  fi
  if [[ "$oracle_name" != actionc-mads-listings.* ]]; then
    echo "refusing to remove unexpected oracle directory name: $oracle_dir" >&2
    return
  fi
  if [[ ! -d "$oracle_dir" || -L "$oracle_dir" ]]; then
    echo "refusing to remove unexpected oracle path: $oracle_dir" >&2
    return
  fi
  local resolved_parent
  resolved_parent="$(cd "$(dirname "$oracle_dir")" && pwd -P)" || return
  if [[ "$resolved_parent" != "$oracle_parent" ]]; then
    echo "refusing to remove relocated oracle path: $oracle_dir" >&2
    return
  fi
  rm -rf -- "$oracle_dir"
}
trap cleanup EXIT

echo "==> building actionc and actionc-emit"
cargo build --quiet --manifest-path "$repo_root/Cargo.toml" --bin actionc --bin actionc-emit
actionc_bin="$repo_root/target/debug/actionc"
emit_bin="$repo_root/target/debug/actionc-emit"

compare_load_files() {
  local expected="$1"
  local actual="$2"
  local label="$3"
  if [[ ! -s "$actual" ]]; then
    echo "$label did not produce a MADS load file: $actual" >&2
    return 1
  fi
  if ! cmp -s "$expected" "$actual"; then
    echo "$label differs from actionc output" >&2
    echo "  actionc: $expected" >&2
    echo "  MADS:    $actual" >&2
    return 1
  fi
}

run_case() {
  local name="$1"
  local mode="$2"
  local profile="$3"
  local backend="$4"
  local source="$repo_root/$5"
  local actionc_load="$oracle_dir/$name.actionc.xex"
  local plain_asm="$oracle_dir/$name.asm"
  local source_asm="$oracle_dir/$name.source.asm"
  local plain_load="$oracle_dir/$name.mads.xex"
  local source_load="$oracle_dir/$name.source.mads.xex"

  "$actionc_bin" --mode "$mode" --output "$actionc_load" \
    --listing "$source_asm" "$source"
  "$emit_bin" --profile "$profile" --backend "$backend" \
    --emit-listing "$source" >"$plain_asm"

  "$mads_bin" "$plain_asm" "-o:$plain_load" -s
  compare_load_files "$actionc_load" "$plain_load" "$name plain listing"

  "$mads_bin" "$source_asm" "-o:$source_load" -s
  compare_load_files "$actionc_load" "$source_load" "$name source listing"

  echo "PASS  $name"
}

run_case "hello-compatibility" "compatibility" "legacy" "classic" \
  "samples/hello-world.act"
run_case "contract-compatibility" "compatibility" "legacy" "classic" \
  "fixtures/listing/mads_contract.act"
run_case "contract-optimized" "optimized" "modern" "classic" \
  "fixtures/listing/mads_contract.act"
run_case "contract-mir6502" "mir6502" "modern" "mir6502" \
  "fixtures/listing/mads_contract.act"

echo "MADS listing oracle passed: 4 compiler cases, 8 assembled listings"
