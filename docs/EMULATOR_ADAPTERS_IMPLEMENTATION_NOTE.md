# Emulator Adapters Implementation Note

Status: planned.

Snapshot date: 2026-08-11.

## Goal

Provide a cross-platform `actionc-run` executable that compiles an Action!
source file, places the resulting load file in the embedded MyDOS image, and
launches a supported Atari emulator through a small adapter boundary.

The first supported emulators are:

- Atari800 on Linux, macOS, and Windows;
- Altirra on Windows.

The first version boots one common disk artifact in both emulators. It does not
use emulator-specific direct executable loading. A small `BOOT.AR0` selects the
Action! resident-library bank, opens `E:` on a dedicated IOCB, and makes that
channel Action!'s default device; then MyDOS loads the generated object from
`PROGRAM.AR1`. The dedicated channel is required because MyDOS still owns IOCB
0 when it invokes RUNAD. With `--runtime standalone` or `--no-cart`, the
bootstrap is omitted and the object is stored directly as `PROGRAM.AR0`.

## Existing Prerequisites

The repository already has the two reusable boundaries needed by the runner:

- `actionc::compiler::compile_file` compiles all three user-facing modes and
  returns complete load-format object bytes without writing a file or exiting;
- `atrcopy_rs::AtrImage` can start from embedded `MYDOS_ATR` and construct
  either autorun layout entirely in memory.

The current `tools/compile-run-atr.sh` and
`tools/lib/atari800-launch.sh` scripts are the behavioral reference for the
Atari800 adapter. They remain available until the Rust runner reaches parity.

## Boundary

The runner is split into four layers:

```text
Action source
    |
    v
compiler API -> load-format object
    |
    v
ATR preparation -> MyDOS image with BOOT.AR0 + PROGRAM.AR1, or PROGRAM.AR0
    |
    v
emulator adapter -> executable plus argument vector
    |
    v
shared process executor -> child lifetime and cleanup
```

Compilation and ATR construction must not know which emulator will run the
program. Emulator adapters must not compile, modify ATR files, create temporary
directories, print, exit, or spawn processes.

## Adapter Contract

Keep the adapter API crate-private initially:

```rust
pub(crate) enum EmulatorKind {
    Atari800,
    Altirra,
}

pub(crate) struct LaunchRequest<'a> {
    pub atr: &'a Path,
    pub cartridge: Option<&'a Path>,
    pub os_rom: Option<&'a Path>,
}

pub(crate) struct CommandSpec {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
}

pub(crate) trait EmulatorAdapter {
    fn kind(&self) -> EmulatorKind;
    fn command(&self, request: &LaunchRequest<'_>)
        -> Result<CommandSpec, EmulatorError>;
}
```

`CommandSpec` uses `OsString`, not shell text. Paths containing spaces and
non-UTF-8 paths supported by the host must cross the boundary without being
split, quoted, or reinterpreted.

Discovery is separate from command construction. A discovered adapter owns the
resolved executable path; command construction must not search the environment.

## Common Run Artifact

The default and initially only launch form is disk boot:

1. Compile the source with the reusable compiler API.
2. Parse embedded `MYDOS_ATR` into `AtrImage`.
3. With a cartridge, add the Action! library-bank and default-output bootstrap
   as `BOOT.AR0` and `CompiledProgram::object_bytes()` as `PROGRAM.AR1`.
4. Without a cartridge, add only `CompiledProgram::object_bytes()` as
   `PROGRAM.AR0`.
5. Write the resulting ATR to a per-run temporary directory.
6. Materialize any embedded ROM assets required by the selected adapter.
7. Launch the emulator with the ATR and cartridge attached.

The runner must not create an intermediate `.com` file. `--no-run` may write an
ATR chosen by the caller, but normal runs keep all generated artifacts in the
temporary run directory.

## Assets

Asset selection is adapter-neutral:

```rust
pub(crate) struct RunAssets {
    pub atr: PathBuf,
    pub cartridge: Option<PathBuf>,
    pub os_rom: Option<PathBuf>,
}
```

- MyDOS is embedded through `atrcopy_rs::MYDOS_ATR`.
- The repository AltirraOS image may be embedded and materialized when an
  adapter needs an explicit OS ROM.
- Cartridge support is present from the first runnable slice. The bundled
  Action! cartridge is the default; `--cart` can replace it, while `--runtime
  standalone` and its `--no-cart` convenience form disable it.
