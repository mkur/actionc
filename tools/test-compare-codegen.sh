#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
source_path="$repo_root/fixtures/mir6502/call_word_arg.act"
out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-codegen-compare-test.XXXXXX")"

cleanup() {
  rm -rf "$out_dir"
}
trap cleanup EXIT

"$script_dir/compare-codegen.sh" \
  --profile modern \
  --no-diffs \
  --out-dir "$out_dir" \
  "$source_path"

artifact_dir="$out_dir/call_word_arg"
for artifact in \
  classic.listing \
  classic.load \
  mir6502.listing \
  mir6502.load \
  mir6502 \
  mir6502.materialized \
  nir
do
  if [[ ! -s "$artifact_dir/$artifact" ]]; then
    echo "missing comparison artifact: $artifact_dir/$artifact" >&2
    exit 1
  fi
done

echo "codegen comparison workflow gate passed"
