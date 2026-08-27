# Action 2027 Modules and Runtime Usage

Implementation status: enabled in standard builds. Named modules, host module
lookup, and the embedded `SYS` and `ATARI.*` modules are part of the supported
`actionc` language and CLI surface.

For a task-oriented introduction, start with the
[modules tutorial](../tutorials/MODULES.md).

Action 2027 named modules make hardware interfaces and reusable project code
explicit while preserving a single-file compiler installation. `SYS`,
`ATARI.*`, runtime bindings, and the original GPL runtime sources are embedded
in `actionc`; they are not extracted beside the executable.

## Named modules and `USE`

One loadable source file defines one named module. Uppercase module names are
the preferred source style. Their automatic host paths are lowercase, so
`MODULE GAME.PLAYER` is stored as `game/player.act`.

A qualified `USE` keeps register ownership visible:

```action
MODULE RAINBOW

USE ATARI.ANTIC
USE ATARI.GTIA

PROC Main()
  ANTIC.WSYNC=0
  GTIA.COLBAK=ANTIC.VCOUNT
RETURN

ENDMODULE
```

`USE SYS` creates the `SYS` module alias. `USE ALL FROM SYS` introduces all
public members without a qualifier:

```action
MODULE CLEAR_BUFFER

USE SYS
USE ALL FROM SYS

BYTE ARRAY buffer(256)=$6000

PROC Main()
  SYS.Zero(buffer,256)
  SetBlock(buffer,256,$55)
RETURN

ENDMODULE
```

Declarations are private unless marked `PUBLIC`. Names introduced by `USE ALL
FROM` are not re-exported, and collisions are errors rather than source-order
dependent.

Runnable versions are in `samples/modules`: `rainbow.act` uses
qualified hardware modules, while `sys-memory-qualified.act` and
`sys-memory-open.act` show the two `SYS` forms. The `project` directory
is a complete multi-file example with a public project-module procedure.

## Native REAL library

Native REAL support is divided by responsibility. `MATH` is the portable
numerical interface, `SYS` owns conversion and I/O, and `ATARI.REAL` is the
Atari OS FPP implementation provider behind `MATH`. Application code normally
imports `MATH` and `SYS`, without naming the target provider:

```action
MODULE CALCULATOR

USE SYS
USE MATH

REAL input, result
STRING text(20)

PROC Main()
  input=2
  MATH.Exp10(@input,@result)
  SYS.StrR(@result,text)
  SYS.PrintRE(@result)
RETURN

ENDMODULE
```

The procedures are pointer-oriented because by-value REAL parameters and
results remain deferred:

- `SYS` conversion: `StrR`, `ValR`;
- `SYS` output: `PrintR`, `PrintRE`, `PrintRD`, `PrintRDE`;
- `SYS` input: `InputR`, `InputRD`;
- `MATH` transcendental operations: `Exp`, `Exp10`, `Ln`, `Log10`, `Power`;
- `MATH` elementary operations: `Abs`, `Sgn`, `Floor`, `Rnd`, `Sqrt`/`Sqr`, `Sin`, `Cos`,
  `Tan`, and `Atan`/`Atn`;
- `MATH` constants: `Pi`, `HalfPi`, `QuarterPi`, `TwoPi`, `E`, `Ln2`, `Ln10`,
  `Log2E`, `Log10E`, `Sqrt2`, `InvSqrt2`, `DegToRad`, and `RadToDeg`;

