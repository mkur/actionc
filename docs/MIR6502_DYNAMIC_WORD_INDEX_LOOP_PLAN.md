# MIR6502 Dynamic Word-Index Loop Plan

Status: implemented. Created 2026-09-04; implemented 2026-09-04.

This plan continues
[`MIR6502_GENERAL_CODEGEN_OPTIMIZATION_PLAN.md`](MIR6502_GENERAL_CODEGEN_OPTIMIZATION_PLAN.md)
and
[`MIR6502_STATIC_ARRAY_AFFINE_INDEX_PLAN.md`](MIR6502_STATIC_ARRAY_AFFINE_INDEX_PLAN.md).
It covers the remaining general code-generation gap exposed by a CRC8 loop over
a runtime pointer and runtime `CARD` length. The benchmark is an observation
workload, not an optimization contract: no analysis or selector may depend on
the CRC routine name, polynomial, source spelling, or a particular buffer size.

The implementation order is:

```text
dynamic word-loop facts
    -> bottom-tested equality loop
    -> pointer/countdown strength reduction
    -> 6502 cursor and countdown selection
    -> bounded constant-trip unrolling
    -> flag-aware CRC-step simplification
```

The first four slices target the generic bound and address cost. The last two
address the independent fixed eight-iteration inner-loop cost.

## Implementation outcome

The implementation adds a pre-home dynamic word counted-loop fact and an
atomic selector that rotates the loop while replacing its index with a virtual
zero-page cursor/countdown pair. The zero-length guard stays outside the loop,
the base is materialized once, and the hot path uses an ordinary indirect-Y
load, a carrying 16-bit cursor increment, and a borrowing 16-bit countdown.
Late zero-page resolution maps the virtual address consumer to its allocated
physical pair before the post-home boundary.

The optimized configuration also enables an exact 2-through-8-iteration
unroller with a bounded growth policy. Every cloned MIR definition receives a
fresh temporary identity. A later flag-aware selector replaces the byte
high-bit test diamond with the carry produced by `ASL`; conservative store
coalescing then keeps consecutive recurrences in A where control-flow and
memory-effect proofs permit it.

The accepted scope is deliberately narrow: byte elements at offset zero, one
invariant pointer base, a dead final index, and no call, opaque effect, or
possibly aliasing write. Absolute, fixed-zero-page, and global memory inputs
are not speculatively hoisted. Rejected candidates retain the existing generic
lowering.

For the standalone Atari CRC8 observation workload, the cursor/countdown path
reduced elapsed time from 73 to 66 PAL ticks. Adding the fixed-eight unroll and
carry-driven recurrence reduced it to 33 ticks (about 0.66 seconds), while the
CRC routine changed from 274 to 124 bytes and the XEX from 1577 to 1427 bytes.
The external `JSR` remains. These measurements are evidence, not test
contracts.

## Motivating MIR shape

NIR already promotes the outer `CARD` index to a loop block parameter. The
corresponding pre-home MIR has the essential shape:

```text
preheader:
    jump header(0:word)

header(index:word):
    bound = load length
    in_range = compare.u16 index < bound
    branch in_range body, exit

body:
    base = load data
    value = load base[index; scale 1]
    ...

latch:
    next = index + 1
    jump header(next)
```

The current post-home code correctly keeps the word induction in zero page, but
each byte still pays for both a two-lane relational comparison and a two-lane
`base + index` calculation. The fixed inner loop already carries its byte
counter in X, so extending only the existing byte carrier does not remove the
main outer-loop cost.

## Architectural boundary

This work belongs in MIR6502. NIR already provides typed word arithmetic,
block parameters, the normalized CFG, structured addresses, and storage/effect
facts. The transformation is motivated by 6502 costs and chooses zero-page
pointer pairs, indirect-Y addressing, and machine flags, so it must not add
6502 strategy to NIR or recover source facts from SemIR.

The rewrite runs before home assignment, after CFG canonicalization has exposed
the loop and while `MirAddr::ComputedIndex` or an equivalent structured address
is still available. Concrete zero-page and A/X/Y choices remain later
materialization decisions. Every CFG mutation consumes explicit counted-loop
facts, rebuilds the affected structured MIR, and is followed by the appropriate
MIR phase verifier.

Calls, machine blocks, volatile accesses, absolute or unknown writes, pointer
writes that may alias a loop input, and opaque effects remain barriers unless
structured effects prove the exact required preservation.

## Slice 0: focused fixture and baseline

Add a small runtime fixture containing an unsigned runtime-pointer loop with a
runtime `CARD` length. Keep the body simpler than CRC8 so the fixture tests the
loop and address contract rather than a specific checksum algorithm. Exercise
lengths 0, 1, 255, 256, 257, 8192, and 65535 where the runtime environment can
do so cheaply, and use bases on both sides of a page boundary.

Record the current pre-home MIR and emitted hot-loop characteristics:

