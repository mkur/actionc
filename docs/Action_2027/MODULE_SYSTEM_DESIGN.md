# Action 2027 Module System Design

Design status: implemented and enabled in standard builds. This note defines
the supported Action 2027 module language contract. Two companion notes own
the mechanisms deliberately kept out of the language contract:

- [`MODULE_LOADER_AND_VFS.md`](./MODULE_LOADER_AND_VFS.md) defines file
  discovery, the embedded virtual filesystem, source identity, and diagnostics;
- [`RUNTIME_INTERFACE_AND_STANDALONE.md`](./RUNTIME_INTERFACE_AND_STANDALONE.md)
  defines `SYS` implementation bindings, `--runtime cart`,
  `--runtime standalone`, and selective runtime inclusion.

The review that led to this split is preserved in
[`MODULE_SYSTEM_DESIGN_REVIEW.md`](./MODULE_SYSTEM_DESIGN_REVIEW.md).

## Motivation

Action! already has a `MODULE` token, but its historical role is primarily to
separate declaration regions inside one compilation. In `actionc` today,
parsed modules still share one global symbol scope, `INCLUDE` expands source
text at the include site, and resident-library symbols are injected into a
global builtin scope.

Action 2027 needs modules to provide stable namespaces, deliberate interfaces,
and reusable target facilities without giving up direct access to the machine.
Two initial uses drive the design:

1. Atari hardware and OS state should be available through typed modules such
   as `ATARI.ANTIC`, `ATARI.GTIA`, and `ATARI.OS`.
2. Resident and standalone system-library implementations should appear
   through one stable `SYS` interface.

The first implementation is whole-program compilation. Modules introduce
namespaces and interfaces, not object files, a linker, runtime dispatch, or
implicit initialization.

## Language Principles

- A named module is a compile-time namespace and adds no runtime storage.
- A source file is either entirely legacy Action! or contains exactly one
  named module. The forms cannot be mixed.
- Module members are private unless their declaration is `PUBLIC`.
- `USE M` creates a qualified module alias; `USE ALL FROM M` introduces every
  public member without a qualifier.
- Equal aliases of the same stable symbol coalesce; equal names denoting
  different symbols conflict.
- Hardware uses ordinary typed storage plus `VOLATILE`; modules do not add
  special register syntax.
- `DEFINE` remains textual and file-local. `CONST`, types, storage, and
  routines form semantic module interfaces.
- SemIR owns module identity, visibility, qualification, and symbol binding.
- NIR and MIR6502 receive resolved stable IDs and never recover module meaning
  from source strings.
- The public `SYS` routine identity is independent of whether its
  implementation is supplied by the cartridge or the standalone runtime.

## Named Modules

A named module is one explicit file-level block:

```action
MODULE DEMO.PLAYER
  ; USE clauses and declarations
ENDMODULE
```

The module name must begin on the same physical source line as `MODULE`. Seeing
that same-line identifier commits parsing to the named form. A missing
`ENDMODULE` is then a hard syntax error; the parser never falls back to legacy
interpretation.

This line-sensitive commitment preserves a legacy declaration boundary
followed by a user-defined type:

```action
MODULE
PLAYER_STATE current
```

Here `MODULE` ends its line, so it remains the historical bare form and
`PLAYER_STATE` begins the next declaration.

A named-module file may contain leading comments and blank lines, but no
legacy declarations before or after its `MODULE ... ENDMODULE` block. It may
not contain a second named module. Tests involving several modules use several
in-memory or temporary source files through the normal loader.

Module names are case-insensitive dotted identifiers. Their canonical display
spelling comes from the declaration. `DEMO.PLAYER`, `Demo.Player`, and
`demo.player` identify the same module. The source convention is to spell
module paths and aliases in uppercase, including `SYS`, `ATARI.ANTIC`, and
`ATARI.GTIA`.

## USE clauses

`USE` clauses occur inside a named module, after its header and before ordinary
declarations:

```action
MODULE DEMO
  USE ATARI.ANTIC
  USE ATARI.GTIA AS VIDEO
  USE SYS

  ; declarations follow
ENDMODULE
```

