#!/usr/bin/env bash

# Build the deterministic Atari800 arguments shared by launcher scripts and
# focused tests. The caller may append the image/run arguments afterwards.
actionc_build_atari800_launch_args() {
  local os_rom="$1"
  local cart_rom="$2"
  local extra_args="${3:-}"

  ACTIONC_ATARI800_LAUNCH_ARGS=()

  # Atari800 has no command-line eject option. An empty configuration prevents
  # a cartridge saved in ~/.atari800.cfg from defeating --no-cart.
  if [[ -z "$cart_rom" ]]; then
    ACTIONC_ATARI800_LAUNCH_ARGS+=(-config /dev/null -no-autosave-config)
  fi

  # TN and the bundled OS image require the XL/XE memory/ROM layout. Explicitly
  # select it instead of inheriting MACHINE_TYPE from the user's configuration.
  ACTIONC_ATARI800_LAUNCH_ARGS+=(-xl)
  if [[ -n "$os_rom" ]]; then
    ACTIONC_ATARI800_LAUNCH_ARGS+=(-xlxe_rom "$os_rom" -xl-rev custom)
  fi
  if [[ -n "$cart_rom" ]]; then
    ACTIONC_ATARI800_LAUNCH_ARGS+=(-cart "$cart_rom")
  fi
  if [[ -n "$extra_args" ]]; then
    local -a parsed_extra_args=()
    read -r -a parsed_extra_args <<< "$extra_args"
    ACTIONC_ATARI800_LAUNCH_ARGS+=("${parsed_extra_args[@]}")
  fi
}
