# Selective Linking Implementation Plan

## Status

This note defines the migration from backend-specific runtime selection and
whole-module application emission to one common selective linker. Slices 1
through 4 are implemented: the Action! program entry and root module are
explicit SemIR identities, application/user-module dependencies use the common
graph, optimized classic and MIR6502 receive the same selected SemIR, and
resident SYS selection filters a common provider SemIR using the generated
embedded graph before either backend lowers it. Compatibility mode still
retains the whole application through its root policy.

## Goal

One backend-independent link plan must select application code, imported user
modules, SYS providers, and compiler runtime packages. Classic and MIR6502 must
consume the same selected source program instead of independently deciding
which source routines exist.

```text
Loaded compilation
        |
      SemIR
        |
Common link planning
  - application and user-module closure
  - external binding resolution
  - runtime-provider closure
        |
  Selected SemIR
     /       \
 classic     NIR -> MIR6502
```

Runtime-helper requirement discovery remains target-owned. Once a backend has
identified a required helper, its root is passed to the same link-closure
implementation used for ordinary source and SYS routines.

## Entry-Point Contract

Action! does not reserve a routine named `Main`. The executable entry is the
last source `PROC` in the root program that emits code. Functions, external
declarations, and fixed-address routine declarations are not candidates.

Semantic lowering records that routine's stable `SymbolId` exactly once. Link
selection, runtime composition, lowering, and emission preserve the identity;
none of those stages may infer an entry from the order of their transformed or
appended routines.

`PUBLIC` controls module visibility and does not make a routine live.

## Ownership

- SemIR owns source-level link meaning: entry identity, resolved calls,
  address-taken routines, storage identity, aliases, initializers, module
  ownership, external interfaces, and structured machine references.
- The common linker owns roots, dependency edges, deterministic closure,
  conservative retention, and selection reasons.
- NIR receives only selected source entities and must not recover module or
  visibility facts.
- MIR6502 owns target helper requirements and target layout strategy. It may
  contribute explicit target/layout edges but must not inspect SemIR.
- Emission writes the selected program and reports its link decisions; it does
  not perform reachability analysis.

## Link Graph

The graph uses stable identities rather than display names. Nodes represent:

- source and runtime routines;
- global or fixed storage declarations;
- effectful top-level items such as `SET`;
- machine/layout groups which cannot safely be split;
- external provider entries.

Edges record both the dependency and a reason:

- direct call;
- routine address or callable value;
- storage read, write, or address;
- alias backing;
- initializer relocation;
- inline-assembly or machine-block relocation;
- runtime binding;
- source or machine-code fallthrough;
- required adjacent bytes or layout group;
- runtime-helper dependency.

The closure and its reason chains must be deterministic so maps, listings, and
tests can explain why every retained entity is present.

## Roots

Optimized classic and MIR6502 start with:

- the explicit last-source-`PROC` entry identity;
- executable top-level statements in the root program;
- compilation-affecting `SET` items;
- routines whose addresses escape into callable or absolute storage;
- entities referenced by retained initializers and machine relocations;
- referenced external interfaces after runtime binding;
- conservative machine/layout groups when a precise boundary is unavailable.

An indirect call retains every routine proven able to reach its callable
storage. If that set cannot be bounded safely, the linker retains the relevant
module or explicit layout group.

Compatibility mode initially uses the same planner with every application
routine and storage declaration seeded as a root. This preserves compatibility
output while still allowing runtime provider selection to use the common
implementation. Application dead stripping in compatibility mode is a separate
contract decision.

## Selection Rules

- Preserve the original order of retained modules and items.
- Preserve semantic `SymbolId` values while filtering one compilation.
- Do not retain an entity merely because it is public.
- Retain referenced ordinary storage and follow aliases and initializer
  relocations transitively.
- Fixed-address hardware declarations emit no backing bytes, but referenced
  declarations remain available for maps, volatility, and relocation facts.
- Routine-owned literals and statics disappear naturally when their routine is
  removed before NIR lowering.
- Named library modules have no implicit initialization. A future module
  initializer feature must add an explicit root and ordering contract.
- Missing dependencies and unrepresentable opaque references fail closed or
  trigger documented conservative retention; they never silently remove code.

## Runtime Providers

External interface selection is part of common link planning:

1. Collect referenced external interface identities from selected application
   code.
2. Resolve each interface through the chosen cart or standalone binding table.
3. Absolute cartridge bindings add no source routine.
4. Source-backed bindings become roots in their runtime provider SemIR.
5. Run the same dependency closure over the provider program.
6. Lower and compose only the selected provider entities for the chosen
   backend.

Application and provider compilations have separate numeric ID spaces. Binding
resolution maps to stable identities inside the provider image; it must not use
display-name equality as executable semantics. Backend-specific rebasing may
remain during the migration, but selection itself is common.

### Embedded resident SYS graph

Resident SYS sources and standalone bindings are compiler inputs, so their
dependency graph is generated once rather than rediscovered for every program.
The checked-in artifact is
[`../../embedded/manifests/sys-link-v1.txt`](../../embedded/manifests/sys-link-v1.txt).
It contains schema and source fingerprints, provider-local routine and storage
nodes, and reasoned direct edges. Regenerate it with:

```sh
cargo run --bin actionc-runtime-link-manifest -- embedded/manifests/sys-link-v1.txt
```