`StrR` destinations should reserve at least 20 bytes. `ValR` delegates runtime
text validation to Atari AFP and therefore exposes the OS routine's invalid
input result. Trigonometric arguments and results use radians. `Sqrt` returns
zero for a non-positive input, while `Tan` exposes the underlying division
behavior at its poles. The trig routines use a bounded nearest-period range
reduction with a split `2*pi` constant, followed by a convergent sine series in
ordinary Action source; `Sin`, `Cos`, and `Tan` return zero as a documented
total-loss result when the absolute argument is at least `1E6`. They do not
require the Atari BASIC cartridge. Both runtime modes require a compatible
Atari OS for this API.
`Floor` implements Atari BASIC `INT` semantics (rounding toward negative
infinity); it uses a different name because `INT` is an Action type keyword.
All routines share the Atari FPP zero-page workspace, including FR0, CIX, and
INBUFF, and are neither reentrant nor safe against interrupts that also use
FPP. `ATARI.REAL` is ordinary provider source and remains independent of the
compiler's native REAL type implementation.

The REAL members of `SYS` are modern, qualified-only extensions. This avoids
adding `StrR`, `PrintR`, and similar names to the compatibility prelude, so
existing Action! sources may continue to declare those names. Write `USE SYS`
when calling them as `SYS.StrR`, `SYS.PrintRE`, and so on.

See `samples/modules/native-real-library.act` for the basic API and
`samples/graphics/sine-surface.act` for a translated graphics program using
`Sqr` and `Sin`.

## Module lookup

The root source directory is the project module root. Add ordered project roots
with repeatable options:

```sh
actionc --module-path ../shared --module-path ./generated game.act
```

For a `USE` clause, the compiler checks already loaded modules, the embedded VFS,
the project root, and then each explicit module path from left to right. It does
not read an environment module path or search beside the compiler executable.
Host files cannot shadow the reserved `SYS` or `ATARI` roots. Host module path
components must use lowercase spelling even on a case-insensitive filesystem.

## User-module inclusion

The loader follows the root module's transitive `USE` graph. An unrelated host
module that is not reachable through that graph is not loaded. Once a user
module is loaded, however, the compiler currently emits the module as a whole:
all of its storage and routine bodies are included even when some routines are
not called.

`PUBLIC`, qualified `USE`, and `USE ALL FROM` control name visibility; they do
not control code inclusion. This whole-module rule applies to project modules
from the source tree and `--module-path`. Keep optional or expensive code in
smaller modules when binary size matters.

Selective linking is currently reserved for compiler-owned runtime code. With
`--runtime standalone`, `actionc` includes the transitive implementation
closure of referenced `SYS` routines and compiler helpers. Merely writing
`USE SYS` adds no runtime implementation code. Maps and listings show which
user routines and runtime implementations were emitted.

User-module selective linking is a possible future optimization, not a source
contract. Programs must not depend on an unused routine being emitted or on
the current addresses or relative order of module members.

## Runtime choice

The backend and runtime are independent:

```sh
actionc --profile modern --backend classic --runtime standalone program.act
actionc --profile modern --backend mir6502 --runtime cart program.act
```

`--runtime cart` is the compatibility default. It uses the Action! cartridge's
resident helper and library entry points. `--runtime standalone` selectively
links implemented routines from the embedded GPL source and does not fall back
to a cartridge address. Both classic and MIR6502 honor the selection.

The standalone runtime covers the complete audited `SYS` interface. It also
provides the Action ABI helpers `SArgs`, shifts, multiplication, division, and
remainder when generated code needs them. `USE SYS` without using a member adds
no code. A missing runtime dependency, an ABI mismatch, or an absolute
helper-slot override is a compile-time error.

The public `SYS` groups are:

- memory: `Zero`, `SetBlock`, `MoveBlock`;
- strings: `SCompare`, `SCopy`, `SCopyS`, `SAssign`, `StrB`, `StrC`, `StrI`;
- memory, devices, and control: `Rand`, `Sound`, `SndRst`, `Paddle`, `PTrig`,
  `Stick`, `STrig`, `Peek`, `PeekC`, `Poke`, `PokeC`, `Error`, `Break`;
- graphics: `Graphics`, `Position`, `DrawTo`, `Locate`, `Plot`, `SetColor`,
  `Fill`;