The first version does not allow `USE` in an implicit legacy unit, inside a
routine, or after ordinary module declarations. This keeps the dependency
graph independent of declaration and control-flow order.

Without `AS`, the final path component is the alias. `USE ATARI.ANTIC` is
referenced as `ANTIC`; for the one-component path `USE SYS`, the alias is
`SYS`. An explicit alias may repeat that spelling; it is harmless and receives
no style warning.

A `USE` clause creates a module alias but does not copy its members into the
current scope:

```action
ANTIC.WSYNC=0
VIDEO.COLBAK=$00
SYS.PrintE("Ready")
```

### Using all public symbols

`USE ALL FROM` introduces every public member directly:

```action
USE ALL FROM SYS

Graphics(0)
PrintE("Ready")
```

`USE SYS` creates the alias `SYS`, while `USE ALL FROM SYS` creates unqualified
member aliases and no module alias. Code that needs both forms writes both
clauses. The earlier `IMPORT SYS.*` spelling is not accepted.

`USE ALL FROM` cannot use `AS`, cannot expose private members, and does not
re-export names. Each introduced name aliases the defining declaration's stable
`SymbolId`; it is not a copied declaration.

Collision rules are independent of source order:

- a public member conflicting with a declaration or module alias is an error;
- equal names from two `USE ALL FROM` clauses are an error when they denote
  different stable symbols;
- two bindings with the same spelling and the same `SymbolId` coalesce
  silently, regardless of which `USE` clause or compatibility-prelude path
  produced them;
- routine-local declarations may shadow a member introduced by `USE ALL FROM`
  under the existing local-over-module lookup rule.

There is no first-clause or last-clause winner.

## Public Declarations

Declarations are private by default:

```action
MODULE EXAMPLE.COUNTER
  BYTE value

  PUBLIC PROC Reset()
    value=0
  RETURN

  PUBLIC BYTE FUNC Current()
  RETURN(value)
ENDMODULE
```

Using modules can call `COUNTER.Reset()` and `COUNTER.Current()`, but
`COUNTER.value` is a private-member error distinct from an unknown-member
error.

`PUBLIC` can qualify:

- `CONST` declarations;
- scalar and array storage declarations;
- `TYPE` and `RECORD` declarations;
- `PROC` and `FUNC` declarations.

Qualifiers apply to the complete declaration. For example, every entry here is
both public and volatile:

```action
PUBLIC VOLATILE BYTE DMACTL=$D400,
                     WSYNC=$D40A,
                     VCOUNT=$D40B
```

There is no per-name visibility within one grouped declaration. A module
splits the declaration when only some entries belong in its public interface.
The compiler does not guess intent or warn about deliberately grouped public
state.

`EXPORT` is not used. It remains available for a possible future operation
that re-exports a member or module brought into scope with `USE`.

### `DEFINE` is not an exported symbol

Action! `DEFINE` performs textual substitution and can represent syntax rather
than a typed value:

```action
DEFINE STRING="CHAR ARRAY"
```

It therefore remains local to the physical source preprocessing context and
cannot be declared `PUBLIC` in the first module version. Cross-module compile-time
values use `PUBLIC CONST`; exported type vocabulary requires a real `TYPE`
facility rather than cross-module token substitution. `PUBLIC DEFINE` receives
an explicit unsupported diagnostic.

Legacy source and textual includes retain existing `DEFINE` behavior.

## Name Resolution and Qualified Use

Each named module has a stable `ModuleId` and a separate module scope. Routine
scopes have their defining module scope as parent. There is no shared global
scope among named modules.

Lookup inside a named module proceeds in this order:

1. routine-local declarations;
2. declarations and aliases introduced by `USE ALL FROM` in the current
   module;
3. module aliases used for qualified lookup;
4. the compatibility prelude, when enabled.

`ANTIC.WSYNC` first resolves `ANTIC` as a module alias, then resolves only
public `WSYNC` within the target module. Private and absent members receive
different diagnostics.

Qualified names work wherever the corresponding unqualified semantic object is
legal, including:

- expression values and lvalues;
- callable targets and routine addresses;
- types and `CONST` expressions;
- array sizes and fixed addresses;
- `SET` values;
- static initializers;
- Action-object references in inline `ASM` and legacy machine blocks.

Qualification does not cause a `DEFINE` token expansion in the using module.

The dot also appears in record-field expressions. SemIR distinguishes the two
forms by resolving the left side:

- a module alias selects a public module member;
- a record value selects a field;
- anything else receives the normal field/type diagnostic.

Parser shape alone does not decide this semantic question. A public record type
from a used module behaves like a local type, and field access on its values
remains record access rather than module lookup.

## `INCLUDE` Inside and Outside Modules

`USE` loads a namespace. `INCLUDE` remains textual insertion.

Inside a named module, an included fragment contributes declarations to that
module's scope. Those declarations are private by default, and `PUBLIC` is
allowed. The fragment cannot declare another named module and cannot escape
into a second namespace.

```action
MODULE GAME
  INCLUDE "game-types.act"
  ; included declarations now belong to GAME
ENDMODULE
```

Outside a named module, `INCLUDE` retains full legacy behavior, including
textual `DEFINE` substitution and historical bare `MODULE` boundaries. A file
cannot use `INCLUDE` to mix legacy declarations and a named module.

Physical include lookup is specified in
[`MODULE_LOADER_AND_VFS.md`](./MODULE_LOADER_AND_VFS.md).

## Legacy `MODULE`

A file without a named module is an implicit legacy compilation unit:

```action
BYTE first
MODULE
BYTE second
```

Bare `MODULE` continues to act as its historical declaration boundary, and all
such regions share the legacy global namespace exactly as today. It is
recognized when `MODULE` ends its physical source line without a module-name
identifier. Legacy source does not acquire private visibility, `USE` clauses,
or qualified module names.

Mixing a named block with legacy declarations in one physical source file is a
hard error rather than an attempt to infer ownership.

## Initialization and Entry Points

Named library modules may contain declarations and routines but no executable
top-level statements. `USE` clauses therefore have no hidden initialization
order or runtime side effects.

Fixed-address storage, static data, `CONST`, `TYPE`, and `RECORD` declarations
require no runtime initializer and are allowed. Action! starts at the last
source `PROC` in the root program that emits code; `Main` has no special
meaning. Semantic lowering records that routine's stable identity as entry
metadata carried through NIR and MIR6502. Runtime routines appended during
selective linking therefore cannot become the executable entry merely because
they are laid out after application code.

A future explicit `PROGRAM`, `ENTRY`, or module-initialization construct is a
separate design. `USE`-clause order must never become implicit execution order.

## Hardware Modules

Hardware modules are ordinary Action source modules whose public storage is
fixed at machine addresses. `VOLATILE` preserves the access ordering and
observability contract already implemented by the compiler.

```action
MODULE ATARI.ANTIC
  PUBLIC VOLATILE BYTE DMACTL=$D400,
                       HSCROL=$D404,
                       VSCROL=$D405,
                       WSYNC=$D40A,
                       VCOUNT=$D40B,
                       NMIEN=$D40E

  ; DLISTL/DLISTH form a low-byte-first little-endian pair.
  PUBLIC VOLATILE CARD DLIST=$D402

  PUBLIC CONST BYTE NORMAL_PLAYFIELD_DMA=$22
ENDMODULE
```

A `VOLATILE CARD` remains two ordered byte accesses and is not atomic. Declaring
a register pair as `CARD` is valid only when the hardware exposes a
low-byte-first pair at consecutive addresses; modules must not generalize that
representation to unrelated register pairs.

OS shadows remain separate from direct hardware registers:

```action
MODULE ATARI.OS
  PUBLIC VOLATILE CARD SDLST=$0230
  PUBLIC VOLATILE BYTE SDMCTL=$022F,
                       GPRIOR=$026F,
                       COLOR4=$02C8,
                       CH=$02FC
ENDMODULE
```

That distinction stays visible at each use:

```action
MODULE PLASMA
  USE ATARI.ANTIC
  USE ATARI.GTIA
  USE ATARI.OS

  PROC Install(CARD displayList)
    OS.SDLST=displayList
    ANTIC.DLIST=displayList
    OS.GPRIOR=GTIA.MODE_9
    GTIA.PRIOR=GTIA.MODE_9
  RETURN
ENDMODULE
```

Higher-level modules such as `ATARI.VIDEO` may encapsulate shadow/direct
coordination while keeping raw modules available. The first version does not
add `READONLY` or `WRITEONLY`; those qualifiers are independent of namespaces.

## Standard Library Namespace

The traditional resident-library surface becomes one public `SYS` namespace:

```action
MODULE HELLO
  USE SYS

  PROC Main()
    SYS.Graphics(0)
    SYS.PrintE("Hello from Action 2027")
  RETURN
ENDMODULE
```

`USE ALL FROM` preserves compact traditional spelling:

```action
USE ALL FROM SYS

Graphics(0)
PrintE("Ready")
```

Representative public areas include text and file I/O, graphics, memory,
strings, sound, input, and resident state. They remain members of `SYS` in the
first version rather than prematurely fixing nested namespaces such as
`SYS.GRAPHICS`.

`SYS` contains runtime-neutral public identities and signatures. A cartridge
address such as `$A46C` is an implementation binding, not an independently
exported numeric constant. Taking `@SYS.PrintE` or using the routine as a
qualified `SET` value resolves to the implementation selected for the current
runtime. A missing binding is a compile-time error.

The source representation and runtime selection rules are defined in
[`RUNTIME_INTERFACE_AND_STANDALONE.md`](./RUNTIME_INTERFACE_AND_STANDALONE.md).

### Compatibility prelude

Existing programs continue to resolve traditional unqualified resident names:

```action
PROC Main()
  Graphics(0)
  PrintE("Hello")
RETURN
```

Compatibility modes receive an implicit legacy prelude whose aliases refer to
the same `SymbolId` values as `SYS`. Consequently, combining the prelude with
`USE ALL FROM SYS` coalesces equal identities rather than producing duplicates.

Action 2027 code should prefer explicit `USE` clauses. Disabling the prelude is a
future user-facing option; this design specifies the semantic toggle but does
not reserve a command-line spelling before that path is implemented.

## Compiler Boundary

### AST and semantic model

The AST records structured module paths, `USE` clauses, visibility qualifiers,
file kind, and source spans. It may retain dotted names as syntax.

The semantic model gains identities equivalent to:

```text
ModuleId
ModulePath
ModuleScope
ModuleUse { target: ModuleId, alias, all }
ModulePublicSymbol { symbol: SymbolId }
```

Public-member and `USE ALL FROM` tables contain stable IDs, not copied
declarations. SemIR resolves qualification, visibility, callable facts,
storage, constants, types, routine addresses, static initializers, `SET`, and
assembler object references before NIR lowering.

### NIR and MIR6502

Verifier-clean NIR contains no module-use operations, unresolved qualified names,
or visibility decisions. It receives stable routine, storage, type-layout, and
callable identities. Module paths may remain as non-executable display/debug
metadata only.

MIR6502 never consults SemIR or qualified source strings to recover module
meaning. Runtime-helper choice remains MIR6502-owned and is specified in the
runtime companion note.

### Emission and listings

Readable listings and maps show defining qualified names:

```text
ATARI.ANTIC.WSYNC
SYS.PrintE
GAME.PLAYER.Update
```

MADS labels use deterministic sanitized qualified names. If two canonical
qualified keys sanitize to the same spelling, the suffix derives from a stable
hash of the complete canonical key and symbol kind—not from allocation order or
a transient numeric `SymbolId`. Module aliases never rename the defining
symbol's emitted label.

`USE` clauses and public fixed-address storage emit no bytes by themselves.
Ordinary module data and routines use existing layout and dead-code policies.

## Implementation Slices

### Slice 1: File kind and syntax

- Track physical line boundaries needed by `MODULE` commitment.
- Parse one named `MODULE ... ENDMODULE` per file, `USE`, `USE ... AS ...`,
  `USE ALL FROM ...`, and `PUBLIC`.
