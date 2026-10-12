# MIR6502 comparison loop pointer reuse implementation plan

Status: Stages 0–2 qualified, 2026-10-12. Starting source revision: `ce662d54`.
See the [maintained measurements](comparison-loops/README.md).

Keep two invariant record addresses available throughout a byte comparison
loop. Then reduce character staging and loop-index traffic where the same
proofs permit it. TN and TNDBG provide application measurements; generic
fixtures define the compiler behavior. Preserve record contents, comparison
order, early returns, source snapshots and the public Action! ABI.

## Objective and current evidence

[MyDosAfter](../../samples/tn/modern/MYDOS.ACT) compares directory flags, eleven
name bytes and then three detail bytes. Its pointers remain unchanged, but
the current MIR6502 name loop constructs `left + 6` and `right + 6` on every
iteration, alternately overwriting the same zero-page pointer pair. It stores
both characters into locals and reloads them for comparison. The detail loop
repeats the pattern with displacement 17.

Current qualified cartridge output, including the shared bitmap mask table:

| Measurement | TN | TNDBG |
| --- | ---: | ---: |
| Classic load-file bytes | 18,090 | 21,579 |
| MIR6502 load-file bytes | 16,764 | 20,501 |
| MIR6502 `MyDosAfter` instruction bytes | 246 | 246 |
| `MyDosAfter` spill / parameter / local bytes | 0 / 4 / 3 | 0 / 4 / 3 |

The latest qualified TNDBG reversed-sort vector takes 534,512 MIR6502 cycles
for 64 entries, including the call wrapper. Re-establish exclusive comparison
cost and both roots' workload timings in Stage 0. The
[historical audit](output-audit-2026-10-12/README.md) identified repeated pointer
setup, but did not validate a replacement. No saving is promised by this plan.

The desired hot-loop shape is conceptually:

```asm
    ; Two pointer pairs prepared before entering the loop.
    ; Y denotes the current byte offset into both records.
loop:
    LDA (leftBase),Y
    CMP (rightBase),Y
    BNE different
    INY
    CPY #endOffset
    BCC loop
```

This is a target shape, not a requirement to remove source locals or reuse
flags without proof. Pointer reuse alone must work when character values or
the source induction variable remain observable.

## Scope and ownership

Implement this in MIR6502. Verified NIR already provides typed loads, stores,
pointer computation and control flow. Matching must use storage identities,
byte offsets and CFG facts, never TN names, source strings or record layouts
looked up in SemIR. Keep production TN source unchanged during compiler work.

Initially support reducible, unsigned BYTE comparison loops with unit-step
induction, a constant bounded interval, two invariant complete WORD bases and
one latch. Include an early mismatch exit as well as normal completion; this
is essential to `MyDosAfter`. Multiple outside entries, nested loops, signed
ordering, WORD index ranges and effectful loop bodies retain existing output.

Keep the flag comparison, name loop and detail loop separate. Loop fusion,
string-library replacement, sort changes, inlining and a new general register
allocator are outside this plan. Classic, MIR65816 and MIR68K are comparison
or unaffected backends; they receive no optimization changes.

## Existing foundations and phase placement

Build on:

- [CFG](../../src/mir6502/analysis/cfg.rs), dominance, reaching definitions and
  the shared pre-home/post-home snapshots and
  [rewrite context](../../src/mir6502/rewrite/context.rs);
- [counted-loop facts](../../src/mir6502/analysis/counted_loops.rs) and
  [dynamic-loop selection](../../src/mir6502/materialize/dynamic_loops.rs);
- [dual-indirect comparison selection](../../src/mir6502/materialize/compare_branch.rs),
  pointer consumers and [zero-page reservation](../../src/mir6502/materialize/zp.rs);
- [address provenance](../../src/mir6502/materialize/ssa_lite.rs), machine
  liveness, exact flag provenance and final physical memory maps;
- the transactional rewrite workflow and the existing final code, home and
  image cost gates.

Existing counted-loop facts do not constitute a proof for arbitrary loops
with early exits. Add a companion comparison-loop fact with an explicit node
set, backedge, entry and exit edges. Preserve existing selectors' contracts.
Expose the new proofs through the shared immutable snapshot/context rather
than adding independent liveness or alias scans to recognizers.

Discover candidates after block-argument lowering, while typed indexed loads
and comparison operands remain intact, before generic address materialization
and comparison expansion. Stage 0 must pin the exact phase from dumped MIR.
Carry a typed proof into final materialization and reconfirm placement after
zero-page allocation, home coloring and branch layout.

`CompareIndirectBytes` currently uses a constant offset; the scaled-Y consumer
has a scale-two contract. Neither represents arbitrary live-Y indexing.
Introduce a narrow MIR6502 consumer/operation for an already prepared pointer
pair with an explicit Y input if existing forms cannot express the selected
schedule. Define its effective address, register/flag effects, memory reads
and verifier rules explicitly. Do not use sentinel offsets or broaden an
existing consumer's meaning silently. Update the
[pseudo-machine contract](../../docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md) and
[rewrite workflow](../../docs/MIR6502_REWRITE_WORKFLOW_PLAN.md) with these
invariants when implementing the form.

