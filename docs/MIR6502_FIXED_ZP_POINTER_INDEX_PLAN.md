# MIR6502 Fixed-Zero-Page Pointer Index Plan

Status: planned. Created 2026-09-01.

This plan continues
[`MIR6502_STATIC_ARRAY_AFFINE_INDEX_PLAN.md`](MIR6502_STATIC_ARRAY_AFFINE_INDEX_PLAN.md)
with the general pointer-code generation gaps exposed by comparing the Action!
and Mad Pascal pointer Flame kernels. Flames is a measurement workload, not an
optimization contract: no selector may depend on a routine name, framebuffer
address, neighbour offsets, loop trip count, or source spelling.

The implementation order is:

```text
fixed-ZP pointer home as an indirect consumer
    -> indirect-indexed byte accumulation in A
    -> exact pointer-region proof for loop alias checks
    -> compact full-range descending latch, if still needed
```

Each slice is independently useful, tested, and committed before the next
slice begins.

## Baseline

The current working-tree Action! source and the attached Mad Pascal binary
perform the same hot computation:

- three word pointers in fixed zero page at `$F0`, `$F2`, and `$F4`;
- an inclusive byte loop from 255 down to 0;
- four byte reads at pointer offsets 30, 31, 32, and 63;
- byte-width addition followed by two right shifts;
- a byte store through the original pointer and a one-byte pointer increment.

The framebuffer addresses differ between the two suites, but their relative
layout and accessed ranges are equivalent. Display setup and the Mad Pascal
DLI/status-line code are outside this plan.

Measured in the same NTSC Atari800 250-frame window:

| Measurement | actionc MIR6502 | attached Mad Pascal binary |
|---|---:|---:|
| completed outer iterations | 54 | 105 |
| hot-loop code | about 240 bytes | about 105 bytes |
| estimated cycles per 256-element iteration | about 369 | about 193 |

The local current Mad Pascal toolchain produces an approximately 93-byte hot
loop by also eliminating the attached binary's one-byte partial-sum spill.
These numbers are validation signals, not regression-test assertions.

The important actionc shapes are:

1. A pointer already resident at `$F0/$F1` is copied to the generic `$AC/$AD`
   consumer before each indexed read.
2. Each input byte and partial sum is routed through a compiler spill.
3. Indirect writes conservatively appear able to alias the `$E0` induction
   home, so the existing counted-loop selector retains a guarded subtract
   rather than selecting a compact `DEC` latch.

Pointer increments already select the desired `INC low / BNE / INC high`
sequence. They are not part of this work.

## Goals

- Use an eligible source-declared fixed-zero-page pointer pair directly as a
  6502 `($zp),Y` consumer.
- Keep single-use byte addition chains in A and emit `ADC ($zp),Y` without
  materializing private RHS or partial-sum homes.
- Prove exact indirect-write regions where a canonical constant pointer
  induction makes that possible, and use the proof to unblock existing loop
  selection.
- Select a source-exact full-range descending byte latch when the existing
  counted-loop forms cannot express it compactly.
- Improve ordinary pointer-heavy programs, not only the benchmark.

## Non-goals

- General points-to or interprocedural alias analysis.
- Assuming that arbitrary pointer dereferences cannot reach hardware, absolute
  storage, compiler-managed storage, or another source object.
- Moving, duplicating, or deleting volatile or otherwise observable pointer
  reads.
- Adding a target addressing mode to NIR or consulting SemIR from MIR6502.
- Optimizing word elements, scaled pointer indexes, descriptor-backed arrays,
  non-unit pointer induction, or arbitrary pointer arithmetic in the initial
  implementation.
- Optimizing the 32-byte random seed loop before the main 256-element kernel is
  correct and measured.

## Architectural boundary

SemIR continues to own pointer type, lvalue legality, volatility, and source
evaluation order. NIR continues to carry explicit typed pointer loads, indexed
loads and stores, arithmetic, storage identity, control flow, and conservative
effects. MIR6502 owns recognition of a physically adjacent fixed-zero-page
pointer pair, selection of `($zp),Y`, A/Y placement, target-local address-range
proofs, and counted-loop latch strategy. Emission only writes the already
selected operation.

