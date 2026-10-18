#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: surveys/tn/check-stability.sh [options]

Compile the full TOMS Navigator source with actionc compat and modern
profiles, then compare load-file size against an original Action! compiler
TN.COM baseline. This is a large-source stability guard, not a byte-exact
small probe.

Options:
  --source <path>      TN source for both profiles
  --compat-source <p>  compat source, default: corpora/tn/original/extracted/SRC/TN.ACT.atascii
  --modern-source <p>  modern source, default: samples/tn/modern/TN.ACT
  --original <path>    original Action! compiler TN.COM baseline
                      default: corpora/tn/original/extracted/TN.COM
  --budget <bytes>     maximum absolute size delta for compat, default: 512
  --modern-budget <n>  maximum absolute size delta for modern, default: 1792
  --out-dir <dir>      write generated files here, default: temp dir
  --keep               keep temporary output and print its path
  -h, --help           show this help

The check exits nonzero when a generated profile exceeds its configured
budget. Use a larger budget only when documenting an intentional layout shift.
USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"

compat_source_path="$repo_root/corpora/tn/original/extracted/SRC/TN.ACT.atascii"
modern_source_path="$repo_root/samples/tn/modern/TN.ACT"
original_path="$repo_root/corpora/tn/original/extracted/TN.COM"
compat_budget=512
modern_budget=1792
out_dir=""
keep=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source)
      [[ $# -ge 2 ]] || { echo "--source requires a path" >&2; exit 2; }
      compat_source_path="$2"
      modern_source_path="$2"
      shift 2
      ;;
    --compat-source)
      [[ $# -ge 2 ]] || { echo "--compat-source requires a path" >&2; exit 2; }
      compat_source_path="$2"
      shift 2
      ;;
    --modern-source)
      [[ $# -ge 2 ]] || { echo "--modern-source requires a path" >&2; exit 2; }
      modern_source_path="$2"
      shift 2
      ;;
    --original)
      [[ $# -ge 2 ]] || { echo "--original requires a path" >&2; exit 2; }
      original_path="$2"
      shift 2
      ;;
    --budget)
      [[ $# -ge 2 ]] || { echo "--budget requires a byte count" >&2; exit 2; }
      compat_budget="$2"
      shift 2
      ;;
    --modern-budget)
      [[ $# -ge 2 ]] || { echo "--modern-budget requires a byte count" >&2; exit 2; }
      modern_budget="$2"
      shift 2
      ;;
    --out-dir)
      [[ $# -ge 2 ]] || { echo "--out-dir requires a directory" >&2; exit 2; }
      out_dir="$2"
      keep=1
      shift 2
      ;;
    --keep)
      keep=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ ! -f "$compat_source_path" ]]; then
  echo "missing TN compat source: $compat_source_path" >&2
  exit 1
fi
if [[ ! -f "$modern_source_path" ]]; then
  echo "missing TN modern source: $modern_source_path" >&2
  exit 1
fi
if [[ ! -f "$original_path" ]]; then
  echo "missing original baseline: $original_path" >&2
  exit 1
fi

cleanup_dir=""
if [[ -z "$out_dir" ]]; then
  out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-tn-stability.XXXXXX")"
  cleanup_dir="$out_dir"
else
  mkdir -p "$out_dir"
  out_dir="$(cd "$out_dir" && pwd)"
fi

cleanup() {
  if [[ -n "$cleanup_dir" && "$keep" -eq 0 ]]; then
    rm -rf "$cleanup_dir"
  fi
}
trap cleanup EXIT

cd "$repo_root"

compat_out="$out_dir/TN-compat.COM"
modern_out="$out_dir/TN-modern.COM"

cargo run --quiet --bin actionc -- --profile compat --output "$compat_out" "$compat_source_path"
cargo run --quiet --bin actionc -- --profile modern --output "$modern_out" "$modern_source_path"

original_size="$(wc -c < "$original_path" | tr -d ' ')"
compat_size="$(wc -c < "$compat_out" | tr -d ' ')"
modern_size="$(wc -c < "$modern_out" | tr -d ' ')"

delta() {
  local size="$1"
  echo $((size - original_size))
}

abs() {
  local value="$1"
  if (( value < 0 )); then
    echo $((-value))
  else
    echo "$value"
  fi
}

compat_delta="$(delta "$compat_size")"
modern_delta="$(delta "$modern_size")"
compat_abs="$(abs "$compat_delta")"
modern_abs="$(abs "$modern_delta")"

printf 'TN compat source:      %s\n' "$compat_source_path"
printf 'TN modern source:      %s\n' "$modern_source_path"
printf 'TN original baseline:  %s\n' "$original_path"
printf 'TN original load size: %6d bytes\n' "$original_size"
printf 'TN compat load size:   %6d bytes  delta %+d  budget +/- %d\n' \
  "$compat_size" "$compat_delta" "$compat_budget"
printf 'TN modern load size:   %6d bytes  delta %+d  budget +/- %d\n' \
  "$modern_size" "$modern_delta" "$modern_budget"

status=0
if (( compat_abs > compat_budget )); then
  echo "FAIL compat: TN size delta exceeds budget"
  status=1
fi
if (( modern_abs > modern_budget )); then
  echo "FAIL modern: TN size delta exceeds budget"
  status=1
fi

if [[ "$keep" -eq 1 ]]; then
  printf 'TN generated outputs:  %s\n' "$out_dir"
fi

exit "$status"
