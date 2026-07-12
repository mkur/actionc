#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

usage() {
  cat <<EOF
Usage: tools/actionc-to-atascii.sh <input> [output]

Convert actionc's escaped ASCII source encoding back to raw ATASCII bytes.
If output is omitted, raw ATASCII is written to stdout.

Supported escapes are the same as actionc source parsing:
  \\{\$HH}       exact ATASCII byte
  \\{CHAR:\$HH}  exact ATASCII byte
  \\{RETURN}     \$9B
  \\{ESC}        \$1B
  \\{CLEAR}      \$7D
  \\{INV:text}   inverse-video ASCII bytes
EOF
}

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
esac

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage >&2
  exit 2
fi

cargo run -q --manifest-path "$repo_root/crates/atrcopy-rs/Cargo.toml" \
  --bin ascii-to-atascii -- "$@"