Normal classic and MIR6502 resident selection parse and validate the embedded
artifact, bind its provider-local identities to the current provider SemIR,
traverse closure from program-specific roots, and filter that SemIR while
preserving source order and identities. Classic projects the selected provider;
MIR6502 lowers the same selected provider through NIR. Neither production path
scans MIR calls or decodes machine bytes. Tests retain the old discovery
implementation as an oracle and compare every individual resident routine
root, including its storage closure.

The separately compiled `SYSLIB` helper package remains dynamic until slice 5.
That package is rooted by target helper discovery and is not the resident SYS
provider graph migrated in slice 2.

## Machine Code and Layout

The existing runtime selector preserves bodyless fallthrough aliases,
machine-block fallthrough, and routines which own byte prefixes required by a
following machine block. The common planner must preserve these relationships
as explicit graph edges or indivisible layout groups.

When analysis cannot prove a safe split, retain the whole annotated layout
group. If no group exists, retain the owning module and record the conservative
reason in the link plan.

## Migration Slices

### 1. Explicit entry identity

- Add the last-source-`PROC` `SymbolId` to `SemProgram`.
- Propagate it to the existing NIR program-entry contract.
- Verify that filtering or appending routines cannot change it.
- Correct documentation and tests which imply that `Main` is special.

### 2. Common graph in audit mode

- Add link node, edge, root, selection, and reason types.
- Extract source dependencies from verifier-clean SemIR.
- Add a generator which analyzes the compiler's embedded SYS sources once and
  writes a deterministic runtime-link manifest containing provider-local
  routine/storage identities, dependency edges, fallthrough edges, backward
  prefix requirements, and indivisible layout groups.
- Embed that generated manifest in the compiler alongside the SYS sources. A
  normal compilation must not rediscover the SYS graph by scanning MIR or
  decoding machine bytes. It binds requested SYS interfaces and helper roots to
  manifest nodes, then performs only deterministic closure traversal.
- Give the manifest a schema version and a fingerprint of every embedded source
  and binding input from which it was generated. Reject a stale or malformed
  manifest instead of silently falling back to per-link graph discovery.
- Keep per-compilation graph extraction for the root program and user modules;
  unlike bundled SYS, those sources are not known when the compiler is built.
- Compute closure in audit mode without deleting anything.
- Add an `--emit-link-plan` representation or equivalent map section.
- Compare runtime closures with the existing audited SYS inventory.
- During migration only, keep the current dynamic MIR analyzer as a test oracle,
  not as a production fallback. For every SYS routine used as an individual
  root, assert that the embedded graph and legacy analyzer select identical
  routine and storage closures.
- Add focused manifest tests for bodyless entry aliases, machine-block
  fallthrough, terminal `RTS`/`RTI`/`JMP`, machine relocations, backward branch
  prefixes, and conservative opaque layout groups. Include named regressions
  for the `InputB`/`InputC`/`InputI`, `ValB`/`ValC`/`ValI`, and
  `Zero`/`SetBlock` families.
- Assert that classic and MIR6502 bind the same roots to the same embedded SYS
  closure and that production linking never invokes SYS graph discovery.

### 3. User-module selection

- Filter SemIR before classic projection or NIR lowering.
- Enable pruning for optimized classic and MIR6502.
- Keep compatibility application output whole through its root policy.
- Replace the test which currently requires unused user routines in all modes.

Implemented behavior preserves original module/item order and semantic IDs.
The entry routine and root-module top-level items seed optimized selection;
routine-address initializers that escape through emitted storage are also
roots. References then retain calls, ordinary storage, aliases, initializer
relocations, and structured assembly targets transitively. A reachable opaque
machine or inline-assembly body conservatively retains its whole source module
as an explicit layout group. Compile-time definitions and type/record metadata
remain available but emit no target storage.

### 4. SYS provider migration

- Resolve referenced external interfaces to provider identities.
- Select resident runtime source through the common graph.
- Make both classic and MIR6502 lower the selected provider SemIR.
- Retire classic name filtering and MIR-only SYS closure authority.

Implemented provider selection validates manifest routine/global identities
against the embedded source SemIR, clears the provider's incidental program
entry, and removes unselected routines, backing storage, and top-level runtime
items before backend lowering. Fixed-address routine declarations and DEFINEs
remain as non-emitting ABI metadata for classic projection; MIR6502 binds only
the manifest-selected executable identities after lowering that selected
source, without performing another dependency analysis.

### 5. Compiler runtime packages

- Feed discovered SYSLIB helper roots to the common closure engine.
- Preserve target ownership of helper discovery and ABI selection.
- Remove duplicated routine/storage closure implementations after parity is
  proven.

### 6. Measurement and cleanup

- Report retained and removed bytes by module and reason.
- Rebuild `sine-surface.xex` in optimized classic and MIR6502 modes.
- Audit the remaining size difference after module selection.
- Remove migration-only adapters and name-based selection paths.

## Required Coverage

- The entry routine has a name other than `Main` and remains the last source
  `PROC` after runtime routines are appended.
- Unused public and private imported routines are removed in optimized modes.
- Private and cross-module transitive callees remain.
- Storage aliases, initializer targets, and routine-address initializers remain.
- Function pointers retain every possible target.
- Structured inline-assembly and machine relocations retain their targets.
- Opaque machine code triggers conservative retention.
- Unused SYS and user modules add no code or backing storage.
- Cart and standalone select the correct provider closures.
- Classic and MIR6502 expose the same semantic link plan.
- Selection order and reason output are deterministic.
- NIR verification rejects dangling routine, storage, static, or machine-block
  references after selection.

After changes to SemIR, NIR, or their boundary, run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml
```
