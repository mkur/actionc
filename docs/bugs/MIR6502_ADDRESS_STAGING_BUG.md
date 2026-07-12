# MIR6502 Address Staging Bug

Snapshot date: 2026-06-02.

This note records a correctness bug introduced while adding address-consumer
materialization for pointer dereference. The fix should be done with a stronger
reasoning/coding model and kept narrowly scoped.

Related documents:

- `docs/archive/implementation-plans/mir6502/MIR6502_ADDRESS_CONSUMER_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/archive/implementation-plans/mir6502/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`

## Failing Source Shape

The failing pattern is a pointer variable initialized to an address, followed by
a word store through the pointer:

```text
CARD POINTER p

PROC Main()
  p = address_constant
  p^ = word_constant
RETURN
```

The important MIR shape is:

```text
store.w p, address_constant
v0 =.w load p
store.w *v0+0, word_constant
return
```

## Symptom

The new address-staging path no longer performs the previous ordinary temp spill,
but it stages the indirect pointer from the wrong memory location.

The source pointer value is stored in the storage for `p`, but the emitted code
stages the zero-page pointer pair from the former temp/spill home instead.

That means the transformation is currently closer to:

```text
stage pointer pair from temp/spill home for v0
```

instead of the required:

```text
stage pointer pair from the producer load source, i.e. p
```

There is also a second correctness issue: the first indirect byte access at
offset zero does not explicitly establish the required index value. Unless the
tracker has proven the index register already has that value, the low-byte
indirect store is unsafe.

## Root Cause Hypothesis

The producer-load-to-address-staging rewrite removed or bypassed the ordinary
word-temp materialization, but it kept using the temp's assigned spill/memory home
as the source for staging.

For this pattern:

```text
v0 =.w load p
store.w *v0+0, value
```

address staging must carry the original producer source:

```text
load source = p
```

not the storage assigned to the produced temp:

```text
temp home = spill(v0)
```

The bug is therefore in MIR materialization / address-staging rewrite, not in
tracked emission alone.

## Required Invariants

### Producer source must be preserved

If a word load producer is consumed only as an address, and materialization fuses
that producer into address staging, the staging operation must read from the
producer load's source memory.

It must not read from the temp's ordinary spill home unless the temp was actually
materialized there and contains the correct value.

### Indirect offset must be established

Every indirect load/store through a zero-page pointer pair must establish the
requested index offset before the memory access, unless tracked state proves that
the index register already contains that value.

For offset zero, either emit the required index setup or prove it is already
known. Do not assume zero implicitly.

### Emission must not recover the meaning

Tracked emission must receive an already-correct staged-indirect MIR shape. It
must not rediscover that the address came from pointer variable `p`.

## Expected Correct Shape

The correct materialized shape should be conceptually:

```text
store.w p, address_constant
stage_addr.zp fixed_pair, mem p
store_indirect.w fixed_pair +0, word_constant
return
```

The staged pointer pair should be loaded from `p` directly, not from a temporary
spill allocated for `v0`.

The indirect word store should perform two byte stores through the staged pair:

```text
store low byte through fixed_pair at offset 0
store high byte through fixed_pair at offset 1
```

Each offset must be explicit or proven by tracked state.

## Regression Requirements

Add a focused regression for the pointer-store shape:

```text
CARD POINTER p
PROC Main()
  p = address_constant
  p^ = word_constant
RETURN
```

The test should assert:

- object code is generated;
- address staging reads from the pointer variable storage, not from a temp/spill
  home;
- the low-byte indirect access establishes offset zero or has a tracked proof that
  the offset is already zero;
- the high-byte indirect access uses offset one;
- existing scalar fixtures remain green.

A byte-level or disassembly-level regression is preferred because this bug can
look plausible at MIR shape level while still emitting incorrect code.

## Out Of Scope

Do not add broad optimization while fixing this bug.

Out of scope:

- memory constant propagation;
- replacing the pointer dereference with direct absolute stores;
- general copy propagation;
- general zero-page allocation;
- register allocation;
- peepholes.

The fix is a correctness repair for address-consumer materialization.

## Suggested 5.5 Task

```text
Fix MIR6502 address-staging materialization correctness.

Goal:
- For a word pointer load consumed by a dereference, stage from the producer load's
  source memory, not from the temp/spill assigned to the loaded value.
- Ensure the first indirect byte access establishes the requested offset, including
  offset zero, unless tracked state proves the offset already has that value.
- Add a regression for the pointer-store pattern.
- Keep the change narrowly scoped to address-staging materialization and indirect
  emission correctness.

Do not implement:
- memory constant propagation;
- direct absolute replacement of the dereference;
- general copy propagation;
- general zero-page allocation;
- peepholes.

Acceptance:
- pointer dereference store emits correct object code;
- staging reads the pointer variable storage directly;
- indirect offsets are established safely;
- existing scalar and pointer fixtures remain green.

Suggested commit:
- mir6502: fix pointer address staging source
```
