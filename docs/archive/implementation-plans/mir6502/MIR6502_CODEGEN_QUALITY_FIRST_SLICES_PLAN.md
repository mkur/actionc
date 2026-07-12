# MIR6502 Codegen Quality: First Slices Plan

Snapshot date: 2026-06-03.

This note is a Codex-ready implementation plan for the first MIR6502 codegen
quality improvements after broad pre-emission/materialization coverage became
usable.

It covers the first five quality clusters observed in generated source listings:

1. multi-argument call ABI / SArgs packing;
2. data-vs-code rendering in source listings;
3. call-result store-consumer materialization;
4. constant aggregate/index direct-offset materialization;
5. record field direct load/store consumers.

Related documents:

- `docs/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/MIR6502_MATERIALIZATION_GAP_CLOSURE_PLAN.md`
- `docs/MIR6502_BUILTIN_TARGET_RESOLUTION_PLAN.md`
- `docs/bugs/MIR6502_WORD_STORE_CONSUMER_SPILL_BUG.md`
- `docs/bugs/MIR6502_LOOP_CONSUMER_SPILLS_BUG.md`
- `docs/bugs/MIR6502_CALL_ABI_FIRST_BYTE_ARG_BUG.md`

## Current Observation

The generated source listings are now broadly correct, but several recurring
quality issues remain. These are no longer broad materialization gaps. They are
local codegen, ABI, listing, and consumer-path improvements.

This plan intentionally avoids global optimization work. Do not add global
constant propagation, alias-sensitive forwarding, register allocation, branch
layout optimization, or broad peepholes in these slices.

## Red Lines

Do not implement:

- global register allocation;
- global or cross-block constant propagation;
- alias-sensitive load/store forwarding;
- dead-store elimination;
- common subexpression elimination;
- branch inversion/layout optimization;
- pointer constant dereference replacement;
- zero-page allocation redesign;
- source-name-based lowering in emission.

The first goal is to clean up obvious local codegen patterns while preserving the
MIR6502-to-tracked-emission boundary.

## Slice 1: Multi-Argument Call ABI / SArgs Packing

### Problem

The multi-argument call listing shows a suspicious overwrite pattern:

```text
LDA first_arg
LDA next_arg_low
STA argument_area+1
...
JSR callee
```

The first loaded byte can be overwritten before the call, which suggests one of:

- the first argument byte is intended to remain in `A` but gets clobbered;
- SArgs / argument-area packing is incomplete;
- the ABI byte-order model for multi-argument calls is wrong;
- the fixture is stressing an ABI path that still uses placeholder homes.

This is potentially semantic, not just cosmetic, so it is the highest-priority
quality cluster.

### Goal

Make multi-argument call packing explicit and ABI-correct.

### Scope

- Inspect the current Action! ABI model for mixed byte/word arguments.
- Confirm how argument byte 0, byte 1, byte 2, and later bytes are represented in
  MIR call homes.
- Fix materialization so each argument byte is placed exactly once into its
  required home.
- Ensure register argument homes are not clobbered by later argument setup.
- Keep scalar direct-call ABI fixtures green.

### Non-goals

- Do not redesign the full ABI.
- Do not implement call inlining.
- Do not add peepholes.
- Do not add source-name recognition in emission.

### Acceptance Criteria

- `call_many_args_sargs.lst` no longer shows a register argument load that is
  overwritten before the call.
- `sargs_many_args.lst` follows the documented Action! ABI byte-home order.
- `call_byte_arg.lst` and `call_word_arg.lst` remain green.
- Emitted byte-level tests assert the expected homes for mixed `BYTE`/`CARD`/`BYTE`
  argument cases.

### Suggested Commit

```text
mir6502: fix multi-argument call ABI packing
```

## Slice 2: Data-Vs-Code Rendering In Source Listings

### Problem

Global/static zero-filled storage is rendered as executable instructions, for
example zero bytes appear as `BRK` before routines. This makes listings look like
bad code even when object layout is correct.

### Goal

Render data ranges as data, not instructions.

### Scope

- Teach the source listing renderer to distinguish code ranges from storage/static
  data ranges.
- Render zero-filled storage as `.BYTE $00`, `.RES`, `.ZEROFILL`, or another
  explicit data directive.
- Render initialized static/string data as data directives.
- Keep routine disassembly unchanged.
- Add section/range headers if useful.

### Non-goals

- Do not change object layout in this slice unless a listing map bug requires it.
- Do not optimize storage placement.
- Do not reorder code/data.

### Acceptance Criteria

- Listings for global/storage fixtures no longer show global zero bytes as `BRK`.
- `global_scalars_layout.lst`, `zero_filled_storage.lst`, string/static layout
  fixtures, and initialized storage fixtures clearly mark data ranges.
- Existing source-listing tests remain deterministic.

### Suggested Commit

```text
mir6502: render storage bytes as data in listings
```

## Slice 3: Call-Result Store-Consumer Materialization

### Problem

Function result consumers still route through an unnecessary temporary storage
home. A word-returning function stores its result in `$A0/$A1`; the caller copies
those result bytes to a temp, then reloads the temp to copy into the final
destination.

### Goal

Materialize call results directly into their immediate consumer homes.

### Scope

Handle patterns equivalent to:

```text
Store(dst, CallResult(byte))
Store(dst, CallResult(word))
Return(CallResult(...)) where applicable
CallResult used as call argument, only if the existing call machinery supports it
```

For the first implementation, focus on `Store(dst, CallResult)`.

### Materialization Strategy