- a head-tested word `<` comparison;
- a per-iteration computed `base + index` address;
- a word increment and unconditional backedge;
- the current CRC8 Atari tick count and routine size as observations.

Exact benchmark ticks and total bytes are not test assertions. Correct results,
analysis facts, verifier acceptance, and selected MIR shapes are contracts.

## Slice 1: dynamic word counted-loop facts

Generalize counted-loop facts without changing existing byte-loop selection.
The fact model needs to distinguish both induction identity and bound kind, for
example:

```rust
enum MirLoopInduction {
    Memory(MirMem),
    BlockParam(MirTempId),
}

enum MirLoopBound {
    Constant(u16),
    Invariant(MirValue),
}
```

The final spelling may differ, but selectors must not infer these distinctions
from sentinel values. Existing post-home byte selectors use accessors for a
memory induction and constant bound and otherwise reject the candidate, keeping
their behavior unchanged.

Recognize this first dynamic profile:

- unsigned byte or word induction;
- initial value zero;
- ascending unit step;
- head test `induction < bound`;
- one preheader, header, body entry, latch, and exit;
- a unique backedge carrying the updated block parameter;
- a bound that is loop invariant under structured use/def and effect analysis;
- an explicit fact describing whether the final induction value is observable.

A parameter load is not automatically invariant. Reject it if a body operation
can write or alias that parameter home. Initially reject calls, machine blocks,
volatile bound loads, unknown writes, and indirect writes with insufficient
alias information.

This is an analysis-only commit. Add positive fact tests for the normalized
pre-home form and negative tests for signed comparisons, non-unit steps,
multiple backedges, bound mutation, unknown effects, and an observable final
index.

Suggested telemetry:

- `dynamic-word-loop-candidate`;
- `dynamic-word-loop-blocked-bound-invariance`;
- `dynamic-word-loop-blocked-shape`;
- `dynamic-word-loop-blocked-final-index`.

## Slice 2: rotate to a bottom-tested equality loop

For a proven unsigned `0 ..< bound` unit-step loop, rewrite:

```text
while index < bound:
    body
    index = index + 1
```

to:

```text
if bound == 0:
    exit
do:
    body
    index = index + 1
while index != bound
```

The zero guard is mandatory for a dynamic bound. For every nonzero 16-bit
bound, starting at zero and advancing by one reaches the bound before unsigned
wrap. The equality latch therefore has the same iteration count as the source
loop.

The initial slice accepts only an index that is dead on exit. A later extension
may reconstruct the source-visible final value as `bound`; it must not silently
drop an observable index store.

Run this rewrite in the pre-home CFG group. Preserve block arguments and edge
types explicitly, recompute analysis after mutation, and verify the pre-home
phase. A cost check should confirm removal of the repeated relational compare
and header backedge rather than relying on a source pattern alone.

Acceptance signals:

- the dynamic zero guard remains outside the hot loop;
- the hot latch compares equality against the invariant bound;
- the word `<` header and unconditional latch-to-header jump are gone;
- zero and maximum-length boundary tests retain exact behavior.

Suggested telemetry: `dynamic-word-loop-rotated`.

## Slice 3: index-to-cursor strength reduction

When all loop uses of the induction are affine, unscaled address uses of one
invariant pointer base, replace the loop-carried index with a cursor and a
remaining count:

```text
cursor = base
remaining = bound
if remaining == 0:
    exit

loop:
    value = *cursor
    ... body ...
    cursor = cursor + 1
    remaining = remaining - 1
    if remaining != 0:
        loop
```

The transformation must use structured MIR address operations. It must not
emit raw instructions or preselect a physical pointer scratch pair. Preserve
16-bit wrapping: advancing `cursor` modulo 65536 is equivalent to computing
`base + index` modulo 65536 for the accepted profile.

Initially require:

- element size one and byte offset zero;
- one invariant pointer base;
- no non-address use of the index;
- no observable final index;
- no store capable of changing the pointer value or remaining count;
- no operation whose effects invalidate the base, bound, or cursor recurrence.

Constant affine byte offsets and multiple loads from the same cursor may be a
later extension once their page-crossing and access-order contracts are
explicit. Scaled elements and mixed bases remain on the general path.

The selected MIR should materialize the base once in the preheader. It must no
longer contain a per-iteration `MaterializeIndexedAddress` using the original
word index.

Suggested telemetry:

- `indexed-loop-cursor-candidate`;
- `indexed-loop-cursor-selected`;
- `indexed-loop-cursor-blocked-index-use`;
- `indexed-loop-cursor-blocked-alias`.

## Slice 4: select the 6502 cursor and word countdown

Feed the cursor recurrence into normal home planning, preferring a virtual
zero-page pair when its whole-loop benefit exceeds its setup cost. Select an
ordinary `IndirectIndexedY` consumer with Y fixed at zero and explicitly
advance the pointer after the body.

