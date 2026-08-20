# Action 2027 Modules and Standalone Runtime Implementation Plan

Plan status: integrated but disabled in default builds. Milestones A, B, and C
are retained behind the `experimental-named-modules` Cargo feature; all 14
slices are complete. The follow-on resident-library plan is also complete
through its Milestone E: all 71 audited
`SYS` routines share one interface and are available in all four
backend/runtime combinations. This note turns the accepted contracts in
[`MODULE_SYSTEM_DESIGN.md`](./MODULE_SYSTEM_DESIGN.md),
[`MODULE_LOADER_AND_VFS.md`](./MODULE_LOADER_AND_VFS.md), and
[`RUNTIME_INTERFACE_AND_STANDALONE.md`](./RUNTIME_INTERFACE_AND_STANDALONE.md)
into dependency-ordered implementation slices.

The work is intentionally split into small vertical commits. Every slice must
leave legacy source working, keep the cartridge runtime as the default, and
finish with focused tests plus the applicable repository-wide checks. A slice
may be divided further when review reveals an independently testable boundary;
unrelated cleanup must not be folded into it.

## Intended Result

The completed work provides:

- one named module per source file, private-by-default declarations, `PUBLIC`,
  qualified `USE` clauses, and explicit `USE ALL FROM` clauses;
- deterministic host module lookup and an embedded read-only VFS for `SYS`,
  `ATARI.*`, and the OSS Action runtime sources;
- source-complete diagnostics for host and embedded files;
- resolved module identities in SemIR and stable IDs below SemIR, with no
  backend re-resolution of qualified source names;
- deterministic qualified symbols in maps and MADS listings;
- explicit, backend-independent `--runtime cart` and
  `--runtime standalone` configuration;
- selective standalone inclusion of `SArgs`, arithmetic helpers, and the
  reachable cross-unit closure of the complete audited `SYS` interface;
- coherent `actionc-run` cartridge and runtime selection.

This is whole-program compilation. Separate objects, interface files, package
versions, dependency cycles, and a linker remain outside this plan.

## Current Compiler Baseline

The following existing shapes determine the order of work:

- `lexer.rs` discards newlines as trivia, and `parser.rs` currently treats
  every bare `MODULE` as another declaration region. `Program.modules` does
  not represent named namespaces.
- `includes.rs` expands physical files into one source string and maps spans
  back to host `PathBuf` values. It has no source-provider or embedded-origin
  abstraction.
- `semantic.rs` creates one builtin scope and one global scope. Every parsed
  module region is analyzed into that same global scope, and routine bodies are
  analyzed immediately after their declarations.
- resident variables and routine signatures are seeded from Rust tables.
- the default classic path still generates directly from AST. Named modules
  cannot use that path because qualification and visibility belong to SemIR,
  not classic code generation.
- MIR6502 already has logical helper kinds and a `Deferred` helper target, but
  materialization currently replaces new helper requirements with cartridge
  addresses. Pre-emission verification correctly rejects unresolved helpers.
- explicit helper-slot `SET` overrides still travel through legacy string NIR
  operands before MIR6502 turns them into `RuntimeSymbol(String)`.
- `CompileRequest` and public `CompileOptions` have no runtime or module-path
  configuration.
- the repository has no build-time VFS generator. The original GPL runtime
  sources already live in `corpora/action-runtime/extracted` and must remain the
  authoritative inputs.

These are migration points, not reasons to duplicate module or runtime meaning
in a new table.

## Architectural Shape

The finished compilation flow should be:

```text
root SourceText
    -> INCLUDE expansion within each physical source context
    -> parsed SourceUnit (legacy or exactly one named module)
    -> dependency graph through host + embedded providers
    -> interface collection
    -> semantic body resolution
    -> SemIR with ModuleId/SymbolId and selected public identities
    -> NIR with stable executable IDs
    -> backend lowering with logical runtime requirements
    -> runtime binding and selective runtime closure
    -> layout/materialization
    -> emission, map, and MADS listing
```

The principal dependency chain is:

