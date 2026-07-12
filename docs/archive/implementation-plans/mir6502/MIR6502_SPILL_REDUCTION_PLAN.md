# MIR6502 Spill Reduction Plan

Snapshot date: 2026-06-08.

TN is now much smaller than the first MIR6502 listings, but spill variables are
still a major source of code and data bloat. The next work should reduce the
number of spill homes created, not just forward around loads and stores after
the fact.

## Goals

- Reduce routine spill data labels.
- Reduce spill reads/writes and adjacent `STA m; LDA m` pairs.
- Shorten temporary lifetimes before MIR temps are assigned storage homes.
- Keep short-lived byte and pointer values in A/X/Y or fixed scratch ZP when
  safe.

## Slices

### 1. Measure Origins and Clean Early Temps

Add spill-origin/use reporting and a conservative pre-materialization cleanup:

- rank spill labels by reads, writes, first access routine, and first access
  address;
- remove side-effect-free temp definitions with no later use;
- collapse simple single-use temp aliases before materialization assigns homes.

### 2. Shorten Temp Lifetimes

Sink address, index, and arithmetic materialization closer to the consuming op.
Avoid naming temps for one-use operands when the consumer can read the source
directly.

Implemented first conservative slice:

- sink single-use pure arithmetic/constant producers to immediately before the
  only consumer when this avoids unrelated work or calls extending the lifetime;
- keep `LeaAddr` in place for array/index fusers, because moving it can hide
  established address-consumer idioms.

### 3. Call-Aware Liveness

Classify temps as not-live-across-call, rematerializable-across-call, argument
only, or truly live-across-call. Rematerialize constants and addresses after
calls instead of reserving routine spill homes.

Implemented first conservative slice:

- rematerialize constants through the pre-materialization cleanup when the temp
  has one post-call consumer;
- rematerialize one-use `LeaAddr` temps directly into later call arguments,
  avoiding live-across-call spill homes for address-only call args.

### 4. Join/Branch Placement

Avoid routine-level spill homes for branch-local values. Sink stores to the edge
or branch where the value is consumed, and keep same-home values through joins
when both predecessors agree.

### 5. Better Home Selection

Prefer A/X/Y for immediate consumers, existing prepared pointer scratch, small
block-local ZP homes, and only then routine spill storage. Track word pairs as
pairs so byte lanes are not spilled independently unless needed.

### 6. Array and Pointer Fusion

Fuse base pointer preparation, index scaling, and indirect load/store consumers.
Keep pointer/index state in `$AC/$AD`, `$AE/$AF`, and Y through the whole
mini-chain when no clobber intervenes.

## Regression Loop

For each slice, compare:

- total spill labels;
- spill reads/writes;
- `LDA+STA` count;
- adjacent `STA m; LDA m` pairs;
- top spill-pressure routines;
- TN instruction count and code bytes.
