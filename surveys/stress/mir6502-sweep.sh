#!/usr/bin/env bash
set -euo pipefail

# Generate unmaterialized MIR6502, materialized MIR6502, and object source
# listings for every .act fixture under fixtures/stress. Output goes to a
# separate directory so generated debugging artifacts do not mix with checked-in
# fixture expectations.
#
# Defaults:
#   fixtures dir: fixtures/stress
#   output dir:   surveys/stress/outputs/mir6502
#   binary:       target/debug/actionc-emit
#
# The MIR CLI flags have changed during development in some local branches. This
# script uses --emit-mir6502 and --emit-materialized-mir6502 by default, but lets
# the caller override the exact commands through MIR_ARGS and
# MATERIALIZED_MIR_ARGS.
#
# Examples:
#   surveys/stress/mir6502-sweep.sh
#   OUT_DIR=target/mir-dumps surveys/stress/mir6502-sweep.sh
#   MIR_ARGS='--emit-mir6502' surveys/stress/mir6502-sweep.sh
#   MATERIALIZED_MIR_ARGS='--emit-mir6502-materialized' surveys/stress/mir6502-sweep.sh
#   ACTIONC_BIN='cargo run -q --bin actionc-emit --' surveys/stress/mir6502-sweep.sh

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

FIXTURE_DIR="${FIXTURE_DIR:-fixtures/stress}"
OUT_DIR="${OUT_DIR:-surveys/stress/outputs/mir6502}"
ACTIONC_BIN="${ACTIONC_BIN:-target/debug/actionc-emit}"
MIR_ARGS="${MIR_ARGS:---emit-mir6502}"
MATERIALIZED_MIR_ARGS="${MATERIALIZED_MIR_ARGS:---emit-materialized-mir6502}"
LISTING_ARGS="${LISTING_ARGS:---profile modern --backend mir6502 --emit-source-listing}"
ORIGIN_ARGS="${ORIGIN_ARGS:-}"

if [[ ! -d "$FIXTURE_DIR" ]]; then
  echo "fixture directory not found: $FIXTURE_DIR" >&2
  exit 2
fi

if [[ "$ACTIONC_BIN" == "target/debug/actionc-emit" && ! -x "$ACTIONC_BIN" ]]; then
  echo "building actionc..." >&2
  cargo build -q --bin actionc
fi

mkdir -p \
  "$OUT_DIR/mir" \
  "$OUT_DIR/materialized-mir" \
  "$OUT_DIR/source-listing" \
  "$OUT_DIR/errors"

# Split user-provided argument strings intentionally through the shell. This keeps
# the script simple for local debugging overrides such as ACTIONC_BIN='cargo run
# -q --bin actionc --'. Keep fixture paths quoted separately below.
run_actionc() {
  local fixture="$1"
  local output="$2"
  local error="$3"
  shift 3
  local args=("$@")

  # shellcheck disable=SC2086
  if $ACTIONC_BIN ${ORIGIN_ARGS} "${args[@]}" "$fixture" >"$output" 2>"$error"; then
    return 0
  fi
  return 1
}

count=0
ok_mir=0
ok_materialized_mir=0
ok_listing=0
failed=0

while IFS= read -r -d '' fixture; do
  rel="${fixture#${FIXTURE_DIR}/}"
  stem="${rel%.act}"
  stem_path="${stem//\//__}"

  mir_out="$OUT_DIR/mir/${stem_path}.mir6502"
  mir_err="$OUT_DIR/errors/${stem_path}.mir.err"
  materialized_mir_out="$OUT_DIR/materialized-mir/${stem_path}.mir6502"
  materialized_mir_err="$OUT_DIR/errors/${stem_path}.materialized-mir.err"
  listing_out="$OUT_DIR/source-listing/${stem_path}.lst"
  listing_err="$OUT_DIR/errors/${stem_path}.source-listing.err"

  count=$((count + 1))
  echo "[$count] $fixture"

  # shellcheck disable=SC2206
  mir_args=($MIR_ARGS)
  if run_actionc "$fixture" "$mir_out" "$mir_err" "${mir_args[@]}"; then
    ok_mir=$((ok_mir + 1))
    rm -f "$mir_err"
  else
    failed=$((failed + 1))
    printf 'FAILED MIR: %s\n' "$fixture" >&2
  fi

  # shellcheck disable=SC2206
  materialized_args=($MATERIALIZED_MIR_ARGS)
  if run_actionc "$fixture" "$materialized_mir_out" "$materialized_mir_err" "${materialized_args[@]}"; then
    ok_materialized_mir=$((ok_materialized_mir + 1))
    rm -f "$materialized_mir_err"
  else
    failed=$((failed + 1))
    printf 'FAILED materialized MIR: %s\n' "$fixture" >&2
  fi

  # shellcheck disable=SC2206
  listing_args=($LISTING_ARGS)
  if run_actionc "$fixture" "$listing_out" "$listing_err" "${listing_args[@]}"; then
    ok_listing=$((ok_listing + 1))
    rm -f "$listing_err"
  else
    failed=$((failed + 1))
    printf 'FAILED source listing: %s\n' "$fixture" >&2
  fi

done < <(find "$FIXTURE_DIR" -type f -name '*.act' -print0 | sort -z)

cat >"$OUT_DIR/README.txt" <<EOF
Generated MIR6502 fixture dumps

Fixture directory: $FIXTURE_DIR
Generated at: $(date -u '+%Y-%m-%dT%H:%M:%SZ')
Action compiler: $ACTIONC_BIN
MIR args: $MIR_ARGS
Materialized MIR args: $MATERIALIZED_MIR_ARGS
Source listing args: $LISTING_ARGS
Origin args: ${ORIGIN_ARGS:-<none>}

Outputs:
- mir/*.mir6502
- materialized-mir/*.mir6502
- source-listing/*.lst
- errors/*.err, only for failed commands

Summary:
- fixtures: $count
- MIR succeeded: $ok_mir
- materialized MIR succeeded: $ok_materialized_mir
- source listings succeeded: $ok_listing
- command failures: $failed
EOF

printf '\nGenerated dumps under %s\n' "$OUT_DIR"
printf 'fixtures=%d mir_ok=%d materialized_mir_ok=%d source_listing_ok=%d failures=%d\n' \
  "$count" "$ok_mir" "$ok_materialized_mir" "$ok_listing" "$failed"

if [[ "$failed" -ne 0 ]]; then
  echo "Some commands failed. See $OUT_DIR/errors." >&2
  exit 1
fi