- Emulator processes receive filesystem paths because both Atari800 and
  Altirra load media from files.

Asset licensing and provenance notices must stay beside the embedded files.
Adding an adapter must not silently add a new redistributable binary.

## Atari800 Adapter

Port the current shell behavior without invoking Bash:

- select Atari 800XL with `-xl`;
- provide an explicit XL/XE OS with `-xlxe_rom` when configured;
- attach the cartridge with `-cart`;
- append the generated ATR as a disk image;
- use a sanitized temporary configuration for no-cartridge runs when a saved
  Atari800 configuration would otherwise reattach a cartridge;
- prevent configuration autosave when using a temporary configuration.

Configuration sanitization belongs to shared runner preparation for this
adapter, not to command formatting. It must never overwrite the user's
configuration.

## Altirra Adapter

Altirra is a native Windows adapter:

- recognize `Altirra64.exe` and `Altirra.exe`;
- attach the ATR explicitly as a disk;
- attach the cartridge explicitly;
- pass every command-line switch as a separate argument;
- select or create an XL/XE-compatible temporary configuration without
  modifying the user's persistent emulator settings;
- use disk boot, not direct `.xex`/`.com` loading, in the first version.

The exact profile/configuration switches must be characterized against a
supported Altirra release and captured in adapter tests. If a fully isolated
profile cannot be selected reliably from the command line, the first adapter
may use the user's current hardware profile but must report that limitation
instead of pretending the run is deterministic.

## Discovery

Expose one emulator selection vocabulary:

```text
auto | atari800 | altirra
```

Discovery order:

1. `--emulator-path`;
2. `ACTIONC_EMULATOR`;
3. executable names on `PATH`;
4. platform-specific standard installation paths.

Automatic preference:

- Windows: Altirra, then Atari800;
- Linux and macOS: Atari800;
- other hosts: Atari800 if discoverable.

Do not automatically launch Altirra through Wine. Failure diagnostics list the
adapter names and paths that were checked.

Discovery must be testable through an injected host environment rather than by
mutating the real process environment in parallel tests.

## `actionc-run` CLI

Keep the first public surface small:

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

Rules:

- the default compiler options preserve source annotations;
- `--mode` maps directly to the public compiler API modes;
- `--emulator-path` selects the executable but does not change adapter kind
  unless the executable name can be identified unambiguously;
- `--no-run` prepares the ATR and never performs emulator discovery;
- `--out-atr` implies that the ATR is retained;
- `--keep` retains the temporary run directory and prints its location;
- invalid arguments and unresolved configuration exit with status 2;
- compilation, ATR construction, discovery, and process failures exit with
  status 1.

Do not add separate `--atari800`, `--altirra`, or adapter-specific video/audio
flags. Raw emulator argument pass-through can be added later if real use cases
cannot be expressed through the shared options.

## Process Lifetime

One shared executor owns launching:

- use `std::process::Command` directly;
- inherit stdin, stdout, and stderr;
- wait for the emulator by default;
- retain all temporary artifacts until the child exits;
- report spawn failures with the executable path and adapter name;
- return a non-zero child exit as a runner failure;
- remove the run directory afterward unless it was explicitly retained.

Detached mode and process-group management are follow-ups. The first version
has a simple lifetime: `actionc-run` remains alive for as long as the emulator.

## Implementation Slices

### Slice 1: contract and command harness

- Add `runner::emulator` types and the adapter trait.
- Add structured adapter/discovery errors.
- Add fake executable and argument-vector test helpers.
- Characterize the current Atari800 shell argument contract.

Acceptance criteria:

- command specs preserve arguments containing spaces;
- adapter code has no filesystem or process side effects;
- focused tests run without an installed emulator.

### Slice 2: ATR preparation and `actionc-run --no-run`

- Add the root path dependency on `atrcopy-rs`.
- Add the `actionc-run` binary and minimal parser.
- Compile through `compile_file`.
- With a cartridge, put the library-bank and default-output bootstrap in
  embedded MyDOS as `BOOT.AR0` and object bytes as `PROGRAM.AR1`.
- Without a cartridge, put object bytes directly in embedded MyDOS as
  `PROGRAM.AR0`.
- Write an explicitly retained ATR without creating a `.com` file.

Acceptance criteria:

