#!/usr/bin/env bash

# Build the deterministic Atari800 arguments shared by launcher scripts and
# focused tests. The caller may append the image/run arguments afterwards.
actionc_build_atari800_launch_args() {
  local os_rom="$1"
  local cart_rom="$2"
  local extra_args="${3:-}"
  local no_cart_config="${4:-}"

  ACTIONC_ATARI800_LAUNCH_ARGS=()

  # Atari800 has no command-line eject option. The caller supplies a sanitized
  # copy of the user's configuration so --no-cart preserves ROM/video settings
  # while clearing saved primary and piggyback cartridges.
  if [[ -z "$cart_rom" && -n "$no_cart_config" ]]; then
    ACTIONC_ATARI800_LAUNCH_ARGS+=(-config "$no_cart_config" -no-autosave-config)
  fi

  # TN and the bundled OS image require the XL/XE memory/ROM layout. Explicitly
  # select it instead of inheriting MACHINE_TYPE from the user's configuration.
  ACTIONC_ATARI800_LAUNCH_ARGS+=(-xl)
  if [[ -n "$os_rom" ]]; then
    ACTIONC_ATARI800_LAUNCH_ARGS+=(-xlxe_rom "$os_rom")
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

actionc_write_no_cart_config() {
  local source_config="$1"
  local target_config="$2"

  awk '
    BEGIN {
      primary_name = primary_type = piggyback_name = piggyback_type = 0
    }
    /^CARTRIDGE_FILENAME=/ {
      print "CARTRIDGE_FILENAME="
      primary_name = 1
      next
    }
    /^CARTRIDGE_TYPE=/ {
      print "CARTRIDGE_TYPE=0"
      primary_type = 1
      next
    }
    /^CARTRIDGE_PIGGYBACK_FILENAME=/ {
      print "CARTRIDGE_PIGGYBACK_FILENAME="
      piggyback_name = 1
      next
    }
    /^CARTRIDGE_PIGGYBACK_TYPE=/ {
      print "CARTRIDGE_PIGGYBACK_TYPE=0"
      piggyback_type = 1
      next
    }
    { print }
    END {
      if (!primary_name) print "CARTRIDGE_FILENAME="
      if (!primary_type) print "CARTRIDGE_TYPE=0"
      if (!piggyback_name) print "CARTRIDGE_PIGGYBACK_FILENAME="
      if (!piggyback_type) print "CARTRIDGE_PIGGYBACK_TYPE=0"
    }
  ' "$source_config" > "$target_config"
}
