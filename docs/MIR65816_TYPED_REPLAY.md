# Native 65816 typed replay

Slice 6 of the [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
makes fresh tracked replay authoritative for emitted code. It preserves the
existing selection decisions, public ABI, frame/DP allocation, stack guards,
interrupt reserves and serialized formats. No new optimization is enabled.

## Contract

The selector records typed instructions and compiler requests through the
existing tracked facade. [Replay](../src/mir65816/emit/replay.rs) starts with a
fresh facade and the native entry contract. It executes request inputs through
the same dispatcher, recomputing effects, modes, home generations, consumed
captures, MIR-entry obligations and X refresh checks. It never seeds the new
tracker from an old state snapshot or treats a recorded success as permission.

Boolean outcomes are observations on the matching request-end site. Both
successful and failed single-use consumption must match the freshly recomputed
result. The selected CFG verifier rejects missing decisions or decisions on
the wrong kind of site. A compound request regenerates its nested requests and
instructions; replay checks that entire sequence and skips the original children.
This preserves their single execution and avoids recursively capturing replay.

Labels retain their symbolic identity. Source start/end events carry MIR
identity, including fused terminators; replay uses fresh byte cursors to derive
their ranges. It regenerates bytes, symbolic/PER fixups, labels, MIR spans and
transfers, conditional branches, instruction boundaries and optional effect/state
traces. Exact replay retains the routine owner/allocation/generation identity.
Old encoded ranges are not inputs to regeneration, so a finalized short branch
can be replayed and finalized again without stale offsets.

The immutable input already owns its verified CFG: initial construction,
`edited` and replay publication all validate private actions and graph together.
Layout can only remap derived encoded positions. Full and prefix replay therefore
do not rebuild the input graph. They still recompute all compiler decisions and
compare exact action/child/environment observations. The fresh output recording
passes its own selected-CFG validation and reconciliation. The
unchanged layout finalizer then runs once on that output. Both flat-image and
o65 serialization consume the same finalized result. A feature-gated direct
reference path remains available for qualification.

This slice accepts compiler-owned verified recordings. It does not introduce a
mutable public selected-code API, a general untrusted-plan executor or rollback.
Checked rewrite transactions remain slice 7; those transactions must validate
edits and regenerate observations before publication. Existing emitter internal
assertions are unchanged; this slice does not use panic-catching as validation.

Selection still creates private provisional bytes while making its existing
choices, and replay regenerates the published bytes. Host compile-time and peak
memory overhead are recorded in the [foundation qualification](MIR65816_ANALYSIS_REWRITE_QUALIFICATION.md)
and [emission simplification](MIR65816_EMISSION_SIMPLIFICATION.md).

## Qualification

Before cutover, shadow replay passed 162 native emitter/proof library tests and
five scoped native tests. After cutover, the
[qualification record](abi/action65816-typed-replay-qualification.json) captures:

- 162 emitter/proof library tests and 61 affected root integration tests.
- 129 full native tests in each of debug and release, covering raw/optimized
  execution, calls/helpers, guards, overflow, preemption and relocated o65.
- An isolated CRLF checkout with 22 converted text fixtures, a fresh build of
  the emission-boundary test and all 129 native tests.
- Exact equality of all 657 native artifacts across debug, release and CRLF,
  preserving all 656 prior native artifacts. Only the replay inventory is new.
- All 462 qualified compiler/fixture input hashes, with the CRLF variants
  explicitly identified.

Five replay unit tests cover mode omissions, successful/failed consumption,
home generations, stale capture identity, invalid decision placement, repeated
replay, short/long dispatch, fused spans and indirect PER continuations. Two
native replay tests compare the direct, replay and ordinary production paths.
The [inventory](benchmarks/65816-analysis-rewrite/replay-summary.json) covers
28 raw/optimized corpus builds with tracing both off and on: 56 combinations,
all 20 request kinds, 192 false and 156 true decisions, and 2,797 state snapshots.
Code metadata and traces match exactly; repeated replay also preserves sites and
CFG successors. Modified public bytes cannot bypass selected-code reconciliation.

The [frozen corpus gate](benchmarks/65816-analysis-rewrite/slice6-equality.json)
authenticates and preserves all 224 artifact files across 56 Action/vbcc builds,
all 264 full comparison records and 528 executions per host profile. All 132
Action records remain correct. The sole failure remains optimized vbcc `unlink`
vector 0. Corpus generation also verifies 28 CRLF Action builds. Optimized
rotation remains 126 bytes / 735 cycles / eight stack bytes; sum-loop(13) remains
120 bytes / 1,092 cycles / six stack bytes.

No NIR, semantic lowering, shared runtime or existing fixture contents changed.
The required emitter/native scope ran; a repository-wide compiler test or NIR
sweep was not needed. Equality tooling is unchanged, so its previously qualified
mutation tests were not repeated. Temporary CRLF source/build caches were removed
after preserving the compact qualification evidence.
