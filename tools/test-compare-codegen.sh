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
  classic.listing.normalized \
  classic.listing.ops \
  classic.load \
  classic.symbols \
  mir6502.listing \
  mir6502.listing.normalized \
  mir6502.listing.ops \
  mir6502.load \
  mir6502.symbols \
  mir6502 \
  mir6502.materialized \
  nir
do
  if [[ ! -s "$artifact_dir/$artifact" ]]; then
    echo "missing comparison artifact: $artifact_dir/$artifact" >&2
    exit 1
  fi
done

if ! grep -q 'ORG $02E2' "$artifact_dir/classic.listing"; then
  echo "classic listing is missing the MADS RUNAD origin" >&2
  exit 1
fi
if grep -Eq '; \$[[:xdigit:]]{4}: [[:xdigit:]]{2}' \
  "$artifact_dir/classic.listing.normalized"
then
  echo "normalized listing retained generated address/byte comments" >&2
  exit 1
fi
if grep -Eq '^[[:space:]]*(\.BYTE|ORG|DTA)([[:space:]]|$)' \
  "$artifact_dir/classic.listing.ops"
then
  echo "instruction-only listing retained MADS data/segment directives" >&2
  exit 1
fi
if ! grep -q '^routine ' "$artifact_dir/classic.symbols"; then
  echo "classic symbol summary is missing routine boundaries" >&2
  exit 1
fi

echo "codegen comparison workflow gate passed"
