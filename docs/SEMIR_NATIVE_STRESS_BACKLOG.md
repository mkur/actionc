# SemIR Native Stress Backlog

Snapshot date: 2026-05-30.

This note tracks the missing SemIR-native backend shapes exposed by the stress
sweep. It is intentionally shape-oriented: stress files are useful symptoms,
but the implementation work should add reusable classifier/materializer support
rather than file-specific lowering rules.

## Validation Command

Use the SemIR sweep in coverage mode for the stress directory:

```sh
cargo run --quiet --bin actionc-semir-sweep -- \
  --candidate native \
  --profile modern \
  --validation coverage \
  fixtures/stress
```

Latest result:

- total stress files: 14;
- files reaching coverage comparison: 13;
- byte-identical with AST backend: 0;
- coverage deltas: 13;
- unsupported SemIR-native shapes: 0;
- AST failures: 1;
- SemIR-native failures: 0;
- load failures: 0.

The stress files currently reaching coverage `DELTA` are:

- `fixtures/stress/advanced_pointers.act`;
- `fixtures/stress/arithmetic_control.act`;
- `fixtures/stress/arrays.act`;
- `fixtures/stress/calls.act`;
- `fixtures/stress/control_flow.act`;
- `fixtures/stress/layout_integration.act`;
- `fixtures/stress/pointer_torture.act`;
- `fixtures/stress/pointer_usage.act`;
- `fixtures/stress/pointers.act`;
- `fixtures/stress/real_expr_chains.act`;
- `fixtures/stress/records.act`;
- `fixtures/stress/strings.act`;
- `fixtures/stress/zero_page_scalars.act`.

`fixtures/stress/zero_page.act` is special in the comparison sweep because
the AST backend fails first with the existing compatible zero-page policy.

## Missing Shape Inventory

| Stress file | First blocker | Reusable backend shape |
| --- | --- | --- |
| `advanced_pointers.act` | now reaches `DELTA` in coverage mode | exactness follow-up after record-field, pointer-index, and direct-record-field shapes |
| `arithmetic_control.act` | now reaches `DELTA` in coverage mode | exactness follow-up after word logic, runtime arithmetic/shift, and word call-arg zero-extension |
| `arrays.act` | now reaches `DELTA` in coverage mode | exactness follow-up after dynamic index-call staging and word array call-arg reads |
| `calls.act` | now reaches `DELTA` in coverage mode | exactness follow-up for call ABI parity |
| `control_flow.act` | now reaches `DELTA` in coverage mode | exactness follow-up after byte-to-word logic operand materialization |
| `layout_integration.act` | now reaches `DELTA` in coverage mode | exactness follow-up after mixed byte/word arithmetic over indexed word values |
| `pointer_torture.act` | now reaches `DELTA` in coverage mode | exactness follow-up after signed word pointer-deref conditions and word deref unary negation |
| `pointer_usage.act` | now reaches `DELTA` in coverage mode | exactness follow-up after builtin call lowering through shared call-argument materialization |
| `pointers.act` | now reaches `DELTA` in coverage mode | exactness follow-up after byte pointer-deref computed stores and pointer-index word store/read shapes |
| `real_expr_chains.act` | now reaches `DELTA` in coverage mode | exactness follow-up after call-result operands in word conditions |
| `records.act` | now reaches `DELTA` in coverage mode | exactness follow-up after record-field store/read, word condition, and record-pointer traversal shapes |
| `strings.act` | now reaches `DELTA` in coverage mode | exactness follow-up after dynamic byte array compound logic operators |
| `zero_page_scalars.act` | now reaches `DELTA` in coverage mode | exactness follow-up for zero-page scalar prologue/emission parity |
| `zero_page.act` | AST backend fails first in coverage mode | resolve comparison harness / AST compatible zero-page policy before SemIR parity can be measured |

## Layer Contract Pointers

The structural rules for this work are already owned by the layer plans:

- `archive/implementation-plans/semir-native/SEMIR_NATIVE_LAYER_PLAN.md` defines the typing, classification,
  materialization, and emission boundaries.
- `archive/implementation-plans/semir-native/SEMIR_NATIVE_CLASSIFICATION_PLAN.md` defines the read-only classifier
  contract.
- `archive/implementation-plans/semir-native/SEMIR_NATIVE_MATERIALIZATION_PLAN.md` defines where reusable value,
  address, call-argument, and store lowering belongs.
- `archive/implementation-plans/semir-native/SEMIR_NATIVE_EMISSION_PLAN.md` defines the tracked-emitter boundary.

This backlog should not become a second rulebook. For stress work, use it only
as a queue of missing shapes. Each implementation slice should cite the owning
layer, add a focused `native_*` regression test, and either reduce the stress
`unsupported` count or expose the next precise missing shape.

Status: all current SemIR-native stress files now compile far enough for
coverage comparison. The backlog is moving from missing-shape implementation to
byte-level exactness and the special `zero_page.act` AST comparison blocker.

## Implementation Order

### 1. Add Focused Regression Probes

Before each lowering slice, add a small SemIR-native test that names the shape:

- record-field computed byte store;
- record-field word store from call result or indexed value;
- `RETURN(0 - x)` for signed word functions;
- word logic, word shifts, and word runtime arithmetic;
- word call arguments that zero-extend byte expressions;
- word pointer-index read;
- word pointer-deref compare;
- builtin call with string and scalar arguments;
- dynamic byte array `==!`, `==&`, and `==%`;
- dynamic array assignments whose index expression calls a routine;
- word array reads passed as call arguments;
- word pointer-deref unary negation;
- computed word RHS assigned through a fixed zero-page pointer or scalar alias.

These probes should be smaller than the stress files and should stay in the
SemIR-native test module near the existing `native_*` cases.

