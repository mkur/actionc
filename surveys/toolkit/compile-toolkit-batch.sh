#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: surveys/toolkit/compile-toolkit-batch.sh [options] [entry ...]

Compile the Toolkit batch with actionc. The batch treats demo/program files as
the preferred top-level sources: when a Toolkit .ACT library has demo files,
the demos are compiled instead of compiling the library directly. ALLOCATE.ACT
is compiled through a generated EndProg harness.

Presets:
  legacy-classic   --profile legacy  --backend classic  --source-set original
  modern-classic   --profile modern  --backend classic  --source-set modern
  modern-mir6502   --profile modern  --backend mir6502 --source-set modern
  all             run all three presets above

Options:
  --preset <name>         preset to run, default: legacy-classic
  --profile <name>        actionc profile for a custom run
  --backend <name>        actionc backend for a custom run
  --codegen-source <name> actionc codegen source, default: ast
  --source-set <name>     original or modern; modern overlays samples/toolkit/modern
  --origin <addr>         pass an explicit origin to actionc
  --output-dir <dir>      output root, default: surveys/toolkit/outputs/batch
  --list                  list batch entries and exit
  -h, --help              show this help

Entry filters may be stems such as CIRCLE1, filenames such as CIRCLE.DM1, or
object names such as CIRCLE1.COM.

Some entries are marked as modernized and always use samples/toolkit/modern
when that source exists, even in the legacy-classic preset.

The legacy-classic preset verifies documented loose-pointer rejections against
the extracted originals, then compiles their maintained replacements with the
legacy profile and classic backend so the batch and ATR remain complete.
USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
original_dir="$repo_root/corpora/toolkit/original/extracted"
modern_dir="$repo_root/samples/toolkit/modern"
output_root="$script_dir/outputs/batch"

preset="legacy-classic"
profile=""
backend=""
codegen_source="ast"
source_set=""
origin_value=""
list_only=0
entry_filters=()
custom_config=0

entries=(
  "ABS:ABS.ACT:ABS.COM:library"
  "ALLOCATE:ALLOCATE.ACT:ALLOCATE.COM:harness"
  "CHARTEST:CHARTEST.ACT:CHARTEST.COM:library"
  "CONSOLE:CONSOLE.ACT:CONSOLE.COM:library"
  "IO:IO.ACT:IO.COM:library"
  "JOYSTIX:JOYSTIX.ACT:JOYSTIX.COM:library"
  "CIRCLE1:CIRCLE.DM1:CIRCLE1.COM:demo"
  "CIRCLE2:CIRCLE.DM2:CIRCLE2.COM:demo"
  "GEMDEM:GEM.DEM:GEMDEM.COM:demo"
  "KALSCOPE:KALSCOPE.DEM:KALSCOPE.COM:demo"
  "MUSICDEM:MUSIC.DEM:MUSICDEM.COM:demo:modernized"
  "PMGDM1:PMG.DM1:PMGDM1.COM:demo"
  "PMGDM2:PMG.DM2:PMGDM2.COM:demo"
  "PRINTF1:PRINTF.DM1:PRINTF1.COM:demo"
  "REALDM1:REAL.DM1:REALDM1.COM:demo"
  "SNAILS:SNAILS.DEM:SNAILS.COM:demo"
  "SORTDM1:SORT.DM1:SORTDM1.COM:demo"
  "SORTDM2:SORT.DM2:SORTDM2.COM:demo"
  "TURTLE1:TURTLE.DM1:TURTLE1.COM:demo"
  "WARPDEM:WARP.DEM:WARPDEM.COM:demo"
)

skipped_by_demo=(
  "CIRCLE.ACT -> CIRCLE.DM1, CIRCLE.DM2"
  "PMG.ACT -> PMG.DM1, PMG.DM2"
  "PRINTF.ACT -> PRINTF.DM1"
  "REAL.ACT -> REAL.DM1"
  "SORT.ACT -> SORT.DM1, SORT.DM2"
  "TURTLE.ACT -> TURTLE.DM1"
)