```text
source providers -> named syntax -> module graph -> semantic interfaces
                                                -> qualified resolution
                                                -> IR/backend hardening

source providers -> embedded VFS -> Atari modules
                                -> runtime source provider

IR/backend hardening -> explicit runtime choice -> SArgs -> helper closure
embedded VFS ------------------------------------^          -> SYS bindings
binding syntax ----------------------------------------------------^
```

## Decisions Required Before Their Dependent Slices

### Gate A: runtime-neutral callable and binding syntax

Decision: approved in `EXTERNAL_RUNTIME_BINDINGS.md`. Public interfaces use
`PUBLIC EXTERNAL PROC/FUNC`; compiler-owned embedded Action binding units map
the stable interface identity with `SET`.

The runtime design deliberately leaves `EXTERNAL` and binding syntax
illustrative. Before the `SYS` binding slice begins, add and approve a short
language note that fixes:

- how a module declares a public callable with a signature but no body;
- how cart and standalone providers bind that one symbol identity;
- whether binding declarations are ordinary Action source or compiler-only
  embedded metadata expressed in Action syntax;
- diagnostics for duplicate, missing, and ABI-incompatible bindings;
- address-taking and `SET` behavior for a bound callable.

Do not encode an interim Rust name-to-address catalog. Earlier module, VFS, and
logical-helper slices do not depend on the final spelling and may proceed.

### Gate B: runtime corpus preflight

Before selective runtime emission, add a read-only probe which tokenizes,
parses, and semantically inventories `SYSLIB.ACT` and the split runtime files.
Record unsupported source forms, routine call edges, machine-block
relocations, static/zero-page dependencies, and top-level helper-slot `SET`
statements. Fix general frontend gaps in separate commits; do not edit the
runtime corpus merely to make the probe pass.

The preflight must determine the real dependency closure of `SArgs`. The
acceptance rule is “`SArgs` plus required dependencies, and no unrelated
routines,” not an assumption that the routine is self-contained.

### Gate C: backend/runtime support matrix

The CLI accepts runtime configuration before every combination is implemented,
but unsupported combinations fail explicitly. The staged matrix is:

| Stage | classic + cart | mir6502 + cart | classic + standalone | mir6502 + standalone |
| --- | --- | --- | --- | --- |
| runtime option introduced | supported | supported | explicit unsupported diagnostic | explicit unsupported diagnostic |
| first standalone linker | supported | supported | explicit unsupported diagnostic | `SArgs` and selected helpers |
| parity milestone | supported | supported | supported | supported |

`--runtime standalone` must never silently change the backend.

## Slice 1: Unified Source Identity and Providers

Goal: introduce source provenance without changing parsing or compilation
semantics.

Implementation:

- Add `SourceId`, `SourceOrigin::{Host, Embedded}`, and `SourceText` in the
  source/loading layer.
- Introduce a provider interface for root/include reads and a separate logical
  module lookup operation. The initial production provider is host-only.
- Keep frontend `Span` compact by assigning each decoded source a range in a
  compilation source arena. Rebase file-local token spans once when a source
  enters the arena; use `SourceId` and `SourceMap` for provenance. Avoid adding
  a source ID field to every IR span in the first slice.
- Replace path-only diagnostic mapping with stable origin display names while
  retaining path access for host diagnostics.
- Route the root file and existing legacy `INCLUDE` behavior through the host
  provider.
- Add an injectable in-memory provider for loader tests. Keep it out of the
  normal CLI search order; copied-binary tests use the production embedded
  provider added later.

Tests and exit criteria:

- existing include ordering, ATASCII decoding, recursive-include diagnostics,
  and excerpts remain unchanged;
- in-memory and host sources produce equivalent AST and mapped locations;
- two sources with identical local spans map to different origins;
- no process environment or current-directory mutation is required by tests.

Suggested commit: `compiler: introduce source providers`.

## Slice 2: Named-Module Syntax and File Classification

Goal: parse the language contract without loading module dependencies yet.

Implementation:

- Preserve physical line information as token metadata, such as a source line
  number or `line_break_before`; do not expose newline as an Action token.
