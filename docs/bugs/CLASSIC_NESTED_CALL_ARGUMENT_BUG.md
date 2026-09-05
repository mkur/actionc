# Classic nested-call argument preservation

Status: nested-argument regression fixed, 2026-09-05. Exposed by the stage 3
Oscar64 port against compiler `590ef47`. The separate word-return accumulator
fact observation below remains open.

## Trigger and regression coverage

`fixtures/runtime/oscar64/fastcalltest.act` retains the original expression:

```action
original=P1(5,P2(C2(2),C2(4)))-13
```

`P1` adds, `P2` multiplies, and `C2` calls the identity function `C1`.
Compatibility formerly produced 8 instead of 0 under both ActionCart and
Standalone. All 512 Compatibility VM cases failed this original residual;
Optimized and MIR6502 passed the same port. The repaired path passes all
1,536 cases, including runtime word/byte inputs, repeated calls, exactly-once
counters, unchanged inputs, and full host-page guards.

From `tools/vm-runtime-tests`:

```sh
cargo test --locked --test oscar64_conformance oscar64_nested_calls
```

This remains an active test, with its original expression and oracle, no
ignored mode, and no expected panic. Root tests verify the fixture's NIR but
do not execute the isolated VM crate.

## Cause and repair

`src/codegen/call.rs::emit_call_or_tail_jump` formerly selected protective
stack staging for nested routine calls only when modern optimizations were
enabled. The Compatibility fallback evaluated the first `C2` result into the
shared public argument/return bytes `$A0/$A1`, then called the second `C2`
without saving the first result. The pre-fix listing shows:

```asm
JSR C2          ; result 2 in $A0/$A1
; no preservation of the first result
JSR C2          ; result 4 overwrites $A0/$A1
LDA $A1
STA $A3
LDA $A0
STA $A2         ; second argument = 4
LDY $A2
LDX $A1
LDA $A0         ; first argument is now also 4
JSR P2
```

Thus the program computed `5 + 4*4 - 13 = 8`. This is argument-lifetime
corruption, not a multiplication error or a leaf-inliner selection problem.
The emitted runtime-input and byte-companion calls had the same missing
preservation shape.

The existing helper is now `emit_left_to_right_staged_call_arguments`, shared
across classic profiles. Its single-call result-forwarding shortcut remains
modern-only, and ordinary no-call argument paths are unchanged. The existing
call-detection visitor also descends through casts, so a cast cannot hide an
argument's nested call from the preservation decision.

Each argument is materialized at the ABI base and immediately pushed, then all
arguments are restored to their final ABI offsets after evaluation. No earlier
argument remains live in shared return slots across a later call. Using the
base also avoids a mixed-width edge found by the focused tests: copying a
word return from `$A0/$A1` into an argument at `$A1/$A2` with low-byte-first
accumulator forwarding overwrites the still-needed high byte. There is no
need for that overlapping copy while earlier arguments are already stacked.

Three focused emitted-execution tests cover both profiles, BYTE/INT/CARD and
mixed-width arguments, nonzero high bytes, nested/repeated calls, cast-wrapped
byte-to-word results, exactly-once producer counts, full result/input guards,
and a stack canary around the caller. Opaque ABI producers/capture routines
isolate caller behavior from helper linking and inferred return facts; the
unchanged Oscar64 port additionally covers real Action! callees and both
runtime linkings. Legacy call-statement diagnostics remain in place.

To inspect the listing, from the repository root:

```sh
cargo run --bin actionc -- --mode compatibility --runtime standalone \
  --listing /tmp/fastcall-compat.asm -o /tmp/fastcall-compat.xex \
  fixtures/runtime/oscar64/fastcalltest.act
```

## Separate observation: optimized word-return accumulator fact

An exploratory pointer-preservation probe also exposed an independent
Optimized failure, outside the `fastcalltest` port:

```action
CARD n=$0600,i=$0602,base=$0604,otherBase=$0606,observed=$0608,updated=$060C
BYTE calls=$060A
CARD POINTER p
CARD FUNC Next()
  calls==+1
  p=otherBase
RETURN(n-i-1)
PROC Main()
  p=base
  calls=0
  observed=p(Next())
  updated=p
RETURN
```

With `n=129`, `i=1`, `base=$5001`, `otherBase=$5803`, the captured-base
load must read word 127 at `$50FF`. Compatibility does; Optimized does not.
The callee correctly stores the subtraction's low byte in `$A0` and high byte
in `$A1`, leaving A holding the **high** byte. The optimized caller immediately
emits `STA $C0; LDA $A1; STA $C1`, incorrectly treating A as the low byte of
the index. The pointer itself is saved/restored correctly around the call.

Audit inferred return-lane facts and their consumers separately. The focused
pointer-order regression uses a byte-returning effectful call with the nested
word subtraction in its caller, isolating base lifetime from this return-value
fault. No pre-existing port or oracle was changed. This observation is recorded
for a subsequent focused execution regression and compiler repair; it is not
counted in the Oscar64 case totals.
