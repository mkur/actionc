# Running Programs with `actionc-run`

`actionc-run` compiles an Action! source file and boots it from an embedded
MyDOS image in Atari800 or Altirra. The image contains a small `BOOT.AR0`
bootstrap followed by the generated object as `PROGRAM.AR1`. The bootstrap
selects the Action! cartridge's resident-library bank and opens `E:` on IOCB 6
before MyDOS starts the program. Action!'s default output device is set to that
dedicated channel because MyDOS still owns IOCB 0 while it invokes the load
file's RUNAD. With `--runtime standalone` or its `--no-cart` convenience form,
the compiler selects the standalone runtime, the bootstrap and cartridge are
omitted, and the generated object is stored directly as `PROGRAM.AR0`. No Bash
or intermediate `.xex` file is required.

Install it with:

```sh
cargo install --path . --bin actionc-run
```

Then compile and run a program with:

```sh
actionc-run samples/hello-world.act
```

The default cartridge is bundled in the executable. Atari800 runs also use the
bundled AltirraOS XL image; Altirra uses its built-in XL AltirraOS kernel.

## Common Commands

Use the optimized compiler mode:

```sh
actionc-run --mode optimized samples/hello-world.act
```

Select an emulator explicitly:

```sh
actionc-run --emulator atari800 samples/hello-world.act
actionc-run --emulator altirra samples/hello-world.act
```

Select a particular emulator executable when discovery is not enough:

```sh
actionc-run \
  --emulator altirra \
  --emulator-path "C:\Program Files\Altirra\Altirra64.exe" \
  samples/hello-world.act
```

Prepare an ATR without discovering or launching an emulator:

```sh
actionc-run \
  --no-run \
  --out-atr build/hello-world.atr \
  samples/hello-world.act
```

Compile and run a standalone arithmetic fixture with no Action! cartridge:

```sh
actionc-run --runtime standalone fixtures/runtime/standalone_arithmetic.act
```

For runner-oriented commands, `--no-cart` is a shorter spelling of the same
standalone compilation and launch choice.

## Options

```text
actionc-run [--mode compatibility|optimized|mir6502]
            [--runtime cart|standalone]
            [--emulator auto|atari800|altirra]
            [--emulator-path <path>]
            [--cart <path>|--no-cart]
            [--no-run]
            [--out-atr <path>]
            [--keep]
            <source.act>
```

- `--mode` selects the same public compiler modes as `actionc`. Without it,
  source annotations remain active and the compiler otherwise uses its
  compatibility defaults.
- `--runtime` selects the same runtime provider as `actionc` and
  `actionc-emit`. `cart` uses the bundled cartridge unless `--cart` supplies a
  replacement. `standalone` runs without a cartridge.
- `--emulator auto` is the default. On Windows it prefers Altirra and then
  Atari800; on Linux and macOS it looks for Atari800.
- `--emulator-path` overrides executable discovery. With `auto`, recognized
  names such as `Altirra64.exe`, `Altirra.exe`, and `atari800` also select the
  adapter. For another filename, specify `--emulator` too.
- `--cart` replaces the bundled Action! cartridge and implies `--runtime cart`.
  `--no-cart` is a convenience form of `--runtime standalone`: it runs without
  a cartridge, omits the Action! bank/default-output bootstrap, stores the
  program as `PROGRAM.AR0`, and prevents Atari800 from restoring a cartridge
  from saved settings. Contradictory runtime and cartridge options are
  rejected.
- `--no-run` writes an ATR and does not inspect emulator configuration or the
  host PATH. Without `--out-atr`, it writes `<source-stem>.atr` in the current
  directory.
- `--out-atr` retains the generated ATR at the selected path, including after
  an emulator run.
- `--keep` retains and reports the otherwise temporary run directory. It is
  only meaningful when an emulator is launched.

`actionc-run` inherits the terminal streams for the emulator and waits until
the emulator exits. Temporary media remains available for that whole time and
is removed afterward unless `--keep` was used.

The experimental standalone runtime covers compiler arithmetic and
argument-copy helpers plus migrated block-memory, string, miscellaneous,
graphics, and core I/O resident routines. Ordinary single-file source keeps
using traditional unqualified calls such as `Zero`, `SCopy`, `Position`,
`Print`, and `PutE`; no module declaration or import is required. A resident
call without a standalone binding is rejected instead of retaining a hidden
cartridge dependency.
Standalone programs that include embedded Action runtime code are subject to
GPL-3.0-or-later; release archives carry the exact embedded runtime sources
under `licenses/runtime-source/` and their provenance in
`licenses/ACTION-RUNTIME-NOTICE.md`.

## Emulator Discovery

Discovery uses this order:

1. `--emulator-path`;
2. the `ACTIONC_EMULATOR` environment variable;
3. recognized executable names on `PATH`;
4. common platform installation paths.

Automatic discovery does not run Altirra through Wine. On a non-Windows host,
an Altirra launch must be selected explicitly and point at a directly
executable command.

## Advanced ATR Workflows

The older [compile-run-atr.sh](../tools/compile-run-atr.sh) helper remains for
compiler-development workflows that need a custom source ATR, object packing,
host-object loading, raw Atari800 arguments, or lower-level profile/backend
selection. Normal source compile-and-run workflows should use `actionc-run`.
