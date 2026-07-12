#!/usr/bin/env bash
set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

origin='$3000'
profile="legacy"
out_dir=""
keep=0
max_diffs=80
show_diffs=1

usage() {
  cat <<EOF
Usage: tools/compare-codegen.sh [options] <source.act>...

Compile focused Action! samples with both codegen paths and keep comparable
artifacts side-by-side. This is intended for MIR6502 materialization debugging:
SemIR, NIR, pre/materialized MIR6502, listings, maps, load files, and diffs.

Options:
  --out-dir <dir>       Write artifacts under this directory.
  --keep                Keep a temporary artifact directory and print its path.
  --origin <addr>       Origin passed to actionc, default: $origin.
  --profile <profile>   actionc profile, default: $profile.
  --max-diffs <n>       Lines of each diff to print, default: $max_diffs.
  --no-diffs            Generate diff files but do not print snippets.
  -h, --help            Show this help.

Examples:
  tools/compare-codegen.sh fixtures/mir6502/call_word_arg.act
  tools/compare-codegen.sh --keep surveys/tn/fixtures/LIB_TEST.ACT
  tools/compare-codegen.sh --out-dir target/experiment-outputs/codegen fixtures/mir6502/*.act
EOF
}

safe_stem() {
  local raw="$1"
  raw="$(basename "$raw")"
  raw="${raw%.*}"
  raw="$(printf '%s' "$raw" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9._-' '_')"
  if [[ -z "$raw" ]]; then
    raw="source"
  fi
  printf '%s' "$raw"
}

parse_addr() {
  local value="$1"
  if [[ "$value" == \$* ]]; then
    printf '0x%s' "${value#\$}"
  else
    printf '%s' "$value"
  fi
}

normalize_profile() {
  case "$1" in
    legacy|compat) printf '%s\n' "legacy" ;;
    modern) printf '%s\n' "modern" ;;
    *) echo "invalid --profile value: $1" >&2; exit 2 ;;
  esac
}

run_actionc() {
  local stdout_path="$1"
  local stderr_path="$2"
  shift 2
  (
    cd "$repo_root"
    "$repo_root/target/debug/actionc-emit" "$@"
  ) >"$stdout_path" 2>"$stderr_path"
}

write_hex_dump() {
  local input="$1"
  local output="$2"
  if [[ -s "$input" ]]; then
    xxd -g1 "$input" >"$output"
  else
    : >"$output"
  fi
}

write_normalized_listing() {
  local input="$1"
  local output="$2"
  if [[ ! -s "$input" ]]; then
    : >"$output"
    return
  fi

  sed -E \
    -e 's/^[[:space:]]*[[:xdigit:]]{4}([[:space:]]+[[:xdigit:]]{2}){0,16}[[:space:]]+//' \
    -e 's/L[[:xdigit:]]{4,}/LADDR/g' \
    -e 's/\$[[:xdigit:]]{4}/$ADDR/g' \
    "$input" >"$output"
}

write_ops_listing() {
  local input="$1"
  local output="$2"
  if [[ ! -s "$input" ]]; then
    : >"$output"
    return
  fi

  write_normalized_listing "$input" "$output.tmp"
  sed -E \
    -e '/^; ===== DATA /d' \
    -e '/^[[:space:]]*\.BYTE /d' \
    -e '/^[[:space:]]*BRK$/d' \
    "$output.tmp" >"$output"
  rm -f "$output.tmp"
}

write_symbol_summary() {
  local input="$1"
  local output="$2"
  if [[ ! -s "$input" ]]; then
    : >"$output"
    return
  fi

  awk '
    function lower(value) {
      return tolower(value)
    }
    function strip_dollar(value) {
      gsub(/[$]/, "", value)
      return value
    }
    function byte_count(line, fields, i, count) {
      count = 0
      split(line, fields, /[[:space:]]+/)
      for (i = 2; i <= length(fields); i++) {
        if (fields[i] ~ /^[[:xdigit:]][[:xdigit:]]$/) {
          count++
        } else {
          break
        }
      }
      return count
    }
    /^; ===== DATA / {
      pending_kind = "data"
      pending_name = lower($4)
      pending_addr = strip_dollar($5)
      next
    }
    pending_kind == "data" {
      printf "%-7s %-28s addr=$%s bytes=%d\n", "data", pending_name, pending_addr, byte_count($0)
      pending_kind = ""
      next
    }
    /^; ===== PROC / {
      name = lower($4)
      range = $5
      gsub(/[$]/, "", range)
      split(range, parts, /\.\./)
      entry = ""
      if ($6 == "entry") {
        entry = strip_dollar($7)
      }
      if (entry != "") {
        printf "%-7s %-28s range=$%s..$%s entry=$%s\n", "routine", name, parts[1], parts[2], entry
      } else {
        printf "%-7s %-28s range=$%s..$%s\n", "routine", name, parts[1], parts[2]
      }
    }
  ' "$input" | sort -k2,2 -k1,1 -k3,3 >"$output"
}

write_compact_mir() {
  local input="$1"
  local output="$2"
  if [[ ! -s "$input" ]]; then
    : >"$output"
    return
  fi

  sed -E \
    -e 's/(^|[^[:alnum:]_])(global g[0-9]+|static s[0-9]+|local l[0-9]+|param p[0-9]+|spill sp[0-9]+)\+0([^[:digit:]]|$)/\1\2\3/g' \
    -e 's/(^|[^[:alnum:]_])return\+0([^[:digit:]]|$)/\1return\2/g' \
    -e 's/\((zp\$[[:xdigit:]]{2})\),y\+0/(\1),y/g' \
    -e 's/(load_indirect \([^)]*\),y)\+0/\1/g' \
    -e 's/(store_indirect \([^)]*\),y)\+0/\1/g' \
    "$input" >"$output"
}

write_spill_report() {
  local dir="$1"
  local output="$dir/mir6502.spills"
  local allocated="$output.allocated.tmp"
  local referenced="$output.referenced.tmp"
  local allocated_slots
  local allocated_unique
  local referenced_unique

  if [[ ! -s "$dir/mir6502.symbols" && ! -s "$dir/mir6502.materialized" ]]; then
    : >"$output"
    return
  fi

  awk '$1 == "data" && $2 ~ /^spill[0-9]+$/ { sub(/^spill/, "sp", $2); print $2 }' \
    "$dir/mir6502.symbols" | sort -u >"$allocated"
  tr -cs 'A-Za-z0-9_' '\n' <"$dir/mir6502.materialized" \
    | awk '/^sp[0-9]+$/ { print }' \
    | sort -u >"$referenced"

  allocated_slots="$(awk '$1 == "data" && $2 ~ /^spill[0-9]+$/ { count++ } END { print count + 0 }' "$dir/mir6502.symbols")"
  allocated_unique="$(wc -l <"$allocated" | tr -d '[:space:]')"
  referenced_unique="$(wc -l <"$referenced" | tr -d '[:space:]')"

  {
    printf 'allocated spill slots:      %s\n' "$allocated_slots"
    printf 'unique allocated spill ids: %s\n' "$allocated_unique"
    printf 'referenced spill ids:       %s\n' "$referenced_unique"
    printf '\nallocated-only ids:\n'
    comm -23 "$allocated" "$referenced" | sed 's/^/  /'
    printf '\nreferenced-only ids:\n'
    comm -13 "$allocated" "$referenced" | sed 's/^/  /'
  } >"$output"

  rm -f "$allocated" "$referenced"
}

file_size() {
  local input="$1"
  if [[ -f "$input" ]]; then
    wc -c <"$input" | tr -d '[:space:]'
  else
    printf '0'
  fi
}

write_size_summary() {
  local dir="$1"
  local output="$dir/summary.txt"
  local classic_load
  local mir_load
  classic_load="$(file_size "$dir/classic.load")"
  mir_load="$(file_size "$dir/mir6502.load")"
  {
    printf 'classic.load bytes: %s\n' "$classic_load"
    printf 'mir6502.load bytes: %s\n' "$mir_load"
    if [[ "$classic_load" =~ ^[0-9]+$ && "$mir_load" =~ ^[0-9]+$ ]]; then
      printf 'delta bytes:        %+d\n' "$((mir_load - classic_load))"
    fi
  } >"$output"
}

write_diff() {
  local left="$1"
  local right="$2"
  local output="$3"
  local label="$4"
  if [[ -s "$left" && -s "$right" ]]; then
    diff -u "$left" "$right" >"$output"
    local status=$?
    if [[ $status -eq 0 ]]; then
      printf '  %-22s identical\n' "$label"
    elif [[ $status -eq 1 ]]; then
      printf '  %-22s differs -> %s\n' "$label" "$output"
      if [[ "$show_diffs" -eq 1 ]]; then
        sed -n "1,${max_diffs}p" "$output"
      fi
    else
      printf '  %-22s diff failed\n' "$label"
      return "$status"
    fi
  else
    : >"$output"
    printf '  %-22s skipped; missing output\n' "$label"
  fi
}

emit_classic_artifacts() {
  local source="$1"
  local dir="$2"
  local origin_value="$3"
  local ok=0

  run_actionc "$dir/classic.source-listing" "$dir/classic.source-listing.err" \
    --backend classic --profile "$profile" --origin "$origin_value" --emit-source-listing "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/classic.listing" "$dir/classic.listing.err" \
    --backend classic --profile "$profile" --origin "$origin_value" --emit-listing "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/classic.map" "$dir/classic.map.err" \
    --backend classic --profile "$profile" --origin "$origin_value" --emit-map "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/classic.load" "$dir/classic.load.err" \
    --backend classic --profile "$profile" --origin "$origin_value" --emit-load "$source"
  [[ $? -eq 0 ]] || ok=1
  write_hex_dump "$dir/classic.load" "$dir/classic.load.hex"

  return "$ok"
}

emit_mir6502_artifacts() {
  local source="$1"
  local dir="$2"
  local origin_value="$3"
  local ok=0

  run_actionc "$dir/semir" "$dir/semir.err" --emit-semir "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/nir" "$dir/nir.err" --emit-nir "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/mir6502" "$dir/mir6502.err" --emit-mir6502 "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/mir6502.materialized" "$dir/mir6502.materialized.err" \
    --emit-materialized-mir6502 "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/mir6502.source-listing" "$dir/mir6502.source-listing.err" \
    --backend mir6502 --origin "$origin_value" --emit-source-listing "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/mir6502.listing" "$dir/mir6502.listing.err" \
    --backend mir6502 --origin "$origin_value" --emit-listing "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/mir6502.map" "$dir/mir6502.map.err" \
    --backend mir6502 --origin "$origin_value" --emit-map "$source"
  [[ $? -eq 0 ]] || ok=1

  run_actionc "$dir/mir6502.load" "$dir/mir6502.load.err" \
    --backend mir6502 --origin "$origin_value" --emit-load "$source"
  [[ $? -eq 0 ]] || ok=1
  write_hex_dump "$dir/mir6502.load" "$dir/mir6502.load.hex"

  return "$ok"
}

print_errors() {
  local dir="$1"
  local shown=0
  local err
  for err in "$dir"/*.err; do
    [[ -s "$err" ]] || continue
    if [[ "$shown" -eq 0 ]]; then
      echo "  errors:"
      shown=1
    fi
    echo "    $(basename "$err")"
    sed -n '1,12p' "$err" | sed 's/^/      /'
  done
}

sources=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-dir)
      [[ $# -ge 2 ]] || { echo "--out-dir requires a directory" >&2; exit 2; }
      out_dir="$2"
      shift 2
      ;;
    --keep)
      keep=1
      shift
      ;;
    --origin)
      [[ $# -ge 2 ]] || { echo "--origin requires an address" >&2; exit 2; }
      origin="$2"
      shift 2
      ;;
    --profile)
      [[ $# -ge 2 ]] || { echo "--profile requires legacy or modern" >&2; exit 2; }
      profile="$(normalize_profile "$2")"
      shift 2
      ;;
    --max-diffs)
      [[ $# -ge 2 ]] || { echo "--max-diffs requires a number" >&2; exit 2; }
      max_diffs="$2"
      shift 2
      ;;
    --no-diffs)
      show_diffs=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      while [[ $# -gt 0 ]]; do
        sources+=("$1")
        shift
      done
      ;;
    -*)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      sources+=("$1")
      shift
      ;;
  esac
done

if [[ ${#sources[@]} -eq 0 ]]; then
  usage >&2
  exit 2
fi

cleanup_dir=""
if [[ -z "$out_dir" ]]; then
  out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-codegen-compare.XXXXXX")"
  cleanup_dir="$out_dir"
else
  mkdir -p "$out_dir"
  out_dir="$(cd "$out_dir" && pwd)"
  keep=1
fi

cleanup() {
  if [[ -n "$cleanup_dir" && "$keep" -eq 0 ]]; then
    rm -rf "$cleanup_dir"
  fi
}
trap cleanup EXIT

origin_value="$(parse_addr "$origin")"
failures=0

echo "==> output directory: $out_dir"
echo "==> origin: $origin  profile: $profile"

echo "==> building actionc"
if ! (
  cd "$repo_root"
  cargo build --quiet --bin actionc
) >"$out_dir/build.stdout" 2>"$out_dir/build.stderr"; then
  echo "failed to build actionc; see $out_dir/build.stderr" >&2
  exit 1
fi

for source in "${sources[@]}"; do
  if [[ ! -f "$source" ]]; then
    echo "missing source: $source" >&2
    failures=$((failures + 1))
    continue
  fi

  source="$(cd "$(dirname "$source")" && pwd)/$(basename "$source")"
  stem="$(safe_stem "$source")"
  dir="$out_dir/$stem"
  mkdir -p "$dir"

  echo
  echo "==> $source"
  echo "$source" >"$dir/source.path"

  classic_ok=0
  mir_ok=0
  emit_classic_artifacts "$source" "$dir" "$origin_value"
  [[ $? -eq 0 ]] || classic_ok=1
  emit_mir6502_artifacts "$source" "$dir" "$origin_value"
  [[ $? -eq 0 ]] || mir_ok=1

  write_diff "$dir/classic.source-listing" "$dir/mir6502.source-listing" \
    "$dir/source-listing.diff" "source listing"
  write_diff "$dir/classic.listing" "$dir/mir6502.listing" \
    "$dir/listing.diff" "listing"
  write_normalized_listing "$dir/classic.listing" "$dir/classic.listing.normalized"
  write_normalized_listing "$dir/mir6502.listing" "$dir/mir6502.listing.normalized"
  write_diff "$dir/classic.listing.normalized" "$dir/mir6502.listing.normalized" \
    "$dir/listing.normalized.diff" "normalized listing"
  write_ops_listing "$dir/classic.listing" "$dir/classic.listing.ops"
  write_ops_listing "$dir/mir6502.listing" "$dir/mir6502.listing.ops"
  write_diff "$dir/classic.listing.ops" "$dir/mir6502.listing.ops" \
    "$dir/listing.ops.diff" "instruction listing"
  write_symbol_summary "$dir/classic.listing" "$dir/classic.symbols"
  write_symbol_summary "$dir/mir6502.listing" "$dir/mir6502.symbols"
  write_diff "$dir/classic.symbols" "$dir/mir6502.symbols" \
    "$dir/symbols.diff" "symbol summary"
  write_compact_mir "$dir/mir6502" "$dir/mir6502.compact"
  write_compact_mir "$dir/mir6502.materialized" "$dir/mir6502.materialized.compact"
  write_spill_report "$dir"
  write_diff "$dir/classic.map" "$dir/mir6502.map" \
    "$dir/map.diff" "map"
  write_diff "$dir/classic.load.hex" "$dir/mir6502.load.hex" \
    "$dir/load.hex.diff" "load hex"
  write_size_summary "$dir"

  print_errors "$dir"

  if [[ "$classic_ok" -ne 0 || "$mir_ok" -ne 0 ]]; then
    failures=$((failures + 1))
    echo "  status: classic=$([[ "$classic_ok" -eq 0 ]] && echo ok || echo failed) mir6502=$([[ "$mir_ok" -eq 0 ]] && echo ok || echo failed)"
  else
    echo "  status: both codegen paths emitted artifacts"
  fi
done

if [[ "$keep" -eq 1 ]]; then
  echo
  echo "==> artifacts kept in $out_dir"
fi

exit "$failures"
