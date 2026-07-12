#!/usr/bin/env bash
set -euo pipefail

survey_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$survey_dir/../.." && pwd)"
stress_dir="$repo_root/fixtures/stress"
vm_root="${ACTION_COMPILER_VM_DIR:-$repo_root/../action-compiler-vm}"

vm_out_dir="${ACTION_VM_STRESS_OUTPUT_DIR:-$survey_dir/outputs/vm}"
actionc_out_dir="${ACTION_STRESS_ACTIONC_OUTPUT_DIR:-$survey_dir/outputs/actionc}"
cart_rom="${ACTION_VM_CART:-$repo_root/roms/action.rom}"
os_rom="${ACTION_VM_OS:-$repo_root/roms/rev02.rom}"
max_steps="${ACTION_VM_MAX_STEPS:-60000000}"

stress_names="
advanced_pointers
arithmetic_control
arrays
calls
control_flow
layout_integration
pointers
real_expr_chains
records
strings
zero_page
zero_page_scalars
"

usage() {
  cat <<EOF
Usage: $0 [--list] [all|stress-name ...]

Compile stress programs with the original Action! cartridge through
action-compiler-vm, compile the same source with actionc, and report whether
the load files match byte-for-byte.

Environment:
  ACTION_COMPILER_VM_DIR          default: $vm_root
  ACTION_VM_STRESS_OUTPUT_DIR     default: $vm_out_dir
  ACTION_STRESS_ACTIONC_OUTPUT_DIR default: $actionc_out_dir
  ACTION_VM_CART                  default: $cart_rom
  ACTION_VM_OS                    default: $os_rom
  ACTION_VM_MAX_STEPS             default: $max_steps
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

list_stress() {
  printf '%s\n' $stress_names
}

stress_output_name() {
  case "$1" in
    advanced_pointers) echo "ADVPTR.COM" ;;
    arithmetic_control) echo "ARICTL.COM" ;;
    arrays) echo "ARRAYS.COM" ;;
    calls) echo "CALLS.COM" ;;
    control_flow) echo "CTRLFLOW.COM" ;;
    layout_integration) echo "LAYOUT.COM" ;;
    pointers) echo "POINTERS.COM" ;;
    real_expr_chains) echo "EXPRCH.COM" ;;
    records) echo "RECORDS.COM" ;;
    strings) echo "STRINGS.COM" ;;
    zero_page) echo "ZEROPG.COM" ;;
    zero_page_scalars) echo "ZPSCAL.COM" ;;
    *) echo "Unknown stress program: $1" >&2; exit 2 ;;
  esac
}

compare_files() {
  local expected="$1"
  local actual="$2"

  if cmp -s "$expected" "$actual"; then
    echo "    exact match"
    return 0
  fi

  echo "    differs"
  local first_diffs
  first_diffs="$((cmp -l "$expected" "$actual" 2>/dev/null || true) | sed -n '1,8p' | tr '\n' ';' | sed 's/;$//')"
  if [[ -n "$first_diffs" ]]; then
    echo "    first differing bytes: $first_diffs"
  fi
}

run_stress() {
  local name="$1"
  local source_path="$stress_dir/$name.act"
  local source_name output_name vm_output actionc_output h_source monitor_input

  require_file "$source_path" "stress source"
  source_name="$(basename "$source_path")"
  h_source="$(printf '%s' "$source_name" | tr '[:lower:]' '[:upper:]')"
  output_name="$(stress_output_name "$name")"
  vm_output="$vm_out_dir/$output_name"
  actionc_output="$actionc_out_dir/$output_name"
  printf -v monitor_input 'C "H:%s"\nW "H:%s"\n' "$h_source" "$output_name"

  mkdir -p "$vm_out_dir" "$actionc_out_dir"
  rm -f "$vm_output" "$actionc_output"

  echo "==> $name: original Action! -> $output_name"
  (
    cd "$vm_root"
    cargo run --quiet -- run \
      --cart "$cart_rom" \
      --os "$os_rom" \
      --hotpatch action-q-input \
      --hotpatch action-headless-getkey \
      --host-file "$h_source:$source_path" \
      --host-output "$output_name:$vm_output" \
      --monitor-key-at-pc '$A2E0' \
      --q-input-at-pc-after '$A2E0:$B2F5:'"$monitor_input" \
      --max-steps "$max_steps" \
      --history 20
  )

  if [[ ! -s "$vm_output" ]]; then
    echo "FAILED: $vm_output was not written or is empty" >&2
    rm -f "$vm_output"
    return 1
  fi

  echo "==> $name: actionc -> $output_name"
  (
    cd "$repo_root"
    cargo run --quiet --bin actionc -- --output "$actionc_output" "$source_path"
  )

  local vm_size actionc_size
  vm_size="$(wc -c < "$vm_output" | tr -d ' ')"
  actionc_size="$(wc -c < "$actionc_output" | tr -d ' ')"
  echo "    original: $vm_size bytes  $vm_output"
  echo "    actionc:  $actionc_size bytes  $actionc_output"
  compare_files "$vm_output" "$actionc_output"
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
    list_stress
    exit 0
    ;;
  all)
    shift
    if [[ $# -ne 0 ]]; then
      echo "'all' cannot be combined with explicit stress names" >&2
      exit 2
    fi
    selected=()
    while IFS= read -r name; do
      selected+=("$name")
    done < <(list_stress)
    ;;
  *)
    selected=("$@")
    ;;
esac

require_file "$cart_rom" "Action! cartridge ROM"
require_file "$os_rom" "Atari OS ROM"
require_file "$vm_root/Cargo.toml" "action-compiler-vm project"

status=0
for name in "${selected[@]}"; do
  if ! run_stress "$name"; then
    status=1
  fi
done

exit "$status"
