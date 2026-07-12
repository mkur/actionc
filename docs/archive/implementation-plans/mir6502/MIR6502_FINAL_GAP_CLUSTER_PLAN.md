# MIR6502 Final Fixture Gap Cluster Plan

Snapshot date: 2026-06-03.

This note clusters the current remaining MIR6502 fixture failures and proposes the
implementation order for closing them. It is intended to be used directly as a
Codex execution plan.

Related documents:

- `docs/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/MIR6502_BUILTIN_TARGET_RESOLUTION_PLAN.md`
- `docs/MIR6502_INDIRECT_CALL_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`

## Current Remaining Error Files

Current failures:

```text
unsized_byte_array_dynamic_write.materialized-mir.err
machine_block_label_ref.source-listing.err
for_loop_word.source-listing.err
record_field_byte_store.source-listing.err
local_card_array_dynamic_read.source-listing.err
unsized_byte_array_dynamic_write.source-listing.err
pass_byte_array_param.source-listing.err
local_card_array_dynamic_read.materialized-mir.err
pass_card_array_param.source-listing.err
global_scalars_layout.source-listing.err
global_scalars_layout.materialized-mir.err
machine_block_global_ref.source-listing.err
for_loop_word.materialized-mir.err
unsized_byte_array_dynamic_read.materialized-mir.err
unsized_byte_array_dynamic_read.source-listing.err
```

These are no longer broad MIR materialization failures. They group into a small
number of focused backend gaps.

## Cluster 1: Unsized BYTE Array Dynamic Indexing

Files:

```text
unsized_byte_array_dynamic_read.materialized-mir.err
unsized_byte_array_dynamic_read.source-listing.err
unsized_byte_array_dynamic_write.materialized-mir.err
unsized_byte_array_dynamic_write.source-listing.err
```

Likely gap:

```text
BYTE ARRAY p
x = p(i)
p(i) = x
```

This is pointer-backed byte-array dynamic indexing. Inline byte indexes and word
pointer indexes have been handled in earlier slices, but unsized byte arrays still
need the pointer-cell dynamic byte-index path.

Implementation direction:

```text
mir6502: materialize unsized byte array dynamic indexes
```

Scope:

- load the unsized byte array pointer cell as the base pointer;
- materialize the dynamic byte index;
- stage base + index into the selected zero-page pointer pair or equivalent
  selected address form;
- emit/materialize byte read/write through that staged address;
- do not use ordinary word temp/spill for the element address;
- do not implement constant propagation or peepholes.

Acceptance criteria:

- both unsized byte dynamic read and write materialize to pre-emission;
- source listing succeeds for both fixtures;
- existing inline byte dynamic indexes and pointer dereference fixtures remain
  green.

## Cluster 2: Local Descriptor-Backed CARD Array Dynamic Read

Files:

```text
local_card_array_dynamic_read.materialized-mir.err
local_card_array_dynamic_read.source-listing.err
```

Likely gap:

```text
local CARD ARRAY a(...)
x = a(i)
```

This is probably a local descriptor/backing-storage variant. Global/sized word
array dynamic reads may already work, but local descriptor placement or local
backing pointer resolution is incomplete.

Implementation direction:

```text
mir6502: materialize local descriptor-backed word array indexes
```

Scope:

- reuse the existing dynamic word-index helper;
- resolve the local descriptor backing pointer from MIR storage/layout facts;
- stage backing pointer + index * 2;
- load low/high element bytes into the consumer home;
- do not source-name lookup local array storage;
- fail with a precise diagnostic if local backing facts are absent.

Acceptance criteria:

- `local_card_array_dynamic_read` materializes and emits, or fails only on a
  precise missing-storage-fact diagnostic;
- global descriptor-backed and unsized word array dynamic index fixtures do not
  regress.

## Cluster 3: Array Parameter Passing Emission

Files:

```text
pass_byte_array_param.source-listing.err
pass_card_array_param.source-listing.err
```

Materialized MIR appears to succeed, so this is probably an ABI/emission bridge
issue rather than basic materialization.

Likely gap:

```text
caller passes array parameter
callee receives pointer/backing pointer in ABI homes
```

Implementation direction:

```text
mir6502: emit array parameter ABI homes
```

Scope:

- inspect exact source-listing errors before coding;
- ensure byte-array arguments pass base pointer;
- ensure card-array arguments pass backing pointer rather than descriptor address
  where Action! ABI requires it;
- resolve ABI homes through existing call argument machinery;
- do not redesign SArgs or call ABI broadly.

Acceptance criteria:

- `pass_byte_array_param.source-listing.err` disappears;
- `pass_card_array_param.source-listing.err` disappears;
- direct scalar call ABI fixtures remain green;
- array dynamic indexing fixtures remain green.

## Cluster 4: Structured Machine Block References

Files:

```text
machine_block_label_ref.source-listing.err
machine_block_global_ref.source-listing.err
```

Known gap:

```text
machine block reference `Target` is not emit-ready
machine block reference `data` is not emit-ready
```

Implementation direction:

```text
mir6502: emit structured machine block references
```

Scope:

- resolve label references inside machine blocks using MIR/object layout facts;
- resolve global/static/routine references inside machine blocks;
- emit reference bytes in the form already represented by MIR;
- preserve machine-block effects and barriers;
- do not parse machine-block source text;
- do not recover source names in emission.

Acceptance criteria:

- `machine_block_label_ref.source-listing.err` disappears;
- `machine_block_global_ref.source-listing.err` disappears;
- existing scalar, array, pointer, call, builtin, and indirect-call fixtures
  remain green.

## Cluster 5: Word FOR Loop Control

Files:

```text
for_loop_word.materialized-mir.err
for_loop_word.source-listing.err
```

Likely gap:

```text
FOR loop with CARD/INT loop variable or bound
```

This may reduce to a missing combination of:

- word compare branch materialization;
- word increment/decrement store-consumer materialization;
- loop-bound temp materialization;
- branch target layout for normalized FOR-loop CFG.

Implementation direction:

```text
mir6502: materialize word for-loop control
```

Scope:

- inspect the exact materialized-MIR error before coding;
- identify whether the failure is compare, increment, or loop-bound storage;
- reuse existing word compare and word store-consumer helpers;
- avoid adding a special FOR-only lowering if normalized NIR/MIR can use existing
  loop primitives;
- do not add branch layout optimization or peepholes in this slice.

Acceptance criteria:

- `for_loop_word.materialized-mir.err` disappears;
- `for_loop_word.source-listing.err` disappears;
- while-loop and scalar branch fixtures remain green.

## Cluster 6: Storage/Layout And Record Field Edge Cases

Files:

```text
global_scalars_layout.materialized-mir.err
global_scalars_layout.source-listing.err
record_field_byte_store.source-listing.err
```

Likely gaps:

- `global_scalars_layout` is probably a storage/layout or map-emission edge case;
- `record_field_byte_store` likely means direct record-field memory placement or
  record field offset emission is incomplete.

Implementation directions:

```text
mir6502: fix global scalar layout emission
mir6502: emit record field direct stores
```

Scope:

- inspect exact errors before coding;
- for global layout, ensure all global scalar storage/backing facts are concrete
  before pre-emission and object layout;
- for record fields, ensure field offsets are resolved to direct memory offsets in
  MIR/materialization and then emitted through normal direct memory emission;
- do not use field names as executable identity;
- do not redesign record layout.

Acceptance criteria:

- global scalar layout fixture materializes and emits;
- record field byte store emits through direct memory with the correct offset;
- existing scalar/global/record fixtures remain green.

## Recommended Implementation Order

Recommended order:

```text
1. structured machine-block reference emission
2. unsized BYTE array dynamic read/write
3. array parameter source-listing failures
4. local CARD array dynamic read
5. word FOR loop control
6. global layout + record field edge cases
```

Rationale:

- Machine-block references are isolated and should close two source-listing
  failures without touching core materialization.
- Unsized byte dynamic indexing is a reusable array/pointer path.
- Array parameter failures likely reuse array address/value materialization.
- Local descriptor-backed CARD arrays are a narrower storage-layout variant.
- Word FOR loops require control-flow care and should not be mixed with array
  work.
- Global layout and record field failures are likely final edge-case cleanup.

## Suggested First Codex Task

```text
Implement MIR6502 structured machine-block reference emission.

Goal:
- Resolve label and global/static/routine references inside machine blocks using
  MIR/object layout facts.
- Emit the reference bytes in the already-represented form.
- Preserve machine-block effects and barriers.
- Do not parse machine-block source text.
- Do not recover source names in emission.
- Add or update regressions for machine_block_label_ref and
  machine_block_global_ref.

Acceptance:
- machine_block_label_ref.source-listing.err disappears.
- machine_block_global_ref.source-listing.err disappears.
- Existing scalar, array, pointer, call, builtin, indirect-call, and compare
  fixtures remain green.

Required checks:
- cargo test -q mir6502 --lib
- cargo test -q mir6502_fixtures_match_snapshots
- scripts/dump_mir6502_fixtures.sh

Suggested commit:
- mir6502: emit structured machine block references
```

## Follow-Up Codex Task

```text
Implement MIR6502 unsized BYTE array dynamic indexing.

Goal:
- Materialize pointer-backed BYTE array dynamic reads and writes.
- Reuse pointer-cell and dynamic-index materialization helpers.
- Avoid ordinary word temp/spill for the computed element address.
- Keep inline byte dynamic indexes and word pointer indexes green.

Acceptance:
- unsized_byte_array_dynamic_read materializes and emits.
- unsized_byte_array_dynamic_write materializes and emits.
- Existing pointer dereference, dynamic word index, and call fixtures remain green.

Suggested commit:
- mir6502: materialize unsized byte array dynamic indexes
```
