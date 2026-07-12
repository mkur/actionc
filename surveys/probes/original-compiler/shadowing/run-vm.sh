#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/../../../.." && pwd)"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"

vm_out_dir="${ACTION_VM_SHADOW_OUTPUT_DIR:-$script_dir/outputs/vm}"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/rev02.rom}"
max_steps="${ACTION_VM_MAX_STEPS:-60000000}"

probe_names="
local_var_shadow
param_shadow
global_builtin_shadow
local_builtin_shadow
duplicate_same_scope
"

usage() {
  cat <<EOF
Usage: $0 [--list] [all|probe-name ...]

Compile shadowing probes with the original Action! cartridge through
action-compiler-vm.

Environment:
  ACTION_COMPILER_VM_DIR      default: $vm_root
  ACTION_VM_SHADOW_OUTPUT_DIR default: $vm_out_dir
  ACTION_VM_CART              default: $cart_rom
  ACTION_VM_OS                default: $os_rom
  ACTION_VM_MAX_STEPS         default: $max_steps
EOF
}

list_probes() {
  printf '%s\n' $probe_names
}

probe_source() {
  case "$1" in
    local_var_shadow) echo "local_var_shadow.act" ;;
    param_shadow) echo "param_shadow.act" ;;
    global_builtin_shadow) echo "global_builtin_shadow.act" ;;
    local_builtin_shadow) echo "local_builtin_shadow.act" ;;
    duplicate_same_scope) echo "duplicate_same_scope.act" ;;
    *) return 1 ;;
  esac
}

probe_host_source() {
  case "$1" in
    local_var_shadow) echo "SHADLOC.ACT" ;;
    param_shadow) echo "SHADPAR.ACT" ;;
    global_builtin_shadow) echo "SHADGBI.ACT" ;;
    local_builtin_shadow) echo "SHADLBI.ACT" ;;
    duplicate_same_scope) echo "SHADDUP.ACT" ;;
    *) return 1 ;;
  esac
}

probe_output() {
  case "$1" in
    local_var_shadow) echo "SHADLOC.COM" ;;
    param_shadow) echo "SHADPAR.COM" ;;
    global_builtin_shadow) echo "SHADGBI.COM" ;;
    local_builtin_shadow) echo "SHADLBI.COM" ;;
    duplicate_same_scope) echo "SHADDUP.COM" ;;
    *) return 1 ;;
  esac
}

probe_expected_compile_failure() {
  case "$1" in
    duplicate_same_scope) return 0 ;;
    *) return 1 ;;
  esac
}

require_file() {
  local path="$1"
  local label="$2"
  if [[ ! -f "$path" ]]; then
    echo "missing $label: $path" >&2
    exit 1
  fi
}

run_probe() {
  local probe="$1"
  local source_name host_source output_name source_path output_path monitor_input

  if ! source_name="$(probe_source "$probe")" ||
     ! host_source="$(probe_host_source "$probe")" ||
     ! output_name="$(probe_output "$probe")"; then
    echo "unknown probe: $probe" >&2
    echo "known probes:" >&2
    list_probes >&2
    exit 2
  fi

  source_path="$script_dir/$source_name"
  output_path="$vm_out_dir/$output_name"
  printf -v monitor_input 'C "H:%s"\nW "H:%s"\n' "$host_source" "$output_name"

  require_file "$source_path" "probe source"
  mkdir -p "$vm_out_dir"
  rm -f "$output_path"

  echo "==> $probe: $source_name -> $output_name"
  (
    cd "$vm_root"
    cargo run --quiet -- run \
      --cart "$cart_rom" \
      --os "$os_rom" \
      --hotpatch action-q-input \
      --hotpatch action-headless-getkey \
      --host-file "$host_source:$source_path" \
      --host-output "$output_name:$output_path" \
      --monitor-key-at-pc '$A2E0' \
      --q-input-at-pc-after '$A2E0:$B2F5:'"$monitor_input" \
      --max-steps "$max_steps" \
      --history 20
  )

  if [[ ! -s "$output_path" ]]; then
    if probe_expected_compile_failure "$probe"; then
      rm -f "$output_path"
      echo "    expected original compiler failure; no output file written"
      return 0
    fi
    echo "FAILED: $output_path was not written or is empty" >&2
    return 1
  fi

  if probe_expected_compile_failure "$probe"; then
    echo "FAILED: expected compile failure, but wrote $output_path" >&2
    return 1
  fi

  local size
  size="$(wc -c < "$output_path" | tr -d ' ')"
  echo "    wrote $size bytes: $output_path"
}

if [[ $# -eq 0 ]]; then
  usage >&2
  exit 2
fi

case "$1" in
  --help|-h)
    usage
    exit 0
    ;;
  --list)
    list_probes
    exit 0
    ;;
  all)
    shift
    if [[ $# -ne 0 ]]; then
      echo "'all' cannot be combined with explicit probe names" >&2
      exit 2
    fi
    selected=()
    while IFS= read -r name; do
      selected+=("$name")
    done < <(list_probes)
    ;;
  *)
    selected=("$@")
    ;;
esac

require_file "$vm_root/Cargo.toml" "action-compiler-vm project"
require_file "$cart_rom" "Action! cartridge ROM"
require_file "$os_rom" "Atari OS ROM"

status=0
for probe in "${selected[@]}"; do
  if ! run_probe "$probe"; then
    status=1
  fi
done

exit "$status"
