# SemIR Native Layer Plan

SemIR-native should stay organized around four separate layers:

1. typing;
2. classification;
3. materialization;
4. emission.

Each layer has its own plan:

- [Backend Status](SEMIR_NATIVE_BACKEND_STATUS.md): current SemIR-native
  maturity, validation state, open risks, and recommended next focus.
- [Typing Layer](SEMIR_NATIVE_TYPING_PLAN.md): semantic facts and language
  legality.
- [Classification Layer](SEMIR_NATIVE_CLASSIFICATION_PLAN.md): read-only
  backend shape recognition.
- [Materialization Layer](SEMIR_NATIVE_MATERIALIZATION_PLAN.md): classified
  shapes to concrete registers, ABI bytes, storage, and zero-page homes.
- [Emission Layer](SEMIR_NATIVE_EMISSION_PLAN.md): tracked 6502 instruction
  writing.
- [Stress Backlog](SEMIR_NATIVE_STRESS_BACKLOG.md): missing native backend
  shapes found by the stress sweep.

When a lowering bug appears, decide which layer owns the missing fact before
adding a special case. If a change touches multiple layers, land it as separate
slices unless the tests require the full vertical path.

## Layer Contracts

Typing answers what a program means in Action terms. It owns widths,
signedness, pointer targets, record identity, lvalue legality, callable
signatures, array origin, record fields, and control-flow facts.

Compatibility caveat: Action source compatibility is not absolute. Old programs
can rely on undocumented typing and lowering behavior, especially around raw
pointer decay, routine addresses, machine blocks, resident library entry points,
and self-modifying or patchable code. Treat newly discovered original-source
idioms as compatibility work: document the behavior, add a focused probe or
test, and place the fix in the owning layer instead of assuming the modern
compiler already matches the cartridge compiler.

Classification answers how the backend can obtain a typed value or place. It
owns shapes such as literal, storage, address value, dereference, indexed
element, call result, computed value, and unsupported backend shape.

Materialization answers how to put a classified shape into a concrete machine
home. It owns register, ABI, target-slot, and zero-page-pointer placement.

Emission answers which exact 6502 bytes are written and how tracked processor
state changes. It owns opcode forms, labels, patching, state tracking,
source-map recording, and raw data barriers.

## Migration Rule

Every SemIR-native cleanup should say which layer it changes:

- typing: adds or corrects semantic facts;
- classification: recognizes a backend shape without emitting code;
- materialization: turns a classified shape into machine homes;
- emission: improves concrete instruction writing or tracked-state behavior.

The preferred flow is typing facts first, then classification, then
materialization, then emission. Compatibility fixes should still preserve this
direction unless a very small localized emission change is enough and does not
add semantic shape logic.

For stress backlog work, the backlog is only the queue of missing shapes. The
owning layer plan remains the source of truth for how to implement each slice.
Most current stress blockers should be materialization slices: first confirm
the SemIR typing facts exist, add only the classifier shape query needed by the
materializer, then route the consumer through that materializer. Emission should
change only when the materializer needs a reusable concrete helper or tracked
state correction.

## Current State

Typing is stable for the native backend's current needs. The backend still
depends on semantic widths, pointer targets, array facts, record fields, and
call signatures, but there is no active evidence that broad typing work is the
next bottleneck.

Classification is also at a healthy plateau. `NativeClassifier` is the backend
shape facade for values, lvalues, address-like values, calls, byte/word
sources, and compare operands. The older classifier migration note remains
useful as history, but the concise layer note is the active contract.

Materialization is at a healthier plateau after the call/ABI staging pass. It
owns most reusable value-to-home, slot-copy, return-slot, address-of,
pointer/index/address, record-field, indirect array-element, and call-argument
staging. Call argument byte homes, the word-to-AX home, and SARGS byte staging
now live in `native_materialize.rs`. No separate ABI-home vocabulary has been
introduced yet because the moved paths did not create enough duplication to
justify it. The stress backlog reopens materialization as the active feature
layer, because the next failures are mostly missing value/address/call
materializers rather than broad typing or emission gaps.

Emission is at a strong plateau. Concrete instruction writing is centralized in
`src/codegen/semir_native/native_emit.rs`, and high-level SemIR-native lowering
is guarded against direct `self.emitter.emit_*` calls. Future emission work
should be targeted, not broad.

## TAC Runway Mode

The layer plans now serve a narrower near-term purpose: keep SemIR-native
correct, compiling, observable, and structured enough to feed TAC. New work
should be accepted when it closes a real compile/runtime gap, improves
validation, or preserves semantic facts that TAC will need. It should be
deferred when it is mainly code-size tuning, AST-byte mimicry, local instruction
scheduling, or broad register/allocation strategy.

Backlog items still move as vertical slices, but each slice should leave behind
a TAC-reusable fact or a cleaner layer boundary. Toolkit byte deltas that do
not block compilation or expose a semantic issue are `tac-deferred`; use them
as measurement data, not as a reason to grow SemIR-native into another
optimizer.

## Recommended Next Focus

Recommended next focus:

1. Keep coverage/mixed sweeps free of `SEMFAIL` and classify any new failure in
   the owning layer before editing backend code.
2. Add markdown sweep output so stress/toolkit state can be captured directly
   in status documents.
3. Add a minimal TAC IR and `--emit-tac` output path fed by SemIR; it should be
   structural and observable before it tries to optimize.
4. Continue stress backlog items only when they are correctness/coverage gaps.
   Record-field computed stores and word pointer/index materialization remain
   valid examples of materialization-owned vertical slices.
5. Mark remaining Toolkit code-size deltas as `tac-deferred` unless comparison
   triage shows a semantic bug or a boundary cleanup TAC will reuse.