- character and string I/O: `Put`, `PutE`, `PutD`, `PutDE`, `Print`, `PrintE`,
  `PrintD`, `PrintDE`, `PrintF`, `PrintH`, `GetD`;
- input and file/device I/O: `InputS`, `InputSD`, `InputMD`, `InputD`, `Open`,
  `Close`, `XIO`, `Note`, `Point`;
- numeric output: the `PrintB*`, `PrintC*`, and `PrintI*` families;
- numeric input and conversion: `InputB`, `InputBD`, `InputC`, `InputCD`,
  `InputI`, `InputID`, `ValB`, `ValC`, `ValI`.

All of these names are available with both `--runtime cart` and `--runtime
standalone`, under both active backends. Their authoritative signatures are in
[`embedded/modules/sys.act`](../../embedded/modules/sys.act). Unqualified
traditional spellings are compatibility aliases for the same symbols.

Representative standalone arithmetic, memory, string, graphics-state, and CIO
output programs are executed through `actionc-vm` by the ignored compatibility
test:

```sh
cargo test --test compatibility standalone_library_runtime_check -- --ignored
```

An explicit local symbolic helper override remains available:

```action
MODULE CUSTOM_HELPER

; RTS is sufficient here only because Four does not inspect copied arguments.
; A real replacement must implement the complete Action SArgs ABI.
PROC LocalSArgs=*() [$60]
SET $4EE=LocalSArgs

PROC Four(BYTE a,b,c,d)
RETURN

PROC Main()
  Four(1,2,3,4)
RETURN

ENDMODULE
```

The local routine wins over the selected runtime provider. In standalone mode,
an override such as `SET $4EE=$A0F5` is rejected because it would restore a
hidden cartridge dependency. The complete compact example is
`samples/modules/local-runtime-override.act`.

## Running with or without the cartridge

`actionc-run` treats compilation and mounted media as one choice:

```sh
# Cart runtime and bundled cartridge (default)
actionc-run program.act

# Cart runtime and a selected cartridge image
actionc-run --cart roms/action.rom program.act

# Standalone runtime and no cartridge
actionc-run --runtime standalone program.act

# Equivalent runner convenience
actionc-run --no-cart program.act
```

`--runtime` is the canonical selector shared with `actionc` and
`actionc-emit`. In the runner, `--no-cart` is a convenience form of
`--runtime standalone`, while `--cart PATH` implies `--runtime cart`.
Contradictory combinations are rejected. Standalone runs store the program
directly as `PROGRAM.AR0`. Cart runs add the small `BOOT.AR0` cartridge-bank
bootstrap and store the program as `PROGRAM.AR1`.

## Inspecting runtime decisions

`actionc-emit --emit-map` begins with the selected runtime and prints every
binding decision as a `runtime-binding` line. Standalone entries identify their
embedded source unit, GPL provenance, and inclusion reason; a local override
also records the suppressed default. Diagnostics use stable names such as
`<runtime:SYSLIB.ACT>`. MADS listings carry the binding information as leading
`; Runtime:` and `; Runtime binding:` comments.

`actionc --version` reports a `vfs=` digest. Identical binaries therefore expose
the exact embedded module, binding, and runtime-source image used for a build.

## Runtime license and corresponding source

Standalone programs that include selected Action runtime routines contain code
derived from GPL-3.0-or-later sources. The repository keeps the exact embedded
inputs under `corpora/action-runtime/extracted`, with provenance in
`corpora/action-runtime/README.md` and `roms/ACTION-ROM-NOTICE.md`. Release
archives include those runtime source inputs under `licenses/runtime-source/`
and their provenance as `licenses/ACTION-RUNTIME-NOTICE.md`. Distributors must
preserve the GPL notice and provide the corresponding source as required by the
license.

When a standalone build includes GPL runtime code, `actionc` and `actionc-run`
print a warning naming the directly selected public `SYS` procedures and
compiler helpers. Their required runtime dependencies are covered by the same
warning even though they are not listed individually.