- `actionc-run --no-run --out-atr output.atr sample.act` succeeds;
- extracting `BOOT.AR0` returns the embedded bootstrap exactly;
- extracting `PROGRAM.AR1` returns the compiler API object bytes exactly;
- with `--runtime standalone` or `--no-cart`, `BOOT.AR0` is absent and
  `PROGRAM.AR0` equals the compiler API object bytes;
- a failed compilation does not create the requested ATR.

### Slice 3: Atari800 adapter

- Port XL/XE, OS, cartridge, disk, and no-cartridge configuration behavior.
- Discover an explicit Atari800 executable and construct its command.
- Launch through the shared executor.

Acceptance criteria:

- Rust argument tests replace the existing shell-only launch tests;
- a fake emulator receives the exact expected arguments;
- an opt-in Atari800 smoke run boots the generated ATR.

### Slice 4: Altirra adapter

- Implement Windows executable recognition and command construction.
- Add disk/cartridge argument characterization.
- Add temporary configuration/profile isolation where supported.

Acceptance criteria:

- Windows argument tests cover both executable names and paths with spaces;
- a fake Altirra executable receives separate disk and cartridge arguments;
- a manual Windows smoke run boots a cartridge-library sample.

### Slice 5: automatic discovery and stable CLI

- Implement `auto`, explicit adapter selection, universal executable override,
  PATH lookup, and standard Windows Altirra locations.
- Produce actionable discovery diagnostics.
- Finish help and exit-code behavior.

Acceptance criteria:

- Windows auto-selection prefers Altirra;
- Unix auto-selection prefers Atari800;
- `--no-run` never requires an installed emulator;
- explicit selection never silently falls back to another adapter.

### Slice 6: lifecycle hardening and migration

- Add temporary-directory retention/cleanup tests.
- Add object/ATR parity coverage with the shell helper.
- Update README and usage documentation.
- Make `compile-run-atr.sh` delegate to `actionc-run`, or deprecate it after
  parity is demonstrated.
- Add an opt-in real-emulator smoke lane.

Acceptance criteria:

- the runner leaves no temporary artifacts after a normal completed run;
- `--keep` and `--out-atr` retain exactly the documented artifacts;
- paths with spaces work on Unix and Windows test hosts;
- the shell helper is no longer a second implementation of compilation and
  emulator launch policy.

## Validation Matrix

At minimum, cover:

| Case | Expected result |
| --- | --- |
| compatibility/optimized/MIR6502 | object packaged without mode drift |
| compiler error | structured error, no ATR |
| embedded MyDOS with cart | `BOOT.AR0` is the bootstrap and `PROGRAM.AR1` equals object bytes |
| embedded MyDOS without cart | `BOOT.AR0` is absent and `PROGRAM.AR0` equals object bytes |
| explicit Atari800 | exact XL/OS/cart/disk argument vector |
| explicit Altirra | exact disk/cart argument vector |
| Windows auto | Altirra preferred |
| Unix auto | Atari800 preferred |
| missing emulator | checked candidates reported |
| path containing spaces | one argument per path |
| `--runtime standalone` / `--no-cart` | no bootstrap or cartridge leaks into run; program uses `.AR0` |
| `--no-run` | no emulator discovery or process spawn |
| normal child exit | temporary directory removed |
| `--keep` | temporary directory retained and reported |

Final checks:

```sh
cargo test
cargo run --bin actionc-run -- \
  --no-run --out-atr target/actionc-run/hello.atr \
  samples/hello-world.act
cargo run --bin actionc-run -- \
  --emulator atari800 samples/hello-world.act
```

Run the Altirra smoke command on Windows CI or a Windows development host.

## Non-Goals

Do not add these to the first implementation:

- `actionc --run`;
- emulator downloading or installation;
- implicit Wine support;
- AltirraBridge/debugger automation;
- headless success detection from Atari memory or video;
- direct host executable loading;
- detached/background launching;
- public plugin registration for arbitrary emulator adapters;
- adapter-specific switches in the stable CLI;
- a general temporary-directory crate if the small runner can manage its own
  directory safely.

## Completion Criteria

The adapter work is ready when:

- `actionc-run` compiles and packages without subprocess compiler tools;
- Atari800 and Altirra share the same ATR boot artifact;
- cartridge support works in both adapters;
- adapters only build commands and can be tested without installed emulators;
- discovery is predictable and platform-aware;
- user emulator configuration is not overwritten;
- temporary artifacts have deterministic lifetime;
- the existing shell helper no longer contains a competing launch pipeline.
