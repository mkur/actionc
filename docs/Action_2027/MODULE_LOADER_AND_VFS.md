# Action 2027 Module Loader and Embedded VFS

Design status: integrated behind the default-off
`experimental-named-modules` Cargo feature. Compiler-owned embedded modules
remain active without that feature, while host named-module roots and search
paths are rejected. This note defines how the compiler finds, identifies,
loads, and diagnoses Action 2027 modules. The source-language contract is in
[`MODULE_SYSTEM_DESIGN.md`](./MODULE_SYSTEM_DESIGN.md). Runtime implementation
selection is in
[`RUNTIME_INTERFACE_AND_STANDALONE.md`](./RUNTIME_INTERFACE_AND_STANDALONE.md).

## Goals

- Ship one `actionc` executable with no installed `lib`, `include`, or module
  catalog.
- Store `SYS`, `ATARI.*`, and OSS runtime sources in a read-only virtual
  filesystem embedded in that executable.
- Feed host and embedded sources through the same tokenizer, parser, semantic
  model, and source-map machinery.
- Make project module lookup deterministic and portable across case-sensitive
  and case-insensitive hosts.
- Preserve complete dependency-chain and source diagnostics without exposing
  build machine paths.
- Never extract embedded files or depend on a writable cache.

## Source-File Contract

Every automatically loadable source file contains exactly one named module.
The requested path and declared identity must agree case-insensitively:

```text
request:     GAME.ENTITIES.PLAYER
virtual path game/entities/player.act
declaration MODULE GAME.ENTITIES.PLAYER
```

A root source containing a named module follows the same one-module-per-file
rule as a dependency source. Tests use several source objects rather than a
special multi-module root grammar.

Automatic source paths are lowercase. The actual path components on disk must
also be lowercase, even on Windows and default macOS filesystems. If
`game/entities/Player.act` would otherwise satisfy
`GAME.ENTITIES.PLAYER`, the loader reports a case-mismatch diagnostic instead
of accepting a project that later fails on Linux.

Case validation may require enumerating each parent directory on a
case-insensitive host. It must not depend only on the spelling returned by
opening a normalized path.

## Logical Source Providers

The compiler exposes one source namespace through two providers:

1. a host provider for the root program, project modules, explicit module
   paths, and legacy includes;
2. a read-only embedded provider for reserved modules and internal runtime
   units.

Conceptually the loader consumes source objects rather than raw paths:

```text
SourceId
SourceOrigin = Host(path) | Embedded(virtual_path)
SourceText { origin, bytes }

ModuleSourceProvider::load(module_path) -> SourceText
RuntimeSourceProvider::load(runtime_unit_id) -> SourceText
```

`SourceId` is unique within one compilation. `SourceOrigin` owns display and
include-resolution provenance; it is not executable language semantics.

Host and embedded providers return the same `SourceText` representation. An
embedded source is not a pre-parsed AST and does not bypass syntax, semantic,
or verifier checks.

## Module Resolution Order

For a `USE` clause, the loader searches in this order:

1. a module already loaded in the current compilation;
2. a reserved module in the embedded VFS;
3. the project module root;
4. each explicit `--module-path` directory in command-line order.

The project module root is the directory containing the root source file. When
the root source has no host path, such as source supplied through an API or
standard input, it defaults to the invoking working directory unless the API
provides an explicit project root.

Lookup is always relative to the project root, not relative to the using
module's nested directory. This makes one module identity map to one project
path regardless of which module uses it.

The first matching source wins. Loading a second physical source declaring an
already loaded identity is a duplicate-module error rather than a later-wins
override.

### `--module-path`

The user-facing spelling is repeatable:

```sh
actionc --module-path ../shared --module-path ./generated program.act
```

Rules:

- paths are searched from left to right;
- relative paths are resolved against the invoking working directory;
- a repeated physical directory is coalesced after canonicalization;
- no `ACTIONC_MODULE_PATH` or other implicit environment path is consulted in
  the first version;