Do not reuse `PagedIndirectIndexedY` for this form. Its current contract keeps
the effective low address in Y and does not carry Y wrap into the prepared
pointer high byte. The cursor loop instead owns an ordinary pointer pair and an
explicit 16-bit increment, so page crossings remain correct.

Select a direct zero-tested 16-bit decrement latch equivalent to:

```asm
    LDA remaining
    BNE no_borrow
    DEC remaining+1
no_borrow:
    DEC remaining
    BNE loop
    LDA remaining+1
    BNE loop
```

and a cursor advance equivalent to:

```asm
    INC cursor
    BNE no_carry
    INC cursor+1
no_carry:
```

Exact instruction order may change when live A or flags make another form
cheaper. Selection must use machine liveness and effect facts and must preserve
all observable A/X/Y/flag values represented by MIR.

Acceptance signals:

- one pointer materialization in the preheader;
- one `(zp),Y` byte access per source access with no per-byte address addition;
- no original index home or word bound comparison in the hot loop;
- exact behavior across `$xxFF -> $xx00` pointer transitions;
- no regression in existing constant byte-loop and Mad Pascal fixtures.

## Slice 5: bounded constant-trip inner-loop unrolling

Add a generic pre-home unroller for small, exact constant trip counts. Begin
with canonical single-entry/single-latch loops of 2 through 8 iterations and no
early exit. Require complete effect and code-growth accounting; calls, machine
blocks, volatile operations, opaque effects, or ambiguous phi/block arguments
reject the candidate.

Unrolling the CRC8 bit loop should remove its X counter, `CPX #8`, increment,
and backedge. This is a speed optimization with deliberate code growth, unlike
the cursor rewrite, so selection needs an explicit bounded growth/cycle policy.
Do not silently turn the existing balanced configuration into unrestricted
unrolling.

Suggested telemetry:

- `small-counted-loop-unroll-candidate`;
- `small-counted-loop-unrolled`;
- `small-counted-loop-unroll-blocked-growth`.

## Slice 6: flag-aware shift/conditional-xor selection

After unrolling exposes each byte recurrence, recognize the general identity:

```text
if value & $80 != 0:
    result = (value << 1) ^ constant
else:
    result = value << 1
```

On 6502, select the carry produced by `ASL` as the condition:

```asm
    ASL A
    BCC no_xor
    EOR #constant
no_xor:
```

The proof must establish byte-width shift semantics, the exact high-bit mask,
identical shifted value on both arms, and no intervening carry clobber. It is a
generic MIR value/flag rewrite, not a CRC-polynomial special case. Existing
machine-value analysis should then keep the recurrence in A across consecutive
unrolled steps and remove dead local reloads/stores.

## Deferred page/tail strip-mining

Do not make dynamic page/tail strip-mining part of the first implementation.
A full-page `INY/BNE` loop is attractive for large buffers, but a generic
runtime length needs both a page loop and a tail loop and may duplicate the
body. That tradeoff needs a cycle-aware optimization goal and a code-growth
model. First measure the cursor/countdown form; add page/tail selection later
only when it is demonstrably profitable under an explicit policy.

## Validation matrix

Every rewriting slice includes unit tests for the analysis and structural MIR
tests for the selected form. The automated runtime coverage includes lengths
0, 1, 255, 256, 257, and 300, with a base immediately before a page boundary;
the standalone benchmark exercises length 8192. It checks low-byte and
high-byte pointer carry and CRC semantics after inner-loop unrolling.

The following cases remain the validation target for future scope extensions:

- a bound stored in a parameter and an ordinary local;
- a practical maximum-length runtime case;
- rejection on pointer mutation, possible aliasing, calls,
  machine blocks, volatile memory, signed comparison, and non-unit steps.

Current negative unit tests cover an observable final index, bound mutation,
opaque effects, excessive unroll growth, and unsafe unroll effects. The
remaining cases above should be added alongside any matcher broadening rather
than weakening the conservative fallback.

After each slice run the focused MIR6502 tests and `cargo test`. If a slice
changes NIR lowering or verification, also run the NIR snapshot and sweep
commands required by `AGENTS.md`. Recompile and run the Atari CRC8 benchmark
after material code-generation slices, reporting ticks and routine/XEX size
without promoting those values to golden tests.

## Completion signal

For the motivating CRC8 function, the completed generic path should have:

- no word `index < length` comparison in the byte loop;
- no per-byte `base + index` materialization;
- one loop-carried cursor and remaining count;
- no fixed-eight inner counter or backedge after speed-oriented unrolling;
- eight flag-aware shift/conditional-xor steps with the CRC value retained in
  a machine register where liveness permits;
- the ordinary external `JSR` to the CRC function unchanged.

The implementation is complete only when the generic fixtures and negative
proof cases pass. Closing a particular benchmark timing gap is evidence of
quality, not the correctness criterion.
