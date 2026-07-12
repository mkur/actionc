# actionc Usage

Install `actionc` from this repository:

```sh
cargo install --path . --bin actionc
```

Compile an Action! source file with:

```sh
actionc [--mode <mode>] [options] [-o <file.com>] [--listing <file.lst>] <file.act>
```

Without `-o` or `--output`, `actionc` writes `<source-stem>.com` in the current
directory. Missing parent directories are created automatically. Developer
representations and raw stdout output are provided by `actionc-emit`.

Install the developer output tool separately when needed:

```sh
cargo install --path . --bin actionc-emit
```

## Build Modes

Modes provide user-facing presets for the compiler's lower-level profile and
backend settings:

- `--mode compatibility` conservatively compiles original Action! source. This
  is the default.
- `--mode optimized` enables the optimized path for maintained and modernized
  source.
- `--mode mir6502` selects the experimental MIR6502 compiler pipeline.

`--mode=<mode>` is also accepted. A mode cannot be combined with explicit
`--profile` or `--backend` options.

## Common Commands

Generate an Atari load-format object file and a source listing in one
invocation:

```sh
actionc samples/hello-world.act \
  --output target/hello-world.com \
  --listing target/hello-world.lst
```

Generate the optimized classic-backend listing:

```sh
actionc samples/hello-world.act \
  --mode optimized \
  --output target/hello-world-modern.com \
  --listing target/hello-world-modern.lst
```

Inspect NIR:

```sh
cargo run --quiet --bin actionc-emit -- \
  --emit-nir samples/hello-world.act
```

Try the experimental MIR6502 backend:

```sh
cargo run --quiet --bin actionc-emit -- \
  --profile modern \
  --backend mir6502 \
  --emit-source-listing samples/hello-world.act
```

## `actionc` Output Options

- `-o <file>` and `--output <file>` select the load-format object path.
- `--listing <file>` additionally writes a source-annotated listing.
- Parent directories are created automatically.
- `-o -` is rejected; use `actionc-emit --emit-load` for binary stdout.

## `actionc-emit` Modes

`actionc-emit` writes one representation to stdout. Emit modes are mutually
exclusive; with no explicit mode it preserves the historical `--emit-code`
hex-text behavior.

- `--emit-load` writes an Atari load-format binary to stdout. Redirect it to a
  `.com` file.
- `--emit-source-listing` writes a disassembly listing with source context.
  `--emit-listing-source` is accepted as an alias.
- `--emit-listing` writes a disassembly listing without source context.
- `--emit-code` writes generated code bytes as hex text.
- `--emit-map` writes the generated code map, including source ranges and
  optimization records.
- `--emit-proofs` writes the proof/fact view for generated code.
- `--emit-proof-attempts` writes proof-attempt diagnostics. `--emit-proof-debug`
  is accepted as an alias.
- `--emit-tokens` writes lexer tokens.
- `--emit-semir` writes the semantic IR.
- `--emit-nir` writes NIR. The old `--emit-tac` alias has been removed.
- `--emit-mir6502` writes MIR6502 before materialization.
- `--emit-materialized-mir6502` writes MIR6502 after materialization.
  `--emit-mir6502-materialized` is accepted as an alias.

## Profiles

Profiles are the lower-level compatibility and optimization policy used by a
mode. Advanced users may select one directly instead of using `--mode`:

- `--profile legacy` is the default. It is the most compatible profile and
  allows most old Action! idioms.
- `--profile compat` is accepted as an alias for `legacy`.
- `--profile modern` enables the modern layout and optimization policy, and it
  requires explicit source forms for some ambiguous old idioms.

Both profiles accept `actionc` syntax extensions such as explicit casts,
address values, function pointers, and machine-block label-byte syntax. See
[docs/CODEGEN_PROFILES.md](docs/CODEGEN_PROFILES.md) and
[docs/SYNTAX_EXTENSIONS.md](docs/SYNTAX_EXTENSIONS.md) for details.

For day-to-day optimized binaries, prefer:

```sh
--profile modern --backend classic
```

## Backends

Backends are the lower-level code-generator selection used by a mode. Advanced
users may select one directly instead of using `--mode`:

- `--backend classic` is the default. It is the mature AST-based backend and the
  supported path for runnable Atari load files today.
- `--backend legacy` and `--backend default` are accepted as aliases for
  `classic`.
- `--backend mir6502` selects the experimental MIR6502 backend. It requires
  `--profile modern`.

The legacy profile is the most compatible. The modern profile with the classic
backend usually produces the smallest binaries. MIR6502 is the future direction
for `actionc`, but it is experimental today.

Source files can provide default compiler settings with leading comments:

```action
;@actionc profile modern
;@actionc backend mir6502
```

These settings are used only when the corresponding command-line option is not
provided. An explicit mode overrides both annotations; explicit `--profile`
and `--backend` flags override their corresponding annotation.

## Other Options

- `--origin <addr>` sets the code origin. Addresses may be decimal, `$` hex, or
  `0x` hex, for example `12288`, `$3000`, or `0x3000`.
