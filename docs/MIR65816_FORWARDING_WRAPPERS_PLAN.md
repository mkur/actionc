# MIR65816 forwarding wrappers

Status: F1 implemented; F2 and F3 remain planned.

## Objective

Compile a pure forwarding wrapper as one far jump to its callee. For example,
Exec's `IsMinListEmpty` forwards its original pointer through a type-only cast
and returns `IsListEmpty`'s BYTE result unchanged:

```asm
IsMinListEmpty:
    JML IsListEmpty
```

With compiler `4bf1b1015ddb7f52db3cc162ba65247e2bcfd38c`, the existing Exec
compile measures this wrapper at 94 code bytes, an 8-byte frame and a 14-byte
local stack peak. Acceptance is **4 code bytes, no frame, and zero additional
local stack use**. The caller's existing argument area and return address, and
the callee's stack requirements, still exist.

The [native ABI](MIR65816_PHYSICAL_ABI_V2.md#4-calls-and-stack-alignment) already
supports this layout: arguments are at entry S + 4 + their ABI offset, and the
original caller owns argument cleanup. JML preserves S and the existing far
return address. The callee's RTL returns to that original caller.

## Scope and admission

Select from verified, typed MIR before allocating homes. Require all of:

- One executable block, containing only direct, nonvolatile reads of incoming
  value parameters, permitted identity casts, one direct call and its return.
  No frame objects, addressed or mutated parameters, stores, other calls,
  machine blocks, branches or extra work. Reject unrecognized operations even
  when their results appear unused.
- Every callee argument traces to the corresponding original parameter. There
  is no argument reordering, duplication, omission, constant replacement or
  arithmetic. Accept explicit parameter loads and already-normalized parameter
  values in raw and optimized MIR.
- Casts preserve representation. Initially accept pointer-to-pointer casts
  with identical physical width, and exact identity casts. Equal byte widths
  alone do not authorize integer conversions or changes to return semantics.
- Caller-facing and callee argument layouts match exactly: count, order,
  offsets, sizes, alignments, padding and reserved argument extent. Compare
  physical ABI facts rather than requiring equal source signature IDs;
  `MinList POINTER` and `List POINTER` intentionally have different types.
- The native result contract matches exactly, and the returned value is the
  call result without conversion. A procedure may forward only to a procedure
  and return immediately; discarding a function result is outside this scope.
- The target is an ordinary generated routine in the current program with a
  known native boundary. External/runtime/helper/indirect targets and raw
  assembly entry points retain ordinary calls in this version.
- No self-forwarding or mutually forwarding cycle. Check the small graph of
  candidate wrappers before selection; an admitted chain must end at an
  ordinary routine. This prevents removing every entry guard from a cycle.

Every operation and temporary must be explained by the proof. Unsupported
wrappers continue through the existing allocation and call selectors.

## Selection, emission and accounting

Introduce one backend-owned forwarding plan carrying the direct target and the
parameter/result correspondence. Compute it with access to the program's
routine contracts before normal frame allocation. Recheck its eligibility in
allocation verification; an empty home map is not evidence of a valid wrapper.
The shared NIR and public native ABI do not need new forms.

An admitted wrapper receives no object, temporary, staging or scalar-DP homes.
Emit a typed terminal far transfer with a `Target::Routine` fixup. Teach the
instruction/effect and selected-code replay paths that it preserves entry S,
D, DBR, I and native widths, inherits the incoming arguments/return address,
and has no continuation in this routine. Do not model it as a JSL followed by
an omitted RTL. Preserve source-span coverage for the removed forwarding
operations and attribute the jump to the call/return selection.

Keep the wrapper's public symbol, signature and distinct entry address. Calls
and function pointers can still address it. Use the existing 24-bit routine
relocation for JML, including destinations in another bank; retain the target
dependency for linking and reachability. Do not merge symbols or alias their
addresses.

There is no wrapper stack reservation or memory access to guard. In checked
builds, the destination checks its own requirements before consuming stack.
Normal routines retain their existing guards, including zero-frame routines
that do not satisfy this proof. Unchecked builds retain their selected policy.

Update allocation's local-peak calculation and image-map construction together:

- `fixed_frame`, `spill_bytes` and `local_stack_peak` are zero; objects and
  temporary maps are empty. Incoming displacements use frame zero.
- Preserve the wrapper's declared argument extent and result layout. Its
  argument extent describes the caller-facing ABI, not a new reservation.
- The existing `calls` map lists actual local call reservations, so it is empty
  for this wrapper. Retain the tail target in compiler selection/fixup data;
  do not invent a normal call with zero outgoing bytes or zero return bytes.
- Keep `whole_task_stack_bound` unknown. Zero local cost must not be presented
  as the stack requirement of the callee or of the whole task.

This fits the current image-map schema and adds no o65 metadata. Confirm it
with image validation and Exec's existing frame-map checker. Relevant compiler
entry points are [materialization](../src/mir65816/emit/mod.rs),
[allocation](../src/mir65816/emit/allocation.rs),
[call-result forwarding](../src/mir65816/emit/call_returns.rs),
[effects](../src/mir65816/emit/effects.rs),
[replay](../src/mir65816/emit/replay.rs) and
[image maps](../src/mir65816/image.rs).

## Implementation slices

Commit each completed slice with its focused checks.

1. **F1: one-argument function wrappers.** Deliver the complete proof,
   allocation, typed JML, relocation and map path for one unchanged scalar or
   pointer argument and an identical native scalar/pointer result contract.
   Include pointer reinterpretation so the exact `IsMinListEmpty` pattern
   qualifies. Preserve ordinary fallback and reject candidate cycles. Test
   emitted raw/optimized code, the four-byte body, sparse maps, direct return
   to the original caller and corrupted admission proofs.
2. **F2: remaining simple forwarding shapes.** Extend the same correspondence
   proof to zero/multiple arguments and procedure wrappers. Test mixed-width
   layouts with alignment gaps and terminal padding, wrapper chains, all native
   result lanes and calls through a pointer to a public wrapper. Complete
   bank-crossing, o65 relocation, stack-boundary and IRQ/NMI checks. No argument
   shuffling, outgoing-area rewrite or general recursive tail-call elimination.
3. **F3: Exec measurement and contract documentation.** Compile unchanged Exec
   inputs with an explicitly recorded local compiler override. Measure both
   function and procedure wrappers, including `IsMinListEmpty` and `NewMinList`;
   list admitted wrappers and reasons for notable fallbacks. Report routine
   bytes, frames, local peaks and total Exec code separately, and validate every
   emitted frame map. Update the emission/ABI documentation to describe the
   inherited call frame and terminal transfer. Keep the compiler pin and play
   image unchanged in this work; hosted image qualification is a separate step.

Reserved bank-zero change for **each slice: 0 fixed bytes, 0 bytes per task**,
including guards, alignment and unused reserved capacity. Reduced dynamic stack
use does not reduce Exec's reserved task-stack regions.

## Validation and completion

Use focused tests during each slice, then batch the affected MIR65816 checks
once at the end. Retain the backend's full CI coverage; no unrelated backend or
full release run is needed for this scoped implementation.

- Positive execution cases cover raw and optimized MIR, parameter identity and
  pointer casts, native result lanes, padding, wrapper chains and cross-bank
  returns. Inspect the four emitted bytes rather than only matching source or
  IR. Preserve native registers/domains and final S according to the ABI.
- Negative cases cover reordered/duplicated arguments, size/layout/result
  mismatches, conversions, extra work, locals, mutable/addressed parameters,
  volatile/indirect reads, unsupported targets and forwarding cycles. Rejected
  cases retain the ordinary implementation and its guards.
- At an exact stack floor, the callee's reservation succeeds; one byte below
  the required floor, it faults before writing. Verify subtraction underflow,
  ceiling checks and interrupt headroom without expecting the removed wrapper
  frame. Exercise IRQ/NMI at the forwarding boundary with existing task-domain
  instrumentation, normalizing and checking LF/CRLF fixture inputs.
- Run affected allocation/selection/effect/replay units, emission and o65
  integrations, and native runtime call/stack tests. Review changed snapshots;
  do not accept output changes solely because size decreased.

Done means the four-byte wrapper executes and relocates correctly, maps and
typed proofs describe its actual costs, rejected cases retain correct behavior,
and the unchanged Exec build has recorded measurements. General tail calls,
inlining, pointer-argument capture cleanup in other routines, combined cleanup
sequences and linker aliases remain separate work.

## Slice results

F1 implements single-argument function forwarding before normal allocation.
The selected terminal transfer has an independent entry-environment check and
typed replay; the frame verifier recomputes admission and cycle rejection.
Image maps retain the incoming signature and omit nonexistent call reservations.
Three focused compiler tests and one native execution test pass, covering raw
and optimized MIR, native result lanes, pointer reinterpretation, negative
admission, corrupted proofs/maps and forwarding cycles. The executed wrapper is
four bytes, preserves the entire entry register state except PC/PBR, performs no
stack writes, and returns to the original caller. Fixed/per-task bank-zero
reservation delta: **0 / 0 bytes**. No Exec image has been refreshed.
