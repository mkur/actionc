# MIR6502 Builtin Target Resolution Plan

Snapshot date: 2026-06-03.

This note is a Codex-ready implementation plan for resolving and emitting all
currently modeled Action! builtins in the MIR6502 backend.

This is intended as the second final-bridge slice after structured machine-block
reference emission. It should cover the whole known builtin family in one
coherent pass, not only the currently visible `PrintE` failure.

Related documents:

- `docs/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`
- `docs/MIR6502_INDIRECT_CALL_MATERIALIZATION_PLAN.md`

## Current Context

The sampled remaining emission failure after recent materialization work is:

```text
os_call_opaque_barrier.source-listing.err:
  builtin call target `PrintE` is unresolved
```

This is not a broad MIR materialization failure. It is a builtin target-resolution
and emission-readiness gap.

## Goal

Add one central builtin target-resolution path for every builtin already modeled
by the frontend/NIR/MIR.

The backend should classify each known builtin as one of:

```text
resolved     -> MIR6502 can emit the call or selected sequence
deferred     -> known builtin, but target address/config/runtime binding is not available
unsupported  -> known builtin, but intentionally not implemented yet
```

The goal is to eliminate generic failures like:

```text
builtin call target `X` is unresolved
```

and replace them with either real emission or precise `deferred` / `unsupported`
diagnostics.

## Red Lines

Do not use source-name lookup in emission.

Do not add new language-level builtins in this slice.

Do not infer builtin semantics from text. Only handle builtins that are already
represented in MIR/NIR as builtin identifiers or equivalent structured call
targets.

Do not mix this slice with:

- machine-block reference emission;
- indirect call materialization;
- call inlining;
- runtime redesign;
- peepholes;
- broad ABI redesign;
- source parser changes.

## Builtin Coverage

Cover all currently represented builtin IDs / builtin call targets, including at
least the family visible in fixtures and existing lowering:

```text
Put
PutE
Print
PrintE
```

Also include any other already-modeled builtins discovered in the codebase, such
as input-style, graphics, color, sound, resident-library, or runtime-like builtins
if they already have structured MIR/NIR identities.

Do not add unsupported names merely because Action! has more builtins. The scope
is current compiler representation, not full language-library discovery.

## Milestone 1: Inventory Existing Builtin IDs

Goal: discover the exact builtin set already represented by the compiler.

Scope:

- Search the frontend/NIR/MIR/call lowering code for builtin identifiers and
  builtin call target enums.
- Produce an internal mapping list in code comments or tests.
- Decide the status of each builtin:

```text
resolved
deferred
unsupported
```

Acceptance criteria:

- Every currently modeled builtin ID has an explicit MIR6502 status.
- There is no default fallthrough to generic unresolved target diagnostics.

Suggested commit:

```text
mir6502: inventory builtin call targets
```

## Milestone 2: Add Central Builtin Target Table

Goal: route all builtin resolution through one table/function.

Add a central resolver equivalent to:

```text
resolve_builtin_target(builtin_id, config) -> BuiltinResolution
```

Where `BuiltinResolution` records:

```text
status: resolved/deferred/unsupported
call target or selected emission sequence
ABI plan / argument homes
effects / clobbers / preserves
required runtime symbol or absolute address, if any
diagnostic reason for deferred/unsupported
```

Rules:

- The table must be used by materialization/emission before source listing.
- The table must not parse display names.
- Unknown builtin IDs should be a verifier or lowering bug, not a silent emission
  guess.

Acceptance criteria:

- `PrintE` resolves through the table.
- Existing builtin fixtures use the resolver.
- Unsupported/deferred builtins produce precise diagnostics.

Suggested commit:

```text
mir6502: add builtin target resolver
```

## Milestone 3: Resolve And Emit Console/Text Builtins

Goal: implement the builtins most likely present in current fixtures.

Scope:

```text
Put
PutE
Print
PrintE
```

For each builtin:

- preserve argument ABI homes;
- preserve effects and barriers;
- resolve known runtime targets or resident entry points;
- emit the selected call/sequence if configured;
- otherwise produce a precise deferred diagnostic.

Rules:

- Do not guess addresses if no target configuration exists.
- Do not use direct source spelling to decide builtin behavior.
- Effects must remain conservative where exact clobbers are not known.

Acceptance criteria:

- `os_call_opaque_barrier.source-listing.err` no longer fails with generic
  `PrintE` unresolved.
- `builtin_putchar_byte` and string/print builtin fixtures remain green or improve.
- Builtin calls still preserve effects/barriers in MIR and source listing.

Suggested commit:

```text
mir6502: resolve console builtin targets
```

## Milestone 4: Cover Remaining Currently Modeled Builtins

Goal: ensure the resolver covers every builtin ID currently emitted by MIR/NIR.

Scope:

- Add entries for all remaining known builtin IDs.
- For each, decide resolved/deferred/unsupported.
- Add focused fixtures or assertions for every status category.

Rules:

- Unsupported builtins should fail with a precise diagnostic that names the
  builtin and reason.
- Deferred builtins should name the missing runtime symbol/config/address.
- Resolved builtins should emit through the same tracked emission path as normal
  calls or a documented special sequence.

Acceptance criteria:

- No fixture fails with generic `builtin call target X is unresolved`.
- All builtin IDs are handled explicitly.
- Source-listing failures, if any, are deliberate unsupported/deferred messages.

Suggested commit:

```text
mir6502: classify remaining builtin targets
```

## Milestone 5: Refresh Fixture Dump And Re-bucket

Goal: confirm builtin failures are gone or precise.

Run:

```sh
scripts/dump_mir6502_fixtures.sh
```

Record:

```text
fixtures total
materialized MIR successes
source-listing successes
command failures
remaining error filenames grouped by diagnostic text
```

Expected progress:

- `builtin call target X is unresolved` diagnostics disappear;
- source-listing successes increase or remain stable with clearer unsupported
  diagnostics;
- remaining failures should likely be machine-block references, explicit
  unsupported payloads, or final edge cases.

Suggested commit:

```text
mir6502: refresh builtin target gap snapshot
```

## Suggested Codex Task

```text
Implement MIR6502 complete builtin target resolution.

Scope:
- Inventory all currently modeled builtin IDs / builtin call targets.
- Add a central builtin target resolver.
- Cover Put, PutE, Print, PrintE, and every other already-modeled builtin.
- For each builtin, classify it as resolved, deferred, or unsupported.
- Emit resolved builtins through tracked emission or documented call sequences.
- Produce precise deferred/unsupported diagnostics for known but not-yet-emittable
  builtins.

Do not implement:
- new language builtins not already represented in MIR/NIR;
- source-name-based builtin recognition in emission;
- machine-block reference emission;
- indirect calls;
- call inlining;
- peepholes;
- broad runtime redesign.

Acceptance:
- `os_call_opaque_barrier.source-listing.err` no longer fails with generic
  unresolved `PrintE`.
- No fixture fails with generic `builtin call target X is unresolved`.
- Builtin fixtures either emit or fail with precise unsupported/deferred reasons.
- Direct calls, indirect calls, pointer/array, compare/branch, and machine-block
  fixture behavior does not regress.

Required checks:
- cargo test -q mir6502 --lib
- cargo test -q mir6502_fixtures_match_snapshots
- scripts/dump_mir6502_fixtures.sh

Suggested commit:
- mir6502: resolve builtin call targets
```