- For byte result: copy the result byte home directly to the destination byte.
- For word result: copy low/high result homes directly to destination low/high.
- Do not allocate ordinary temp/spill storage unless the call result has multiple
  uses or crosses a block/effect boundary.

### Non-goals

- Do not do call inlining.
- Do not devirtualize indirect calls.
- Do not perform global copy propagation.
- Do not reuse result homes past calls unless effects prove safety.

### Acceptance Criteria

- `func_returns_word.lst` no longer copies `$A0/$A1` through an ordinary temp
  before storing to the final destination.
- `func_returns_byte.lst` remains direct and correct.
- Direct and indirect function-call fixtures remain green.
- Call effects continue to invalidate tracked state conservatively.

### Suggested Commit

```text
mir6502: materialize call results into store consumers
```

## Slice 4: Constant Aggregate/Index Direct-Offset Materialization

### Problem

Some constant aggregate/index accesses still create address/value scaffolding even
when the final byte offsets are known. For example, constant `CARD ARRAY` element
reads can create address temporaries and copy through extra storage instead of
loading direct low/high element bytes.

### Goal

For constant aggregate/index access with known backing storage, materialize direct
byte-lane memory offsets into the immediate consumer home.

### Scope

Support direct-offset materialization for:

- inline `BYTE` / `CHAR` array constant indexes;
- inline `CARD` / `INT` array constant indexes;
- descriptor-backed constant indexes when backing storage and offset are known;
- string/char array constant indexes where storage is known;
- array element loaded directly into scalar store destination.

### Materialization Strategy

```text
constant index -> byte offset
source base + byte offset -> direct source byte lane(s)
consumer destination -> direct destination byte lane(s)
```

Examples of expected conceptual shapes:

```text
BYTE element: load base+idx -> store dst
WORD element: load base+idx*2 -> store dst.lo; load base+idx*2+1 -> store dst.hi
```

### Non-goals

- Do not implement dynamic index optimization in this slice.
- Do not add pointer constant propagation.
- Do not change array layout.
- Do not add general CSE or dead-store elimination.

### Acceptance Criteria

- Constant `CARD ARRAY` read fixtures materialize and list as direct low/high byte
  loads/stores without element-address temp scaffolding.
- Constant byte-array fixtures remain green.
- Descriptor/backing facts are used only when already present in MIR storage
  records.
- Dynamic index fixtures do not regress.

### Suggested Commit

```text
mir6502: materialize constant indexed elements directly
```

## Slice 5: Record Field Direct Load/Store Consumers

### Problem

Record field loads/stores can still copy through an ordinary temp even when the
field byte offset and consumer destination are known.

### Goal

Materialize record field load/store consumers as direct memory accesses using
numeric byte offsets from MIR/NIR layout facts.

### Scope

Support:

- byte field load into byte destination;
- byte field store from byte source;
- word field load/store where field width is two bytes;
- nested field offsets when already resolved by MIR/NIR;
- record field address consumers only if already represented as an address value.

### Materialization Strategy

```text
record base + field offset -> direct field memory
Store(dst, FieldLoad(record, offset)) -> load field byte(s) directly into dst byte(s)
Store(Field(record, offset), value) -> materialize value directly into field byte(s)
```

### Rules

- Use numeric offsets, not field names, as executable identity.
- Do not recompute layout from source text.
- Do not redesign record layout.

### Acceptance Criteria

- `record_field_read.lst` no longer copies through an ordinary temp for simple
  byte field reads.
- `record_field_byte_store.lst` emits direct field stores once the fixture emits.
- `record_field_word_store.lst`, `nested_record_field.lst`, and
  `record_array_field_read.lst` remain correct or improve.
- Existing array and scalar store-consumer fixtures remain green.

### Suggested Commit

```text
mir6502: materialize record field consumers directly
```

## Suggested Execution Order

Use this order:

```text
1. mir6502: fix multi-argument call ABI packing
2. mir6502: render storage bytes as data in listings
3. mir6502: materialize call results into store consumers
4. mir6502: materialize constant indexed elements directly
5. mir6502: materialize record field consumers directly
```

Rationale:

- Call ABI packing may be correctness-critical.
- Listing data rendering improves diagnostics without touching codegen semantics.
- Call-result consumers remove a visible and common temp-spill pattern.
- Constant aggregate/index direct offsets are local and safe.
- Record field consumers reuse the same direct-offset/store-consumer principle.

## Required Checks

For each slice, run focused tests first, then the broader fixture dump.

```sh
cargo test -q mir6502 --lib
cargo test -q mir6502_fixtures_match_snapshots
scripts/dump_mir6502_fixtures.sh
```

For ABI changes, also add or run emitted-byte assertions for the relevant call
fixtures.

## First Codex Task

```text
Implement MIR6502 codegen quality Slice 1: multi-argument call ABI / SArgs packing.

Goal:
- Inspect and fix mixed argument packing so register and SArgs homes are assigned
  and materialized in the correct order.
- Ensure no register argument load is overwritten before the call.
- Add emitted-byte/source-listing regression for a mixed BYTE/CARD/BYTE call.
- Keep existing single-byte and word call fixtures green.

Do not implement:
- call inlining;
- ABI redesign beyond the required packing fix;
- peepholes;
- global register allocation;
- source-name recognition in emission.

Acceptance:
- `call_many_args_sargs.lst` no longer shows the first argument load being
  clobbered before the call.
- `sargs_many_args.lst` follows the documented ABI home order.
- `call_byte_arg.lst` and `call_word_arg.lst` remain green.

Suggested commit:
- mir6502: fix multi-argument call ABI packing
```
