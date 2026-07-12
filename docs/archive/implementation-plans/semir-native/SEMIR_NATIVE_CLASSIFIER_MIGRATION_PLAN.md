# SemIR Native Classifier Migration Plan

SemIR-native should lower typed semantic nodes through a classifier before it
emits instructions. The classifier is not another type checker. It answers the
backend question: how can this value, place, or address be obtained on a 6502?

For the broader typing, classification, materialization, and emission boundary,
see `SEMIR_NATIVE_LAYER_PLAN.md`. For the concise classification-layer
contract, see `SEMIR_NATIVE_CLASSIFICATION_PLAN.md`.

## Shape Vocabulary

The first classifier should recognize these shapes:

- literal: scalar byte/word constants known at compile time;
- storage: a concrete scalar slot or pointer-valued array parameter slot;
- address value: `@symbol`, array decay, string literals, routine labels, and
  current location;
- dereference: `p^` and record/pointer places that must read through an
  address;
- indexed element: inline array, descriptor array, array parameter, or pointer
  indexed access;
- call result: an ordinary routine call whose result is produced by the ABI;
- computed expression: arithmetic, logic, casts, or other expressions that need
  materialization;
- unsupported: a semantic shape the backend does not lower yet.

## Implementation Order

1. Add a read-only classifier beside the current SemIR-native emitter. Done.
2. Back it with focused tests for each core shape. Done for the currently
   recognized core shapes.
3. Keep behavior unchanged in the first slice. Done.
4. Migrate call argument lowering to consume classified byte/address sources.
   Done for simple classified literal/storage/address byte sources, with
   fallbacks retained for computed/indexed/deref/call shapes.
5. Migrate assignment and return lowering to consume byte/word/address
   materializers. Done for scalar assignment, returns, dynamic array staging,
   indirect byte stores, and record-field stores where the value is a simple
   classified source.
6. Migrate branch/compare lowering to consume classified operands. Done for
   byte IF/WHILE comparisons and FOR-bound byte comparisons, including explicit
   byte-to-word zero extension for word loop targets.
7. Delete source-shape helper duplication once every consumer has moved. Done
   for the byte-source classifier/materializer surface; broader expression
   helper cleanup remains future work.

## Current Status

The classifier exists inside `src/codegen/semir_native.rs` as a read-only
classification layer over the SemIR read model. It recognizes:

- scalar literals;
- concrete storage slots;
- address values for storage bases, storage pointers, routine/current-location
  addresses, and string literals;
- dereference shapes;
- indexed shapes for inline arrays, descriptor arrays, array parameters, and
  pointer values;
- routine call results;
- computed and unsupported fallback shapes.

The first reusable consumer layer is also in place:

- `NativeByteSource` represents a byte that can be loaded from an immediate or
  storage address.
- `NativeByteSourceMode` makes width policy explicit: exact byte extraction or
  zero-extension to a word compare.
- byte materialization can target A/X/Y through one helper;
- word materialization composes two classified byte sources;
- string literal inline storage remains an explicit materialization special
  case, because it emits data at the current code location.

Current consumers:

- call arguments use classified byte/address sources for register and SARGS
  materialization;
- scalar assignment and return lowering use byte/word materializers;
- dynamic array assignment staging uses byte/word materializers before storing
  through the existing array store paths;
- byte indirect stores and record-field stores use the byte/word materializers
  for simple classified sources;
- IF/WHILE byte comparisons use classified sources for LHS materialization and
  RHS EOR/CMP operands;
- FOR bounds use classified compare sources with explicit zero-extension.

Important remaining fallbacks:

- computed expressions still lower through the older expression lowering paths;
- indexed/deref/call-result shapes are classified, but only some consumers use
  them directly;
- arithmetic and logic operand lowering still has source-shape helpers and
  should be migrated later;
- pointer/array effective-address construction is still mostly helper-driven,
  not fully classifier-driven;
- diagnostics are better than before, but unsupported-shape reporting is not yet
  consistently phrased around classifier shape names.

## Next Cleanup Direction

Do not add broad new special cases to the direct codegen. Prefer these next
steps:

1. Extract the classifier/materializer vocabulary from `semir_native.rs` into a
   small module once it stabilizes further.
2. Migrate arithmetic/logic operand lowering to consume `NativeByteSource`
   where it is clearly safe.
3. Add explicit classified address/effective-address materializers before
   changing more array and pointer paths.
4. Keep computed/indexed/deref/call-result fallbacks until each has a reusable
   materializer and focused regression tests.

## Extraction Plan

The next architecture step is to pull the classifier boundary out of the large
`semir_native.rs` file without changing lowering behavior. This should happen
in small slices and stop before new backend consumers are migrated.

### Slice 1: Extract Data Types Only

Status: done in `src/codegen/semir_native/native_classify.rs`.

Move classifier vocabulary into a `semir_native` submodule:

- value/address/indexed shapes;
- address and indexed-storage kinds;
- byte source and byte-source mode;
- byte materialization register selector.

This slice should only move definitions and update imports.

### Slice 2: Extract Read-Only Classifier Helpers

Status: done. Read-only classifier helpers now live beside the vocabulary in
`native_classify.rs`.

Move pure classification helpers into the same submodule while keeping them as
read-only operations:

- value classification;
- symbol/lvalue/address classification;
- array decay classification;
- call/indexed classification;
- byte-source classification.

The helper code may temporarily remain implemented for `SemIrNativeEmitter`, but
it must not emit instructions or mutate state. The goal is a clean file/module
boundary first.

