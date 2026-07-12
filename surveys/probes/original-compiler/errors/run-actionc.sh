#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$repo_root"

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-negative.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT

status=0
for source in surveys/probes/original-compiler/errors/[0-9][0-9][0-9]_*.act; do
  name="$(basename "$source")"
  if cargo run --quiet --bin actionc -- --profile legacy --backend classic \
    --output "$tmp_dir/actionc-negative.com" "$source" \
    >"$tmp_dir/actionc-negative.out" 2>"$tmp_dir/actionc-negative.err"; then
    echo "UNEXPECTED OK  $name"
    status=1
  else
    echo "EXPECTED FAIL  $name"
  fi
done

exit "$status"
