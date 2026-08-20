# Standalone Runtime Rollout Plan

Status: proposed rollout after the separate `CONST` and `VOLATILE` merges.

## Outcome

Ship cartridge-independent programs through one explicit runtime option without
requiring users to adopt or understand named modules:

```sh
actionc --runtime standalone program.act
```

Traditional single-file Action! source and unqualified resident calls remain
valid:

```action
PROC Main()
  PrintE("HELLO")
RETURN
```

Named modules remain internal compiler infrastructure during this rollout.
`MODULE`, `USE`, `ATARI.*`, project module paths, and qualified `SYS` examples
are not part of the initial standalone-runtime announcement or stability
promise.

## Release Contract

- `--runtime cart` remains the default.
- `--runtime standalone` never falls back to an Action! cartridge address.
- Runtime selection remains independent of classic versus MIR6502 codegen.
- Both active backends accept ordinary non-module source in standalone mode.
- Traditional unqualified resident names resolve through the implicit
  compatibility prelude.
- Missing implementations, ABI mismatches, and absolute helper overrides fail
  at compile time.
- Only the transitive runtime dependency closure is linked.
- Internal module identities do not appear in source requirements or ordinary
  diagnostics.

## Merge Strategy

Build every rollout branch from the then-current `main`. Do not stack new pull
requests on the historical `Action-2027` ancestry, because the earlier feature
pull requests were squash-merged. Reapply the existing commits in their
original order, resolving only integration drift from `main`.

Keep each pull request independently green. Do not combine unrelated indexed
`CASE`, sample polish, listing-size work, or generated probe artifacts with the
runtime rollout.

## Pull Request 1: Internal Source And Identity Foundation

Goal: land the compiler substrate required to compile embedded interfaces and
runtime sources without advertising modules as a public workflow.

Bring over the implementation represented by:

```text
280b5de compiler: introduce source providers
d411f3a language: add named module file syntax
8007984 compiler: load named source modules
1d2a72c compiler: coalesce repeated module roots
71dc50e semantic: collect module interfaces and visibility
4443c13 semantic: resolve qualified module references
7b3185c nir: preserve resolved module identities
6a724cd compiler: embed module virtual filesystem
```

The parser and semantic support are present because the compiler's embedded
sources use them. They remain latent rather than promoted as the next public
language feature.

Exit criteria:

- existing non-module sources remain byte-identical where expected;
- repeated and cyclic internal source loads are deterministic and diagnosed;
- semantic and NIR identities remain stable across embedded units;
- no host search path is needed for compiler-owned sources;
- default cart compilation is unchanged.

## Pull Request 2: Runtime Selection And Standalone Kernel

Goal: expose the runtime axis and make compiler-required helpers work without a
cartridge.

Land:

- embedded runtime bindings and source packaging;
- explicit `cart` versus `standalone` selection in the compiler API and CLI;
- selective `SArgs`, shift, multiply, divide, and remainder helpers;
- external callable bindings with ABI validation;
- classic and MIR6502 standalone emission;
- runner behavior that omits the cartridge for standalone output.

Use the existing implementation sequence beginning with `62969df` and ending
with the initial `SYS` runtime interface at `678de27`. Module-system samples
and public module-usage documentation are not required for this pull request.

Exit criteria:

- a helper-free program emits no runtime code;
- representative argument and arithmetic programs execute without a cart;
- both active backends pass the same runtime contract tests;
- cart output remains unchanged when runtime selection is omitted;
- object maps show only selected helper closures;
- no standalone artifact contains an accidental cartridge call.

At this point `--runtime standalone` may be documented as experimental, but
resident-library coverage is not yet declared complete.

## Pull Request 3: Unified Resident Image And Compatibility Prelude

Goal: make standalone useful to existing Action! programs without requiring
`USE SYS`.

Land the unified embedded runtime image and authoritative `SYS` identity:

- one embedded public signature source;
- cart and standalone bindings for the same stable symbols;
- cross-source runtime dependency closure;
- implicit aliases for traditional unqualified resident names;
- memory, strings, miscellaneous helpers, graphics, and core I/O groups;
- deterministic maps and source provenance.

The relevant historical work starts with generalized SYS bindings at
`f7f02c6`, adds the unified image at `2f0a4fc` and `6024280`, and reaches the
implicit compatibility aliases at `e08f6c1`.

Exit criteria:

- ordinary source can call migrated resident routines without module syntax;
- qualified and unqualified internal spellings share one symbol identity;
- unused public interfaces add no code;
- dependencies may cross physical runtime source units and are emitted once;
- missing standalone bindings fail closed;
- deterministic memory/string, graphics-state, and I/O cases execute in the
  VM under both backends and both runtimes.

## Pull Request 4: Complete Resident Coverage And Remove Duplicate Ownership

Goal: finish the audited runtime surface and make embedded `SYS` the single
source of truth.

Land:

- remaining numeric, console, device, graphics, formatted-output, error, break,
  and conversion families;
- verified `PrintF`, `PrintH`, `InputD`, and `PrintBDE` interfaces;
- derivation of the legacy compatibility prelude from `SYS`;
- classic and MIR6502 target resolution through runtime bindings;
- removal of duplicated resident catalogs from active backend paths.

This corresponds to the feature work through `9bff4a4`. Keep SemIR-native
deprecation separate if it makes review or rollback clearer.

Exit criteria:

- all 71 audited public routines are classified exactly once;
- every advertised routine has compatible cart and standalone bindings;
- both backends consume the same resolved runtime identities;
- no backend recovers resident meaning from source names;
- the complete standalone inventory and minimal dependency closures pass.

## Pull Request 5: Execution Evidence And Release Polish

Goal: promote standalone from structurally complete to release-ready.

Land only runtime-relevant listing cleanup, VM execution coverage, closure
audits, documentation, and runtime-test pin updates from the remaining
Action-2027 history.

Exit criteria:

- Linux, Windows, and macOS CI pass;
- the VM runtime-contract crate passes all resident and helper families;
- cart and standalone behavior agree for deterministic routines;
- hardware and CIO routines have controlled execution or explicit coverage;
- every standalone root has an audited minimal linker closure;
- copied compiler binaries still contain the exact embedded runtime image;
- release notes describe `--runtime standalone`, its OS requirement, and its
  fail-closed behavior without presenting named modules as required.

## Validation Gate For Every Code Pull Request

Run:

```sh
cargo test --locked
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml
python3 -m unittest discover -s tools/tests -p 'test_*.py'
cargo test nir_fixtures_match_snapshots
cargo run --locked --bin actionc-nir-sweep -- fixtures/nir
```

Also compile at least one ordinary, non-module program in these configurations:

```text
classic + cart
classic + standalone
mir6502 + cart
mir6502 + standalone
```

For runtime-bearing pull requests, execute representative standalone output in
`actionc-vm` and inspect the final object for unresolved or cartridge-bound
targets.

## Public Rollout

1. Merge the internal foundation without a module-feature announcement.
2. Mark `--runtime standalone` experimental after the standalone kernel lands.
3. Declare full resident coverage only after the complete VM and closure gates
   pass.
4. Keep cart as the default through the preview period.
5. Consider changing the default only in a separate policy decision backed by
   real program and emulator experience.
6. Launch named modules later with their own syntax, lookup, visibility,
   packaging, and compatibility review.

## Rollback And Risk Control

- Each pull request preserves cart as the known-good default and can be
  reverted independently.
- Standalone never uses hybrid fallback, so incomplete coverage is visible as
  a compile-time failure rather than a hidden cartridge dependency.
- Embedded runtime source and binding digests make packaging drift observable.
- Conservative call and machine-block effects remain in force throughout the
  rollout.
- Public module documentation and examples are intentionally delayed; internal
  implementation details do not become accidental compatibility commitments.
