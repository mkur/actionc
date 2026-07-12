#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

out_dir="${TMPDIR:-/tmp}/actionc-stress"
mkdir -p "$out_dir"

status=0

expected_failure_reason() {
  case "$1" in
    zero_page)
      echo "policy mismatch: actionc treats zero-page pointer initializers as storage aliases"
      ;;
    *)
      return 1
      ;;
  esac
}

for src in "$root"/fixtures/stress/*.act; do
  name="$(basename "$src" .act)"
  out="$out_dir/$name.com"
  err="$out_dir/$name.err"
  if cargo run --quiet --bin actionc -- --output "$out" "$src" 2>"$err"; then
    bytes="$(wc -c < "$out" | tr -d ' ')"
    printf 'OK    %-24s %s bytes  %s\n' "$name" "$bytes" "$out"
  else
    if reason="$(expected_failure_reason "$name")"; then
      printf 'XFAIL %-24s %s\n' "$name" "$reason"
    else
      cat "$err" >&2
      printf 'FAIL  %s\n' "$name"
      status=1
    fi
  fi
done

exit "$status"
