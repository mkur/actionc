# MIR6502 Call ABI Bug: First BYTE Argument Uses Stack Placeholder

Snapshot date: 2026-06-02.

This note records a MIR6502 call ABI correctness bug. It is placed under
`docs/bugs/` as a concrete regression note. Higher-level design plans should stay
in `docs/`; specific failing cases and fix handoffs should live here.

Related documents:

- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/archive/implementation-plans/mir6502/MIR6502_IMPLEMENTATION_PLAN.md`
- `docs/archive/implementation-plans/mir6502/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/ACTION_STORAGE_LAYOUT.md`

## Failing Source Shape

The minimal failing shape is a direct procedure call with one byte argument:

```action
PROC Take(BYTE x)
RETURN

PROC Main()
  Take(7)
RETURN
```

## Current Wrong MIR Shape

Current MIR shows the first byte argument assigned to a stack placeholder at
address zero:

```text
mir6502 program

routine r0 Take
b0 bb0:
  return

routine r1 Main
b0 bb1:
  call r0 args=[#$07.b -> stack $0000+0] result=- clobbers=- preserves=- effects=stack=?,reads=none,writes=none
  return
```

This is wrong for the Action! call ABI. The placeholder home must not reach MIR
emission.

## Expected MIR Shape

For a direct call with one `BYTE` argument, the first argument byte should be
assigned to register `A`:

```text
routine r1 Main
b0 bb1:
  call r0 args=[#$07.b -> A] result=- clobbers=... preserves=... effects=...
  return
```

A later pre-emission shape may materialize this as:

```text
A = #$07
call r0
return
```

The exact printer syntax may differ. The invariant is that the first byte
argument home is `A`, not `StackFrame { base: 0, offset: 0 }` or an absolute
store to zero page address zero.

## Symptom In Object Code

The emitted code currently stores the argument to address zero before the call,
then calls the procedure. This is a sign that a placeholder stack-frame home has
escaped into object emission.

Object emission is following the MIR it was given. The bug is in call ABI
planning / call materialization, not in tracked emission alone.

## Required ABI Invariant

For direct Action! calls:

```text
argument byte 0 -> A
argument byte 1 -> X
later bytes -> documented Action! ABI homes / SArgs-style argument area
```

The exact later-byte homes should follow the existing storage-layout and ABI
notes, but the first byte is clear: a single `BYTE` argument must be passed in
`A`.

If the callee reads the parameter, the callee side must consume the incoming ABI
home correctly. It may store `A` into parameter storage in a prologue, or it may
use an optimized equivalent, but the caller must not store the argument to a
placeholder address.

## Root Cause Hypothesis

The call ABI planner still uses a placeholder `StackFrame` home with base zero
for argument byte zero. That placeholder was useful while calls were only
represented structurally, but it is not a valid pre-emission ABI home.

Likely area to inspect:

```text
src/mir6502/call_plan.rs
src/mir6502/abi.rs
src/mir6502/materialize.rs
src/mir6502/emit.rs
```

The fix should start in the ABI planner. Emission should not special-case this by
recognizing the callee or source program.

## Regression Requirements

Add a focused MIR fixture for the minimal call:

```action
PROC Take(BYTE x)
RETURN

PROC Main()
  Take(7)
RETURN
```

The test should assert:

- MIR call argument byte zero is assigned to `A`;
- no `stack $0000+0` or equivalent placeholder appears in the call argument home;
- object emission does not emit a store to address zero for the argument;
- the call still targets the correct routine.

Add a second fixture once callee parameter reads are supported:

```action
BYTE seen

PROC Take(BYTE x)
  seen = x
RETURN

PROC Main()
  Take(7)
RETURN
```

That test should assert:

- caller places the argument in `A`;
- callee parameter use consumes the incoming `A` value correctly;
- no placeholder address-zero argument home is used.

## Out Of Scope

Do not fix this by adding broad call optimization.

Out of scope:

- full multi-argument ABI completion;
- indirect calls;
- SArgs frame packing beyond what is required to keep first-byte ABI correct;
- call inlining;
- peepholes;
- register allocation.

This is a first-byte direct-call ABI correctness fix.

## Suggested Fix Task

```text
Fix MIR6502 direct call ABI for the first byte argument.

Goal:
- In the call ABI planner, assign direct-call argument byte zero to `A`.
- Ensure `StackFrame { base: 0, offset: 0 }` or any equivalent placeholder cannot
  reach pre-emission MIR for the first byte argument.
- Update the MIR printer/tests so `Take(7)` shows the first argument home as `A`.
- Add an object-code regression proving no store to address zero is emitted for
  the argument.
- If callee parameter use is already in scope, ensure the callee consumes incoming
  `A` correctly.

Do not implement:
- full SArgs packing;
- indirect calls;
- call inlining;
- peepholes;
- broad register allocation.

Acceptance:
- `PROC Take(BYTE x); PROC Main(); Take(7)` passes through MIR and object emission.
- MIR shows the argument home as `A`.
- Object code loads the constant into `A` before the call.
- No argument store to address zero is emitted.
- Existing scalar and pointer fixtures remain green.

Suggested commit:
- mir6502: assign first byte call argument to A
```