### 2. Generalize Indirect Stores First

The record-field byte store path currently has a narrow byte-only fast path.
Route both byte and word record-field stores through the existing
`materialize_value_to_array_addr_element(value, width)` path. This already
handles call results by preserving `ARRAY_ADDR` across the call, and it gives
record fields the same staging behavior as pointer-backed array stores.

Status: landed for computed byte values, byte field values, computed byte field
expressions, computed word values, record-field word operands, record-field word
conditions, direct record-field reads, and pointer-index word operands. This
moved both `records.act` and `advanced_pointers.act` to `DELTA` in coverage
mode. Byte pointer-deref computed stores now share the same staging direction
and moved `pointers.act` to `DELTA`.

### 3. Make Byte Extraction A Real Materializer

Add or finish one reusable materializer for "byte N of value to register". It
should accept:

- literals and storage slots;
- address values;
- return-slot bytes;
- byte or word dereferences;
- byte or word indexed values;
- record-field values;
- computed byte values;
- zero-extension where the consumer explicitly requests it.

Then route returns, call arguments, comparisons, indirect stores, and arithmetic
operands through this materializer instead of falling back to
`required_addressable_slot`.

### 4. Extend Word Indexed And Deref Reads

Add a target-slot materializer for width-2 pointer-index and pointer-deref
values. Pointer-index target reads are in place for scalar assignments,
computed word operands, and pointer-index store RHS staging. Continue with
pointer-deref word stores from staged pointer-index reads. Use the shared
materializers from:

- scalar assignments;
- returns;
- word binary operands;
- word equality and signed/unsigned branch conditions;
- call argument staging.

Status: pointer-index reads/stores and word pointer-deref branch operands are
covered for the current stress files. `pointer_torture.act` now reaches
`DELTA` after signed zero comparisons and word deref unary negation.

### 5. Fix Mixed-Width Word Expressions

Word expression lowering should explicitly zero-extend byte literals and byte
expressions when the expression's semantic type is word-sized. Important
examples:

- `RETURN(0 - x)`;
- `word + byte`;
- `word - byte`;
- word comparisons against byte constants;
- call arguments where the callee parameter width is word-sized.

This should remove the `arrays.act` return blocker and reduce the arithmetic
stress failures.

Status: landed for `RETURN(0 - x)`, byte-left word add/sub, word call arguments
that zero-extend byte expressions, and word array reads passed as call
arguments. `arrays.act` now reaches `DELTA`.

### 6. Port AST Arithmetic Recipes Carefully

Use the AST backend as an oracle for proven 6502 sequences, not as the shape
model. Useful references:

- `src/codegen/arith.rs`: runtime helper ABI for `*`, `/`, `MOD`, `LSH`, and
  `RSH`;
- `src/codegen/expr.rs`: pointer and array indexed slot construction;
- `src/codegen/array.rs`: dynamic array address construction;
- `src/codegen/call.rs`: public Action ABI packing and builtin/runtime call
  handling.

SemIR-native should still decide from typed SemIR classifier shapes. Avoid
porting AST source-shape checks directly into `semir_native.rs`.

### 7. Add Word Logic And Runtime Operators

After mixed-width word assignment is stable, add word expression materializers
for:

- `AND`, `OR`, `XOR` as per-byte operations;
- dynamic `LSH` and `RSH` through the runtime helper ABI;
- signed/word `*`, `/`, and `MOD` through the runtime helper ABI;
- indexed or dereferenced word operands, with staging when source and target
  homes overlap.

This is the main path for `arithmetic_control.act`, `real_expr_chains.act`, and
`layout_integration.act`.

Status: landed for direct word `AND`/`OR`/`XOR`, byte-to-word logic operands,
dynamic word `LSH`/`RSH`, and word `*`/`/`/`MOD` through the runtime helper ABI.
This moved `arithmetic_control.act` and the new control-flow stress file to
`DELTA`.

### 8. Teach Calls About All SemIR Callable Kinds

`emit_call` currently expects `SemCallable::User`. Extend it for:

- `SemCallable::Builtin`;
- `SemCallable::Runtime` with a known address;
- `SemCallable::Indirect` for pointer calls, once the effective target address
  materializer is ready.

The call planner should use the `SemCall` signature and effects, flatten
arguments into public Action ABI bytes, and materialize those bytes through one
path. Do not add builtin-specific one-offs for `Print`, `PrintF`, `PutE`, or
similar library calls.

Status: builtin calls with known system addresses now use the same flattened
argument materialization path as user calls. This moved `pointer_usage.act` to
`DELTA`. Runtime calls with known addresses share the dispatch shape; indirect
call targets remain a future callable-kind slice.

### 9. Add Dynamic Array Compound Logic

Dynamic byte array compound assignment currently supports `+=` and `-=`.
Extend inline and pointer-backed dynamic array compounds to support:

- `==&`;
- `==%`;
- `==!`.

The implementation should stage RHS values when needed and preserve the
computed element address across that staging, following the existing indirect
store patterns.

Status: landed for inline and pointer-backed dynamic byte array compound logic.
This moved `strings.act` to `DELTA`.

## Verification

After each slice, run:

```sh
cargo test native_ --quiet
cargo run --quiet --bin actionc-semir-sweep -- \
  --candidate native \
  --validation-policy coverage \
  --dashboard \
  fixtures/stress
```

A successful slice should reduce `unsupported` or move a broad stress file to
`DELTA`. Byte deltas are acceptable in coverage mode until exactness is
classified separately.

## Design Rule

The AST backend is useful for machine-code recipes and compatibility clues.
SemIR-native should not recreate AST codegen under a different file name. New
support should enter through classifier shapes and materializers, with precise
unsupported-shape diagnostics when the backend still cannot lower a semantic
form.
