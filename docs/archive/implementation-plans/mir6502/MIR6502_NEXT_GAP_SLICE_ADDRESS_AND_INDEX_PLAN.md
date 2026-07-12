# MIR6502 Next Gap Slice: Address Values And Dynamic Byte Indexes

Snapshot date: 2026-06-03.

This note is a Codex-ready implementation plan for the next focused MIR6502
materialization gap slice after the first consumer-home work.

It is intentionally narrower than `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`.
The goal is to close the next visible cluster from the updated fixture dump:
address-value virtual temps and dynamic inline byte/char array index addresses.

Related documents:

- `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`
- `docs/MIR6502_ADDRESS_CONSUMER_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/MIR6502_FULL_LANGUAGE_EXPANSION_PLAN.md`

## Current Snapshot

The updated fixture dump summary is:

```text
fixtures: 115
materialized MIR succeeded: 86
source listings succeeded: 82
command failures: 62
```

Representative remaining errors:

```text
address_of_local.source-listing.err:
  pre-emission MIR cannot contain virtual temp `v0`

byte_array_dynamic_read.materialized-mir.err:
  pre-emission MIR cannot contain virtual temp `v0`
  pre-emission MIR cannot contain virtual temp `v1`
  computed index addresses must be materialized before emission

card_array_dynamic_read.materialized-mir.err:
  pre-emission MIR cannot contain word-width pseudo ops
  pre-emission MIR cannot contain virtual temp `v2`
  pre-emission MIR cannot contain virtual temp `v1`
  pointer index addresses must be materialized before emission
```

This slice should address the first two classes directly and prepare the third
class without attempting full word/descriptor-backed dynamic indexing yet.

## Goal

Implement reusable materialization paths for:

1. address values such as address-of local/global/static values;
2. dynamic inline `BYTE` / `CHAR` array indexes where the backing storage is
   inline and the element width is one byte.

The target is not to support every array form. The target is to make the next
low-risk address/index materialization path solid and reusable.

## Red Lines

Do not include these in this slice:

- dynamic `CARD` / `INT` array indexing;
- descriptor-backed array indexing;
- unsized/pointer-backed dynamic arrays, unless a tiny helper is required but not
  activated broadly;
- signed `INT` comparisons;
- short-circuit boolean materialization;
- indirect calls;
- peepholes;
- general register allocation;
- global constant propagation;
- alias-sensitive forwarding;
- broad zero-page allocation.

If a fixture outside the intended slice becomes easy to fix only because this
work naturally unlocks it, keep the code generic but do not widen the acceptance
criteria.

## Milestone 1: Address-Value Consumer Materialization

Goal: eliminate virtual temps for simple address values consumed by stores, calls,
returns, or pointer assignments.

Support address values for:

```text
local storage address
global storage address
static storage address
absolute-backed symbol address, if represented
routine address, only if the existing MIR already supports it cleanly
```

Implement or extend helpers equivalent to:

```text
materialize_address_value_to_word_home(address_value, destination_home)
materialize_address_value_to_mem(address_value, dst_mem)
materialize_address_value_to_call_arg(address_value, arg_home)
materialize_address_value_to_return_home(address_value)
```

Rules:

- Address values are word values with low/high bytes.
- The materializer must write low/high bytes directly into the consumer home when
  the consumer is known.
- Do not force address values through ordinary virtual temps/spills just to copy
  them afterward.
- Emission must not recover source-level address meaning.

Acceptance criteria:

- `address_of_local.source-listing.err` disappears.
- Existing address-of global/static fixtures remain green or improve.
- Pointer assignment from address-of local/global does not introduce a word-temp
  spill when the destination is known.
- No new failures appear in scalar store, pointer deref, or direct-call fixtures.

Suggested commit:

```text
mir6502: materialize address values directly
```

## Milestone 2: Dynamic Inline Byte Index Address Materialization

Goal: materialize computed index addresses for inline `BYTE` / `CHAR` arrays
before pre-emission.

Scope:

Support dynamic indexing when all of these are true:

```text
array backing storage is inline
array element width is one byte
index value is byte-sized or can already be safely materialized to a byte index
base storage has a concrete direct address form or layout placement
```