- Treat new extension words (`USE`, `ALL`, `FROM`, `AS`, `PUBLIC`, and
  `ENDMODULE`) as
  contextual keywords. This avoids inventing Action cartridge token IDs and
  preserves their use as ordinary identifiers outside the relevant grammar.
- Replace the ambiguous AST use of `Program.modules` with an explicit source
  classification equivalent to:

  ```text
  SourceUnitKind = Legacy { regions } | Named(NamedModuleDecl)
  NamedModuleDecl { path, uses, items, span }
  UseDecl { path, alias, all, span }
  Visibility = Private | Public
  ```

- Commit to named parsing only when an identifier follows `MODULE` on the same
  physical source line. Require the matching `ENDMODULE`.
- Parse dotted module paths, default and explicit aliases, and `USE ALL FROM`
  clauses. Reject obsolete `IMPORT` and `USE M.*` forms with migration
  diagnostics.
- Restrict USE clauses to the named-module header region.
- Apply `PUBLIC` and existing `VOLATILE` to the complete grouped declaration.
- Reject `PUBLIC DEFINE`, `AS` on `USE ALL FROM`, a second named module,
  executable top-level statements in a named library module, and named/legacy
  mixing.
- Keep bare legacy `MODULE` boundaries byte-for-byte compatible, including the
  case where a user-defined type begins on the next line.
- Classify included physical fragments before insertion so a fragment cannot
  smuggle in a named module. After validation, textual insertion continues to
  use the owning module context.

Tests and exit criteria:

- same-line commitment and missing-`ENDMODULE` diagnostics;
- comments and blank lines around both module forms;
- the complete syntax rejection matrix from the language design;
- grouped `PUBLIC VOLATILE` propagation;
- unchanged lexer token IDs for historical keywords;
- existing parser and original-compiler compatibility fixtures remain green.

Suggested commit: `language: add named module file syntax`.

## Slice 3: Deterministic Host Module Graph

Goal: turn parsed `USE` clauses into a deterministic set of source units without
yet giving them cross-module meaning.

Implementation:

- Add canonical `ModulePath` and per-compilation `ModuleId`. Preserve declared
  spelling for display and lowercase components for identity and lookup.
- Add ordered module paths to compiler configuration and repeatable CLI
  `--module-path PATH` parsing. Relative paths resolve against the invocation
  working directory; the project root remains the root source directory.
- Map `A.B.C` to `a/b/c.act`, enumerate actual directory entries, and diagnose
  any case mismatch even on case-insensitive hosts.
- Search already-loaded units, the embedded provider hook, the project root,
  and explicit module paths in the specified order. The embedded hook is empty
  until Slice 7.
- Reserve the `SYS` and `ATARI` roots at this boundary even while the embedded
  hook is empty. A missing reserved module must not fall through to a host
  source with the same identity.
- Validate one named module per automatically loaded file and compare requested
  and declared module identities.
- Build one deterministic dependency graph, parse each source once, reuse its
  `ModuleId`, and reject cycles with the complete closing chain.
- Preserve `INCLUDE` as source-local textual insertion. Never search module
  paths for includes.
- Return a `LoadedCompilation` containing the root unit, all reachable units,
  graph order, aggregate source arena, and source map.

Public API consequence:

- Extend compilation configuration with an ordered `Vec<PathBuf>` of module
  paths and an optional project root for pathless API sources. `CompileOptions`
  can stop being `Copy`; it should remain a reusable, side-effect-free value.

Tests and exit criteria:

- earliest-match lookup and repeated-directory coalescing;
- declared/requested mismatch and duplicate identity;
- portable lowercase-path failure using enumerated directory entries;
- deterministic complete cycle chains;
- diamond dependencies parse a shared module exactly once;
- used modules are resolved relative to the project root, not the using
  module's directory;
- legacy single-file compilation remains unchanged.

Suggested commit: `compiler: load named source modules`.

## Slice 4: Module Scopes and Interface Collection

Goal: establish symbol ownership, visibility, and declaration-order-independent
public interfaces in SemIR's source model.

Implementation:

- Add one semantic module scope per `ModuleId`; routine scopes inherit from
  their defining module scope. Preserve the current global scope only for the
  implicit legacy unit.
