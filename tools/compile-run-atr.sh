#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

profile="legacy"
backend="${ACTIONC_BACKEND:-classic}"
codegen_source="${ACTIONC_CODEGEN_SOURCE:-ast}"
origin_args=()
out_dir=""
out_atr=""
object_path=""
pack_object_path=""
name_override=""
run=1
run_mode="disk"
atari800_bin="${ATARI800:-atari800}"
cart_rom="${ACTIONC_ATARI800_CART:-${ACTION_VM_CART:-$repo_root/roms/action.rom}}"
os_rom="${ACTIONC_ATARI800_OS:-${ACTION_VM_OS:-$repo_root/roms/rev02.rom}}"
default_source_atr="$repo_root/atr/mydos.atr"

usage() {
  cat <<EOF
Usage: tools/compile-run-atr.sh [options] <source.act> [source.atr]
       tools/compile-run-atr.sh --pack-object <file.com> [options] [source.atr]

Compile an Action! source file with actionc, copy the generated load-format
object into a new ATR derived from <source.atr>, then launch atari800.
With --pack-object, skip compilation and copy the existing load-format object
into the ATR instead.
If [source.atr] is omitted, use atr/mydos.atr.

Options:
  --profile <legacy|modern>  actionc profile, default: $profile
  --backend <classic|mir6502> actionc backend, default: $backend
  --origin <addr>            pass an explicit origin to actionc
  --name <stem>              Atari output filename stem, default: source basename
  --out-dir <dir>            directory for generated artifacts
  --out-atr <file.atr>       explicit output ATR path
  --object <file.com>        explicit generated object path
  --pack-object <file.com>   skip compilation and pack this existing object
  --run-mode <disk|host>     disk: boot/attach ATR; host: atari800 -run object
  --atari800 <path>          atari800 executable, default: \$ATARI800 or atari800
  --cart <rom>               attach an Atari cartridge ROM when launching
  --no-cart                  ignore ACTIONC_ATARI800_CART/ACTION_VM_CART
  --os <rom>                 Atari XL/XE OS ROM for atari800, default: roms/rev02.rom
  --no-os                    use atari800 config/default OS ROM
  --no-run                   build the ATR but do not start atari800
  --keep                     keep temporary artifacts and print their path
  -h, --help                 show this help

Advanced compiler-development options:
  --codegen-source <source>   classic-backend codegen source: ast, semir,
                              semir-native; default: $codegen_source
  --codegen <source>          alias for --codegen-source

Environment:
  ACTIONC_BACKEND             actionc backend override: classic or mir6502
  ACTIONC_CODEGEN_SOURCE      codegen source override for development runs
  ACTIONC_ATARI800_CART       cartridge ROM passed to atari800 as -cart
  ACTION_VM_CART              fallback cartridge ROM, shared with VM tools
                              default: $repo_root/roms/action.rom
  ACTIONC_ATARI800_OS         XL/XE OS ROM passed to atari800 as -xlxe_rom
  ACTION_VM_OS                fallback OS ROM, shared with VM tools
                              default: $repo_root/roms/rev02.rom
  ATARI800                   atari800 executable override
  ATARI800_ARGS              extra words appended to atari800 invocation

Examples:
  tools/compile-run-atr.sh samples/foo.act
  tools/compile-run-atr.sh samples/foo.act samples/dos.atr
  tools/compile-run-atr.sh --profile modern --backend mir6502 --no-run foo.act dos.atr
  tools/compile-run-atr.sh --profile legacy --name FOO --no-run foo.act dos.atr
  tools/compile-run-atr.sh --pack-object FOO.COM --no-run
  tools/compile-run-atr.sh --pack-object FOO.COM --no-run dos.atr
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
    raw="PROGRAM"
  fi
  printf '%.8s' "$raw"
}

normalize_codegen_source() {
  local value="$1"
  case "$value" in
    ast)
      printf '%s\n' "ast"
      ;;
    semir|sem-ir)
      printf '%s\n' "semir"
      ;;
    native|semir-native|sem-ir-native|native-ir|modern-ir)
      printf '%s\n' "semir-native"
      ;;
    *)
      echo "invalid --codegen-source value: $value" >&2
      exit 2
      ;;
  esac
}

normalize_backend() {
  local value="$1"
  case "$value" in
    classic|legacy|default)
      printf '%s\n' "classic"
      ;;
    mir6502|mir|6502)
      printf '%s\n' "mir6502"
      ;;
    *)
      echo "invalid --backend value: $value" >&2
      exit 2
      ;;
  esac
}

normalize_profile() {
  local value="$1"
  case "$value" in
    legacy|compat)
      printf '%s\n' "legacy"
      ;;
    modern)
      printf '%s\n' "modern"
      ;;
    *)
      echo "invalid --profile value: $value" >&2
      exit 2
      ;;
  esac
}

abs_path() {
  local path="$1"
  if [[ -d "$path" ]]; then
    (cd "$path" && pwd)
  else
    (cd "$(dirname "$path")" && printf '%s/%s\n' "$(pwd)" "$(basename "$path")")
  fi
}