Materialization strategy:

```text
1. materialize the index value into the selected index home;
2. select the inline byte-array indexed address form;
3. materialize the load/store through that selected address form;
4. ensure no computed-index abstract address survives to pre-emission.
```

The first implementation may use the existing backend's preferred index register
or a structured MIR indexed address form. The important invariant is that the
index/address strategy is selected before emission.

Rules:

- Constant indexes should continue to lower to direct offsets; do not regress
  constant-index fixtures.
- Dynamic byte indexes should not be staged through word address spills.
- Do not silently change absolute-indexed addressing into zero-page-indexed
  addressing if wraparound semantics could differ.
- The tracker may encode an already-selected address form but must not select the
  semantic address strategy.

Acceptance criteria:

- `byte_array_dynamic_read.materialized-mir.err` disappears.
- `byte_array_dynamic_write.materialized-mir.err` disappears if present.
- `char_array_dynamic_read.materialized-mir.err` disappears if it is the same
  inline byte-width case.
- Materialized MIR contains a concrete indexed address/load/store strategy, not a
  computed-index abstract address.
- Existing constant-index byte-array fixtures remain green.

Suggested commit:

```text
mir6502: materialize dynamic inline byte indexes
```

## Milestone 3: Prepare Word Dynamic Indexing Without Implementing It Fully

Goal: make the remaining `CARD` / `INT` dynamic index failures more specific and
ready for the next slice.

Do not implement full word dynamic indexing in this milestone. Instead:

- identify the shared helper boundary for index scaling;
- add precise diagnostics for unsupported word dynamic indexes if they still reach
  pre-emission;
- ensure the byte-index path is not hardcoded in a way that prevents adding word
  scaling next.

Expected next slice after this note:

```text
mir6502: materialize dynamic word indexes
```

That later slice should handle:

```text
CARD/INT element width = 2
index scaling by 2
low/high lane load/store from computed element address
pointer-backed and descriptor-backed dynamic forms in separate sub-slices
```

Acceptance criteria:

- `card_array_dynamic_read` may still fail, but the failure should be specific to
  word index scaling or pointer-index address materialization, not generic
  virtual-temp leakage from the byte-index path.
- Byte dynamic array support remains green.

Suggested commit:

```text
mir6502: isolate dynamic word index materialization gap
```

## Milestone 4: Refresh The Fixture Dump

Goal: measure the effect of this slice and re-bucket the remaining failures.

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

Acceptance criteria:

- Materialized-MIR successes increase from 86.
- Source-listing successes increase from 82, or at minimum do not regress.
- Dynamic byte/char index errors are gone or replaced by a more specific emission
  unsupported form.
- Remaining dynamic word/pointer/descriptor indexes are clearly separable from
  byte inline dynamic indexes.

Suggested commit:

```text
mir6502: refresh address and index gap snapshot
```

## Suggested First Codex Task

```text
Implement MIR6502 address-value materialization and dynamic inline byte-index
materialization.

Scope:
- Materialize address-of local/global/static values directly into consumer homes.
- Materialize dynamic inline BYTE/CHAR array read/write addresses before
  pre-emission.
- Keep constant-index lowering unchanged.
- Add focused fixtures or update existing fixture expectations for address_of_local,
  byte_array_dynamic_read, byte_array_dynamic_write, and char_array_dynamic_read.

Do not implement:
- CARD/INT dynamic indexing;
- descriptor-backed arrays;
- unsized/pointer-backed dynamic arrays;
- signed compares;
- short-circuit booleans;
- indirect calls;
- peepholes.

Acceptance:
- address_of_local.source-listing.err disappears.
- byte_array_dynamic_read.materialized-mir.err disappears.
- byte_array_dynamic_write.materialized-mir.err disappears if present.
- char_array_dynamic_read.materialized-mir.err disappears if it is the same
  inline byte-width case.
- Existing scalar, pointer deref, direct-call, store-consumer, and constant-index
  array fixtures remain green.

Required checks:
- cargo test -q mir6502 --lib
- cargo test -q mir6502_fixtures_match_snapshots
- scripts/dump_mir6502_fixtures.sh

Suggested commit:
- mir6502: materialize address values and byte indexes
```
