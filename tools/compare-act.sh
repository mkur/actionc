#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/rev02.rom}"
max_steps="${ACTION_VM_MAX_STEPS:-60000000}"
out_dir=""
keep=0
max_diffs=20
disassemble_original=0
origin_args=()
compare_mode="all"

usage() {
  cat <<EOF
Usage: tools/compare-act.sh [options] <source.act>

Compile one ACT file with the original Action! compiler VM, then run
actionc-compare against the captured load file. The comparison includes both
actionc legacy and modern profiles.

Options:
  --out-dir <dir>          Keep VM artifacts in this directory.
  --keep                  Keep artifacts in a temporary directory and print it.
  --name <stem>            Override generated Atari H: file/output stem.
  --origin <addr>          Pass an explicit origin to actionc-compare.
  --mode <mode>            actionc-compare mode: all, legacy, modern, profiles.
  --max-diffs <n>          Diff lines passed to actionc-compare, default: $max_diffs.
  --disassemble-original   Include original disassembly in actionc-compare output.
  -h, --help              Show this help.

Environment:
  ACTION_COMPILER_VM_DIR   default: $vm_root
  ACTION_VM_CART           default: $cart_rom
  ACTION_VM_OS             default: $os_rom
  ACTION_VM_MAX_STEPS      default: $max_steps
EOF
}

require_file() {
  local path="$1"
  local label="$2"
  if [[ ! -f "$path" ]]; then
    echo "Missing $label: $path" >&2
    exit 1
  fi
}

safe_atari_stem() {
  local raw="$1"
  raw="$(basename "$raw")"
  raw="${raw%.*}"
  raw="$(printf '%s' "$raw" | tr '[:lower:]' '[:upper:]' | tr -cd 'A-Z0-9')"
  if [[ -z "$raw" ]]; then
    raw="SOURCE"
  fi
  printf '%.8s' "$raw"
}

name_override=""
source_path=""
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
    --name)
      [[ $# -ge 2 ]] || { echo "--name requires a stem" >&2; exit 2; }
      name_override="$2"
      shift 2
      ;;
    --origin)
      [[ $# -ge 2 ]] || { echo "--origin requires an address" >&2; exit 2; }
      origin_args=(--origin "$2")
      shift 2
      ;;
    --mode)
      [[ $# -ge 2 ]] || { echo "--mode requires all, legacy, modern, or profiles" >&2; exit 2; }
      case "$2" in
        all|legacy|compat|modern|profiles|profile) compare_mode="$2" ;;
        *) echo "invalid --mode value: $2" >&2; exit 2 ;;
      esac
      shift 2
      ;;
    --max-diffs)
      [[ $# -ge 2 ]] || { echo "--max-diffs requires a number" >&2; exit 2; }
      max_diffs="$2"
      shift 2
      ;;
    --disassemble-original)
      disassemble_original=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n "$source_path" ]]; then
        echo "only one ACT source may be supplied" >&2
        exit 2
      fi
      source_path="$1"
      shift
      ;;
  esac
done

if [[ -z "$source_path" && $# -gt 0 ]]; then
  source_path="$1"
  shift
fi
if [[ -z "$source_path" || $# -ne 0 ]]; then
  usage >&2
  exit 2
fi

require_file "$source_path" "ACT source"
require_file "$cart_rom" "Action! cartridge ROM"
require_file "$os_rom" "Atari OS ROM"
require_file "$vm_root/Cargo.toml" "action-compiler-vm project"

source_path="$(cd "$(dirname "$source_path")" && pwd)/$(basename "$source_path")"
stem="$(safe_atari_stem "${name_override:-$source_path}")"
h_source="$stem.ACT"
h_output="$stem.COM"

cleanup_dir=""
if [[ -z "$out_dir" ]]; then
  out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-compare-act.XXXXXX")"
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

original_com="$out_dir/$h_output"
symbols_json="$out_dir/$stem.symbols.json"
snapshots_json="$out_dir/$stem.symbol-snapshots.json"
printf -v monitor_input 'C "H:%s"\nW "H:%s"\n' "$h_source" "$h_output"

echo "==> original Action!: $source_path -> $original_com"
(
  cd "$vm_root"
  cargo run --quiet -- run \
    --cart "$cart_rom" \
    --os "$os_rom" \
    --hotpatch action-q-input \
    --hotpatch action-headless-getkey \
    --host-file "$h_source:$source_path" \
    --host-output "$h_output:$original_com" \
    --monitor-key-at-pc '$A2E0' \
    --q-input-at-pc-after '$A2E0:$B2F5:'"$monitor_input" \
    --dump-symbols-on-stop "$symbols_json" \
    --dump-symbol-snapshots-on-stop "$snapshots_json" \
    --action-symbol-hooks \
    --max-steps "$max_steps" \
    --history 20
)

if [[ ! -s "$original_com" ]]; then
  echo "FAILED: original compiler did not write $original_com" >&2
  exit 1
fi

compare_args=(
  --original "$original_com"
  --max-diffs "$max_diffs"
  --mode "$compare_mode"
)
if [[ -s "$symbols_json" ]]; then
  compare_args+=(--original-symbols "$symbols_json")
fi
if [[ -s "$snapshots_json" ]]; then
  compare_args+=(--original-symbol-snapshots "$snapshots_json")
fi
if [[ "$disassemble_original" -eq 1 ]]; then
  compare_args+=(--disassemble-original)
fi
if [[ ${#origin_args[@]} -ne 0 ]]; then
  compare_args+=("${origin_args[@]}")
fi

echo "==> actionc-compare: mode=$compare_mode"
(
  cd "$repo_root"
  cargo run --quiet --bin actionc-compare -- "${compare_args[@]}" "$source_path"
)

if [[ "$keep" -eq 1 ]]; then
  echo "==> artifacts kept in $out_dir"
fi