- `--origin=<addr>` is also accepted.
- `--mode=<mode>` is accepted as an alternative to `--mode <mode>`.
- `--profile=<profile>` and `--backend=<backend>` are also accepted.
- `--diagnostic-byte-ranges` includes byte ranges in diagnostics.
- `--debug-diagnostic-spans` is accepted as an alias for
  `--diagnostic-byte-ranges`.
- `-h` and `--help` print the short usage line.

## Advanced Development Options

`--codegen-source` is a compiler-development switch for the classic backend. It
is useful when validating internal lowering paths, but normal users should leave
it at the default.

- `--codegen-source ast` is the default and the primary classic backend path.
- `--codegen-source semir` uses the SemIR bridge. It lowers through SemIR, then
  reconstructs the AST-facing backend input so output can be compared with the
  AST path.
- `--codegen-source semir-native` uses the SemIR-native path.
- `--codegen-source=<source>` is also accepted.

Accepted aliases are:

- `sem-ir` for `semir`;
- `native`, `sem-ir-native`, `native-ir`, and `modern-ir` for `semir-native`.

For compatibility with older scripts, `--profile semir-native`,
`--profile sem-ir-native`, `--profile native-ir`, and `--profile modern-ir`
select the modern profile and the SemIR-native codegen source.

`--codegen-source` does not select the MIR6502 pipeline. `--backend mir6502`
always lowers through SemIR, NIR, optimized NIR, and MIR6502.

## Compile And Run Helper

[tools/compile-run-atr.sh](tools/compile-run-atr.sh) compiles a source file,
copies the generated object into a bootable ATR, and can launch `atari800`:

```sh
tools/compile-run-atr.sh samples/hello-world.act
```

The full command form is:

```sh
tools/compile-run-atr.sh [options] <source.act> [source.atr]
tools/compile-run-atr.sh --pack-object <file.com> [options] [source.atr]
```

If `[source.atr]` is omitted, the helper uses `atr/mydos.atr`.

With compilation:

```sh
tools/compile-run-atr.sh \
  --profile modern \
  --backend classic \
  --out-dir target/readme-sample \
  samples/hello-world.act
```

With an existing object:

```sh
tools/compile-run-atr.sh \
  --pack-object target/hello-world.com \
  --no-run
```

Helper options:

- `--profile <legacy|modern>` selects the `actionc` profile. The default is
  `legacy`; `compat` is accepted as a legacy alias.
- `--profile=<legacy|modern>` is also accepted.
- `--backend <classic|mir6502>` selects the `actionc` backend. The default is
  `classic`; `legacy` and `default` are accepted as classic aliases, and `mir`
  and `6502` are accepted as MIR6502 aliases.
- `--backend=<classic|mir6502>` is also accepted.
- `--origin <addr>` passes an explicit origin to `actionc`.
- `--name <stem>` chooses the Atari output filename stem. The default is the
  source basename or packed-object basename, uppercased and truncated to fit an
  Atari 8.3 `.COM` filename.
- `--out-dir <dir>` keeps generated artifacts in a chosen directory.
- `--out-atr <file.atr>` writes the ATR to an explicit path.
- `--object <file.com>` writes the generated object to an explicit path.
- `--pack-object <file.com>` skips compilation and packs an existing object.
- `--input-object <file.com>` is accepted as an alias for `--pack-object`.
- `--run-mode disk|host` selects how `atari800` is launched.
- `--atari800 <path>` selects the emulator executable. The default is
  `$ATARI800` or `atari800`.
- `--cart <rom>` attaches an Atari cartridge ROM when launching the emulator.
- `--no-cart` ignores `ACTIONC_ATARI800_CART` and `ACTION_VM_CART`.
- `--os <rom>` selects the Atari XL/XE OS ROM passed to `atari800`.
- `--no-os` uses the emulator's configured/default OS ROM.
- `--no-run` builds the ATR without launching `atari800`.
- `--keep` keeps temporary artifacts and prints their path.
- `-h` and `--help` print the helper usage.

Advanced helper options:

- `--codegen-source <ast|semir|semir-native>` passes the classic-backend
  development switch through to `actionc`. The default is `ast`.
- `--codegen-source=<source>` is also accepted.
- `--codegen <source>` and `--codegen=<source>` are accepted as aliases for
  `--codegen-source`.

Helper environment variables:

- `ACTIONC_BACKEND` overrides the default helper backend.
- `ACTIONC_CODEGEN_SOURCE` overrides the default helper codegen source for
  compiler-development runs.
- `ACTIONC_ATARI800_CART` selects the cartridge ROM passed as `atari800 -cart`.
- `ACTION_VM_CART` is the fallback cartridge ROM setting shared with VM tools.
- `ACTIONC_ATARI800_OS` selects the OS ROM passed as `atari800 -xlxe_rom`.
- `ACTION_VM_OS` is the fallback OS ROM setting shared with VM tools.
- `ATARI800` overrides the emulator executable.
- `ATARI800_ARGS` appends extra words to the `atari800` invocation.
