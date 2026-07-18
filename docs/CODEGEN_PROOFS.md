# Codegen Proofs

This note documents the current proof and fact layer used by modern direct
codegen. These proofs are deliberately conservative. Their job is to make
optimization decisions explicit and inspectable while remaining useful later as
inputs to TAC/SSA lowering.

## Tooling

Accepted proof-guided lowering events:

```sh
cargo run --bin actionc-emit -- --profile modern --emit-proofs FILE.ACT
```

Accepted and rejected proof attempts for instrumented consumers:

```sh
cargo run --bin actionc-emit -- --profile modern --emit-proof-attempts FILE.ACT
```

`--emit-proofs` is the normal concise view. `--emit-proof-attempts` is a debug
view for answering why a lowering did not fire. It is currently instrumented
only around the first proof consumers, not every proof-producing function.

## Expression Side-Effect Facts

Type: `ExpressionSideEffectFacts`.

Fields:

- `has_routine_call`
- `has_unknown_raw`
- `reads_memory`
- `reads_pointer`
- `reads_volatile`
- `writes_through_pointer`
- `evaluation_order_sensitive`

Derived queries:

- `is_read_only()`
- `can_duplicate()`
- `can_reorder()`

Main rules:

- constants, strings, chars, and current-location expressions are pure;
- scalar names read memory and may be volatile for selected system/runtime
  locations;
- pointer dereference reads memory through a pointer;
- field access reads memory, and pointer-record field access also reads through
  a pointer;
- binary/index expressions merge child facts;
- real routine calls are evaluation-order-sensitive;
- Action array-call syntax is treated as indexing, not as a routine call.

Current use:

- guards left-to-right evaluation decisions;
- rejects index-address proofs when the index is not read-only;
- feeds future decisions about duplication/reordering.

## Value Range Facts

Type: `ValueRangeFact`.

Values:

- `Unknown`
- `Byte`
- `Exact(u16)`

Main rules:

- numeric constants become `Exact`;
- expressions with known one-byte type become `Byte`;
- everything else is `Unknown`.

Current use:

- proves whether an index is byte-sized enough for direct indexed addressing.

## Index Address Proofs

Type: `IndexAddressProof`.

Fields:

- `base: StorageSlot`
- `element_size: u16`
- `index_width: Option<u16>`
- `index_range: ValueRangeFact`
- `effects: ExpressionSideEffectFacts`
- `mode: IndexAddressMode`
- `reject_reason: Option<IndexAddressRejectReason>`

Modes:

- `AbsoluteY`
- `IndirectY`
- `NeedsScaling`
- `Unsupported`

Reject reasons:

- `IndexHasSideEffects`
- `NonByteIndex`
- `ElementNeedsScaling`
- `UnsupportedBase`

Main rules:

- indexes with side effects are unsupported;
- non-byte indexes are unsupported;
- element sizes other than one byte require scaling;
- inline arrays with byte elements and byte indexes can use `absolute,Y`;
- pointer/descriptor arrays and pointer variables with byte elements can use
  `(zp),Y`.

Current consumers:

- `byte_index_effective_address` for `(zp),Y` style loads/stores;
- inline byte-array scalar-index loads;
- call-argument loading for inline byte arrays before generic lvalue fallback.

Instrumented observability:

- accepted `index-address` events for proof-guided `absolute,Y` lowering;
- rejected `index-address` events for unsupported/mismatched shapes in that
  consumer.

## Pointer Dereference Proofs

Type: `PointerDereferenceProof`.

Fields:

- `pointer: StorageSlot`
- `kind: PointerDereferenceKind`
- `pointee_size: u16`
- `signed: bool`
- `field: Option<RecordField>`
- `index: Option<IndexAddressProof>`
- `mode: PointerDereferenceMode`
- `reject_reason: Option<PointerDereferenceRejectReason>`

Kinds:

- `Direct`
- `Indexed`
- `RecordField`

Modes:

- `IndirectY`
- `IndirectYWithOffset`
- `NeedsAddressArithmetic`
- `NeedsIndexScaling`
- `Unsupported`

Reject reasons:

- `NotPointer`
- `UnknownRecordField`
- `FieldOffsetTooWide`
- `IndexHasSideEffects`
- `NonByteIndex`
- `ElementNeedsScaling`
- `UnsupportedShape`

Main rules:

