# Modules tutorial

## Contents

- [What modules provide](#what-modules-provide)
- [Named modules and the legacy `MODULE` directive](#named-modules-and-the-legacy-module-directive)
- [Build a one-file module](#build-a-one-file-module)
- [Split a program into two files](#split-a-program-into-two-files)
- [Initialize module state](#initialize-module-state)
- [Control the imported names](#control-the-imported-names)
- [Use the embedded Atari modules](#use-the-embedded-atari-modules)
- [Choose the runtime](#choose-the-runtime)
- [Add shared module directories](#add-shared-module-directories)
- [Understand code inclusion](#understand-code-inclusion)
- [Troubleshoot common errors](#troubleshoot-common-errors)
- [Next examples and reference](#next-examples-and-reference)

## What modules provide

A named module gives a source file its own namespace and lets it expose a
deliberate public interface. Modules are useful for:

- splitting a program across source files;
- keeping implementation state private;
- identifying the Atari chip, OS, or library that owns a name;
- sharing project code without copying it with `INCLUDE`.

Named modules work in the standard `actionc` build and in all three user-facing
compiler modes. A source file contains at most one named module:

```action
MODULE NAME

; Imports and declarations go here.

ENDMODULE
```

Use uppercase module names in Action! source. For user modules, qualified
module names map to lowercase paths: `MODULE GAME.PLAYER` is found in
`game/player.act`.

## Named modules and the legacy `MODULE` directive

The original Action! cartridge supports a bare `MODULE` directive:

```action
MODULE

BYTE first

MODULE

BYTE second
```

This historical form starts another top-level section in the same compilation.
It does not create a new scope: declarations on both sides of the directive
remain in the shared global namespace. It has no module name, no closing
`ENDMODULE`, no private interface, and no `USE` imports.

An `actionc` named module is a separate, additive language feature:

```action
MODULE GAME.PLAYER

USE ATARI.GTIA

PUBLIC PROC Draw()
RETURN

ENDMODULE
```

| | Legacy cartridge form | Named `actionc` form |
|---|---|---|
| Opening syntax | bare `MODULE` | `MODULE NAME` |
| End | next bare `MODULE` or end of file | required `ENDMODULE` |
| Names | shared legacy namespace | separate module namespace |
| Imports | none | `USE`, `USE ... AS`, `USE ALL FROM` |
| Visibility | no public/private interface | private by default; `PUBLIC` exports |
| File lookup | no name-to-file mapping | `GAME.PLAYER` maps to `game/player.act` |

The distinction is line-sensitive. A module name on the same physical line as
`MODULE` selects the named form and requires `ENDMODULE`. If `MODULE` ends the
line, it remains the legacy directive, even when the next declaration begins
with a user-defined type name:

```action
MODULE ; Parsed as a legacy module because no name follows on this line.
PLAYER_STATE current
```

Existing Action! source continues to compile as before. It does not need to be
wrapped in a named module, and old bare `MODULE` boundaries, `INCLUDE` files,
and traditional unqualified system-library names remain supported. Named
modules are used only when source explicitly opts into `MODULE NAME`.

## Build a one-file module

The complete example is
[`samples/modules/hello.act`](../../samples/modules/hello.act):

```action
MODULE HELLO

USE SYS
USE ATARI.OS

PROC Main()
  SYS.Close(0)
  SYS.Open(0,"E:",12,0)

  SYS.PrintE("Hello from a module!")
  OS.CH=OS.NO_KEY
  SYS.PrintE("Press any key to continue.")
  DO
  UNTIL OS.CH#OS.NO_KEY
  OD
RETURN

ENDMODULE
```

Compile it without an Action! cartridge dependency:

```sh
actionc --runtime standalone --output build/hello.xex hello.act
```

Or compile and launch it in a configured emulator:

```sh
actionc-run --runtime standalone hello.act
```

`actionc` makes the `SYS` interface available by default through traditional
unqualified names, so existing code can call `PrintE` without an import. The
explicit `USE SYS` above creates the qualifier needed for `SYS.PrintE`; it does
not add every system routine to the executable. With the standalone runtime,
the call links `PrintE` and the runtime routines on which it depends.
Closing IOCB 0 and reopening `E:` ensures that the editor device is available
when the XEX is booted directly in an emulator.
`USE ATARI.OS` provides the OS keyboard latch and `NO_KEY` constant. Clearing
`OS.CH` discards any earlier key, and the loop ends as soon as the OS records a
new keypress; Return is not required.

The name `Main` is a convention, not special syntax. As in Action!, the last
code-emitting `PROC` in the root source file is the program entry point. Keep
the procedure that should start the program last in that file. A trailing
`FUNC` does not replace it. The entry procedure does not need to be `PUBLIC`:
the compiler writes its address to the executable's `RUNAD` vector rather than
calling it through another module. Use `PUBLIC` only when code in another
module must refer to a declaration.

## Split a program into two files

The complete example in
[`samples/modules/project`](../../samples/modules/project) has this layout:

```text
project/
  main.act
  demo/
    color.act
```

The reusable file `demo/color.act` declares the module whose name matches that
path:

```action
MODULE DEMO.COLOR

USE ATARI.GTIA
USE ATARI.OS

BYTE phase

PUBLIC PROC Advance()
  phase==+1
  GTIA.COLPF2=phase+OS.RTCLOK_LO
RETURN

ENDMODULE
```

Declarations are private unless they have `PUBLIC`. Here callers can use
`Advance`, but they cannot access the module's `phase` variable.

The root file imports the module and gives it a short, descriptive alias:

```action
MODULE GALLERY

USE ATARI.ANTIC
USE ATARI.OS
USE DEMO.COLOR AS PALETTE

PROC Main()
  OS.CH=OS.NO_KEY

  DO
    ANTIC.WSYNC=0
    PALETTE.Advance()
  UNTIL OS.CH#OS.NO_KEY
  OD
RETURN

ENDMODULE
```

Compile from any working directory by naming the root source file:

```sh
actionc --runtime standalone samples/modules/project/main.act
```

The directory containing `main.act` becomes the project module root, so no
extra module-path option is needed. `USE DEMO.COLOR` makes `actionc` look for
`demo/color.act` below that root and verifies that it declares
`MODULE DEMO.COLOR`.

## Initialize module state

Named modules do not have automatic initializers. `USE` only makes declarations
available; it does not execute the imported module, and the order of `USE`
clauses is not a runtime initialization order. Executable top-level statements
are not allowed in imported modules.

When a module needs runtime setup, expose an initialization procedure:

```action
MODULE GAME.SCORE

BYTE value

PUBLIC PROC Init()
  value=0
RETURN

ENDMODULE
```

Call it explicitly from the root entry procedure:

```action
MODULE GAME

USE GAME.SCORE

PROC Start()
  SCORE.Init()
RETURN

ENDMODULE
```

This keeps initialization order visible in the application. `Start` must be
the last code-emitting `PROC` in the root source if it is intended to be the
program entry point.

## Control the imported names

Plain `USE` imports a module under a qualifier derived from the final component
of its name:

```action
USE DEMO.COLOR

; The default alias is the last component of the module name.
COLOR.Advance()
```

Use `AS` when the default alias is unclear or conflicts with another name:

```action
USE DEMO.COLOR AS PALETTE

PALETTE.Advance()
```

`USE ALL FROM` makes every public member available without a qualifier:

```action
USE ALL FROM SYS

PrintE("No qualifier")
```

`USE ALL FROM` cannot use `AS`, never exposes private declarations, and does
not re-export the imported names from the current module. Importing two
different declarations under the same unqualified name is an error. Prefer
qualified imports for larger programs because ownership remains visible at
each use.

## Use the embedded Atari modules

`actionc` embeds the standard modules, so users do not need to install or copy
their source files:

| Module | Purpose |
|---|---|
| `SYS` | Action! system-library interface |
| `ATARI.OS` | OS variables, shadow registers, constants, and entry points |
| `ATARI.ANTIC` | ANTIC hardware registers and constants |
| `ATARI.GTIA` | GTIA hardware registers and constants |
| `ATARI.POKEY` | POKEY hardware registers and constants |
| `ATARI.PIA` | PIA hardware registers and constants |
| `MATH` | Portable pointer-oriented native REAL operations |
| `ATARI.REAL` | Atari OS FPP implementation provider used by `MATH` |

Qualified hardware code shows whether an access is direct or mediated by an OS
shadow:

```action
USE ATARI.ANTIC
USE ATARI.OS

; Immediate write to the ANTIC hardware register.
ANTIC.DMACTL=0

; OS shadow copied to ANTIC during vertical blank.
OS.SDMCTL=ANTIC.NORMAL_PLAYFIELD_DMA
```

Use the chip module for direct hardware access. Use `ATARI.OS` for OS variables
and shadow registers when the operating system should own the update.

`MATH` is layered on the native `REAL` type, which requires the modern profile.
The `optimized` and `mir6502` modes select that profile. REAL text conversion
and I/O are qualified `SYS` operations. Start with the
[native REAL tutorial](REAL.md) for their use. The complete library contract
and Atari FPP workspace restrictions are documented in the [modules and
runtime reference](../Action_2027/MODULES_AND_RUNTIME_USAGE.md#native-real-library).

## Choose the runtime

Module imports and runtime selection solve different problems:

- the system-library interface is available by default under traditional
  unqualified names;
- `USE SYS` additionally creates the qualifier used by names such as
  `SYS.PrintE`;
- `--runtime cart` resolves those calls through the resident Action! cartridge;
- `--runtime standalone` links the required implementations into the program.

The cartridge runtime is the compatibility default:

```sh
actionc program.act
actionc-run program.act
```

Select a standalone executable explicitly when no Action! cartridge should be
required:

```sh
actionc --runtime standalone program.act
actionc-run --runtime standalone program.act
```

For `actionc-run`, `--no-cart` is a convenience spelling of
`--runtime standalone`:

```sh
actionc-run --no-cart program.act
```

When a standalone build includes GPL runtime code, the compiler prints a
warning naming the selected public `SYS` procedures and compiler helpers. Their
required runtime dependencies are covered by the same warning.

The selected runtime does not change module syntax. The same source can
normally be built against either provider.

## Add shared module directories

For modules outside the root project, add one or more search roots:

```sh
actionc \
  --module-path ../shared \
  --module-path ./generated \
  game.act
```

For each `USE`, lookup proceeds through:

1. modules already loaded for the build;
2. compiler-embedded modules;
3. the root source file's directory;
4. each `--module-path` directory from left to right.

`--module-path` is repeatable. It names a module root, not a particular source
file. For example, `USE TOOLS.TEXT` searches for `tools/text.act` below each
root.

Host files cannot replace compiler-reserved modules below `SYS` or `ATARI`.
Use lowercase path components for host modules even on a case-insensitive file
system.

## Understand code inclusion

The compiler follows the transitive `USE` graph from the root module. It does
not load unrelated files merely because they are present in a module path.

Loaded user modules are currently emitted as whole units: their private and
public storage and all routine bodies are included, even if only one public
routine is called. `PUBLIC` controls visibility, not linking. Put optional or
large features in separate modules when output size matters.

Standalone compiler-owned runtime code is different: referenced `SYS` routines
and compiler helpers are selectively linked with their required dependencies.
Merely importing `SYS` does not pull the entire runtime into the program.

## Troubleshoot common errors

### `cannot find module`

Check that:

- the source is below the root file's directory or a `--module-path` root;
- `MODULE GAME.PLAYER` is stored as `game/player.act`;
- host path components use lowercase spelling;
- the file's declared module name exactly matches the name in `USE`.

### A member is private or unknown

Add `PUBLIC` to declarations that form the module interface. Do not expose
implementation state only to work around a qualified-name error.

### An imported name collides

Replace `USE ALL FROM` with qualified `USE`, or add a distinct alias with
`USE ... AS ...`. The compiler deliberately rejects ambiguous imports.

### The executable is larger than expected

Remember that a loaded user module is emitted as a whole. Split independent or
optional features into separate source modules so unreachable module files are
not loaded.

### The wrong procedure starts

Move the intended entry procedure so it is the last code-emitting `PROC` in the
root source file. Renaming it to `Main` alone does not select it.

## Next examples and reference

- [`samples/modules`](../../samples/modules/README.md) contains runnable hardware,
  `SYS`, project, and native-`REAL` examples.
- [Modules and runtime usage](../Action_2027/MODULES_AND_RUNTIME_USAGE.md) is the
  detailed behavior and inclusion reference.
- [Command-line usage](../../USAGE.md) documents every compiler and runner option.
- [Syntax extensions](../SYNTAX_EXTENSIONS.md) covers the other language features
  accepted by `actionc`.
