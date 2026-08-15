#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

usage() {
  cat <<'EOF'
Usage: tools/check-mads-listings.sh

Generate plain and source-annotated actionc listings, assemble them with MADS,
and compare the complete Atari load files byte for byte. The re-origining
contract fixture is also assembled at two edited origins and compared with
direct actionc compilation at those origins.

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

rewrite_origin_definition() {
  local input="$1"
  local output="$2"
  local old_origin="$3"
  local new_origin="$4"
  local old_definition="ACTIONC_ORIGIN = $old_origin"
  local new_definition="ACTIONC_ORIGIN = $new_origin"
  local matches

  matches="$(grep -Fxc -- "$old_definition" "$input" || true)"
  if [[ "$matches" != 1 ]]; then
    echo "expected exactly one editable origin definition in $input, found $matches" >&2
    return 1
  fi
  awk -v old="$old_definition" -v new="$new_definition" \
    '{ if ($0 == old) print new; else print }' "$input" >"$output"
  if [[ "$(grep -Fxc -- "$new_definition" "$output" || true)" != 1 ]]; then
    echo "failed to rewrite the editable origin definition in $input" >&2
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

run_reorigin_case() {
  local name="$1"
  local mode="$2"
  local profile="$3"
  local backend="$4"
  local source="$repo_root/$5"
  local origin_a="$6"
  local origin_b="$7"
  local actionc_a="$oracle_dir/$name.a.actionc.xex"
  local actionc_b="$oracle_dir/$name.b.actionc.xex"
  local plain_a="$oracle_dir/$name.a.asm"
  local source_a="$oracle_dir/$name.a.source.asm"
  local plain_b="$oracle_dir/$name.b.asm"
  local source_b="$oracle_dir/$name.b.source.asm"
  local plain_a_load="$oracle_dir/$name.a.mads.xex"
  local source_a_load="$oracle_dir/$name.a.source.mads.xex"
  local plain_b_load="$oracle_dir/$name.b.mads.xex"
  local source_b_load="$oracle_dir/$name.b.source.mads.xex"

  "$actionc_bin" --mode "$mode" --origin "$origin_a" --output "$actionc_a" \
    --listing "$source_a" "$source"
  "$actionc_bin" --mode "$mode" --origin "$origin_b" --output "$actionc_b" \
    "$source"
  "$emit_bin" --profile "$profile" --backend "$backend" --origin "$origin_a" \
    --emit-listing "$source" >"$plain_a"

  "$mads_bin" "$plain_a" "-o:$plain_a_load" -s
  compare_load_files "$actionc_a" "$plain_a_load" "$name unchanged plain listing"
  "$mads_bin" "$source_a" "-o:$source_a_load" -s
  compare_load_files "$actionc_a" "$source_a_load" "$name unchanged source listing"

  rewrite_origin_definition "$plain_a" "$plain_b" "$origin_a" "$origin_b"
  rewrite_origin_definition "$source_a" "$source_b" "$origin_a" "$origin_b"
  "$mads_bin" "$plain_b" "-o:$plain_b_load" -s
  compare_load_files "$actionc_b" "$plain_b_load" "$name re-originated plain listing"
  "$mads_bin" "$source_b" "-o:$source_b_load" -s
  compare_load_files "$actionc_b" "$source_b_load" "$name re-originated source listing"

  echo "PASS  $name ($origin_a -> $origin_b)"
}

run_case "hello-compatibility" "compatibility" "legacy" "classic" \
  "samples/hello-world.act"
run_case "contract-compatibility" "compatibility" "legacy" "classic" \
  "fixtures/listing/mads_contract.act"
run_case "contract-optimized" "optimized" "modern" "classic" \
  "fixtures/listing/mads_contract.act"
run_case "contract-mir6502" "mir6502" "modern" "mir6502" \
  "fixtures/listing/mads_contract.act"

for origin_pair in '$3000:$41C7' '$2B40:$52D3'; do
  origin_a="${origin_pair%%:*}"
  origin_b="${origin_pair##*:}"
  pair_name="${origin_a#\$}-to-${origin_b#\$}"
  run_reorigin_case "reorigin-compatibility-$pair_name" \
    "compatibility" "legacy" "classic" \
    "fixtures/listing/mads_reorigin_contract.act" "$origin_a" "$origin_b"
  run_reorigin_case "reorigin-optimized-$pair_name" \
    "optimized" "modern" "classic" \
    "fixtures/listing/mads_reorigin_contract.act" "$origin_a" "$origin_b"
  run_reorigin_case "reorigin-mir6502-$pair_name" \
    "mir6502" "modern" "mir6502" \
    "fixtures/listing/mads_reorigin_contract.act" "$origin_a" "$origin_b"
done

echo "MADS listing oracle passed: 10 compiler cases, 32 assembled listings"