- Extend symbols with defining-module identity, visibility, canonical qualified
  key, and readable qualified name. Allocation IDs remain compilation-local;
  the canonical key is the stable input to emitted-name hashing.
- Split semantic analysis into explicit passes:

  1. allocate module scopes and declaration identities;
  2. collect types, constants, storage facts, callable signatures, and public
     tables;
  3. install module aliases and aliases from `USE ALL FROM`;
  4. resolve bodies and initializers in graph dependency order.

- Store module aliases as references to existing `SymbolId` values, never as
  copied declarations.
- Implement equal-name/equal-ID coalescing and equal-name/different-ID
  collision diagnostics independently of USE-clause order.
- Keep `DEFINE` in its source preprocessing context and out of module public
  tables.
- Diagnose private and absent members separately.

Tests and exit criteria:

- private-by-default behavior for every declaration class;
- public interface collection before body resolution;
- default aliases, explicit aliases, `USE ALL FROM`, local shadowing, and
  collision/coalescing rules;
- included declarations belong to the owner module and are private unless
  explicitly public;
- no named module can see another named module's private names or names from an
  unused module;
- legacy global and builtin lookup tests remain green.

Suggested commit: `semantic: collect module interfaces and visibility`.

## Slice 5: Complete Qualified Semantic Resolution

Goal: make qualification legal everywhere promised by the language contract,
with no backend assistance.

Implementation:

- Add structured qualified-name syntax to types and all symbol-bearing AST
  positions that currently retain only a `String`.
- Resolve values, lvalues, callable targets, routine addresses, types,
  constants, array sizes, fixed addresses, static initializers, and `SET`
  operands against module scopes.
- Resolve Action-symbol references in inline assembler and legacy machine
  blocks to stable semantic targets. Keep source spelling only as display
  metadata.
- Disambiguate `A.B` semantically: a module alias selects a public member and a
  record value selects a field. Parser shape alone must not choose.
- Ensure public record types from used modules retain their field layouts and
  ordinary record access.
- Reject qualified `DEFINE` use rather than attempting cross-file token
  substitution.
- Extend SemIR modules and symbol references with `ModuleId` and canonical
  display/link keys; do not emit module-use operations into executable SemIR.

Tests and exit criteria:

- positive and negative coverage for every qualified context listed above;
- private-member versus unknown-member behavior in each context;
- record/module dot ambiguity and public record layouts from used modules;
- qualified volatile access remains exactly once in SemIR;
- qualified inline-assembly calls, reads, writes, addresses, and constants are
  represented by resolved IDs.

Suggested commit: `semantic: resolve qualified module references`.

## Slice 6: Stable IR Identities and Backend Entry

Goal: make named modules safe below SemIR and across both backends.

Implementation:

- Lower named modules only from semantically resolved SemIR. Raw-AST code
  generation remains available for unchanged legacy compatibility inputs, but
  is not a legal backend entry for named-module source.
- For classic code generation, extend the existing SemIR-to-AST compatibility
  projection so every resolved symbol receives a deterministic, collision-free
  internal link name. This projection serializes semantic decisions; classic
  must not repeat module lookup or visibility checks.
- Replace executable NIR name dependencies exposed by modules with IDs,
  including direct callees, global aliases, routine-address initializers,
  helper override targets, and inline-assembler relocations.
- Do not add optimizer passes over `NirOperand`, unresolved names, or stringly
  compatibility forms. Migrate a form, then tighten the verifier so it cannot
  reappear in verifier-clean NIR.
- Ensure MIR6502 consumes only resolved NIR IDs and never consults SemIR or
  qualified source strings.
- Keep module paths in SemIR/NIR only as display, diagnostic, and listing
  metadata after executable references are resolved.

Tests and exit criteria:

- the same named-module fixture compiles with compatibility classic, modern
  classic, and MIR6502;
- two private symbols with the same short spelling remain distinct;
- direct calls, address-taking, aliases, static relocations, and assembler
  references survive lowering by ID;
- new verifier tests reject each retired stringly executable form;
- NIR snapshots change only where the IR contract intentionally becomes
  stricter.