display_path() {
  local path="$1"
  if [[ -n "${tmp_dir:-}" && "$path" == "$tmp_dir"/* ]]; then
    printf '[tmp]/%s' "${path#"$tmp_dir"/}"
    return
  fi
  case "$path" in
    "$repo_root"/*) printf '%s' "${path#"$repo_root"/}" ;;
    *) printf '%s' "$path" ;;
  esac
}

sanitize_text_paths() {
  local text="$1"
  text="${text//"$repo_root"\//}"
  if [[ -n "${tmp_dir:-}" ]]; then
    text="${text//"$tmp_dir"/[tmp]}"
  fi
  printf '%s' "$text"
}

sanitize_file_paths() {
  local file="$1"
  local sanitized="$tmp_dir/sanitized.$(basename "$file")"
  while IFS= read -r line || [[ -n "$line" ]]; do
    sanitize_text_paths "$line"
    printf '\n'
  done < "$file" > "$sanitized"
  mv "$sanitized" "$file"
}

escape_md() {
  local text="$1"
  text="${text//\\/\\\\}"
  text="${text//|/\\|}"
  printf '%s' "$text"
}

first_error() {
  local file="$1"
  local line
  line="$(sed -n '/[^[:space:]]/ { s/[[:space:]]\+/ /g; p; q; }' "$file")"
  if [[ -z "$line" ]]; then
    line="command failed without diagnostics"
  fi
  sanitize_text_paths "$line"
}

normalize_backend() {
  case "$1" in
    classic|legacy|default) printf '%s\n' classic ;;
    mir6502|mir|6502) printf '%s\n' mir6502 ;;
    *) echo "invalid backend: $1" >&2; exit 2 ;;
  esac
}

normalize_profile() {
  case "$1" in
    legacy|compat) printf '%s\n' legacy ;;
    modern) printf '%s\n' modern ;;
    *) echo "invalid profile: $1" >&2; exit 2 ;;
  esac
}

normalize_source_set() {
  case "$1" in
    original|modern) printf '%s\n' "$1" ;;
    *) echo "invalid source set: $1" >&2; exit 2 ;;
  esac
}

apply_preset() {
  local name="$1"
  case "$name" in
    legacy-classic|compat-legacy)
      profile="legacy"
      backend="classic"
      source_set="original"
      ;;
    modern-classic|modern-legacy)
      profile="modern"
      backend="classic"
      source_set="modern"
      ;;
    modern-mir6502)
      profile="modern"
      backend="mir6502"
      source_set="modern"
      ;;
    *)
      echo "invalid preset: $name" >&2
      exit 2
      ;;
  esac
}

preset_slug() {
  printf '%s-%s-%s' "$source_set" "$profile" "$backend"
}

effective_codegen_source_label() {
  if [[ "$backend" == "mir6502" ]]; then
    printf '%s\n' "n/a (mir6502 uses optimized NIR)"
  else
    printf '%s\n' "$codegen_source"
  fi
}

entry_matches_filter() {
  local filter="$1"
  local stem="$2"
  local source="$3"
  local object="$4"
  [[ "$filter" == "$stem" || "$filter" == "$source" || "$filter" == "$object" ]]
}

entry_selected() {
  local stem="$1"
  local source="$2"
  local object="$3"
  local filter
  if [[ "${#entry_filters[@]}" -eq 0 ]]; then
    return 0
  fi
  for filter in "${entry_filters[@]}"; do
    if entry_matches_filter "$filter" "$stem" "$source" "$object"; then
      return 0
    fi
  done
  return 1
}

select_source() {
  local source="$1"
  local source_policy="${2:-auto}"
  if [[ "$source_policy" == "modernized" && -f "$modern_dir/$source" ]]; then
    printf '%s\n' "$modern_dir/$source"
  elif [[ "$source_set" == "modern" && -f "$modern_dir/$source" ]]; then
    printf '%s\n' "$modern_dir/$source"
  else
    printf '%s\n' "$original_dir/$source"
  fi
}

expected_legacy_rejection_reason() {
  local stem="$1"
  if [[ "$profile" != "legacy" || "$backend" != "classic" || "$source_set" != "original" ]]; then
    return 1
  fi
  case "$stem" in
    KALSCOPE)
      printf '%s\n' "original source passes untyped 0 where GetParam requires CARD POINTER"
      ;;
    PMGDM1|PMGDM2)
      printf '%s\n' "original source passes a byte value where Zero requires BYTE POINTER"
      ;;
    PRINTF1)
      printf '%s\n' "original source relies on loose cross-pointee pointer assignment"
      ;;
    *)
      return 1
      ;;
  esac
}

expected_legacy_rejection_diagnostic() {
  case "$1" in
    KALSCOPE)
      printf '%s\n' '`GetParam` argument 3 expects'
      ;;
    PMGDM1|PMGDM2)
      printf '%s\n' '`Zero` argument 1 expects'
      ;;
    PRINTF1)
      printf '%s\n' 'cannot assign ValueType { base: Fund(Card), pointer: true } to ValueType { base: Fund(Int), pointer: true }'
      ;;
    *)
      return 1
      ;;
  esac
}

declares_endprog() {
  local source="$1"
  sed 's/;.*//' "$source" | grep -Eiq '^[[:space:]]*CARD[[:space:]].*EndProg'
}

list_entries() {
  local entry stem source object kind source_policy selected
  printf '%-10s %-14s %-14s %-8s %-11s %s\n' "Stem" "Source" "Object" "Kind" "Policy" "Modern overlay"
  for entry in "${entries[@]}"; do
    IFS=: read -r stem source object kind source_policy <<<"$entry"
    source_policy="${source_policy:-auto}"
    selected="-"
    if [[ -f "$modern_dir/$source" ]]; then
      selected="yes"
    fi
    printf '%-10s %-14s %-14s %-8s %-11s %s\n' "$stem" "$source" "$object" "$kind" "$source_policy" "$selected"
  done
}

compile_one_preset() {
  local active_preset="$1"
  if [[ "$custom_config" -eq 0 ]]; then
    apply_preset "$active_preset"
  fi

  backend="$(normalize_backend "$backend")"
  profile="$(normalize_profile "$profile")"
  source_set="$(normalize_source_set "$source_set")"

  local slug out_dir tmp_dir overlaid_toolkit_dir compat_original_dir report actionc gate_failures successes expected_rejections entry
  if [[ "$active_preset" == "custom" ]]; then
    slug="$(preset_slug)"
  else
    slug="$active_preset"
  fi
  out_dir="$output_root/$slug"
  tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/actionc-toolkit-batch.XXXXXX")"
  report="$out_dir/TOOLKIT_BATCH.md"
  mkdir -p "$out_dir"
  rm -f "$out_dir"/*.COM "$out_dir"/*.log "$out_dir"/*.err "$report"
  trap 'rm -rf "$tmp_dir"' RETURN

  cd "$repo_root"
  cargo build --quiet
  actionc="$repo_root/target/debug/actionc-emit"

  overlaid_toolkit_dir="$tmp_dir/toolkit"
  mkdir -p "$overlaid_toolkit_dir"
  while IFS= read -r toolkit_file; do
    ln -s "$toolkit_file" "$overlaid_toolkit_dir/$(basename "$toolkit_file")"
  done < <(find "$original_dir" -maxdepth 1 -type f | sort)

  if [[ -d "$modern_dir" ]]; then
    while IFS= read -r overlay_file; do
      rm -f "$overlaid_toolkit_dir/$(basename "$overlay_file")"
      ln -s "$overlay_file" "$overlaid_toolkit_dir/$(basename "$overlay_file")"
    done < <(find "$modern_dir" -maxdepth 1 -type f ! -name '.*' | sort)
  fi

  compat_original_dir="$tmp_dir/original/extracted"
  mkdir -p "$compat_original_dir"
  while IFS= read -r toolkit_file; do
    ln -s "$toolkit_file" "$compat_original_dir/$(basename "$toolkit_file")"
  done < <(find "$overlaid_toolkit_dir" -maxdepth 1 \( -type f -o -type l \) | sort)

  {
    echo "# Toolkit Batch Compile"
    echo
    echo "Generated by \`surveys/toolkit/compile-toolkit-batch.sh\`."
    echo
    echo "- Preset: \`$active_preset\`"
    echo "- Source set: \`$source_set\`"
    echo "- Profile: \`$profile\`"
    echo "- Backend: \`$backend\`"
    echo "- Codegen source: \`$(effective_codegen_source_label)\`"
    if [[ -n "$origin_value" ]]; then
      echo "- Origin: \`$origin_value\`"
    fi
    echo
    echo "Libraries skipped because demos are compiled instead:"
    echo
    local skip
    for skip in "${skipped_by_demo[@]}"; do
      echo "- \`$skip\`"
    done
    echo
    echo "| Stem | Input | Actual Source | Status | Bytes | Detail |"
    echo "| --- | --- | --- | --- | ---: | --- |"
  } > "$report"

  gate_failures=0
  successes=0
  expected_rejections=0
  for entry in "${entries[@]}"; do
    local stem source object kind source_policy selected_source compile_source harness_include_source object_path log_path err_path status detail bytes expected_reason expected_diagnostic replacement_path replacement_compile_source replacement_err_path diagnostic replacement_diagnostic reported_source
    local cmd
    IFS=: read -r stem source object kind source_policy <<<"$entry"
    source_policy="${source_policy:-auto}"
    if ! entry_selected "$stem" "$source" "$object"; then
      continue
    fi

    selected_source="$(select_source "$source" "$source_policy")"
    reported_source="$selected_source"
    if [[ ! -f "$selected_source" ]]; then
      status="missing"
      detail="source file not found"
      bytes=0
      gate_failures=$((gate_failures + 1))
      printf '| `%s` | `%s` | `%s` | %s | %s | %s |\n' \
        "$stem" "$source" "$(escape_md "$(display_path "$selected_source")")" \
        "$status" "$bytes" "$(escape_md "$detail")" >> "$report"
      continue
    fi

    compile_source="$selected_source"
    if [[ "$source_set" == "modern" || "$selected_source" == "$modern_dir/$source" ]]; then
      compile_source="$overlaid_toolkit_dir/$source"
    fi
    if [[ "$kind" == "harness" ]] && ! declares_endprog "$selected_source"; then
      harness_include_source="$compile_source"
      compile_source="$tmp_dir/$stem-harness.act"
      printf 'CARD EndProg\nINCLUDE "%s"\n' "$harness_include_source" > "$compile_source"
    fi

    object_path="$out_dir/$object"
    log_path="$out_dir/$stem.log"
    err_path="$out_dir/$stem.err"
    rm -f "$object_path" "$log_path" "$err_path"

    echo "==> [$slug] $stem: $(display_path "$selected_source") -> $(display_path "$object_path")"
    cmd=(
      "$actionc"
      --emit-load
      --profile "$profile"
      --backend "$backend"
      --codegen-source "$codegen_source"
    )
    if [[ -n "$origin_value" ]]; then
      cmd+=(--origin "$origin_value")
    fi
    expected_reason=""
    expected_diagnostic=""
    replacement_path=""
    replacement_compile_source=""
    replacement_err_path="$tmp_dir/$stem-modern.err"
    if expected_reason="$(expected_legacy_rejection_reason "$stem")"; then
      expected_diagnostic="$(expected_legacy_rejection_diagnostic "$stem")"
      if [[ -f "$modern_dir/$source" ]]; then
        replacement_path="$(display_path "$modern_dir/$source")"
      else
        replacement_path="missing modern replacement"
      fi
    fi

    if "${cmd[@]}" "$compile_source" >"$object_path" 2>"$err_path"; then
      bytes="$(wc -c < "$object_path" | tr -d ' ')"
      if [[ -n "$expected_reason" ]]; then
        status="unexpected-success"
        detail="expected rejection: $expected_reason; replacement: $replacement_path"
        gate_failures=$((gate_failures + 1))
        bytes=0
        rm -f "$object_path"
      else
        status="ok"
        detail="wrote $(display_path "$object_path")"
        successes=$((successes + 1))
      fi
      rm -f "$err_path"
    else
      bytes=0
      sanitize_file_paths "$err_path"
      diagnostic="$(first_error "$err_path")"
      if [[ -n "$expected_reason" ]] \
        && [[ -f "$modern_dir/$source" ]] \
        && grep -Fq -- "$expected_diagnostic" "$err_path"; then
        expected_rejections=$((expected_rejections + 1))
        replacement_compile_source="$overlaid_toolkit_dir/$source"
        rm -f "$replacement_err_path"
        if "${cmd[@]}" "$replacement_compile_source" >"$object_path" 2>"$replacement_err_path"; then
          bytes="$(wc -c < "$object_path" | tr -d ' ')"
          status="ok-modernized"
          detail="original rejected as expected: $expected_reason; compiled replacement: $replacement_path; original diagnostic: $diagnostic"
          reported_source="$modern_dir/$source"
          successes=$((successes + 1))
          rm -f "$replacement_err_path"
        else
          bytes=0
          sanitize_file_paths "$replacement_err_path"
          replacement_diagnostic="$(first_error "$replacement_err_path")"
          status="failed"
          detail="maintained replacement failed under legacy classic: $replacement_diagnostic; original rejection: $diagnostic"
          gate_failures=$((gate_failures + 1))
          rm -f "$object_path"
        fi
      else
        status="failed"
        if [[ -n "$expected_reason" && ! -f "$modern_dir/$source" ]]; then
          detail="expected legacy rejection has no maintained modern replacement"
        elif [[ -n "$expected_reason" ]]; then
          detail="expected diagnostic containing '$expected_diagnostic'; got: $diagnostic"
        else
          detail="$diagnostic"
        fi
        gate_failures=$((gate_failures + 1))
      fi
      if [[ "$status" != "ok-modernized" ]]; then
        rm -f "$object_path"
      fi
    fi
    {
      echo "source: $(display_path "$selected_source")"
      echo "source_policy: $source_policy"
      echo "compile_source: $(display_path "$compile_source")"
      if [[ -n "$replacement_compile_source" ]]; then
        echo "artifact_source: $(display_path "$replacement_compile_source")"
      fi
      echo "profile: $profile"
      echo "backend: $backend"
      echo "codegen_source: $(effective_codegen_source_label)"
      echo "status: $status"
      echo "detail: $detail"
      if [[ -f "$err_path" ]]; then
        echo
        echo "validation_stderr:"
        cat "$err_path"
      fi
      if [[ -f "$replacement_err_path" ]]; then
        echo
        echo "replacement_stderr:"
        cat "$replacement_err_path"
      fi
    } > "$log_path"

    printf '| `%s` | `%s` | `%s` | %s | %s | %s |\n' \
      "$stem" "$source" "$(escape_md "$(display_path "$reported_source")")" \
      "$status" "$bytes" "$(escape_md "$detail")" >> "$report"
  done

  {
    echo
    echo "Summary:"
    echo
    echo "- Successes: $successes"
    echo "- Expected rejections: $expected_rejections"
    echo "- Gate failures: $gate_failures"
  } >> "$report"

  cat "$report"
  if [[ "$gate_failures" -ne 0 ]]; then
    return 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --preset)
      [[ $# -ge 2 ]] || { echo "--preset requires a value" >&2; exit 2; }
      preset="$2"
      shift 2
      ;;
    --preset=*)
      preset="${1#*=}"
      shift
      ;;
    --profile)
      [[ $# -ge 2 ]] || { echo "--profile requires a value" >&2; exit 2; }
      profile="$(normalize_profile "$2")"
      custom_config=1
      shift 2
      ;;
    --profile=*)
      profile="$(normalize_profile "${1#*=}")"
      custom_config=1
      shift
      ;;
    --backend)
      [[ $# -ge 2 ]] || { echo "--backend requires a value" >&2; exit 2; }
      backend="$(normalize_backend "$2")"
      custom_config=1
      shift 2
      ;;
    --backend=*)
      backend="$(normalize_backend "${1#*=}")"
      custom_config=1
      shift
      ;;
    --codegen-source|--codegen)
      [[ $# -ge 2 ]] || { echo "$1 requires a value" >&2; exit 2; }
      codegen_source="$2"
      shift 2
      ;;
    --codegen-source=*|--codegen=*)
      codegen_source="${1#*=}"
      shift
      ;;
    --source-set)
      [[ $# -ge 2 ]] || { echo "--source-set requires original or modern" >&2; exit 2; }
      source_set="$(normalize_source_set "$2")"
      custom_config=1
      shift 2
      ;;
    --source-set=*)
      source_set="$(normalize_source_set "${1#*=}")"
      custom_config=1
      shift
      ;;
    --origin)
      [[ $# -ge 2 ]] || { echo "--origin requires an address" >&2; exit 2; }
      origin_value="$2"
      shift 2
      ;;
    --output-dir|--out-dir)
      [[ $# -ge 2 ]] || { echo "$1 requires a directory" >&2; exit 2; }
      output_root="$2"
      shift 2
      ;;
    --output-dir=*|--out-dir=*)
      output_root="${1#*=}"
      shift
      ;;
    --list)
      list_only=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      while [[ $# -gt 0 ]]; do
        entry_filters+=("$1")
        shift
      done
      ;;
    -*)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      entry_filters+=("$1")
      shift
      ;;
  esac
done

if [[ "$list_only" -eq 1 ]]; then
  list_entries
  exit 0
fi

if [[ "$custom_config" -eq 1 ]]; then
  : "${profile:=modern}"
  : "${backend:=classic}"
  : "${source_set:=modern}"
fi

case "$preset" in
  all)
    overall=0
    for one in legacy-classic modern-classic modern-mir6502; do
      custom_config=0
      if ! compile_one_preset "$one"; then
        overall=1
      fi
    done
    exit "$overall"
    ;;
  legacy-classic|modern-classic|modern-mir6502|compat-legacy|modern-legacy)
    if [[ "$custom_config" -eq 1 ]]; then
      if compile_one_preset "custom"; then
        exit 0
      fi
      exit 1
    fi
    if compile_one_preset "$preset"; then
      exit 0
    fi
    exit 1
    ;;
  *)
    echo "invalid preset: $preset" >&2
    exit 2
    ;;
esac
