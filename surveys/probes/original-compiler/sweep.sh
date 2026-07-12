#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: surveys/probes/original-compiler/sweep.sh [--update-artifacts]

Regenerate actionc Atari load files for VM-captured probes and compare them
against the original Action! compiler captures.

Options:
  --update-artifacts  Also refresh actionc .hex and .lst files.
  -h, --help          Show this help.
USAGE
}

update_artifacts=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --update-artifacts)
      update_artifacts=1
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
  shift
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../.." && pwd)"
actionc_out="$script_dir/outputs/actionc"
vm_out="$script_dir/outputs/vm"

mkdir -p "$actionc_out"

probes=(
  "ABICALLS:abi_calls:exact"
  "ABISYS:abi_system_call:exact"
  "ARGTHR:argthr:exact"
  "ARITH:arith:exact"
  "ARRASN:array_assign:exact"
  "ARRFIX:array_fixed_origin:exact"
  "ARRTHB:array_inline_boundary:accepted-divergence"
  "ARRTHG:array_inline_global_threshold:accepted-divergence"
  "ARRTHL:array_inline_local_threshold:exact"
  "ARRAYS:arrays:exact"
  "ARRPAR:array_params:exact"
  "ARRREF:array_refs:exact"
  "BOOLEDGE:bool_edges:exact"
  "BOOLEQ:booleq:exact"
  "BOOLS:bools:exact"
  "BOOLTHEN:boolthen:exact"
  "BOOLWORD:boolword:exact"
  "CONTROL:control_flow:exact"
  "DRAWFRM:drawframe:exact"
  "EMPTYPR:empty_proc:exact"
  "EXTCALL:external_call:exact"
  "FNAMECMP:fnamecmp:accepted-divergence"
  "FUNC:functions:exact"
  "GRCALL:graphics_calls:exact"
  "IDXSCALE:index_scaling:exact"
  "LGLARR:large_local_arrays:accepted-divergence"
  "LAYOUT:layout_order:exact"
  "LOCALS:locals:exact"
  "LOCARR:locarr:exact"
  "NESTED:nested_calls:exact"
  "OPTARGS:optional_args:exact"
  "POINTERS:pointers:exact"
  "PREZP:precode_zp:exact"
  "RECARGS:record_args:accepted-divergence"
  "RECORDS:records:exact"
  "RETFLOW:retflow:exact"
  "RETURNS:returns:exact"
  "SARGS:sargs:exact"
  "SIGNEDGE:signedge:exact"
  "STRIDX:stridx:exact"
  "STRINIT:strinit:exact"
  "STRLIT:strlit:exact"
  "STRLOC:strloc:exact"
  "STRMUT:strmut:exact"
  "STRNAM:strnam:exact"
  "STRPASS:strpass:exact"
  "TNVAL:tn_value_index:accepted-divergence"
  "YWALK:ywalk:exact"
)

accepted_reasons_record_args="original compiler emits broken direct TYPE-field call setup (JSR \$0000); actionc keeps sane semantics"
accepted_reasons_fnamecmp="pointer-deref EOR/CMP indirect-indexed semantics now match; actionc remains a few bytes smaller from known-register and branch/layout choices"
accepted_reasons_large_local_arrays="large backing storage and array-to-array pointer code now match; remaining two bytes are apparent original uninitialized small local array residue"
accepted_reasons_array_inline_boundary="threshold layout matches; remaining bytes are original inline local byte-array metadata residue"
accepted_reasons_array_inline_global_threshold="threshold and marker layout match; original chooses different unsaved backing pointer addresses for descriptor-backed global byte arrays"
accepted_reasons_tn_value_index="new TN drift probe; direct function-return compare, Value(v(i)+1), indexed FOR-bound caching, and indexed byte equality now match closer original shapes; actionc is 1 byte smaller due retained Y=0 reuse"

exact_count=0
accepted_count=0
unexpected_count=0

cd "$repo_root"

for entry in "${probes[@]}"; do
  IFS=: read -r vm_name probe expected <<<"$entry"
  src="$script_dir/$probe.act"
  vm_file="$vm_out/$vm_name.COM"
  com_file="$actionc_out/$probe.com"
  hex_file="$actionc_out/$probe.hex"
  lst_file="$actionc_out/$probe.lst"

  if [ ! -f "$src" ]; then
    echo "MISSING source: $probe ($src)" >&2
    unexpected_count=$((unexpected_count + 1))
    continue
  fi
  if [ ! -f "$vm_file" ]; then
    echo "MISSING VM capture: $probe ($vm_file)" >&2
    unexpected_count=$((unexpected_count + 1))
    continue
  fi

  cargo run --quiet --bin actionc-emit -- --emit-load "$src" > "$com_file"
  if [ "$update_artifacts" -eq 1 ]; then
    cargo run --quiet --bin actionc-emit -- --emit-code "$src" > "$hex_file"
    cargo run --quiet --bin actionc-emit -- --emit-listing "$src" > "$lst_file"
  fi

  if cmp -s "$vm_file" "$com_file"; then
    if [ "$expected" = "accepted-divergence" ]; then
      echo "NOTE  $probe is now exact; review accepted-divergence policy"
    else
      echo "OK    $probe"
    fi
    exact_count=$((exact_count + 1))
    continue
  fi

  if [ "$expected" = "accepted-divergence" ]; then
    reason_var="accepted_reasons_$probe"
    reason="${!reason_var:-accepted divergence}"
    echo "ALLOW $probe ($reason)"
    accepted_count=$((accepted_count + 1))
    continue
  fi

  echo "FAIL  $probe"
  echo "      vm:      $vm_file ($(wc -c < "$vm_file" | tr -d ' ') bytes)"
  echo "      actionc: $com_file ($(wc -c < "$com_file" | tr -d ' ') bytes)"
  echo "      first byte diffs:"
  cmp -l "$vm_file" "$com_file" | sed -n '1,12p'
  unexpected_count=$((unexpected_count + 1))
done

echo
echo "Probe sweep summary: exact=$exact_count accepted=$accepted_count unexpected=$unexpected_count"

if [ "$unexpected_count" -ne 0 ]; then
  exit 1
fi
