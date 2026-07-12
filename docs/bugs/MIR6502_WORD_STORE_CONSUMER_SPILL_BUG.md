# MIR6502 Word Store-Consumer Spill Bug

Snapshot date: 2026-06-02.

This note records a MIR6502 code-quality bug that is close to a materialization
correctness issue: word expressions consumed immediately by a store are being
materialized through ordinary word spills instead of directly into the store
destination.

This belongs under `docs/bugs/` because it is a concrete regression/debug handoff.
The higher-level plans remain in `docs/`.

Related documents:

- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/archive/implementation-plans/mir6502/MIR6502_IMPLEMENTATION_PLAN.md`
- `docs/archive/implementation-plans/mir6502/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/archive/implementation-plans/mir6502/MIR6502_ADDRESS_CONSUMER_MATERIALIZATION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`

## Failing Source Shape

The representative source shape is:

```action
CARD x,y

PROC Main()
  x = 1
  y = x + $0102
  y ==- 1
RETURN
```

Exact constants may vary. The important pattern is:

```text
word expression -> immediately stored to word destination
word compound assignment -> load/compute/store same destination
```

## Symptom

Generated code is correct-looking but very spill-heavy. The current lowering /
materialization path effectively does:

```text
x = 1
load x -> temp0
compute temp0 + constant -> temp1
copy temp1 -> y
load y -> temp2
compute temp2 - constant -> temp3
copy temp3 -> y
```

This creates multiple ordinary word temp/spill homes for values that are consumed
immediately by a store.

The desired target-materialization shape is:

```text
x = 1
compute x + constant directly into y
compute y - constant directly into y
```

This should be done as byte-lane operations with explicit carry/borrow facts.

## Root Cause Hypothesis

The materializer treats word expression results as durable values by default. It
assigns each intermediate word value an ordinary temp/spill home, then later
copies that home into the store destination.

For a single-use word expression consumed immediately by `Store`, the durable
ordinary word home is unnecessary. The store destination itself is the natural
consumer home.

This is analogous to the address-consumer problem, but the consumer is a word
store destination instead of a zero-page pointer pair.

## Required Invariant

When a word expression is consumed immediately by a store, materialization should
target the destination memory directly when it is safe to do so.

The key invariant is:

```text
Do not materialize into an ordinary temp/spill unless a durable temp is actually
needed.
```

A durable temp may still be required when:

- the value has multiple uses;
- the value crosses a block boundary;
- the consumer requires a durable home;
- calls, barriers, machine blocks, or effects force materialization;
- aliasing or storage instability makes direct materialization unsafe.

## Expected Materialized Shape

For this MIR pattern:

```text
v0 =.w load x
v1 =.w add v0, constant
store.w y, v1
```

materialization should produce a byte-lane sequence equivalent to:

```text
load x.lo
add constant.lo with carry_in=Clear
store y.lo
load x.hi
add constant.hi with carry_in=FromPrevious
store y.hi
```

For this compound-assignment pattern:

```text
v2 =.w load y
v3 =.w sub v2, constant
store.w y, v3
```

materialization should produce a byte-lane sequence equivalent to:

```text
load y.lo
sub constant.lo with carry_in=Set
store y.lo
load y.hi
sub constant.hi with carry_in=FromPrevious
store y.hi
```

The exact MIR/printer syntax may differ. The invariant is that no ordinary word
spill appears between the expression and the immediately consuming store.

## Regression Requirements

Add a focused fixture for word store-consumer materialization:

```action
CARD x,y

PROC Main()
  x = 1
  y = x + $0102
  y ==- 1
RETURN
```

The test should assert:

- object code is generated;
- the `x + constant` word expression is materialized directly into `y`;
- the `y ==- 1` compound assignment is materialized in place or directly into
  `y`;
- no ordinary word-temp spill appears between the expression and the store;
- explicit carry/borrow facts are preserved;
- existing scalar, pointer, and call fixtures remain green.

A byte-level or disassembly-level regression is preferred because the MIR can be
structurally valid while still creating excessive temporary traffic.

## Out Of Scope

Do not turn this into a general optimizer.

Out of scope:

- global constant propagation;
- alias-sensitive load/store forwarding;
- dead store elimination;
- common subexpression elimination;
- general register allocation;
- cross-block value propagation;
- cross-block branch layout optimization;
- automatic zero-page allocation;
- hardware-register-aware store removal.

The fix is a local materialization improvement for immediate word store
consumers.

## Suggested Fix Task

```text
Implement MIR6502 word store-consumer materialization.

Goal:
- Recognize single-block word expressions consumed immediately by `Store`.
- For word Add/Sub with constants or direct loaded values, materialize byte-lane
  operations directly into the destination memory.
- Preserve explicit carry/borrow facts.
- Handle direct compound-assignment shape where the source and destination are the
  same word storage.
- Add a regression for `CARD x,y; y=x+const; y==-1`.

Do not implement:
- global copy propagation;
- memory constant propagation;
- alias-sensitive optimization;
- general register allocation;
- peepholes.

Acceptance:
- no temporary word spill appears between `x+const` and storing to `y`;
- no temporary word spill appears for `y==-1`;
- object code remains correct;
- existing scalar, pointer, and call fixtures remain green.

Suggested commit:
- mir6502: materialize word store consumers directly
```
