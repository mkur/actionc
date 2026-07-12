#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: surveys/toolkit/pack-toolkit-atrs.sh [options] [preset ...]

Pack all successfully compiled Toolkit .COM objects and their required runtime
assets into one ATR per setting. By default the script rebuilds the Toolkit
batch outputs for the selected presets before packing them.

Presets:
  legacy-classic
  modern-classic
  modern-mir6502
  all              pack all three presets, default

Options:
  --no-build             use existing surveys/toolkit/outputs/batch objects
  --source-atr <file>    base ATR, default: atr/mydos.atr
  --batch-dir <dir>      batch output root, default: surveys/toolkit/outputs/batch
  --output-dir <dir>     ATR output directory, default: surveys/toolkit/outputs/atr
  -h, --help             show this help

USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
source_atr="$repo_root/atr/mydos.atr"
batch_root="$script_dir/outputs/batch"
output_dir="$script_dir/outputs/atr"
runtime_asset_dir="$repo_root/corpora/toolkit/original/extracted"
build=1
presets=()

display_path() {
  local path="$1"
  case "$path" in
    "$repo_root"/*) printf '%s' "${path#"$repo_root"/}" ;;
    *) printf '%s' "$path" ;;
  esac
}

normalize_preset() {
  case "$1" in
    legacy-classic|modern-classic|modern-mir6502) printf '%s\n' "$1" ;;
    compat-legacy) printf '%s\n' legacy-classic ;;
    modern-legacy) printf '%s\n' modern-classic ;;
    all) printf '%s\n' all ;;
    *) echo "invalid preset: $1" >&2; exit 2 ;;
  esac
}

add_preset() {
  local preset
  preset="$(normalize_preset "$1")"
  if [[ "$preset" == "all" ]]; then
    presets=(legacy-classic modern-classic modern-mir6502)
  else
    presets+=("$preset")
  fi
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --no-build)
      build=0
      shift
      ;;
    --source-atr)
      [[ $# -ge 2 ]] || { echo "--source-atr requires a path" >&2; exit 2; }
      source_atr="$2"
      shift 2
      ;;
    --source-atr=*)
      source_atr="${1#*=}"
      shift
      ;;
    --batch-dir)
      [[ $# -ge 2 ]] || { echo "--batch-dir requires a path" >&2; exit 2; }
      batch_root="$2"
      shift 2
      ;;
    --batch-dir=*)
      batch_root="${1#*=}"
      shift
      ;;
    --output-dir|--out-dir)
      [[ $# -ge 2 ]] || { echo "$1 requires a directory" >&2; exit 2; }
      output_dir="$2"
      shift 2
      ;;
    --output-dir=*|--out-dir=*)
      output_dir="${1#*=}"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      while [[ "$#" -gt 0 ]]; do
        add_preset "$1"
        shift
      done
      ;;
    -*)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      add_preset "$1"
      shift
      ;;
  esac
done

if [[ "${#presets[@]}" -eq 0 ]]; then
  presets=(legacy-classic modern-classic modern-mir6502)
fi

if [[ ! -f "$source_atr" ]]; then
  echo "missing source ATR: $source_atr" >&2
  exit 1
fi

mkdir -p "$output_dir"

cd "$repo_root"
cargo build --quiet --manifest-path crates/atrcopy-rs/Cargo.toml --bin atrcopy-rs

for preset in "${presets[@]}"; do
  if [[ "$build" -eq 1 ]]; then
    if ! "$script_dir/compile-toolkit-batch.sh" --preset "$preset" --output-dir "$batch_root"; then
      echo "warning: $preset batch had failures; packing successful objects" >&2
    fi
  fi

  preset_dir="$batch_root/$preset"
  if [[ ! -d "$preset_dir" ]]; then
    echo "missing batch output directory: $preset_dir" >&2
    exit 1
  fi

  objects=()
  while IFS= read -r object; do
    objects+=("$object=$(basename "$object")")
  done < <(find "$preset_dir" -maxdepth 1 -type f -name '*.COM' | sort)

  if [[ "${#objects[@]}" -eq 0 ]]; then
    echo "no .COM objects found in $preset_dir" >&2
    exit 1
  fi

  runtime_assets=()
  if [[ -f "$preset_dir/MUSICDEM.COM" ]]; then
    music_screen="$runtime_asset_dir/MUSIC.SCR"
    if [[ ! -f "$music_screen" ]]; then
      echo "missing MUSICDEM runtime asset: $music_screen" >&2
      exit 1
    fi
    music_screen_size="$(wc -c < "$music_screen" | tr -d ' ')"
    if [[ "$music_screen_size" -ne 3600 ]]; then
      echo "invalid MUSICDEM runtime asset size: $music_screen_size (expected 3600)" >&2
      exit 1
    fi
    runtime_assets+=("$music_screen=MUSIC.SCR")
  fi

  additions=("${objects[@]}" "${runtime_assets[@]}")

  out_atr="$output_dir/$preset.atr"
  echo "==> packing ${#objects[@]} objects and ${#runtime_assets[@]} runtime asset(s) for $preset -> $(display_path "$out_atr")"
  cargo run --quiet --manifest-path crates/atrcopy-rs/Cargo.toml --bin atrcopy-rs -- \
    "$source_atr" add -o "$out_atr" "${additions[@]}"
  cargo run --quiet --manifest-path crates/atrcopy-rs/Cargo.toml --bin atrcopy-rs -- \
    "$out_atr" list
done