- `p^` where `p` is a pointer can use `(zp),Y`;
- `p(i)` derives its mode from the nested index-address proof;
- byte-indexed byte pointers can use `(zp),Y`;
- wider pointee sizes require index scaling;
- record pointer fields can use `(zp),Y` with an offset when the field offset
  fits in Y addressing;
- record fields crossing the one-byte Y offset limit require address arithmetic.

Current consumers:

- guards pointer-backed effective-address lowering;
- documents why pointer/record shapes can or cannot use direct indirect-Y
  addressing.

## Value Availability Proofs

Type: `ValueAvailabilityProof`.

Fields:

- `width: Option<u16>`
- `source: ValueAvailabilitySource`
- `bytes: [Option<ValueByteAvailability>; 2]`

Sources:

- `Constant`
- `Storage`
- `RoutineReturn`
- `Unknown`

Byte availability:

- `Constant(u8)`
- `Slot { slot, byte_index }`
- `Register(A/X/Y)`
- `PublicReturnSlot { slot, byte_index }`

Main rules:

- constants expose immediate low/high bytes;
- direct scalar storage exposes slot bytes;
- scalar routine calls expose bytes according to the routine internal ABI;
- known return facts may prove a result byte is already in `A`, `X`, or `Y`;
- unavailable or unsupported expression bytes remain `None`.

Current consumers:

- scalar call-result byte loads;
- byte function return comparisons against constants;
- simple byte loads for constants/storage;
- assignment fallback for constants/storage.

Instrumented observability:

- accepted `value-availability` events for proven call result bytes already in
  `A`;
- accepted assignment fallback events;
- rejected events when an instrumented consumer sees the wrong source kind,
  unknown width, unsupported width, or unavailable byte.

## Routine Visibility Facts

Type: `RoutineVisibilityFacts`.

Fields:

- `retargetable`
- `address_taken`
- `internal_only_candidate`

Main rules:

- generated from `RoutineBoundaryProof`;
- routine assignment targets are treated as retargetable/address-taken.

Current use:

- helps decide whether modern codegen may treat a routine as internal-only or
  must preserve public/patchable entry behavior.

## Routine Boundary Proofs

Type: `RoutineBoundaryProof`.

Fields:

- `name`
- `kind`
- `system_address`
- `retargetable`
- `address_taken`
- `internal_only_candidate`
- `public_entry_required`
- `patchable_entry_required`
- `internal_abi_candidate`

Kinds:

- `System`
- `Retargetable`
- `InternalCandidate`

Main rules:

- system routines are public boundaries;
- compatible routine-name assignment makes its target retargetable and requires
  public, patchable entry behavior;
- plain non-retargetable routines are internal ABI candidates.

An address-observable entry does not by itself require a trampoline. A direct
entry label remains stable for calls, `@routine`, machine-block address bytes,
and `RUNAD`; only code that rewrites the entry `JMP` operand requires that
physical instruction. The current `address_taken` fact is the conservative
legacy routine-assignment fact, not an inventory of ordinary label relocations.

Current use:

- supports modern entry/trampoline decisions;
- protects Action routine assignment semantics.

## Call Boundary Proofs

Type: `CallBoundaryProof`.

Fields:

- `temp_home: VirtualTempHome`
- `callee_effects: RoutineEffects`
- `survives: bool`

Main rule:

- wraps the zero-page temp survival check against known callee effects.

Current use:

- foundation for carrying virtual temp facts across calls only when known safe.

## Zero-Page Temp Lifetime Proofs

Type: `ZeroPageTempLifetimeProof`.

Fields:

- `temp_home`
- `calls_crossed`
- `survives_all_calls`
- `first_blocking_call`

Main rule:

- scans crossed call effects and records the first call that clobbers the temp,
  if any.

Current use:

- foundation for deciding whether a virtual temp can remain in zero page across
  a span of code.

## Zero-Page Temp Placement Proofs

Type: `ZeroPageTempPlacementProof`.

Fields:

- `width`
- `candidate`
- `blocked_by_occupied_slot`

Main rule:

- scans the configured zero-page temp pool and chooses the first candidate that
  does not overlap occupied slots.

Current use:

- foundation for configurable modern zero-page temp allocation.

## Current Boundary

The proof layer is still mostly codegen-local. It is not yet a whole-program
analysis framework and it does not try to prove every optimization opportunity.
The preferred direction is:

- add facts/proofs in small, semantic units;
- log accepted proof-guided lowering;
- expose rejected attempts only through debug tooling;
- avoid encoding large source-pattern rewrites directly in proof consumers;
- carry these facts forward into TAC/SSA rather than replacing that layer.
