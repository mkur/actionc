# Record-array regressions exposed by Oscar64 stage 5

Status: fixed, 2026-09-06. The structurally faithful `structmembertest.act`
port and its independent memory oracle exposed the gaps below. These are
general record-array/storage issues, not missing embedded-array syntax.
The earlier checksum-load defect has its own
[indirect scalar-copy note](CLASSIC_INDIRECT_SCALAR_COPY_POINTER_BUG.md).

## Record-array decay versus single-record addressing

SemIR's expected-pointer conversion checked implicit record addressing before
array decay. A record array's lvalue type describes its element, so `Shuffle(v)`
incorrectly became `RecordToPointer(v)` instead of `ArrayDecay(v)`. Classic
passed the array descriptor's address; the callee could overwrite generated
storage/code. Array assignment to a record pointer had the same ambiguity.

The existing array-place/array-symbol query now takes precedence. Individual
records still use implicit address-of. This is owned by SemIR; neither backend
needs to rediscover source meaning. Global, local, initialized, fixed and
parameter arrays retain their existing array-decay forms and storage identities.

## Local fixed backing and descriptor extent

Classic's routine allocation selected generated descriptor backing for sized
word/record arrays before consulting their fixed numeric initializer. Excluding
fixed-address declarations from that selection lets the existing fixed-array
path preserve the requested address.

NIR's local lowering separately omitted the global path's fixed-pointer image
and known backing facts for larger arrays. It allocated a zero-filled array
instead. Local initialization now receives the resolved address, shares the
existing fixed-pointer byte-image builder and records the backing by semantic
symbol identity. The established global descriptor policy is unchanged.

MIR's local allocation formerly used `local.ty.width`, which is the element
width for arrays. A four-byte descriptor for six-byte Vector elements then
failed initializer verification. MIR now consumes NIR's `local.layout.size`,
with scalar-width and initializer extent checks retained. Descriptor/backing
storage stays distinct from element width. Existing explicit pointer views
(including routine-address array aliases used by TN/TNDBG) keep their
pointer-sized representation. No new NIR representation is needed.

## MIR dual-pointer copy fusion

The indexed word-copy fusion holds destination `$AE/$AF` and source `$AC/$AD`
simultaneously. Constant-stride expansion above two uses `$AE/$AF` as index
scratch. It could corrupt the destination during either address calculation.
The original six-byte vectors and the seven-byte mixed-width companion exposed
this; the latter could overwrite the hardware stack at the 127/128 boundary.

Fusion now requires both element strides to be 1 or 2. Other strides use the
existing staged word load/store path, calculating one address at a time.
The costed constant-scale selector is retained; this is an eligibility repair,
not a new optimization or a blanket removal of constant-stride addressing.
Future fusion for larger strides needs an explicit nonconflicting scratch plan.

## Classic computed-field comparisons

The comparison materialization predicate did not recognize a field whose base
was indexed. The word-equality fallback reused the low-byte XOR result (zero)
with a high-byte OR, but recomputing `v(i).x`'s address overwrote A first.
The port counted 100 failed comparisons despite correct memory contents.

The predicate and protected comparison staging now reuse the existing
`expr_address_needs_nested_scratch` query in both classic profiles. Operands
are captured once before comparing their bytes, including calls in indexes.
No record-specific comparison selector was introduced.

## Coverage

- SemIR checks record-array decay for globals, locals and parameters while
  retaining implicit addressing for a single record, followed by NIR verification.
- `record_array_pointer_decay.act`: 48 executions across all three modes and
  both runtimes; eight indexes through 257; global/local initialized arrays,
  local ordinary arrays, fixed backing, array-parameter forwarding, pointer
  assignment and scalar records. Checks addresses, complete records and guards.
- `classic_record_field_comparisons.act`: 448 executions across both classic
  profiles/runtimes, ordinary and effectful indexes, equal/unequal and signed
  boundary values, all six predicates in both operand orders, two computed
  operands, guards and exactly-once counts.
- MIR fusion eligibility checks all 64 source/destination combinations of
  strides 1, 2, 3, 4, 6, 7, 128 and 255, with no partial output on rejection.
- The Oscar64 member port retains its original loops and independent oracle:
  30 host cases / 120 modern VM executions, with Compatibility rejection
  checked separately. The 198 record-array copy cases remain active too.

Focused commands:

```sh
cargo test --lib record_array
cargo test --lib indexed_word_copy_fusion_requires_scratch_free_address_scales
cd tools/vm-runtime-tests
cargo test --locked --test oscar64_conformance oscar64_record
```