No slice may make MIR6502 recover facts from source text or display names. If
an exact pointer fact is unavailable, the existing `$AC/$AD` materialization
and conservative memory effects remain the fallback.

## Shared safety invariants

- The pointer low and high bytes must resolve to the same logical word at
  adjacent fixed-zero-page addresses. `$FF` cannot be the low byte.
- Bypassing a pointer snapshot must not change source evaluation order. If
  evaluation of an index or value can change the pair, or lowering has already
  made an explicit snapshot semantically necessary, retain the snapshot.
- The fixed pair and Y must remain unchanged from preparation through the
  indirect consumer.
- Every source byte is read exactly once and in source order.
- Calls, machine blocks, opaque effects, volatile barriers, writes that may
  modify the pair, and unsupported CFG crossings reject a candidate.
- Pointer dereferences remain conservative indirect memory effects unless an
  exact selector-local region proof applies. Do not globally reinterpret
  unknown indirect accesses as ordinary RAM.
- All rewrites must win the MIR6502 cost model and leave verifier-clean
  post-home MIR.

## Slice 1: consume fixed-ZP pointer homes directly

### Goal

Recognize a pointer value loaded from a word whose physical low/high homes are
an eligible fixed-zero-page pair and select:

```text
MirAddr::FixedIndirectIndexedY { zp: pair_low }
```

instead of copying the word into `DEFAULT_POINTER_PAIR` before the access.

### Implementation

Add one shared analysis helper beside the indexed-address materializer in
`src/mir6502/materialize/indexes.rs`. Its result should include:

- the logical low/high `MirMem` identities;
- the resolved `MirFixedZpSlot` low byte;
- the operations or values proving that the consumer reads that word;
- the stability range over which the pair may be consumed directly.

Initially accept the exact `pointer_value_from_mem` word shape: two
`PointerCell` lanes from offsets zero and one of the same logical storage.
Resolve absolute-backed globals and aliases through `MaterializeLayout`; check
that both addresses are below `$100` and consecutive. Keep logical storage
identity in the analysis and introduce the physical slot only in post-home
MIR.

Teach byte indexed reads and writes to request that consumer before falling
back to `DEFAULT_POINTER_PAIR`. Stores that are already recovered by a later
peephole should converge on the same helper so read and write eligibility does
not diverge.

Do not look through an explicit temporary word snapshot in this slice. Such a
snapshot may encode required evaluation order. Dynamic index expressions are
accepted only when the existing MIR proves their evaluation cannot modify the
pair before the consumer.

### Tests

Add focused materializer tests for:

- fixed pairs at `$00`, `$F0`, and another non-benchmark address;
- constant and byte-valued indexes;
- direct byte reads and writes;
- an absolute-backed global alias that resolves to an eligible pair;
- a pair at `$FF`, nonadjacent lanes, an ordinary global pointer, and a
  descriptor-backed value retaining the scratch path;
- an index-side write to either pointer lane retaining the snapshot;
- a call, machine block, or opaque barrier between preparation and use;
- a word-element access retaining the general path.

Add an end-to-end neutral pointer fixture that checks the generated MIR6502
shape without naming Flames or using its addresses.

### Acceptance criteria

- Eligible reads and writes contain `(fixed_zp $xx),y` in post-home MIR.
- The accepted access contains no writes to `$AC/$AD` solely to stage the
  pointer.
- The pointer pair is read by the operation's structured effects.
- All negative fixtures remain on the current general materialization path.

Commit:

```text
mir6502: address through fixed zero-page pointer homes
```

## Slice 2: indirect-indexed byte accumulation in A

### Goal

Generalize the static-array A-chain so an eligible addition chain can emit:

```asm
ldy #30
lda ($f0),y
iny
clc
adc ($f0),y
```

and continue through subsequent inputs without private loaded-value or
partial-sum spills.

### MIR shape

Replace the absolute-only source contract of
`BinaryDirectIndexedByte` with a small structured post-home source enum, for
example:

```text
MirByteIndexedSource
    Absolute { base: MirMem, index: X|Y }
    FixedIndirectY { zp: MirFixedZpSlot }
```

Rename the operation to `BinaryIndexedByte` if that produces a clearer
contract than retaining the historical name. Do not use an unrestricted
`MirAddr`: the verifier should admit only the addressing forms emission can
write.

The operation still has A as its implicit left operand and destination, and
retains explicit binary, carry-input, and carry-output fields. The first
implementation continues to admit only byte addition with clear carry input
and ignored carry output.

### Implementation

- Extend the existing analyzed rewrite in
  `src/mir6502/materialize/indexes.rs`; do not add an instruction-stream
  peephole in emission.
- Recognize an indirect source only after Slice 1 has established a stable
  fixed pair. Preserve the original order of all language-visible reads.
- Remove only compiler-private home traffic for single-use loaded values and
  partial sums. Do not move an indirect access across another executable
  operation.
- Require the loaded byte and intermediate result to have the same narrow
  use/def and liveness properties as the existing absolute-indexed selector.
- Record Y and both fixed pointer bytes as machine/memory inputs and record an
  indirect memory read in the effects summary.
- Extend verifier, printer, standalone remapping/validation, rewrite visitors,
  liveness, census tooling, size estimation, and emission together.
- Emit `ADC ($zp),Y` through the typed emitter; never reconstruct an opcode in
  a generic printer or peephole.

The rewrite may eliminate private spills even though an arbitrary source
pointer could numerically address compiler workspace: those homes are not
source-language stores. It must nevertheless preserve the number and order of
all pointer reads and reject any explicit source storage access or observable
effect in the removed range.

### Tests

Positive coverage:

- two- and four-input byte sums through one fixed pointer;
- a chain switching between two stable fixed pointer pairs;
- constant Y changes using `INY` and explicit `LDY` forms;
- shifts and an indirect store consuming the final A value;
- page crossings of the pointed-to address.

Negative coverage:

- multiple use of an input or partial sum;
- carry-in from a previous operation or observed carry-out;
- subtraction until it has an independently specified contract;
- a write to either pointer byte, Y clobber, call, barrier, or machine block;
- volatile access movement or an attempted duplicate read;
- a non-fixed, nonadjacent, or `$FF` pointer pair.

### Acceptance criteria

- A four-input eligible sum emits one indirect indexed `LDA` and three
  indirect indexed `ADC` instructions.
- Each input remains a single read in source order.
- No RHS or partial-sum spill remains in the selected chain.
- Static-array users of the existing operation remain unchanged.
- Verifier tests reject every unsupported indexed source form.

Suggested telemetry:

```text
fixed-indirect-byte-binary-selected
```

Commit:

```text
mir6502: accumulate fixed-pointer byte loads in A
```

## Slice 3: exact fixed-pointer regions for alias checks

### Goal

Prove the address interval written by a narrow class of canonical fixed-pointer
loops. Use that proof to answer whether an indirect body write can modify the
counted-loop induction home, instead of treating every such write as
`may_write_any` for that one selector.

### Proof object

Add a target-local fact such as `MirFixedPointerRegionFact` containing:

- the fixed pointer pair identity;
- a dominating exact initial pointer value;
- a canonical unit pointer update and its direction;
- the proven iteration range;
- the minimum and maximum access displacement;
- the checked absolute interval touched by each accepted read or write;
- the CFG blocks over which the fact is valid.

Initially derive it only when:

- a unique dominating assignment gives the complete 16-bit initial value;
- counted-loop facts provide a constant trip range;
- all loop paths perform the same unit update or no update as required;
- address arithmetic does not wrap `$FFFF`;
- no call, machine block, opaque write, pointer escape, or write to either pair
  lane can invalidate the value;
- the indirect operation uses a constant displacement or a separately proven
  byte range.

Unknown cases produce no fact.

### Integration

Keep `op_may_write_mem` conservative. Add a selector-scoped query used by the
counted-loop latch analysis which first consults an exact pointer-region fact
and otherwise delegates to the existing conservative query. A body is
non-aliasing only when every indirect write has a proven region disjoint from
the induction home.