Required checks for this and every later NIR-changing slice:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Suggested commit: `nir: preserve resolved module identities`.

## Slice 7: Deterministic Embedded VFS

Goal: embed reviewable Action source without introducing a parallel semantic
database or runtime filesystem dependency.

Implementation:

- Add a build step which reads the declared module/runtime inputs, sorts them
  by canonical virtual key, and writes a Rust byte table to `OUT_DIR`.
- Record kind, canonical key, lowercase virtual path, bytes, per-file SHA-256,
  and an aggregate SHA-256. Add explicit `rerun-if-changed` entries.
- Generate byte literals rather than embedding absolute repository paths in
  generated source or diagnostics.
- Implement the read-only embedded provider and stable origins such as
  `<embedded:ATARI.ANTIC>` and `<runtime:SYSLIB.ACT>`.
- Reserve `SYS` and `ATARI` before generic duplicate resolution. The production
  CLI always installs the built-in image; tests may inject a replacement.
- Add the aggregate VFS digest to `actionc --version` and build information.
- Prove that lookup neither extracts files nor reads support files beside the
  executable.

Tests and exit criteria:

- stable ordering and digest independent of input enumeration order;
- source excerpts and dependency chains use virtual origins without build paths;
- host attempts to shadow `SYS` or any `ATARI` descendant fail first with the
  reserved-root diagnostic;
- provider tests resolve embedded bytes without touching the host filesystem;
- a copied compiler binary reports the same VFS digest in an empty directory
  and creates no extracted files. Compilation from that copy is added with the
  first public embedded modules in Slice 8.

Suggested commit: `compiler: embed module virtual filesystem`.

## Slice 8: Embedded Atari Modules and Deterministic Symbols

Goal: deliver the first useful module clients before runtime linking changes.

Implementation:

- Add reviewed Action sources for `ATARI.ANTIC`, `ATARI.GTIA`, `ATARI.OS`,
  `ATARI.POKEY`, and `ATARI.PIA` under the embedded module tree.
- Express hardware through ordinary fixed-address `PUBLIC VOLATILE` storage and
  typed `PUBLIC CONST` values. Keep OS shadows separate from direct registers.
- Validate each embedded path against its declared uppercase module identity.
- Emit readable defining qualified names in maps and listings.
- Sanitize MADS labels deterministically. When canonical keys collide after
  sanitization, derive the suffix from a stable hash of the complete canonical
  qualified key plus symbol kind, never from allocation order or numeric
  `SymbolId`.
- Convert one copied sample, preferably the Action 2027 plasma variant, to
  qualified hardware uses without changing its generated behavior.

Tests and exit criteria:

- fixed addresses, types, volatility, and low-byte-first `CARD` pairs are
  verified in every public compiler mode;
- qualified volatile reads and writes occur exactly once after optimization;
- deliberate MADS sanitization collisions produce stable distinct labels;
- identical builds produce byte-identical module-derived maps and listings;
- the converted sample compiles using only the compiler binary.

Suggested commit: `modules: add embedded Atari interfaces`.

## Slice 9: Explicit Runtime Choice and Logical MIR Helpers

Goal: expose runtime choice while preserving current cart output and moving
physical helper binding out of MIR helper discovery.

Implementation:

- Add public `Runtime::{ActionCart, Standalone}` and thread it through
  `CompileOptions`, internal requests, resolved configuration, CLI help, and
  artifact metadata. Parse only `--runtime cart` and
  `--runtime standalone`; keep cart as the default.
- Make classic runtime configuration explicit. Remove the inference that
  currently derives `RuntimeTarget` from segment-storage behavior.
- Change MIR helper materialization to create logical/deferred helper targets.
  Add a runtime-resolution stage before pre-emission verification; cart
  resolution maps them to the current resident addresses.
- Replace `RuntimeSymbol(String)` helper overrides with resolved routine IDs or
  explicit absolute targets before verifier-clean MIR.
- Refactor helper selection into a MIR-owned pre-layout legalization/planning
  step. It must discover every helper needed by target strategy before storage
  and routine layout, so standalone runtime routines can be merged before
  addresses are assigned.
