# MIR6502 Counted-Loop Latch Relaxation Plan

Status: proposed. Created 2026-09-04.

This plan is a focused continuation of
[`MIR6502_GENERAL_CODEGEN_OPTIMIZATION_PLAN.md`](MIR6502_GENERAL_CODEGEN_OPTIMIZATION_PLAN.md).
It extends the existing counted-loop analysis and latch selector; it does not
introduce a second loop recognizer or an AES-specific transform.

The motivating AES observation is useful because it exposes two general
limitations. The selector currently requires A and every modeled 6502 flag to
be dead at both the body and exit of a head-tested loop, and it accepts an
exact loop rotation only when the whole routine becomes smaller. Consequently,
only one counted-loop latch is selected in the current AES build (`SubBytes`).
Canonical loops in `AddRoundKey`, `MixColumns`, and `XorWithIv` are recognized
but cannot be rotated because their bodies consume the induction value that
the header leaves in A. A repeated cycle saving can also be rejected when
reconstructing first-entry state adds a few static bytes.

The implementation order is:

```text
candidate/rejection telemetry
    -> explicit first-entry state contract
    -> A-live exact head rotation
    -> live compare-flag preservation
    -> trip-count-aware profitability
    -> conservative rollout to other exact-compare shapes
```

Each slice is independently useful, receives focused tests, and should be one
reviewable commit. Correctness relaxation and profitability relaxation must not
be combined in the same commit.

## Baseline and expected scope

After the recent MIR6502 address-reuse work, the standalone Atari AES workload
measures 35.9 seconds, or 1795 PAL ticks, and produces a 4064-byte XEX. The
previous measurement was 39.9 seconds, or 1997 ticks, and 4271 bytes. The
current counted-loop latch selector reports one application in `SubBytes`.

Representative missed loops have the following target shape:

```text
preheader:
    induction = 0
    jump header

header:
    A = load induction
    compare A, bound
    branch body, exit

body:
    use A as the index or arithmetic input
    ...
    jump latch

latch:
    increment induction
    jump header
```

The current rotation redirects `preheader` directly to `body` and places the
unchanged `header` after `latch`. This makes the latch-to-header edge a
fall-through, but skips the header on first entry. The all-dead gate is
therefore safe but stronger than necessary: first-entry A and flag state can be
reconstructed explicitly, while every backedge continues to execute the
original header.

AES timings, routine names, bounds, and addresses are observations rather than
selection contracts. The expected AES gain is modest compared with inlining
the very frequently called `XTime`; the purpose of this work is to remove a
general loop-rotation restriction before adding a broader inliner.

## Architectural boundary

The existing division remains intact:

- `mir6502::analysis::counted_loops` recognizes typed induction, direction,
  step, bound, entry guard, final-value observability, and loop shape;
- `MirMachineLiveness` says which physical register and flag values are
  observable on each CFG edge;
- `mir6502::materialize::cfg` chooses and applies the target-specific layout;
- the emitter only encodes the already selected MIR operations and CFG.

Do not add a source-level `FOR` fact, benchmark matcher, new MIR opcode, or
emitter peephole. Reuse the canonical header already accepted by counted-loop
analysis. If first-entry replay metadata is useful only to this selector, keep
its extraction local to materialization; enrich `MirCountedLoop` only when a
second consumer needs the same fact.

Calls, machine blocks, volatile or address-taken induction storage, ambiguous
predecessors, aliasing writes, non-unit steps, wrapping ranges, and required
dynamic entry guards retain their existing conservative treatment. This plan
does not weaken memory-effect or induction-update proofs.

## Correctness contract

For a head-tested loop with a statically proven first iteration, rotation must
preserve the machine state visible at both successors of the original header:

- first body entry observes the same live A value and live C/Z/N/V flags as it
  did after the original header;
- later body entries and the final exit still pass through the unchanged
  header and therefore keep their original state;
- the induction memory is initialized, updated, and finally observed exactly
  as before;
- the number and order of volatile operations are unchanged;
- no execution that originally took the initial exit may be redirected into
  the body.

The canonical byte header uses a load of the induction value and a compare
against the bound. For a proven-entered loop, the selector may reproduce the
header's state-producing prefix in the preheader while dropping only its
known control decision. The proof must describe this explicitly; absence of a
liveness bit is not a value-equivalence proof.

In particular:

- if A is live, its required value is the induction value loaded by the
  canonical header;
- if C, Z, or N is live, replay the exact compare after establishing the same
  A value;