Do not cache a region fact across an operation that can invalidate its
dominating assignment or update proof. Do not use read-only ranges to narrow
write effects.

### Tests

- A constant fixed pointer advanced once per iteration, writing a region
  disjoint from a zero-page counter, permits the loop transformation.
- Boundary-touching and overlapping intervals reject it.
- High-address overflow, pointer wrap, an unknown index, missing update on one
  path, an early exit, pair mutation, a call, and an opaque effect reject it.
- Two and three independently proven pointer regions can coexist.
- An unproved pointer write elsewhere in the same loop keeps the conservative
  answer.

### Acceptance criteria

- The exact proof discharges only the induction-alias check for eligible
  loops.
- Global pointer effects and unrelated optimizers remain no less conservative.
- The neutral fixture selects an existing direct `DEC` latch after the proof.
- Flames no longer retains `ADC #$FF` merely because its framebuffer writes
  are indirect.

Commit:

```text
mir6502: prove fixed-pointer loop write regions
```

## Slice 4: full-range descending byte latch, only if required

### Entry condition

After Slice 3, inspect the neutral fixture and Flames listing. Skip this slice
if the existing counted-loop selector already emits a compact source-exact
form.

### Goal

For a proven-entered inclusive unsigned loop from 255 through 0 with step -1,
select the equivalent shape:

```asm
        jmp body
latch: dec counter
body:  ; body leaves counter unchanged and non-aliased
        ...
        lda counter
        bne latch
```

The body executes once for every byte value and the source-visible final value
remains zero; the counter never undergoes a terminal underflow.

### Implementation and safety

Extend typed counted-loop analysis or its MIR6502 selector with a named
full-range descending shape. Do not infer it from block labels or emitted
comparisons. Require:

- byte width, unsigned direction, step one, initial 255, and bound zero;
- a proven first entry and canonical single latch/backedge;
- no unsupported exit, counter mutation, or unproved alias in the body;
- dead or explicitly preserved machine flags and accumulator state;
- a source-visible final counter value of zero on the normal exit;
- a strict size win over the guarded form.

Reuse the region proof from Slice 3 for indirect writes. Calls, barriers,
machine blocks, dynamic starts, other bounds, and noncanonical CFGs reject the
shape.

### Tests and acceptance criteria

- The transformed loop visits 255, 254, 1, and 0 exactly once.
- Empty, one-iteration, dynamic-start, early-exit, nested, and aliasing cases
  retain their supported general forms.
- The selected latch has one `DEC`, no per-iteration top comparison, and no
  terminal restoration block.
- Counter and machine-state observations after the loop match the original
  MIR.

Commit:

```text
mir6502: select full-range descending byte latches
```

## Validation and documentation

After every slice:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Also run the focused MIR6502 verifier/materializer tests and compile a neutral
pointer kernel in both debug-listing and executable modes. If an MIR6502
fixture changes, classify it as an intentional target-materialization change,
not an NIR contract change.

At the end:

- update `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md` for the new indexed-binary
  source and any new counted-loop shape;
- record selection/rejection telemetry for the fixed-pointer consumer,
  indirect A-chain, pointer-region proof, and optional latch;
- regenerate the benchmark listing and confirm that all three pointer rows use
  their declared pairs directly;
- rerun the 250-frame emulator measurement, reporting iteration count, hot-loop
  bytes, and a static cycle estimate without turning those values into brittle
  assertions;
- compare at least two non-benchmark pointer workloads to catch code-size or
  register-pressure regressions.

The target outcome for the Flame hot row is the general shape:

```asm
ldy #30
lda ($f0),y
iny
clc
adc ($f0),y
iny
clc
adc ($f0),y
ldy #63
clc
adc ($f0),y
lsr
lsr
ldy #0
sta ($f0),y
```

The exact register schedule may differ when a cost-equivalent form preserves
stronger liveness facts. Correctness, verifier strictness, and generality take
precedence over matching one compiler's listing byte for byte.