- Report runtime choice and cart bindings in maps/listings.
- Until later slices land, reject standalone for each backend with the explicit
  support-matrix diagnostic.

Tests and exit criteria:

- default compilation remains byte-identical to explicit `--runtime cart`;
- every current MIR helper still resolves to its established cart address;
- pre-emission verification rejects any deferred helper;
- helper selection is complete before layout and deterministic;
- configuration errors never silently select another backend or runtime.

Suggested commit: `compiler: add explicit runtime selection`.

## Slice 10: Selective MIR6502 `SArgs`

Goal: prove the complete standalone source-to-output path with the smallest
high-value runtime requirement.

Implementation:

- Embed `SYSLIB.ACT` directly from `corpora/action-runtime/extracted` as a
  private runtime unit and parse it through the ordinary frontend.
- Add an internal runtime-source compilation mode with isolated semantic
  scope. Historical top-level helper `SET` statements describe bindings but do
  not become executable roots.
- Produce a backend runtime unit whose routines, static data, relocations,
  effects, ABI facts, source origins, and dependency edges use stable IDs.
- Select `SArgs` when MIR call planning requires an argument frame larger than
  three direct bytes. Compute and merge its real dependency closure before
  global MIR layout, then bind logical `SArgs` calls to the selected routine ID.
- Recognize `SET $4EE=<local routine>` as a resolved local override under both
  runtimes and suppress embedded `SArgs`.
- Allow absolute `$04EE` overrides in cart mode and reject them in standalone
  mode.
- Record the selection reason, origin, and suppressed default in map/listing
  metadata.

Tests and exit criteria:

- a small call frame emits no standalone `SArgs` bytes;
- a large frame emits one `SArgs` dependency group and no unrelated SYSLIB
  routines;
- symbolic local override wins and is emitted once;
- absolute external override fails closed under standalone;
- relocated runtime machine-block references and zero-page requirements are
  correct;
- a copied binary compiles and runs the large-frame probe without an Action
  cartridge.

Suggested commit: `runtime: selectively emit sargs`.

## Slice 11: Arithmetic Helpers and Runtime Dependency Closure

Goal: generalize the `SArgs` path into a deterministic selective runtime
linker.

Implementation:

- Add `LShift`, `RShift`, `MultI`, `DivI`, and `RemI` bindings from
  `SYSLIB.ACT`.
- Derive resolved routine, static-data, machine-block, logical-helper, and
  zero-page dependencies from the compiled runtime source.
- Compute a deterministic call graph, collapse strongly connected components,
  and retain the transitive closure rooted by application/runtime
  requirements and conservative address-taken or indirect-call facts.
- Rebase runtime-local IDs when merging into the application without changing
  canonical symbol keys or display provenance.
- Preserve conservative effects and ABI facts at the logical helper call and
  verify the selected implementation matches them.
- Add exactly-once storage allocation and deterministic group/layout order.

Tests and exit criteria:

- each arithmetic operation selects only its necessary closure;
- multiple dependency paths include a routine/group once;
- recursive runtime call groups are complete and stable;
- unused helpers, storage, and zero-page allocations are absent;
- missing implementations, unresolved dependencies, and ABI mismatches fail
  before emission;
- standalone arithmetic probes execute without the cartridge.

Suggested commit: `runtime: link required arithmetic helpers`.

## Slice 12: Authoritative `SYS` Interface and Bindings

Prerequisite: Gate A is approved.

Goal: replace duplicated resident-library naming with one runtime-neutral public
interface.

Milestone B deliberately establishes the first coherent `SYS` surface with
`Zero`, `SetBlock`, and `MoveBlock` from `SYSBLK.ACT`. Expanding the same
interface to I/O, graphics, strings, and input remains incremental breadth work:
those source units have cross-unit runtime dependencies which must not silently
fall back to cartridge entries in standalone mode.

Implementation:

- Add one embedded `MODULE SYS` source declaring public names, types,
  signatures, and source-visible effects using the approved external-interface
  syntax.