## Required proofs

1. **Loop entry and exits.** Record the unique entry, dominated loop nodes,
   backedge and every exit. Initialize both complete pointer pairs on every
   entered path before use. A backedge cannot initialize the first iteration.
   Keep zero-trip behavior and early-return paths unchanged. Reject missing,
   foreign or irreducible edges.
2. **Invariant captured bases.** Identify the exact low/high definitions and
   source cells. Ordinary parameter/local loads can move only when every path
   preserves their bytes and evaluation snapshots. One-lane writes, escaped
   cells, unresolved aliases, fixed/hardware-backed cells and intervening
   effects block motion. Hoisting address arithmetic does not authorize
   rereading a mutable pointer cell instead of its captured value.
3. **Owned pointer pairs.** Reserve two distinct, contiguous pairs through the
   existing reservation/allocation path. Follow the compiler's scratch
   ownership contract and account for every implicit write. Neither setup nor
   loop use may corrupt source-owned storage, canonical arguments, ABI cells,
   another live pointer or a byte the original loop observes. Confirm physical
   overlap after allocation; unknown ownership or unavailable pairs falls back.
4. **Unchanged memory observations.** Preserve the count and left-before-right
   order of pointee reads, on the same iterations and paths. Do not hoist,
   duplicate, reverse or speculate character reads. Aliased or equal record
   pointers are valid for a read-only comparison; they do not require a
   disjointness assumption. Calls, helpers, machine blocks, unknown writes and
   unrelated hardware/absolute accesses are initial barriers.
5. **Exact addressing and induction.** Prove unit stride and that all indexed
   offsets fit the selected Y window. Preserve 16-bit effective-address wrap
   and page-crossing behavior. Y itself must not wrap within the admitted
   interval. Preserve the original guard, iteration count and final induction
   value whenever it is used or exposed; otherwise keep the existing index
   home as fallback.
6. **Value and flag consumers.** Reaching definitions and complete use facts
   must justify every removed character store/reload. `MyDosAfter` uses the
   same captured characters for both inequality and ordering on the mismatch
   exit. An equality-only matcher cannot delete those values. Reusing one CMP's
   C/Z flags for both decisions requires exact producer identity, dominance
   and no intervening clobber on that exit path. Keep source-visible locals
   when their definitions remain observable. A capture store can alias a
   later pointee read: prove removed homes cannot be observed through those
   reads, or retain the stores. Exclusive temp use alone is insufficient.
7. **All machine exits and ABI.** Verify A/X/Y, flags and both pair lanes across
   the whole region and all successors. Preserve canonical parameter captures,
   result stores at `$A0`, return encoding and return count. Internal register
   convenience never changes the public calling convention.
8. **Final cost and placement.** Check final emitted routine code, ordinary
   homes, added zero-page occupancy and the complete image against fallback.
   Initial enablement requires no code/home/image growth and no increased
   global zero-page window. Unknown cost or physical interference rejects the
   candidate. Report first-byte mismatch cost separately from long-prefix
   benefit; a hot-loop estimate is not a saving on every input.

Apply each loop as an atomic, generation-checked transaction. Validate the
effect/control-flow delta independently of discovery, verify the candidate
and rebuild analyses after acceptance. Rollback must restore the original
routine and reservations, then reconfirm costs and placement. Prevent repeated
selection of the same loop and suppress trial-only reports.

## Delivery sequence

Implement and commit each stage separately after its checks pass. Store new
measurements under `surveys/tn/comparison-loops/`; keep historical audit and
qualification reports immutable. Keep substantial build caches under one
task directory and remove disposable caches after qualification.

### Stage 0 Freeze acceptance and current output

Add generic record/byte-array comparison fixtures with multiple lengths and
field displacements, including equality-only loops and first-difference
ordering. Include values deliberately used after mismatch and completion.
Dump typed, selected and final MIR to identify why current dual-indirect and
loop selectors do not select them.

Freeze compiler identity, configuration, source hashes, both TN roots and
routine/home/zero-page census. Measure direct comparisons at mismatch
positions 0, middle and last, equal inputs, and detail-only mismatch. Profile
real reversed and common-prefix sorts, attributing exclusive cycles and
executed pointer-pair, parameter, local and spill traffic to `MyDosAfter`.
Retain a feature-disabled path for every later differential measurement.

Exit: host-oracle VM results pass before optimization, and the starting
snapshot plus acceptance matrix can be reproduced from a pinned compiler.

### Stage 1 Add loop and pointer lifetime proofs

Add typed comparison-loop facts through the analysis snapshot/context. Prove
entry dominance, all exits, complete invariant bases, index bounds, pair
ownership and live-out requirements. Add structured blocker reporting for
unsupported shapes, effects, missing lanes, alias hazards and Y/pair clobbers.

Keep emitted code unchanged. Unit tests cover early exits, zero trips,
conflicting backedges, outside entries, stale generations and source writes.