keep=0
source_path=""
source_atr=""
backend="$(normalize_backend "$backend")"
codegen_source="$(normalize_codegen_source "$codegen_source")"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      [[ $# -ge 2 ]] || { echo "--profile requires legacy or modern" >&2; exit 2; }
      profile="$(normalize_profile "$2")"
      shift 2
      ;;
    --profile=*)
      profile="$(normalize_profile "${1#*=}")"
      shift
      ;;
    --backend)
      [[ $# -ge 2 ]] || { echo "--backend requires classic or mir6502" >&2; exit 2; }
      backend="$(normalize_backend "$2")"
      shift 2
      ;;
    --backend=*)
      backend="$(normalize_backend "${1#*=}")"
      shift
      ;;
    --codegen-source|--codegen)
      [[ $# -ge 2 ]] || { echo "$1 requires ast, semir, or semir-native" >&2; exit 2; }
      codegen_source="$(normalize_codegen_source "$2")"
      shift 2
      ;;
    --codegen-source=*|--codegen=*)
      codegen_source="$(normalize_codegen_source "${1#*=}")"
      shift
      ;;
    --origin)
      [[ $# -ge 2 ]] || { echo "--origin requires an address" >&2; exit 2; }
      origin_args=(--origin "$2")
      shift 2
      ;;
    --name)
      [[ $# -ge 2 ]] || { echo "--name requires a stem" >&2; exit 2; }
      name_override="$2"
      shift 2
      ;;
    --out-dir)
      [[ $# -ge 2 ]] || { echo "--out-dir requires a directory" >&2; exit 2; }
      out_dir="$2"
      keep=1
      shift 2
      ;;
    --out-atr)
      [[ $# -ge 2 ]] || { echo "--out-atr requires a path" >&2; exit 2; }
      out_atr="$2"
      keep=1
      shift 2
      ;;
    --object)
      [[ $# -ge 2 ]] || { echo "--object requires a path" >&2; exit 2; }
      object_path="$2"
      keep=1
      shift 2
      ;;
    --pack-object|--input-object)
      [[ $# -ge 2 ]] || { echo "$1 requires a path" >&2; exit 2; }
      pack_object_path="$2"
      shift 2
      ;;
    --run-mode)
      [[ $# -ge 2 ]] || { echo "--run-mode requires disk or host" >&2; exit 2; }
      case "$2" in
        disk|host) run_mode="$2" ;;
        *) echo "invalid --run-mode value: $2" >&2; exit 2 ;;
      esac
      shift 2
      ;;
    --atari800)
      [[ $# -ge 2 ]] || { echo "--atari800 requires a path" >&2; exit 2; }
      atari800_bin="$2"
      shift 2
      ;;
    --cart)
      [[ $# -ge 2 ]] || { echo "--cart requires a ROM path" >&2; exit 2; }
      cart_rom="$2"
      shift 2
      ;;
    --no-cart)
      cart_rom=""
      shift
      ;;
    --os)
      [[ $# -ge 2 ]] || { echo "--os requires a ROM path" >&2; exit 2; }
      os_rom="$2"
      shift 2
      ;;
    --no-os)
      os_rom=""
      shift
      ;;
    --no-run)
      run=0
      shift
      ;;
    --keep)
      keep=1
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
      if [[ -z "$source_path" ]]; then
        source_path="$1"
      elif [[ -z "$source_atr" ]]; then
        source_atr="$1"
      else
        echo "unexpected argument: $1" >&2
        usage >&2
        exit 2
      fi
      shift
      ;;
  esac
done

if [[ -z "$source_path" && $# -gt 0 ]]; then
  source_path="$1"
  shift
fi
if [[ -z "$source_atr" && $# -gt 0 ]]; then
  source_atr="$1"
  shift
fi
if [[ -n "$pack_object_path" && -z "$source_atr" && -n "$source_path" ]]; then
  source_atr="$source_path"
  source_path=""
fi
if [[ -z "$source_atr" ]]; then
  source_atr="$default_source_atr"
fi
if [[ -n "$pack_object_path" && -n "$object_path" ]]; then
  echo "--object is for generated compile output and cannot be combined with --pack-object" >&2
  exit 2
fi
if [[ "$backend" == "mir6502" && "$profile" != "modern" ]]; then
  echo "--backend mir6502 requires --profile modern" >&2
  exit 2
fi
if [[ -n "$pack_object_path" ]]; then
  if [[ $# -ne 0 ]]; then
    usage >&2
    exit 2
  fi
else
  if [[ -z "$source_path" || $# -ne 0 ]]; then
    usage >&2
    exit 2
  fi
fi

if [[ -n "$pack_object_path" ]]; then
  require_file "$pack_object_path" "object file"
else
  require_file "$source_path" "ACT source"
fi
require_file "$source_atr" "source ATR"
if [[ -z "$pack_object_path" ]]; then
  require_file "$repo_root/Cargo.toml" "actionc project"
fi
require_file "$repo_root/crates/atrcopy-rs/Cargo.toml" "atrcopy-rs project"

if [[ -n "$pack_object_path" ]]; then
  pack_object_path="$(abs_path "$pack_object_path")"
else
  source_path="$(abs_path "$source_path")"
fi
source_atr="$(abs_path "$source_atr")"

stem_input="${name_override:-${pack_object_path:-$source_path}}"
stem="$(safe_atari_stem "$stem_input")"
atari_object="$stem.COM"

cleanup_dir=""
if [[ -z "$out_dir" ]]; then
  out_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-run-atr.XXXXXX")"
  cleanup_dir="$out_dir"
else
  mkdir -p "$out_dir"
  out_dir="$(abs_path "$out_dir")"
fi

cleanup() {
  if [[ -n "$cleanup_dir" && "$keep" -eq 0 ]]; then
    rm -rf "$cleanup_dir"
  fi
}
trap cleanup EXIT

if [[ -n "$pack_object_path" ]]; then
  object_path="$pack_object_path"
elif [[ -z "$object_path" ]]; then
  object_path="$out_dir/$atari_object"
else
  mkdir -p "$(dirname "$object_path")"
  object_path="$(abs_path "$object_path")"
fi

if [[ -z "$out_atr" ]]; then
  out_atr="$out_dir/${stem}.atr"
else
  mkdir -p "$(dirname "$out_atr")"
  out_atr="$(abs_path "$out_atr")"
fi

if [[ "$out_atr" == "$source_atr" ]]; then
  echo "output ATR must be different from source ATR" >&2
  exit 2
fi

if [[ -n "$pack_object_path" ]]; then
  echo "==> pack-object: $object_path -> $atari_object"
else
  echo "==> actionc: $source_path -> $object_path (profile=$profile, backend=$backend, codegen=$codegen_source)"
  (
    cd "$repo_root"
    actionc_args=(--profile "$profile" --backend "$backend" --codegen-source "$codegen_source")
    if [[ ${#origin_args[@]} -ne 0 ]]; then
      actionc_args+=("${origin_args[@]}")
    fi
    actionc_args+=(--output "$object_path" "$source_path")
    cargo run --quiet --bin actionc -- "${actionc_args[@]}"
  )
fi

if [[ ! -s "$object_path" ]]; then
  if [[ -n "$pack_object_path" ]]; then
    echo "FAILED: object file is empty: $object_path" >&2
  else
    echo "FAILED: actionc did not write an object file: $object_path" >&2
  fi
  exit 1
fi

echo "==> atrcopy-rs: $source_atr + $atari_object -> $out_atr"
(
  cd "$repo_root"
  cargo run --quiet --manifest-path crates/atrcopy-rs/Cargo.toml --bin atrcopy-rs -- \
    "$source_atr" add -o "$out_atr" "$object_path=$atari_object"
)

if [[ ! -s "$out_atr" ]]; then
  echo "FAILED: atrcopy-rs did not write an ATR: $out_atr" >&2
  exit 1
fi

echo "==> object: $object_path"
echo "==> atr:    $out_atr"
echo "==> Atari filename: D:$atari_object"

if [[ "$run" -eq 0 ]]; then
  if [[ "$keep" -eq 1 ]]; then
    echo "==> artifacts kept in $out_dir"
  fi
  exit 0
fi

if ! command -v "$atari800_bin" >/dev/null 2>&1 && [[ ! -x "$atari800_bin" ]]; then
  echo "Missing atari800 executable: $atari800_bin" >&2
  exit 1
fi

if [[ -n "$cart_rom" ]]; then
  require_file "$cart_rom" "cartridge ROM"
  cart_rom="$(abs_path "$cart_rom")"
fi
if [[ -n "$os_rom" ]]; then
  require_file "$os_rom" "Atari OS ROM"
  os_rom="$(abs_path "$os_rom")"
fi

atari800_args=()
if [[ -n "$os_rom" ]]; then
  atari800_args+=(-xlxe_rom "$os_rom")
fi
if [[ -n "$cart_rom" ]]; then
  atari800_args+=(-cart "$cart_rom")
fi
if [[ -n "${ATARI800_ARGS:-}" ]]; then
  extra_atari800_args=()
  read -r -a extra_atari800_args <<< "$ATARI800_ARGS"
  atari800_args+=("${extra_atari800_args[@]}")
fi
case "$run_mode" in
  disk)
    echo "==> atari800: boot/attach ATR"
    if [[ ${#atari800_args[@]} -eq 0 ]]; then
      "$atari800_bin" "$out_atr"
    else
      "$atari800_bin" "${atari800_args[@]}" "$out_atr"
    fi
    ;;
  host)
    echo "==> atari800: run host object with ATR attached"
    if [[ ${#atari800_args[@]} -eq 0 ]]; then
      "$atari800_bin" -run "$object_path" "$out_atr"
    else
      "$atari800_bin" "${atari800_args[@]}" -run "$object_path" "$out_atr"
    fi
    ;;
esac

if [[ "$keep" -eq 1 ]]; then
  echo "==> artifacts kept in $out_dir"
fi
