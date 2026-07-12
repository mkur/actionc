# SemIR Native Classification Layer Plan

Owner: `src/codegen/semir_native/native_classify.rs`.

Classification answers how the native backend can obtain a typed value or
place. The detailed migration history lives in
`SEMIR_NATIVE_CLASSIFIER_MIGRATION_PLAN.md`; this note is the layer contract.

## Responsibilities

- classify values as literal, storage, address, dereference, indexed element,
  call result, computed value, or unsupported backend shape;
- classify lvalues and address-like values without emitting code;
- expose byte, word, compare-byte, direct-storage, and address queries through
  `NativeClassifier`;
- reuse semantic typing and storage/layout facts instead of duplicating
  semantic legality checks;
- produce shape-oriented unsupported diagnostics.

The classifier is read-only. It may inspect SemIR and storage/layout metadata,
but it must not emit instructions, allocate temporaries, mutate tracked
processor state, or choose opcode sequences.

## Current Direction

`NativeClassifier` is already the facade for value, lvalue, address, byte,
word, compare-byte, pointer-base, routine-call, indexed-call, and direct-storage
queries.

Current state: stable. The classification layer should not be the main next
focus. The stress backlog may require small classifier additions, but those
additions should be pulled by materializers and remain read-only and
shape-oriented. Do not restart broad classifier migration unless new
materializers reveal repeated local shape rediscovery.

Stress-backed classifier gaps to watch:

- a single query for byte `N` of a value shape, including zero-extension policy,
  so returns, call arguments, comparisons, and arithmetic operands do not each
  rediscover literals, storage, address values, dereferences, indexes, record
  fields, and call results;
- callable shape classification for `User`, `Builtin`, `Runtime`, and
  `Indirect` callables without deciding how to emit the call;
- pointer-index, pointer-deref, and record-field shapes that preserve width,
  signedness, pointee width, and field metadata for materializers;
- direct-storage queries that remain backed by the storage model but do not
  become the only accepted source shape for assignments and returns;
- unsupported diagnostics that name the missing backend shape rather than the
  helper that happened to fail.

Near-term plan:

1. Keep `NativeClassifier` as the only public classifier surface used by
   SemIR-native lowering.
2. Move remaining local syntax rediscovery behind classifier APIs only when a
   materializer is ready to consume the classified shape.
3. Keep direct storage classification backed by the existing storage model until
   storage/layout ownership changes deliberately.
4. Keep diagnostics phrased around backend shapes, not fallback helper names.
5. Add classifier APIs for array, pointer, record, or call shapes only when a
   materializer is ready to consume the classified shape.
6. For each stress backlog slice, prefer extending the existing
   `NativeClassifier` facade over adding fresh syntax checks in
   `semir_native.rs`.

## Boundary Checks

Classification code should not:

- emit instructions;
- call `NativeTrackedEmitter`;
- update tracked A/X/Y/flag/zero-page state;
- choose where a value should be materialized.

Classification code may:

- return slots, shapes, widths, and address facts;
- reject backend-unsupported shapes with precise diagnostics;
- rely on semantic analysis having already accepted the program.