- a module path cannot shadow the reserved `SYS` or `ATARI` roots;
- an API caller may provide an ordered list directly without changing process
  environment state.

These rules affect user project modules only. The compiler does not search
beside its executable for support files.

## Reserved Embedded Roots

`SYS` and `ATARI` are reserved for compiler-supplied modules. A host file that
declares `MODULE SYS`, `MODULE ATARI`, or any descendant such as
`MODULE ATARI.ANTIC` receives a reserved-root-shadow diagnostic before generic
duplicate-module handling.

Tests and embedding applications may inject a replacement VFS through an
explicit compiler API. The normal command-line compiler always uses its fixed
embedded image; the current directory cannot change the meaning of a standard
or hardware module.

## Dependency Graph and Loading Phases

Every used module is loaded and parsed once. Repeated `USE` clauses reuse the
module's `ModuleId` and source map.

The first version requires an acyclic dependency graph. A cycle reports the
complete ordered chain, including the closing edge:

```text
GAME.A -> GAME.B -> GAME.C -> GAME.A
```

Runtime routine-call cycles do not form module-dependency cycles; internal
runtime selection handles them as routine dependency groups.

For an acyclic graph, semantic loading still separates interface collection
from body resolution:

1. parse every reachable module and allocate its `ModuleId` and declaration
   identities;
2. collect each module's public names and signatures;
3. resolve dependencies, aliases, bodies, expressions, and initializers in
   dependency order.

This makes `USE`-clause order irrelevant without promising support for cyclic
module interfaces. Cross-module recursive types remain deferred with
dependency cycles.

## `INCLUDE` Resolution

`INCLUDE` remains a textual source operation rather than module lookup.

For host source, a relative include is resolved first against the directory of
the including physical source, then under the existing legacy include rules.
It does not search `--module-path`; that option is for named modules.

Inside a named module, included declarations acquire that module's scope and
private-by-default visibility as specified by the language contract. An
included fragment cannot declare a named module.

Public embedded modules use `USE` for their dependencies and do not include
host files. Resolving `SYS` or `ATARI.*` therefore cannot escape from the VFS
into the current project. Internal runtime sources may use an explicitly
embedded-relative include resolver, but never the host provider.

## Embedded Virtual Filesystem

The repository keeps embedded inputs as reviewable text files. Initial public
module sources are expected under paths such as:

```text
embedded/modules/sys.act
embedded/modules/atari/antic.act
embedded/modules/atari/gtia.act
embedded/modules/atari/os.act
embedded/modules/atari/pokey.act
embedded/modules/atari/pia.act
```

The authoritative OSS runtime inputs already exist under:

```text
corpora/action-runtime/extracted/SYSLIB.ACT
corpora/action-runtime/extracted/SYSBLK.ACT
corpora/action-runtime/extracted/SYSIO.ACT
corpora/action-runtime/extracted/SYSGR.ACT
corpora/action-runtime/extracted/SYSMISC.ACT
corpora/action-runtime/extracted/SYSSTR.ACT
```

The build maps repository inputs to stable private virtual names. For example,
`SYSLIB.ACT` appears as `<runtime:SYSLIB.ACT>`; its repository path is never
required on an end user's machine.

### Image contents

For each file, the deterministic image records:

```text
source kind: public module or internal runtime unit
canonical module path or internal runtime key
canonical lowercase virtual path
source bytes
content digest
```

This is a VFS directory, not a second declaration database. Public names,
types, addresses, constants, routine signatures, and implementations remain in
the Action source bytes and are discovered by the normal frontend.

The initial implementation may generate a sorted Rust static byte table during
the compiler build. Compression is optional and should be added only if source
size materially affects the executable. Image ordering and its aggregate digest
must not depend on hash-map order, timestamps, absolute repository paths, or
the build working directory.

`actionc --version` reports the aggregate embedded-source digest so a bug report
can identify the exact VFS image independently of the compiler version string.

### Diagnostics

Stable virtual names appear in diagnostics:

```text
<embedded:SYS>
<embedded:ATARI.ANTIC>
<runtime:SYSLIB.ACT>
```

Line, column, source excerpt, and dependency-chain reporting work as for host
files.
No diagnostic exposes the build machine's source path.

The compiler currently models diagnostics as a span and message. This design
does not introduce module-only public error codes. If structured diagnostic
kinds are added, they should be a compiler-wide facility. Module tests must
still distinguish at least these cases:

- declared module name does not match the requested identity;
- dependency cycle;
- private member versus unknown member;
- `USE ALL FROM` collision;
- reserved-root shadowing;
- lowercase path mismatch;
- missing embedded source;
- binding missing for the selected runtime.

## Single-Binary Invariant

The defining integration test copies only the `actionc` executable into an
empty temporary directory and compiles:

```action
MODULE VFS_TEST
  USE ALL FROM SYS
  USE ATARI.ANTIC

  PROC Main()
    ANTIC.WSYNC=0
  RETURN
ENDMODULE
```

The test runs with no network, no support-file environment variables, no
adjacent module directory, and no writable cache. Compilation creates no
extracted source files.

A second copied-binary test uses `--runtime standalone` and a call frame larger
than three bytes. It must load `SYSLIB.ACT` from the embedded VFS, select
`SArgs`, emit no unrelated runtime routine, and produce a program that runs
without the Action! cartridge.

## Implementation Slices

### Slice 1: Unified source objects

- Introduce `SourceId`, `SourceOrigin`, and `SourceText`.
- Route the root source and legacy includes through the host provider.
- Preserve existing source spans and diagnostics.

Suggested commit: `compiler: introduce source providers`.

### Slice 2: Deterministic project module loader

- Implement lowercase dotted-path mapping and one-module-per-file validation.
- Add ordered, repeatable `--module-path` and its compiler API equivalent.
- Detect actual filename-case mismatches on every supported host.
- Build the acyclic module graph and report complete cycles.
- Collect public interfaces before resolving bodies.

Suggested commit: `compiler: load named source modules`.

### Slice 3: Embedded VFS

- Generate the sorted read-only image at build time.
- Reserve `SYS` and `ATARI` and reject project shadowing.
- Preserve virtual source maps and stable diagnostic names.
- Add the aggregate VFS digest to `actionc --version`.
- Prove that no embedded input is read from or extracted beside the binary.

Suggested commit: `compiler: embed module virtual filesystem`.

### Slice 4: Embedded module clients

- Add ANTIC, GTIA, POKEY, PIA, and OS module sources.
- Validate each virtual path against its declared uppercase identity.
- Add `SYS` and the runtime units required by the runtime companion plan.
- Convert selected Action 2027 samples to qualified `USE` clauses.

Suggested commit: `modules: add embedded Atari interfaces`.

## Validation Matrix

- one named module per host or embedded source;
- lowercase path enforcement on Linux, Windows, and macOS;
- declared/requested identity mismatch;
- deterministic earliest-match `--module-path` resolution;
- repeated path coalescing;
- reserved-root shadow rejection before duplicate handling;
- exactly-once parsing for repeated `USE` clauses;
- complete deterministic cycle chains;
- interface collection independent of USE-clause order;
- relative host includes and VFS-confined embedded dependencies;
- stable virtual filenames and source excerpts in diagnostics;
- identical inputs producing a byte-identical VFS image and digest;
- identical symbol inputs producing byte-identical MADS labels;
- version output containing the VFS digest;
- copied-binary compilation with no adjacent files or extraction;
- injected test VFS support without affecting command-line behavior.

## Non-Goals

The first loader does not provide:

- dependency cycles;
- package versions or network resolution;
- implicit environment search paths;
- installed system module directories;
- first-run extraction or a module cache;
- multiple modules in one physical source;
- separate compilation or persistent interface files;
- incremental compilation.

A future package or interface-file design must layer on the same logical module
identity and must not weaken reserved embedded roots or deterministic source
provenance.
