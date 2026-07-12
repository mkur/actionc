#!/usr/bin/env bash
set -euo pipefail

# Classify MIR6502 stress dump errors by verifier diagnostic bucket.
#
# Run after:
#   surveys/stress/mir6502-sweep.sh
#
# Usage:
#   surveys/stress/classify-mir6502-errors.sh
#   OUT_DIR=target/mir-dumps surveys/stress/classify-mir6502-errors.sh

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

OUT_DIR="${OUT_DIR:-surveys/stress/outputs/mir6502}"
ERROR_DIR="$OUT_DIR/errors"

if [[ ! -d "$ERROR_DIR" ]]; then
  echo "stress error directory not found: $ERROR_DIR" >&2
  exit 2
fi

diagnostic_class() {
  local line="$1"

  case "$line" in
    *"computed index addresses must be materialized before emission"*)
      printf 'computed index addresses must be materialized before emission'
      ;;
    *"dynamic word index addresses must be materialized before emission"*)
      printf 'dynamic word index addresses must be materialized before emission'
      ;;
    *"dynamic pointer word index addresses must be materialized before emission"*)
      printf 'dynamic pointer word index addresses must be materialized before emission'
      ;;
    *"pre-emission MIR cannot contain word-width pseudo ops"*)
      printf 'pre-emission MIR cannot contain word-width pseudo ops'
      ;;
    *"pre-emission MIR cannot contain virtual temp byte"*)
      printf 'pre-emission MIR cannot contain virtual temp byte'
      ;;
    *"pre-emission MIR cannot contain virtual temp"*)
      printf 'pre-emission MIR cannot contain virtual temp'
      ;;
    *"pre-emission MIR cannot contain abstract bool branch conditions"*)
      printf 'pre-emission MIR cannot contain abstract bool branch conditions'
      ;;
    *"call arity mismatch"*)
      printf 'raw MIR/NIR failure: call arity mismatch'
      ;;
    *)
      printf 'other'
      ;;
  esac
}

bucket_rows="$(mktemp "${TMPDIR:-/tmp}/mir6502-stress-buckets.XXXXXX")"
fixture_rows="$(mktemp "${TMPDIR:-/tmp}/mir6502-stress-fixtures.XXXXXX")"
raw_rows="$(mktemp "${TMPDIR:-/tmp}/mir6502-stress-raw.XXXXXX")"
listing_rows="$(mktemp "${TMPDIR:-/tmp}/mir6502-stress-listing.XXXXXX")"
trap 'rm -f "$bucket_rows" "$fixture_rows" "$raw_rows" "$listing_rows"' EXIT

shopt -s nullglob
for file in "$ERROR_DIR"/*.err; do
  name="$(basename "$file")"
  fixture="${name%%.*}"
  phase="${name#*.}"
  phase="${phase%.err}"

  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "$line" ]] && continue
    class="$(diagnostic_class "$line")"

    if [[ "$class" == raw\ MIR/NIR\ failure:* ]]; then
      printf '%s\t%s\n' "$fixture" "$class" >>"$raw_rows"
      continue
    fi

    if [[ "$phase" == "mir" ]]; then
      printf '%s\t%s\n' "$fixture" "raw MIR/NIR failure: $class" >>"$raw_rows"
      continue
    fi

    if [[ "$phase" == "source-listing" ]]; then
      printf '%s\t%s\n' "$fixture" "source-listing / emission failure: $class" >>"$listing_rows"
      continue
    fi

    if [[ "$phase" != "materialized-mir" ]]; then
      continue
    fi

    printf '%s\n' "$class" >>"$bucket_rows"
    printf '%s\t%s\n' "$fixture" "$class" >>"$fixture_rows"
  done < "$file"
done

echo "MIR6502 stress materialization error classes"
echo "Output directory: $OUT_DIR"
echo

echo "Materialized MIR buckets:"
if [[ ! -s "$bucket_rows" ]]; then
  echo "  <none>"
else
  sort "$bucket_rows" | uniq -c | sort -nr | awk '{$1=sprintf("%6d", $1); print}'
fi
echo

echo "Materialized MIR by fixture:"
if [[ ! -s "$fixture_rows" ]]; then
  echo "  <none>"
else
  sort "$fixture_rows" | uniq -c | awk -F '\t' '{
    count=$1
    sub(/^[[:space:]]+/, "", count)
    split($0, parts, "\t")
    fixture=parts[1]
    sub(/^[[:space:]]*[0-9]+[[:space:]]+/, "", fixture)
    printf "%-24s %6d  %s\n", fixture, count, parts[2]
  }' | sort
fi
echo

echo "Raw MIR/NIR failures:"
if [[ ! -s "$raw_rows" ]]; then
  echo "  <none>"
else
  sort "$raw_rows" | uniq -c | awk -F '\t' '{
    count=$1
    sub(/^[[:space:]]+/, "", count)
    split($0, parts, "\t")
    fixture=parts[1]
    sub(/^[[:space:]]*[0-9]+[[:space:]]+/, "", fixture)
    printf "%-24s %6d  %s\n", fixture, count, parts[2]
  }' | sort
fi
echo

echo "Source-listing / emission failures:"
if [[ ! -s "$listing_rows" ]]; then
  echo "  <none>"
else
  sort "$listing_rows" | uniq -c | awk -F '\t' '{
    count=$1
    sub(/^[[:space:]]+/, "", count)
    split($0, parts, "\t")
    fixture=parts[1]
    sub(/^[[:space:]]*[0-9]+[[:space:]]+/, "", fixture)
    printf "%-24s %6d  %s\n", fixture, count, parts[2]
  }' | sort
fi