- Add embedded cart and standalone binding sources which refer to those same
  symbols. Cart bindings contain resident addresses; the initial standalone
  bindings point into selectively compiled `SYSBLK` routines. Later surface
  additions use `SYSIO`, `SYSGR`, `SYSMISC`, and `SYSSTR` through the same
  mechanism once their cross-unit dependencies are modeled.
- Validate binding completeness and ABI compatibility during semantic/runtime
  resolution. A fixed cart address never becomes a public numeric constant.
- Replace the Rust-seeded resident procedure catalog incrementally with a
  compatibility prelude whose aliases refer to the same `SYS` `SymbolId`
  values. Keep genuinely compiler-intrinsic types/operations separate.
- Make qualified calls, `USE ALL FROM`, traditional unqualified calls,
  address-taking, static initializers, and `SET` values converge on one
  identity and the selected implementation.
- `USE SYS` alone must not root code. Compute the standalone closure from
  referenced/address-taken routines and backend helper requirements.
- Preserve runtime GPL provenance in maps, listings, and distribution
  documentation.

Tests and exit criteria:

- `SYS.Zero` and `USE ALL FROM SYS` followed by `Zero` resolve to the same symbol;
- an unqualified resident name without a migrated standalone binding is
  rejected instead of retaining its cart address;
- cart and standalone expose identical source signatures;
- address-taking observes the selected implementation address;
- a missing or mismatched binding fails closed;
- an unused `USE SYS` adds no bytes;
- the initial memory routines select only their real dependency closures,
  including `Zero`'s source-level fallthrough into `SetBlock`;
- later I/O, graphics, string, and input additions use this same binding path
  after their cross-unit dependency graph is modeled;
- no second Rust string table owns the implemented public system-library
  surface.

Suggested commit: `modules: bind std to selected runtime`.

## Slice 13: Classic Standalone Parity

Goal: fulfill backend/runtime orthogonality rather than treating standalone as
a MIR6502 alias.

Implementation:

- Reuse the same logical runtime requirement and selected source identities in
  the classic SemIR compatibility path.
- Route classic-plus-standalone legacy inputs through the resolved SemIR
  compatibility projection as well; the raw-AST path cannot participate in
  source-runtime linking.
- Add a classic pre-layout runtime-link stage which compiles selected runtime
  routines with classic strategy, merges their storage and relocations, and
  resolves helper/`SYS` calls without cart addresses.
- Apply the same local helper override, absolute-target rejection, closure,
  provenance, and deterministic ordering policies as MIR6502.
- Remove the temporary classic-plus-standalone diagnostic only when its
  validation matrix passes. Do not silently fall back to MIR6502.

Tests and exit criteria:

- the same standalone semantic fixtures run through modern classic and
  MIR6502;
- classic cart output remains unchanged;
- both backends include equivalent logical dependency closures, with expected
  backend-specific code bytes;
- unsupported runtime routines remain explicit compile errors rather than cart
  fallbacks.

Suggested commit: `runtime: support standalone classic codegen`.

## Slice 14: Runner, Distribution, Samples, and User Documentation

Goal: make runtime selection coherent and usable outside compiler unit tests.

Implementation:

- Make `actionc-run --no-cart` compile with `Runtime::Standalone` and launch
  without a cartridge.
- Make `actionc-run --cart PATH` compile with `Runtime::ActionCart` and mount
  that cartridge. Diagnose contradictory future runtime options.
- Add cross-platform CLI tests for argument propagation and emulator command
  construction.
- Document module paths, reserved roots, runtime defaults, standalone failure
  policy, map/listing annotations, and GPL corresponding-source obligations.
- Add compact named-module examples for qualified hardware, `SYS`, `USE ALL
  FROM`, and a local helper override.
- Update the Action 2027 plasma sample only after both its module form and
  runtime requirement are supported; retain the existing sample for regression
  comparison while the branch is experimental.

Tests and exit criteria:

- runner runtime choice and cartridge mounting cannot disagree;
- copied release binaries compile embedded-module examples on Windows, Linux,
  and macOS without adjacent support directories;
- a standalone runner integration test starts without the Action cartridge;
- `actionc --version` exposes the VFS digest and release artifacts preserve the
  runtime source/notice material.