- Reject missing `ENDMODULE`, multiple named modules, legacy/named mixing,
  misplaced `USE` clauses, `PUBLIC DEFINE`, and invalid `AS` on `USE ALL FROM`.
- Preserve legacy bare `MODULE` followed by fundamental or user-defined type
  declarations.

Suggested commit: `language: add named module file syntax`.

### Slice 2: Semantic scopes and visibility

- Add `ModuleId`, module scopes, public-member tables, and module aliases.
- Resolve qualified values plus values introduced by `USE ALL FROM`, including
  lvalues, calls, types,
  constants, `SET`, static initializers, and assembler symbols.
- Apply the general equal-identity coalescing rule.
- Distinguish private-member, unknown-member, and `USE ALL FROM` collision
  diagnostics.
- Preserve record-field versus module-member disambiguation.

Suggested commit: `semantic: resolve module visibility and uses`.

### Slice 3: IR and emission hardening

- Carry resolved stable IDs into NIR.
- Tighten verification against executable use aliases or qualified strings.
- Keep classic and MIR6502 independent of module-name lookup.
- Emit qualified display names and deterministic collision suffixes.

Suggested commit: `nir: preserve resolved module identities`.

### Slice 4: First embedded clients

- Implement the loader/VFS slices from the loader companion note.
- Add `ATARI.ANTIC`, `ATARI.GTIA`, and `ATARI.OS` as embedded modules.
- Verify volatile hardware behavior in every public compiler mode.
- Add the runtime-neutral `SYS` interface and runtime bindings described by the
  runtime companion note.

Suggested commits are maintained in the companion notes.

## Validation Matrix

The language implementation must cover:

- named-header commitment on the same line and a missing-`ENDMODULE` error;
- legacy bare `MODULE` followed by a user-defined type declaration;
- rejection of multiple named modules and mixed legacy/named files;
- `USE` clauses only in the module header;
- default and explicit aliases, including one-component `SYS`;
- qualified `USE` and `USE ALL FROM` for every public semantic symbol class;
- rejection of `PUBLIC DEFINE` with unchanged legacy `DEFINE` behavior;
- propagation of `PUBLIC VOLATILE` across a grouped declaration;
- equal-ID alias coalescing and different-ID name collisions;
- private-member versus unknown-member diagnostics;
- routine-local shadowing of names introduced by `USE ALL FROM`;
- public record types from used modules and record/module dot disambiguation;
- qualified use in sizes, addresses, `SET`, initializers, and inline assembly;
- `INCLUDE` inside a named module using private-by-default module scope;
- public volatile hardware reads and writes in every compiler mode;
- exactly-once volatile behavior after qualification and optimization;
- compatibility-prelude and `USE ALL FROM SYS` coexistence;
- runtime-neutral `SYS` routine identity under both runtimes;
- collision-free, deterministic MADS labels for sanitization collisions;
- verifier-clean NIR with no executable module-name lookup.

Required semantic/NIR checks remain:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

## Deferred Features

The first version deliberately defers:

- dependency cycles, including mutually recursive public types across modules;
- separate compilation, object-level interfaces, `.aci` files, and a linker;
- incremental compilation and persistent cross-compilation symbol identities;
- selective uses and transitive re-exports;
- exported textual macros;
- versioned packages and network dependency resolution;
- generic or parameterized modules;
- implicit runtime module initialization;
- explicit `PROGRAM` or `ENTRY` declarations;
- read-only/write-only hardware qualifiers;
- target-independent selection among Atari and non-Atari hardware modules.

Future separate compilation should introduce an explicit stable external symbol
key. It must not assume that the current per-compilation numeric `SymbolId` is a
persistent ABI.

## Recommended Sequence

Implement syntax and semantic namespaces first, followed by the deterministic
loader and embedded hardware modules. Once qualification and volatile access
are correct across all backends, add the runtime-neutral `SYS` interface and the
selective standalone runtime.

The hardware modules are the first practical namespace client. `SArgs` is the
first practical standalone-runtime client. Keeping those milestones separate
prevents module correctness, VFS loading, and runtime relocation from becoming
one indivisible change.
