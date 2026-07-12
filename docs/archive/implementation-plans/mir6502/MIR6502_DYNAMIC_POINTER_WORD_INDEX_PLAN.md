# MIR6502 Dynamic Pointer Word-Index Materialization Plan

Snapshot date: 2026-06-03.

This note is a Codex-ready implementation plan for the next MIR6502 materialization
slice after address-value and inline byte-index materialization.

It targets the remaining dynamic word-index address failures, especially pointer-
backed and descriptor-backed `CARD` / `INT` array element access.

Related documents:

- `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`
- `docs/MIR6502_NEXT_GAP_SLICE_ADDRESS_AND_INDEX_PLAN.md`
- `docs/MIR6502_ADDRESS_CONSUMER_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_FULL_LANGUAGE_EXPANSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/ACTION_STORAGE_LAYOUT.md`

## Current Snapshot

The latest fixture dump summary is:

```text
fixtures: 115
materialized MIR succeeded: 96
source listings succeeded: 90
command failures: 44
```

Representative remaining errors:

```text
card_array_dynamic_read.materialized-mir.err:
  dynamic pointer word index addresses must be materialized before emission
  dynamic pointer word index addresses must be materialized before emission

unsized_card_array_dynamic_read.materialized-mir.err:
  dynamic pointer word index addresses must be materialized before emission
  dynamic pointer word index addresses must be materialized before emission
```

This means the previous inline byte/char dynamic index slice helped, but word
indexing through pointer-like backing still reaches pre-emission as an abstract
address form.

## Goal

Materialize dynamic `CARD` / `INT` element addresses before pre-emission when the
array access is pointer-backed or descriptor-backed.

A word element index must select a concrete address strategy before emission:

```text
base pointer + index * 2 -> staged element address -> low/high byte access
```

The materializer must emit or produce MIR for two byte-lane accesses to the
selected element address. Pre-emission MIR must not contain abstract dynamic
pointer word-index address forms.

## Red Lines

Do not mix this slice with unrelated gaps.

Out of scope:

- signed `INT` relational compares;
- short-circuit boolean materialization;
- indirect calls and callable values;
- builtins/runtime/OS calls;
- general zero-page allocation;
- global register allocation;
- pointer constant propagation;
- replacing pointer-backed access with direct absolute access;
- descriptor layout redesign;
- peepholes.

If a descriptor-backed case requires a missing descriptor fact, add a precise
unsupported diagnostic and keep the pointer-backed case working.

## Background Rules

From the storage layout contract:

```text
CARD and INT elements are two bytes: low byte, then high byte.
Unsized arrays are two-byte pointer variables.
Sized non-byte arrays are descriptor-backed.
Array parameters behave as pointer-backed arrays inside the callee.
Dynamic non-byte array indexing follows the pointer/descriptor path through a
zero-page pointer pair and indirect-indexed byte accesses.
```

The MIR materializer owns this address strategy. Tracked emission must only encode
already-selected loads/stores and must not recover array or pointer meaning.

## Milestone 1: Define A Reusable Dynamic Word-Index Address Helper

Goal: isolate the address calculation strategy behind a helper that all word-array
paths can reuse.

Add a helper equivalent to:

```text
materialize_dynamic_word_index_address(base, index, element_width=2, out)
```

Inputs should be target-shaped, not source-shaped:

```text
base pointer value or pointer-cell memory
index value
selected zero-page pointer pair / scratch policy
array element width = 2
```

Output should be a staged address or concrete indirect address form that can feed
low/high byte loads/stores.

Rules:

- Do not require the caller to first materialize the element address into an
  ordinary word temp/spill.
- Do not return an abstract computed address to pre-emission MIR.
- Keep the helper conservative around calls/barriers/effects.

Acceptance criteria:

- There is one helper path for dynamic word element address materialization.
- Both unsized arrays and descriptor-backed arrays can call it once they provide
  a base pointer value.
- Existing pointer dereference and byte dynamic-index fixtures remain green.

Suggested commit:

```text
mir6502: add dynamic word index address helper
```

## Milestone 2: Unsized `CARD` / `INT` Dynamic Index Reads

Goal: support the simplest pointer-backed word array dynamic read first.

Representative source shape:

```text
unsized CARD or INT array pointer
byte or word index expression
read array(index) into a word consumer
```

Materialization strategy:

```text
1. load the array pointer cell as the base pointer;
2. materialize the index value;
3. scale index by two;
4. stage base + scaled index into the selected zero-page pointer pair;
5. load low byte at offset 0;
6. load high byte at offset 1;
7. materialize the two bytes directly into the consumer home.
```

For the first implementation, support the common case where the final consumer is
a word store destination. Other consumers can be routed through the same byte-lane
value helpers later.

Acceptance criteria:

- `unsized_card_array_dynamic_read.materialized-mir.err` disappears.
- The materialized MIR contains concrete staged/indirect low/high byte reads.
- No dynamic pointer word-index address reaches pre-emission.
- No ordinary element-address word spill is introduced.
- Existing unsized byte-array and pointer dereference fixtures remain green.

Suggested commit:

```text
mir6502: materialize unsized word array dynamic reads
```

## Milestone 3: Sized `CARD` / `INT` Dynamic Reads Via Descriptor/Backing Pointer

Goal: support descriptor-backed word array dynamic reads after the unsized pointer
case is stable.

Representative source shape:

```text
sized CARD or INT array
read array(index) into a word consumer
```

Materialization strategy:

```text
1. load descriptor backing pointer bytes 0..1;
2. materialize the index value;
3. scale index by two;
4. stage backing pointer + scaled index into the selected zero-page pointer pair;
5. load low/high element bytes;
6. materialize directly into the consumer home.
```

Rules:

- Descriptor/backing facts must come from MIR storage/layout records.
- Do not reconstruct descriptor layout from source text.
- If the descriptor does not carry enough backing facts, emit a precise diagnostic
  before pre-emission.

Acceptance criteria:

- `card_array_dynamic_read.materialized-mir.err` disappears for the descriptor-
  backed fixture if the fixture uses a sized non-byte array.
- Descriptor-backed base pointer loading is visible in materialized MIR.
- No abstract pointer index address reaches pre-emission.
- Existing initialized descriptor-layout fixtures do not regress.

Suggested commit:

```text
mir6502: materialize descriptor-backed word array dynamic reads
```

## Milestone 4: Dynamic Word Index Writes

Goal: extend the same address helper to writes.

Representative fixtures:

```text
card_array_dynamic_write
unsized_card_array_dynamic_write
local_card_array_dynamic_read/write if present
```

Materialization strategy:

```text
1. materialize element address using the shared dynamic word-index helper;
2. materialize source word into low/high byte values;
3. store low byte at offset 0;
4. store high byte at offset 1.
```

Acceptance criteria:

- `card_array_dynamic_write.materialized-mir.err` disappears.
- `unsized_card_array_dynamic_write.materialized-mir.err` disappears.
- Word dynamic read fixtures remain green.
- No ordinary word element-address spill is introduced.

Suggested commit:

```text
mir6502: materialize dynamic word array writes
```

## Milestone 5: Local And Large Array Variants

Goal: extend the same mechanism to local descriptor-backed arrays and large byte
arrays only if the previous milestones expose the required storage facts.

Potential fixtures:

```text
local_card_array_dynamic_read
large_byte_array_dynamic_read
```

Rules:

- Do not special-case local arrays by source name.
- Local descriptors should feed the same descriptor-backed base pointer path.
- Large byte arrays are descriptor-backed but element width is one. If this path
  falls out naturally from the descriptor helper, enable it. Otherwise leave it
  for a separate descriptor-backed byte-array slice.

Acceptance criteria:

- Local descriptor-backed dynamic reads either materialize or fail with a precise
  missing-storage-fact diagnostic.
- Large byte array dynamic read either materializes through descriptor-backed byte
  indexing or remains explicitly deferred.
- No regression in inline byte dynamic indexing.

Suggested commit:

```text
mir6502: extend dynamic index materialization to local descriptors
```

## Milestone 6: Refresh Fixture Dump And Re-bucket

Goal: confirm the slice closed the intended failures and did not regress earlier
work.

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
remaining errors grouped by diagnostic text
```

Expected progress:

- materialized MIR successes increase from 96;
- source-listing successes increase from 90 or at least do not regress;
- dynamic pointer word-index diagnostics disappear or become narrower;
- remaining large buckets should be signed compare, short-circuit boolean, and
  indirect-call/callable materialization.

Suggested commit:

```text
mir6502: refresh dynamic word index gap snapshot
```

## Suggested First Codex Task

```text
Implement MIR6502 dynamic pointer word-index materialization for unsized CARD/INT
array reads.

Scope:
- Add a reusable helper for base pointer + index*2 element address materialization.
- Use it for unsized CARD/INT array dynamic reads first.
- Materialize the resulting element read as low/high byte accesses into the
  immediate consumer home.
- Do not implement descriptor-backed arrays in the same commit unless the helper
  naturally supports them without broadening the change.
- Do not implement signed compares, short-circuit booleans, indirect calls,
  peepholes, or zero-page allocation.

Acceptance:
- `unsized_card_array_dynamic_read.materialized-mir.err` disappears.
- Existing inline byte dynamic index fixtures remain green.
- Existing scalar, pointer deref, direct-call, store-consumer, and address-value
  fixtures remain green.

Required checks:
- cargo test -q mir6502 --lib
- cargo test -q mir6502_fixtures_match_snapshots
- scripts/dump_mir6502_fixtures.sh

Suggested commit:
- mir6502: materialize unsized word array dynamic reads
```