Suggested commit: `runner: align cartridge and runtime selection`.

## Cross-Slice Validation Policy

Every slice runs its focused unit and integration tests, `cargo fmt --check`,
and `cargo test`. Slices touching semantic lowering, NIR, its verifier, or its
printer also run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Before each milestone, run the relevant fixture sweeps and compare legacy cart
artifacts in all public modes. Any changed fixture must be classified as one
of:

- an intentional language/IR contract change;
- a printer-only qualified-name change;
- a bug fix;
- an unintended regression, which blocks the slice.

The release-level validation matrix includes:

- historical bare `MODULE`, `INCLUDE`, `DEFINE`, `SET`, and resident-name
  compatibility under the default cart runtime;
- named modules through compatibility classic, modern classic, and MIR6502;
- host lookup behavior on case-sensitive and case-insensitive filesystems;
- deterministic code, maps, listings, dependency chains, VFS image, digest, and
  runtime closure for identical inputs;
- volatile hardware accesses in optimized and unoptimized paths;
- copied-binary compilation with no adjacent files, extraction, cache, network,
  or hidden environment lookup;
- standalone execution without an Action cartridge;
- explicit failure for every missing implementation or unsupported
  backend/runtime combination.

## Milestones

### Milestone A: useful modules with unchanged runtime behavior

Slices 1 through 8 complete. Users can access embedded Atari hardware modules,
and all existing programs still use the cart runtime by default.

### Milestone B: practical MIR6502 standalone programs

Slices 9 through 12 complete for MIR6502. `SArgs`, arithmetic helpers, and the
initial `SYS` memory surface are selectively linked from embedded GPL sources.
Named-root `Main` is retained as an explicit IR entry fact, so appending a
runtime closure cannot change the executable RUN address.

### Milestone C: runtime/backend orthogonality

Slices 13 and 14 complete. Classic and MIR6502 both honor the selected runtime,
and runner behavior matches compiler behavior.

## Primary Risks and Containment

- **Line-sensitive `MODULE` parsing:** preserve line metadata in tokens and pin
  legacy same-line/next-line cases before changing the AST.
- **Include expansion hiding physical file boundaries:** classify and validate
  every physical source before textual insertion, then retain the owner module
  and source map.
- **Global-scope assumptions in semantic and classic codegen:** introduce
  module scopes before qualified use and require named inputs to pass through
  SemIR-resolved backend entry.
- **Stringly executable NIR identities:** migrate each required form to IDs and
  immediately tighten the verifier; never teach an optimizer to reason about
  the legacy string form.
- **Standalone requirements discovered too late:** make helper selection a
  MIR-owned pre-layout step so selected source routines participate in one
  layout.
- **Runtime corpus source surprises:** use the preflight inventory and fix
  general parsing/lowering gaps separately from runtime linking.
- **Standard-library duplication:** treat the embedded `SYS` source as the only
  interface authority and migrate the compatibility prelude to aliases of its
  IDs.
- **Cross-platform lookup drift:** enumerate actual path components and test
  case mismatch independently of host filesystem behavior.
- **Non-reproducible embedded images or labels:** sort canonical keys and hash
  semantic inputs, never traversal order, timestamps, build paths, or transient
  IDs.
- **Scope creep into a package manager/linker:** retain whole-program loading,
  acyclic dependencies, reserved roots, and no implicit catalogs for this plan.

## Completion Criteria

The plan is complete only when:

1. the language, loader/VFS, and runtime contracts all match implemented and
   tested behavior;
2. legacy cart compilation remains the default and its intentional compatibility
   fixtures remain green;
3. named modules compile through every supported public mode without backend
   name re-resolution;
4. verifier-clean NIR contains no executable module-use, visibility, qualified-name,
   or newly migrated string-identity forms;
5. embedded Atari and `SYS` modules work from a copied compiler binary;
6. standalone output includes exactly the required OSS runtime closure and
   runs without the Action cartridge;
7. maps, listings, version output, licenses, and corresponding-source material
   make every embedded/runtime decision reproducible and inspectable.
