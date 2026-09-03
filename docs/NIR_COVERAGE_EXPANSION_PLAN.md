# NIR Coverage Expansion Plan

Snapshot date: 2026-09-03.

Status: in progress. Slices 1 through 4 are implemented.

## Objective

Make NIR contract coverage explicit, reviewable, and resilient as new targets
and language constructs are added. Coverage is measured over structured NIR
variants rather than inferred from snapshot text or raw Rust line coverage.

The coverage suite has four complementary responsibilities:

1. focused source-to-NIR contract snapshots;
2. directly constructed invalid NIR for verifier rejection;
3. selected before/after optimizer contracts and preservation barriers;
4. broad lowered and optimized NIR verification across the source corpus.

## Slices

### Slice 1: typed cases and structural inventory

Status: complete. The snapshot harness now uses typed cases that select source,
target, stage, and expected output independently. An exhaustive structured NIR
visitor records the variants reached by the registered cases, and a committed
inventory makes additions and losses visible in review.

### Slice 2: focused fixture organization

Status: complete. The former lexical-declarations catch-all is split into
focused local-storage-view, local-aggregate-declaration, and lexical-type-scope
cases. Lexical foreign-symbol resolution remains in the already focused
lexical-block fixture. Production compiler behavior is unchanged.

### Slice 3: positive executable shapes

Status: complete. Focused cases now cover the remaining arithmetic and compare
operators, pointer casts and offsets, direct/indirect/runtime call forms,
volatile aggregate copies, link values, and typed data relocations. Small
verified structural probes cover valid NIR without a current source producer:
block parameters, `Exit`, runtime bindings, absolute places, global aliases,
routine-address initialization, and conservative effect forms. A required
executable-feature floor prevents a snapshot refresh from silently accepting a
coverage loss. `Open`, `Unsupported`, and the legacy untyped `Param` and
`GlobalAddr` values remain outside the executable contract.

### Slice 4: cross-target contracts

Status: complete. The focused matrix snapshots activation/storage, aggregate
layout, callable and data-pointer widths, descriptor widths, scalar data
fragments, field offsets, alignment, and endianness on Atari 6502, 65816 native,
65816 small, and 68000. A separate target-feature floor keeps all four profiles
and both activation/storage models represented without applying every fixture
to every target.

### Slice 5: verifier-negative matrix

Use directly constructed or deliberately mutated NIR to cover invalid stable
IDs, CFG and use-def errors, types, calls, storage facts, aliases, relocations,
effects, and target/address-space mismatches. Pair rejection tests with nearby
accepted shapes.

### Slice 6: optimizer transformations and barriers

Add selected before/after cases for constant folding, propagation, branch and
CFG cleanup, dead temps, promotion, and home elision. Verify both forms and
cover calls, volatility, aliases, address escape, REAL, and foreign code as
preservation barriers.

### Slice 7: broad corpus gate

Extend the NIR sweep to verify both lowered and optimized programs across NIR,
SemIR, MIR6502, runtime, and module-aware sample entry points. Report explicit
load, semantic, lowering, verification, and optimization outcomes.

## Completion Gates

- Every executable NIR variant is covered or explicitly classified as
  construction-only or quarantined.
- Target-sensitive representations have focused coverage for every relevant
  target profile.
- Every verifier-rule family has a rejection test.
- Both lowered and optimized NIR verify across the supported source corpus.
- Fixture changes remain separate from production compiler behavior changes.