### Slice 3: Add First-Class Shape APIs

Status: done. `NativeClassifier` exposes first-class value, lvalue, address,
byte-source, and word-source queries.

Introduce an explicit classifier facade once the moved helpers are stable. The
facade should expose semantic backend questions directly:

- `value_shape(expr)`;
- `place_shape(lvalue)`;
- `address_shape(...)`;
- `byte_source(expr, index, mode)`;
- `word_source(expr, mode)`.

At this stage the existing emitter may still call compatibility wrappers. The
important thing is to establish the vocabulary and API direction before more
consumers migrate.

### Slice 4: Add Materializer Boundary

Status: done for the initial source materializers. `native_materialize.rs`
contains the byte-source-to-register and word-source-to-target boundary. Broader
address/value/return-slot materializers remain future work and should be added
only when consumers are migrated deliberately.

Keep materializers on the emitter side, but name them as consumers of classified
sources and shapes:

- byte source to A/X/Y;
- word source to target;
- address source to ABI registers;
- value to target;
- value to return slot.

This slice should continue to preserve current behavior. It should stop before
the next migration phase.

### Slice 5: Migrate Manual Redetection Gradually

Status: started. The first safe migrations are complete:

- classified byte/compare source queries now go through `NativeClassifier`;
- value width inference now lives behind `NativeClassifier::value_width`;
- addressable-slot detection now lives behind `NativeClassifier::addressable_slot`;
- pointer-base detection now lives behind `NativeClassifier::pointer_base_slot`;
- array-index and pointer-index call checks now live behind classifier methods.
- array-index expression/base-symbol recognition now lives behind classifier
  APIs;
- ordinary routine-call expression recognition now lives behind
  `NativeClassifier::routine_call_expr`;
- byte arithmetic/logic operands now use classified byte operand sources.
- required direct expression/lvalue storage queries now go through
  `NativeClassifier` before falling back to the existing storage model.

Important remaining manual detections:

- array/effective-address construction still uses older helpers such as
  `array_slot_from_expr`;
- the classifier-backed direct storage query still delegates to older
  resolved-slot helpers internally while storage/layout ownership remains in the
  emitter;
- dereference, record-field, and indexed-element lowering still inspect
  expression/lvalue syntax in their local emitters;
- broader arithmetic/logic lowering still has shape-specific expression
  decomposition, though simple byte operands now use the classifier.

Continue Phase 5 one narrow lowering family at a time. Do not migrate
array/pointer/record effective-address construction until there is a reusable
classified address/effective-address materializer.

### Slice 6: Build Address And Value Materializers

Status: planned. This is the next architecture step after the direct-storage
classifier migration. The goal is to make array, pointer, record, and deref
lowering consume classified shapes instead of rediscovering syntax locally.

Build this layer in small behavior-preserving slices:

1. Add materializer vocabulary for address destinations and preservation policy.
   Start with the concrete homes the current emitter already uses:
   `ARRAY_ADDR`, `ELEMENT_ADDR`, A/X/Y, and a scalar target slot. Keep this as
   emitter-side vocabulary because materialization emits instructions and
   mutates tracked processor state.
2. Add a narrow effective-address materializer for one classified family first:
   pointer dereference to `ARRAY_ADDR`. It should consume the classified
   pointer storage/source shape, verify byte/word pointee width, and then call
   the existing pointer-loading opcode sequence.
3. Migrate pointer deref reads and writes onto that materializer. This should
   delete local `SemLValueKind::Deref` rediscovery from the byte/word read and
   store paths without changing generated code.
4. Generalize the same address materializer to indexed pointer values. Reuse
   the existing scaled-index routines, but put the classification boundary
   before address construction.
5. Add array element effective-address materialization for inline arrays,
   descriptor arrays, and array parameters. Inline constant indexes may still
   become direct storage slots; dynamic and pointer-backed indexes should
   materialize to `ARRAY_ADDR` through one API.
6. Add record-field effective-address materialization last. Record fields depend
   on both resolved base storage and field layout metadata, so they should move
   only after pointer and array address materialization have proven the API.
7. Once address materializers exist, add higher-level value materializers that
   consume `NativeValueShape`:
   byte value to A/X/Y, word value to target, address value to target/registers,
   call result to target, and lvalue/value to return slot.

Initial API direction:

- `materialize_pointer_deref_address(pointer_expr, dest) -> pointee_width`;
- `materialize_indexed_address(indexed_shape, dest) -> element_width`;
- `materialize_lvalue_address(lvalue_shape, dest) -> width`;
- `materialize_value_to_target(value_shape, target) -> bool`;
- `materialize_value_byte_to_register(value_shape, byte_index, register) -> bool`.

Keep these rules while implementing the layer:

- materializers may emit code and update tracked state; classifiers may not;
- materializers should consume classifier shapes or classifier-owned slots, not
  inspect raw SemIR syntax except at temporary compatibility boundaries;
- every migration should remove at least one local shape check from
  `semir_native.rs`;
- each slice should have focused SemIR-native tests and preserve current
  generated-code behavior unless the test intentionally documents an existing
  bug fix;
- do not collapse array, pointer, and record addressing into one large helper
  until the repeated shape is obvious from two migrated consumers.

## Constraints

- The classifier may rely on semantic typing and legality checks already being
  complete.
- It should still produce precise unsupported-shape diagnostics.
- It should not emit code or mutate processor state.
- It should not duplicate storage/layout logic; it should reuse the existing
  storage model until that model is replaced.
- Each migration step must preserve the single tracked-emitter path for
  instruction emission.
