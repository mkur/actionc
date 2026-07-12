# SemIR Native Materialization Layer Plan

Owner: `src/codegen/semir_native/native_materialize.rs`.

Materialization answers how to put a classified shape into a concrete machine
home. This layer is at a useful plateau after the classifier migration and
emission-helper pass, but the stress backlog makes it the active feature layer
again: the next failures are mostly missing reusable value, address, call, and
store materializers.

## Responsibilities

- byte source to A/X/Y;
- word source to a target slot or ABI bytes;
- address value to registers, target storage, or zero-page pointer pair;
- lvalue effective address to `ARRAY_ADDR` or another chosen pointer pair;
- call result to target;
- value to return slot or call argument homes.

Materializers may emit instructions and mutate tracked processor state. They
should consume classifier shapes or classifier-owned slots, not rediscover raw
SemIR syntax except at temporary compatibility boundaries.

## Current Status

The initial materializer boundary exists for byte-source-to-register and
word-source-to-target. Address destination vocabulary now exists for
`ARRAY_ADDR`, and pointer deref, pointer-index, pointer-backed array, and
record-field effective-address materialization have started. Inline array
element reads/writes now have materializer entry points. Scalar byte/word
value, string-literal address, address-of, call-result, and indirect
`ARGS`-to-`ARRAY_ADDR` store materializers have moved into this layer. A
small value-to-target dispatcher now covers assignment, array-store staging,
and return-value staging. Slot-to-target copies now also have a materializer
used by assignment, expression-target, and return fallback paths. Indirect
value stores can now preserve `ARRAY_ADDR` across call results. Indirect
reads from `ARRAY_ADDR` now materialize through shared byte-to-A and
element-to-target helpers.

After the emission helper pass, high-level SemIR-native lowering no longer
calls `self.emitter.emit_*` directly. That makes the materialization follow-ups
around call/ABI staging safer to resume: materializers can now rely on concrete
emission helpers instead of spelling opcode details.

Call-argument staging now lives in this layer for the current native ABI paths:
byte homes to A/X/Y, the single word-to-AX path, and SARGS byte staging. This
completed the parked call/ABI cleanup without needing a new ABI-home vocabulary.

The last focused validation was `cargo test semir_native --lib` with 40 passing
tests. The layer is healthy, but it is again the primary implementation layer
for `SEMIR_NATIVE_STRESS_BACKLOG.md`.

Near-term plan:

1. Add materializer vocabulary for destination homes and preservation policy:
   started for `ARRAY_ADDR`; broaden only as consumers need new homes.
2. Add pointer dereference effective-address materialization first. Done for
   addressable pointer deref shapes.
3. Migrate pointer deref reads and writes onto that API. Done for current
   byte/word deref read and store paths.
4. Extend the same pattern to indexed pointer values. Done for current
   byte/word pointer-index assignment and byte pointer-index read paths.
5. Add array element effective-address materialization for inline arrays,
   descriptor arrays, and array parameters. Done for pointer-backed reads and
   writes. Inline byte/word reads and `ARGS`-to-inline writes now use
   materializer APIs while still choosing direct/indexed storage forms.
6. Add record-field effective-address materialization after pointer and array
   paths prove the API. Done for current record-field read and store paths.
7. Build higher-level value materializers only after two or more consumers need
   the same operation. Started for scalar value-to-target, call-result-to-target,
   address-of-to-target, and `ARGS`-to-addressed-element stores. The first
   `materialize_value_to_target` dispatcher is in use for simple assignments,
   array-store values, and returns. Slot-copy materialization is in place for
   source-slot-to-target moves. `materialize_value_to_array_addr_element`
   now handles pointer-index stores, word pointer-deref stores, and word
   record-field stores, including call-result values that require `ARRAY_ADDR`
   preservation. Pointer-deref staged word stores also share the indirect
   `ARGS` store materializer.
   Pointer-backed array, pointer-index, pointer-deref, and record-field reads
   are starting to share indirect read materializers.
