# MIR6502 Loop Consumer Spill Bug

Snapshot date: 2026-06-02.

This note records a MIR6502 code-quality bug in loop lowering/materialization.
The generated code is structurally correct but introduces unnecessary ordinary
spills for two local consumer patterns:

- compare result consumed immediately by a branch;
- byte compound assignment consumed immediately by a store.

This belongs under `docs/bugs/` as a concrete regression/debug handoff.

Related documents:

- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/archive/implementation-plans/mir6502/MIR6502_IMPLEMENTATION_PLAN.md`
- `docs/archive/implementation-plans/mir6502/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/bugs/MIR6502_WORD_STORE_CONSUMER_SPILL_BUG.md`

## Failing Shape

Representative source shape:

```text
BYTE x
Main loop:
  while x is less than a small constant:
    increment x by one
```

The important lowering patterns are:

```text
condition: load x, compare x with constant, branch
body:      load x, add constant, store back to x
```

## Symptom

Generated code currently materializes both the condition and increment through
ordinary temp/spill homes.

Conceptual current shape:

```text
condition:
  load x -> temp
  compare temp, constant
  branch

body:
  load x -> temp
  add temp, constant -> temp
  store temp -> x
```

Desired baseline shape:

```text
condition:
  compare x directly with constant
  branch

body:
  load x
  add constant
  store x
```

A later peephole may turn the increment into a dedicated increment instruction.
That peephole is out of scope for this bug.

## Root Cause Hypothesis

The materializer treats intermediate byte values as durable values by default.
It assigns a temp/spill home before the immediate consumer has a chance to use
the value directly.

Two consumer-directed paths are missing or incomplete:

1. compare/branch consumer materialization;
2. byte store-consumer materialization.

## Required Invariants

### Compare/branch consumers

When a value is loaded only to be compared and the compare feeds a branch, the
materializer should avoid assigning that value an ordinary temp/spill unless a
stable home is actually required.

For a direct memory left operand and constant right operand, materialization
should load the memory byte into the compare home and compare against the
constant, then branch from flags.

### Byte store consumers

When a byte expression is consumed immediately by a store, materialization should
target the destination memory directly when safe.

For byte compound assignment, the source and destination may be the same storage.
The materializer should preserve that in-place store target.

### Do not jump to peepholes first

Do not initially replace the load/add/store shape with a dedicated increment or
decrement instruction. First implement the general consumer-directed
materialization path. A later peephole may specialize the result.

## Regression Requirements

Add a focused loop fixture with:

```text
one BYTE variable
one loop comparing it with a small constant
one compound increment in the loop body
```

The test should assert:

- object code is generated;
- the condition does not store the loop variable into an ordinary temp/spill
  before comparing;
- the loop body does not store the intermediate increment source/result into
  ordinary temps/spills before storing back to the loop variable;
- branch targets remain correct;
- explicit carry facts are preserved for the byte add;
- existing scalar, pointer, call, and word-store consumer fixtures remain green.

A byte-level or disassembly-level regression is preferred because MIR can be
valid while still producing unnecessary temp traffic.

## Out Of Scope

Do not turn this into a broad optimizer.

Out of scope:

- increment/decrement peephole;
- global constant propagation;
- alias-sensitive load/store forwarding;
- dead store elimination;
- common subexpression elimination;
- general register allocation;
- cross-block value propagation;
- branch layout optimization;
- automatic zero-page allocation.

The fix is local consumer-directed materialization for compare/branch and byte
store consumers.

## Suggested Fix Task 1: Byte Store Consumers

```text
Implement MIR6502 byte store-consumer materialization.

Goal:
- Recognize single-block byte expressions consumed immediately by Store.
- For byte Add/Sub/And/Or/Xor with constants or direct loaded values, materialize
  directly into the destination memory.
- Preserve explicit carry/borrow facts for Add/Sub.
- Handle byte compound-assignment shape where source and destination are the same
  storage.

Do not implement:
- increment/decrement peephole;
- global copy propagation;
- memory constant propagation;
- alias-sensitive optimization;
- register allocation.

Acceptance:
- no temporary byte spill appears between increment expression and storing back
  to the variable;
- object code remains correct;
- existing scalar, pointer, call, and word-store fixtures remain green.

Suggested commit:
- mir6502: materialize byte store consumers directly
```

## Suggested Fix Task 2: Compare/Branch Consumers

```text
Implement MIR6502 compare/branch consumer materialization.

Goal:
- Recognize values loaded only for a compare whose result feeds a branch.
- Materialize the compare directly from the source value into the compare/flag
  path without an ordinary temp/spill when safe.
- Start with byte compare against constant.
- Preserve branch targets and signedness/unsignedness semantics.

Do not implement:
- broad compare optimization;
- word/signed compare expansion beyond the existing supported path;
- branch layout optimization;
- cross-block value propagation.

Acceptance:
- no temporary byte spill appears between loading the loop variable and comparing
  it with the loop bound;
- branch output remains correct;
- existing scalar, pointer, call, and store-consumer fixtures remain green.

Suggested commit:
- mir6502: materialize byte compare branch consumers directly
```