- V is preserved by the canonical load/compare sequence, so replay is allowed
  only when the incoming V value is itself preserved;
- exit liveness is not a reason to reject an exact rotation because the exit
  remains reached through the original header. Keep the existing exit gates on
  compare-free countdown and underflow transforms until separately proved.

## Slice 0: candidate and rejection telemetry

Make the selector explain why recognized byte counted loops do not become
latch plans. Return a small selection report rather than only an application
count, then feed it into the existing MIR6502 peephole report. At minimum,
distinguish:

- recognized candidates;
- required initial guard;
- live A requiring first-entry reconstruction;
- live compare flags requiring first-entry reconstruction;
- unsupported shape or unsafe induction memory;
- failed layout/profitability check;
- selected exact rotation, with or without reconstructed state.

De-duplicate sites by stable routine/header identity because the selector
reanalyzes the routine after every successful rewrite. Do not count every
non-loop block or turn the report into a generic diagnostics framework.

Acceptance:

- existing aggregate and per-routine reports remain deterministic;
- the focused A-live and flag-live fixtures report the intended blocker;
- the current AES build identifies the three representative missed loops
  without consulting their names;
- emitted code is unchanged.

Suggested commit: `mir6502: report counted-loop latch blockers`.

## Slice 1: explicit first-entry state contract

Replace the boolean all-dead prerequisite for exact head rotation with a small
plan describing first-entry repair. One possible local representation is:

```text
FirstEntryState
    None
    LoadInductionIntoA
    ReplayHeaderPrefix
```

The representation is illustrative; the implementation may instead carry a
verified operation prefix. The important property is that planning records
what state is required and application performs exactly that repair.

Add one helper that validates the canonical state-producing header prefix. It
must prove that the operations can be replayed, that they read the same
non-volatile induction and bound, and that no omitted header operation has an
observable effect. Refuse headers with extra stores, calls, opaque machine
effects, volatile reads, or values whose equivalence depends on block-local
definitions.

Teach `apply_rotated_head_tested_plan` to insert the planned repair immediately
before the preheader terminator, after the existing induction initializer. The
applicator should validate block identity and edge shape before mutation so a
failed application leaves the routine unchanged.

This slice establishes the proof and plan plumbing but may retain the current
selection policy. It is complete when tests can construct, validate, apply,
and reject first-entry plans without enabling new production candidates.

Suggested commit: `mir6502: model first-entry state for loop rotation`.

## Slice 2: allow an A-live body

Enable exact head rotation when A is live on first body entry and the canonical
header proves that A contains the current byte induction value. Prefer, in
order:

1. no inserted load when existing machine-value facts prove the preheader
   already leaves the same induction value in A;
2. one explicit `Load A, induction` after initialization;
3. rejection when neither proof is available.

The first implementation need not add new global value analysis. Use existing
machine-value facts when directly available and otherwise emit the explicit
load. This keeps the optimization general while avoiding a new mechanism just
to remove one setup instruction.

Keep compare flags dead on first body entry in this slice. Keeping that gate
makes A reconstruction independently testable and limits the semantic change.
Because the exit still executes the original header, remove the A/flag
deadness requirement at the exit only for this exact rotation, not for the
other countdown plans.

Acceptance fixtures include:

- an ascending `0..<16` byte loop whose body immediately stores A as an index;
- a body whose first operation shifts A;
- non-zero constant initial values and one-iteration loops;
- a dead-A loop that still uses the zero-repair path;
- rejection for a dynamic initial guard, volatile/address-taken induction,
  non-canonical header, and ambiguous preheader;
- byte-for-byte unchanged behavior for zero-iteration and rejected loops.

The selected MIR must show an explicit first-entry load unless equivalence was
proved, an unchanged compare header on the backedge, and no unconditional hot
backedge jump after final layout.

Suggested commit: `mir6502: rotate counted loops with live induction in A`.

## Slice 3: preserve live compare flags

Generalize first-entry repair to C, Z, and N values produced by the canonical
compare. When any of those flags is live at body entry, replay the exact load
and compare prefix in the preheader. Do not synthesize an equivalent-looking
flag sequence: cloning the verified canonical prefix avoids signedness and
boundary mistakes.

V needs separate treatment because `LDA` and `CMP` preserve it. Rotation is
safe only when the replay also preserves the same incoming V. Use existing
flag liveness and unobserved-before-redefinition helpers; do not clear or
manufacture V.