Exit: both `MyDosAfter` loops have valid structural/base proofs or precise
blockers requiring the next slice; no existing verifier guarantee is weakened.

### Stage 2 Retain both bases across the loop

Create a transactional selector and the narrowly defined live-Y memory
consumer if required. Prepare two invariant bases once on the entered path,
then use them for the original byte loads. Keep character homes, comparisons,
induction updates and result handling unchanged in this slice.

Wire explicit effects, verifier, printer, zero-page reservation/resolution and
emission together. Preserve read order and full address carry. Use the existing
final cost/placement framework; coordinate with address-value and predicate
costing rather than nesting another unrestricted series of whole-image trials.
Leave the selection disabled in normal profiles until qualification.

Exit: eligible loops retain two bases on their backedges without repeated
address construction; unsafe ownership or placement retains identical fallback.
VM tests establish independent semantics before any comparison fusion.

### Stage 3 Fuse proven character comparisons

First select the exclusive-use unsigned BYTE comparison case as an indirect
load followed by an indirect CMP. Then support the bounded first-difference
shape: Eq/Ne controls continuation and the same captured pair feeds unsigned
ordering on the mismatch exit. Reuse comparison flags only along proven paths.

Remove character staging only when all replaced reads and definitions are
accounted for. Retain it for escaped locals, extra users, later reads or
clobbered flags. Preserve left/right ordering for `>`, `<`, `<=` and `>=`;
swapping machine operands must not swap observable memory reads.

Exit: `MyDosAfter` can use a shared character comparison for continuation and
ordering where proven. Generic extra-use variants still benefit from pointer
reuse while retaining necessary character captures.

### Stage 4 Carry the bounded index in Y

Extend the existing register-carrier/latch framework to the proven comparison
region. Initialize the offset once, advance it with INY and compare against
the proven end. Keep the mismatch decision before latch instructions overwrite
C/Z. Translate offsets separately for the two original source loops.

Eliminate the ordinary induction home only when no source use, address exposure
or successor observes it. Reject Y interference or unsupported wrap while
retaining Stage 2/3 selection as an independent fallback.

Exit: eligible equal-prefix paths have a short indirect compare/advance loop;
all exit paths and observable index variants remain correct.

### Stage 5 Qualify and enable optimized selection

Run feature-off, pointer-only, comparison-fused and Y-carried variants on the
same fixtures and compiler. Test interactions with existing address selection,
loop unrolling, parameter handling and predicate costing. Reconfirm physical
proofs after rollback or any allocator/layout change.

Qualify TN and TNDBG with their existing bitmap table, directory layout and
provider contract. Compare routine code/homes, complete image size, reserved
zero-page bytes, direct-call costs and sort cycles. Preserve public parameter
homes and measure reduced traffic separately from removed storage. Enable only
the qualified selections in `Mir6502Config::optimized`; default and explicit
feature-off profiles preserve fallback.

Exit: generic correctness and backend checks pass, both real comparison loops
show reduced executed pointer setup, representative comparison/sort vectors
improve, and the initial size/zero-page budgets hold. Record short-input costs
and remaining blockers rather than claiming universal speedups.

## Validation matrix and required checks

Use an independent host lexicographic oracle and guarded backing records.
Cover all BYTE ordering boundaries, including 0, 127, 128 and 255; equal,
identical-pointer and overlapping inputs; mismatch at every position; lengths
0/1 and the admitted Y boundary; nonzero field offsets; pointer-low page
crossings and guarded 16-bit address wrap. A zero-length case must not read
record contents. Check both read order and guards when testing observable
memory, not just the final Boolean.

Negative cases cover either pointer lane changing, calls/helpers, opaque
machine code, indirect writes, absolute-backed source cells, physical scratch
overlap, unavailable pairs, Y/flag clobbers, multiple entries, unsupported
index wrap and observable locals/induction. Verify rejection keeps valid code
and preserves behavior, rather than requiring one textual instruction layout.
Include records that overlap exposed character or pointer homes to test the
distinction between a read-only comparison and observable capture stores.

During each compiler slice run affected MIR6502 unit tests and focused
integration/VM targets. Add `comparison_loops` VM coverage in cartridge and
standalone runtimes, feature on/off, LF and CRLF. Reuse the existing
`tn_directory_model`, `tn_mydos_directory`, `tn_dispatch` and `tn_size_profile`
targets for application qualification; profile both TN and TNDBG.

Final backend qualification includes the full MIR6502 library tests, affected
MIR6502 integration targets, fixture snapshots/sweep and its VM/runtime/ABI
suite, including existing classic indexed-mask coverage as a source control.
Normalize host text before newline-sensitive instrumentation and execute the
actual LF/CRLF compilation path. Do not run other target backends solely
because their code shares a crate.

If implementation changes NIR, semantic lowering or shared IR contracts,
separate that migration and run the repository's required NIR snapshots,
`actionc-nir-sweep -- fixtures/nir` and full `cargo test`. Backend-only changes
do not require that expansion.