8. Work through the stress backlog as targeted materialization slices:
   - route byte and word record-field stores through
     `materialize_value_to_array_addr_element`. Computed byte/word values,
     byte field values, record-field word operands, direct record-field reads,
     and pointer-index word operands are now covered by focused regressions;
   - add a value-byte-to-register materializer that can consume indexed,
     deref, record-field, call-result, address, storage, literal, and computed
     byte shapes with explicit zero-extension policy;
   - extend width-2 pointer-index and pointer-deref reads to target slots,
     return slots, call arguments, and branch operands. Pointer-index reads and
     stores, word pointer-deref branch operands, and word deref unary negation
     have landed for the current pointer stress slice;
   - support mixed-width word expression materialization for word targets and
     returns. Byte-left word add/sub, `RETURN(0 - x)`, and byte-expression
     zero-extension for word call arguments are covered;
   - add word logic and runtime arithmetic/shift materializers using the
     existing runtime helper ABI. Direct word `AND`/`OR`/`XOR`,
     byte-to-word logic operands, dynamic word `LSH`/`RSH`, and word
     `*`/`/`/`MOD` are now covered;
   - route builtin/runtime/indirect call argument packing through the same call
     materialization path as user calls. Builtins with known system addresses
     now share the user-call argument path; indirect call targets are still
     pending;
   - add dynamic byte array compound logic operators while preserving the
     computed element address across RHS staging. Dynamic index-call staging
     for array assignments is also in place.

## Parked Follow-Ups

These are worth doing only when a nearby change needs them:

- add ABI-home vocabulary if another consumer repeats the same destination
  logic now used by call-argument staging;
- add a single `materialize_lvalue_address(lvalue_shape, dest) -> width`
  dispatcher after more callers need lvalue-address polymorphism;
- broaden preservation policy beyond `ARRAY_ADDR` only when another zero-page
  home needs explicit save/restore behavior.

Recommended next materialization sequence:

1. Start with record-field computed stores because they should reuse existing
   indirect value-store machinery and shrink a narrow local guard.
2. Add the value-byte-to-register path only around the first consumers that need
   it, then reuse it from returns, call arguments, comparisons, and arithmetic
   operands as those slices land.
3. Extend word pointer-index/deref reads before broad word arithmetic, so word
   expression materializers can consume the same indirect read API.
4. Move the stress backlog from missing-shape work to exactness triage for the
   files now reaching coverage `DELTA`.
5. Add indirect call lowering through the call materialization path when a
   stress case or real program needs callable-pointer dispatch.
6. Resolve the `zero_page.act` comparison blocker, which currently fails in the
   AST backend before SemIR-native parity can be measured.
7. Add vocabulary only around proven duplication; avoid broad pre-emptive
   dispatchers until two or more consumers need the same home.

API direction:

- `materialize_addressable_pointer_to(pointer_expr, dest) -> pointee_width`;
- `materialize_pointer_deref_address(deref_shape, dest) -> pointee_width`;
- `materialize_pointer_index_address(indexed_shape, dest) -> element_width`;
- `materialize_pointer_backed_array_index_address(indexed_shape, dest) -> element_width`;
- `materialize_inline_array_byte_to_a(indexed_shape)`;
- `materialize_inline_array_word_to_target(indexed_shape, target)`;
- `materialize_args_to_inline_array_element(indexed_shape) -> element_width`;
- `materialize_args_to_array_addr_element(width)`;
- `materialize_value_to_array_addr_element(value, width) -> bool`;
- `materialize_array_addr_element_to_target(target) -> bool`;
- `materialize_array_addr_element_to_a()`;
- `materialize_array_addr_to_stack()`;
- `materialize_stack_to_array_addr()`;
- `materialize_record_field_address(field_shape, dest) -> width`;
- `materialize_lvalue_address(lvalue_shape, dest) -> width`;
- `materialize_value_to_target(value, target) -> bool`;
- `materialize_value_byte_to_register(value, byte_index, register, mode) -> bool`;
- `materialize_word_expression_to_target(value, target) -> bool`;
- `materialize_slot_to_target(source, target) -> bool`;
- `materialize_word_value_to_target(value, target) -> bool`;
- `materialize_byte_value_to_target(value, target) -> bool`;
- `materialize_return_slot_to_target(target)`;
- `materialize_address_of_to_target(lvalue, target)`;
- `materialize_byte_source_to_register(source, byte_index, register)`;
- `materialize_call_arg_byte_to_a(expr, byte_index)`;
- `materialize_call_arg_byte_to_x(expr, byte_index)`;
- `materialize_call_arg_byte_to_y(expr, byte_index)`;
- `materialize_word_call_arg_to_ax(expr)`;
- `materialize_sargs_call_args(arg_bytes)`.

## Boundary Checks

Materialization code should not:

- decide semantic legality;
- duplicate type compatibility rules;
- introduce raw opcode writes outside the tracked-emitter path;
- grow one-off source-shape checks when a classifier shape can represent the
  case.

Materialization code may:

- emit instruction sequences through emitter helpers;
- choose concrete registers, ABI bytes, target slots, or zero-page pointer
  pairs;
- preserve or clobber tracked state according to explicit policy;
- stage effectful values when evaluation order requires it.