Acceptance fixtures independently consume C, Z, N, and V before redefinition,
then exercise combinations of A and flag liveness. Include negative cases
where an intervening preheader operation changes V or where the header prefix
cannot be replayed exactly.

Suggested commit: `mir6502: preserve live flags across loop rotation`.

## Slice 4: trip-count-aware profitability

Replace the strict
`candidate_layout_bytes < original_layout_bytes` requirement for exact head
rotation with a target cost decision that accounts for one-time repair and the
repeated latch edge.

First add a tested helper that derives an exact trip count from the existing
unsigned byte facts only when the range is non-wrapping and the initial guard
has been proved unnecessary. Cover ascending and descending boundaries,
including 0, 1, 127, 128, 255, and full-range exclusions. Unknown or dynamic
trip counts do not receive a speculative frequency.

Then estimate the transform delta from:

- static bytes added by the first-entry repair and changed branch layout;
- setup cycles executed once;
- header, branch, and latch-edge cycles at their proven execution counts;
- taken versus fall-through control edges, including any long-branch
  expansion already represented by layout estimation.

Reuse `rewrite::posthome::estimated_6502_cost` for operation costs and the CFG
layout estimators for encoded control size. Add only the small branch-cycle
model that those helpers do not currently provide. Keep cost calculation next
to the selector rather than creating a compiler-wide execution-frequency
framework.

Selection policy:

- always accept a semantics-safe candidate that reduces size and cycles;
- allow only named, bounded static growth for a proven positive total cycle
  saving;
- require a small positive margin so an uncertain page-crossing cycle cannot
  decide the transform;
- retain the current strict size-decrease rule when trip count is unknown;
- do not relax compare-free countdown/underflow policies in this slice.

Use documented constants analogous to the existing bounded growth for fast
countdowns. Preserve the current configuration behavior initially; add a new
profile knob only if measurements demonstrate a real size-versus-speed policy
need.

Report the selected plan's estimated byte delta, trip count, and cycle saving
through telemetry. Unit tests assert the decision at threshold boundaries,
not AES timing.

Suggested commit: `mir6502: cost counted-loop rotation by trip count`.

## Slice 5: conservative shape rollout

Re-run the candidate census after the exact head-tested path is complete.
Apply the same first-entry-state proof to descending exact-compare loops only
where their header has the same replayable contract. Underflow, full-range,
bottom-guarded, and compare-free countdown plans keep their existing gates
until a fixture proves the precise state each transform must reproduce.

This slice is evidence-driven: if the census shows no additional safe and
profitable family, commit only any missing regression fixtures and leave the
other selectors unchanged. Do not factor superficially similar transforms
together when their exit or flag contracts differ.

Suggested commit, if warranted: `mir6502: reuse entry-state repair for exact countdowns`.

## Validation matrix

Every behavior-changing slice runs focused unit tests in
`src/mir6502/materialize/cfg.rs`, relevant source-to-MIR integration tests, and
the full test suite:

```sh
cargo test mir6502::materialize::cfg
cargo test
```

The focused matrix covers:

- ascending and descending byte loops;
- zero, one, small, high-bit, and boundary trip counts;
- A dead, A live as induction, and A live with a different required value;
- C/Z/N/V dead, individually live, and live in combinations;
- observable and dead final induction values;
- dynamic entry guards and genuinely zero-iteration inputs;
- calls in the body, aliasing writes, volatile induction, address-taken homes,
  nested loops, extra predecessors, and multiple exits;
- layouts where a branch is short and where expansion would be required.

After each material slice, rebuild and run the standalone AES workload. Verify
the expected ciphertext/checksum first, then record PAL ticks, XEX size,
per-routine byte counts, selected latch sites, first-entry repair kinds, and
profitability estimates. These measurements decide whether to continue the
rollout, but are not test assertions.

## Stop/go criteria and follow-on

Proceed from state repair to cost relaxation only after runtime equivalence and
machine-state tests pass. Keep the telemetry even if no new candidate survives
the cost model; it documents the next real blocker without forcing a rewrite.

The work is successful when canonical loops that consume their induction in A
can use the existing rotated latch under an explicit proof, profitable repeated
savings are no longer rejected solely by a small setup cost, and no selector
depends on AES structure.

After rebenchmarking, the next larger AES opportunity remains a general,
costed small-routine inliner. Affine pointer/index work, register-home changes,
loop unrolling, and routine specialization are explicitly outside this plan.
