# MIR6502 home value invalidation implementation plan

Prepared 2026-11-12. Status: complete; slices 0–3 delivered. Starting revision: `95e334be`.
Implement in four slices and commit each after its checks pass.

Replace blanket invalidation in the existing SSA-lite byte-value environment
with invalidation of facts affected by a write. The first acceptance case is
the redundant request copy in the real checked TN `Alloc`. Require a smaller
TN load file before expanding the scope.

This plan includes the narrow fact-transfer contract. It requires no new NIR
form, general memory SSA, allocator analysis or source-library change.

## Confirmed gap and first target

`SsaLiteValueEnv` in [ssa_lite.rs](../src/mir6502/materialize/ssa_lite.rs)
already tracks register values and memory equalities. Its direct byte-store
transfer currently clears every memory equality and then learns the current
store. Thus an unrelated spill write loses useful earlier equalities.

The optimized TN `Alloc` lowers to the following relevant operations:

```text
request = incoming n
call ArenaCursor
captured = call result
spill_request_lo = request.lo
spill_request_hi = request.hi
remaining_lo = low($2800 - captured)
compare remaining_hi, spill_request_hi
if equal: compare remaining_lo, spill_request_lo
```

The last three stores erase the preceding memory facts. The direct comparison
selector also misses the earlier word reload because subtraction intervenes.
The existing later byte-consumer forwarding can use the request homes if its
equalities survive those stores and the generated comparison blocks.

Keep the initial save of `n` before `ArenaCursor()`. New useful facts are
learned after that call; the fix needs no stronger cross-call proof.

The [checked-arena scorecard](benchmarks/6502-checked-arena/RESULTS.md)
provides these controls, to be rechecked against a fresh baseline:

| Measurement | Baseline |
| --- | ---: |
| MIR6502 `Alloc` body | 99 bytes |
| Shared `ArenaCursor` body | 69 bytes |
| Shared `ArenaStop` body | 6 bytes |
| Successful `Alloc(1)`, including the 17-cycle wrapper | 223 cycles |
| TN cartridge / standalone load file | 10,308 / 10,433 bytes |
| TNDBG cartridge / standalone load file | 13,403 / 13,528 bytes |

The request copy is four absolute LDA/STA instructions: 12 code bytes and
16 nominal cycles. An 87-byte `Alloc` is the local prediction if nothing else
changes. Measure actual routine, complete XEX and cycle deltas after allocation
and branch placement; do not report the local estimate as the net image gain.

## Scope and code homes

SemIR and verified NIR retain their existing meanings and executable forms.
MIR6502 owns home identity, byte overlap, value availability and forwarding.
Emission supplies actual placement where a physical alias proof is needed.

| Code home | Responsibility |
| --- | --- |
| [ssa_lite.rs](../src/mir6502/materialize/ssa_lite.rs) | Refine `SsaLiteValueEnv` store transfer and reuse its byte-consumer forwarding |
| [materialize.rs](../src/mir6502/materialize.rs) | Supply routine/allocation context at existing SSA-lite calls and retain pass ordering |
| [machine_values.rs](../src/mir6502/analysis/machine_values.rs), [context.rs](../src/mir6502/rewrite/context.rs) | Reuse or narrowly expose existing overlap and available-value queries |
| [layout.rs](../src/mir6502/materialize/layout.rs), [record_bases.rs](../src/mir6502/analysis/record_bases.rs) | Existing storage descriptions and final physical-home resolution |
| [posthome.rs](../src/mir6502/rewrite/posthome.rs), [driver.rs](../src/mir6502/rewrite/driver.rs) | Existing typed plans, removed-definition checks and generation validation |
| [spills.rs](../src/mir6502/materialize/spills.rs), [dead_spills.rs](../src/mir6502/materialize/dead_spills.rs) | Existing cleanup of dead compiler-private stores and homes |

Keep `SsaLiteV2ObserveEnv` as observability infrastructure; do not turn it into
a replacement optimizer or rename the modules. Match stable home IDs, owners
and byte offsets, never `Alloc`, `request`, spill numbers or fixed addresses.

Initial precision covers direct byte writes among proved-disjoint ordinary
scalar homes and compiler-owned scratch. Words benefit through their existing
byte expansion. Retain existing calls, helper, machine-block, indirect-write
and unknown-effect barriers. Raw/volatile/hardware-backed sources do not gain
new forwarding permission. New cases with unproved effects keep the baseline.

Do not add join/loop propagation, cross-call retention, expression keys, word
reassociation, new callee summaries or allocator-check simplification. Retain
the current single-forward-predecessor memory propagation and its exclusions.

## Fact transfer contract

A fact `D = S` means the current byte in destination home D equals the current
byte in source home S. It is not an immutable historical value of S. A fact
`D = constant` has no memory-source dependency.

For a write to byte range W:

1. Capture the stored value's existing key before applying the write.
2. Drop a memory fact if W may overlap its destination or any memory source
   named by that fact. Keep it only when all relevant bytes are proved disjoint.
3. Drop A/X/Y provenance that names a possibly overwritten source byte. A
   direct store does not change the actual register; an old memory identity
   can nevertheless become invalid. Preserve unrelated constant facts.
4. Learn the new destination fact only when its value key remains true after
   the write. Do not relabel a pre-write snapshot as a changed source's current
   value. Handle an exact self-copy explicitly or conservatively omit the
   trivial equality; never infer that unknown sources are equal.
5. Keep the existing fallback for unsupported writes. Every write must still
   invalidate facts even when its stored value is unknown.

The required distinction is visible here:

```text
spill = request
other = value       // retain spill = request when both dependencies are disjoint
request = value    // drop spill = request; spill still holds the old value
```

Use the full written byte range for partial WORD writes and implicit writes.
Audit every writer in `observe_op`, including updates, word arithmetic in
memory and implicit pointer-pair definitions. Newly retained facts must not
pass through a writer handled only by exact-ID invalidation. Route its complete
write range through the same dependency check, or conservatively clear memory
dependencies for that operation. Add no new forwarding shapes for those ops.
This does not authorize forwarding across an unproved read effect or removing
volatile/absolute/indirect reads.

### Proving disjointness

Provide a narrow query using existing MIR6502 storage facts: proven disjoint,
overlapping, or unknown. Both of the latter outcomes invalidate the affected
fact. Scope local/parameter/spill identities by routine and canonicalize
declared aliases and byte offsets. Distinct enum variants, IDs or offsets
outside a known object are not a disjointness proof.

Use ownership guarantees that survive address changes for compiler-owned
objects only where the current phase establishes them. Coloring and zero-page
allocation can merge logical homes; rebuild facts after those remaps. The
early `MaterializeLayout` address estimate is not a substitute for final
emission placement. Physical proofs require the current actual map and replay
if a later rewrite moves storage or changes allocations.

Start with the disjoint ordinary local/spill bytes needed by `Alloc`. Absolute
aliases, fixed/virtual zero-page overlays and unresolved placements remain
unknown unless the existing phase-specific machinery resolves them soundly.
Do not introduce a general alias solver to admit more cases.

Cap retained memory facts per environment, initially at 256 byte facts. On
exhaustion discard the memory-dependent facts and use the conservative policy
for the rest of that scan. Keep the work bounded through existing forward
chains; no new dataflow convergence scheme is needed.

## Slice 0 Freeze real compiler and runtime controls

1. Build and save the baseline compiler or measurement executable before
   changing propagation. Record compiler/source hashes and the actual modes.
   Keep unrelated workspace changes outside the validation checkout.
2. Capture fresh checked TN/TNDBG MIR6502 images, listings, `Alloc` MIR, routine
   sizes, frame/reservation maps and representative successful/failing calls
   in both runtimes. Confirm the baseline values above.
3. Add a neutral source regression with renamed locals, a saved word, unrelated
   stores/arithmetic and a word comparison. Exercise raw and optimized NIR
   through the normal lowering/materialization pipeline, not hand-built
   assembly alone. Use a matching negative case that changes the source.
4. Add narrowly scoped integration/VM targets, provisionally
   `mir6502_home_value_invalidation` and `home_value_invalidation`. Save baseline
   raw/optimized NIR images through the same harness before rebuilding it.

Acceptance: controls reproduce the redundant two-byte copy and the checked
arena semantics. No optimization code changes in this slice.

Suggested commit: `Freeze home value invalidation and TN Alloc controls`.

## Slice 1 Implement selective invalidation with focused proofs

1. Add the narrow disjointness query and dependency-aware transfer helper beside
   existing home analyses. Thread explicit routine/allocation context as needed.
   Do not replace the active store transfer until the helper's cases pass.
2. Implement destination/source/register invalidation and safe learning under
   the contract above. Keep unknown/unsupported cases conservative and maintain
   truthful learned/killed statistics.
3. Test independent low/high copies plus a third unrelated store, destination
   overwrite, source overwrite, partial-word overlap, constant values, unknown
   stored values, self-copies and source/destination aliasing during a store.
   Include implicit writes and in-place updates between capture and consumer.
4. Test different identities resolving to the same byte, declared aliases,
   colored/reused slots, zero-page overlaps, unresolved maps, owner scoping and
   the fact budget. Test stale placement/generation rejection where applicable.

Synthetic tests must declare the homes and allocations their disjointness
proof needs. Do not weaken the query to satisfy fixtures with invented IDs in
an empty frame. Preserve valid existing single-copy cases and their barriers.

Acceptance: multiple independent equalities survive; either-end interference
and unknown overlap invalidate them. With the transfer not yet connected to
rewriting, emitted baseline images remain unchanged.

Suggested commit: `Prove selective invalidation of MIR6502 home values`.

## Slice 2 Connect forwarding and establish the real TN saving

1. Use the helper in the existing SSA-lite byte-store path, with unknown cases
   retaining the old coarse behavior. Use before/after frozen compilers for
   A/B qualification; this refinement needs no new public optimizer toggle.
2. Let existing byte operand forwarding consume retained facts, including the
   current single-predecessor chains created by word-comparison expansion.
   Preserve current restrictions at joins, backedges and barriers.
3. Use the existing typed driver and cleanup. Prove operand equality explicitly;
   `structural_plan` alone does not prove it. Keep removed-definition,
   dependency, liveness and A/X/Y/flags/SP checks intact. Remove only dead
   compiler-private copies, not source-visible stores or the pre-call save.
4. Rebuild snapshots after rewrites/remaps. If current scheduling cannot supply
   a sound proof, keep early phases conservative and use the existing final
   phase with correct allocation facts. Replay any placement-dependent proof
   against the final candidate; never reuse a stale map after cleanup.
5. Compile real TN/TNDBG immediately. Require direct request-home comparison
   operands, disappearance of the redundant request spill copies, preserved
   checks and a smaller complete TN image. Measure all collateral changes by
   routine and report actual cycles, not only rule counts.

Acceptance: the neutral case and real TN `Alloc` improve through general home
facts; mutation/alias controls retain correct snapshots. Repeated application
is stable. If TN does not improve, diagnose this slice before expanding scope.

Suggested commit: `Reuse unchanged homes across independent MIR6502 stores`.

Slice 2 uses the existing typed rewrite driver after logical temps have been
expanded into explicit home writes. Storage views are rebuilt for each routine
generation. Existing coarse behavior remains in earlier phases. The real
`Alloc` compares both request homes directly: 99 → 87 code bytes and two fewer
spill bytes. `Init` retains a staged byte across an unproved indexed write,
adding four code bytes; each complete TN/TNDBG image shrinks by ten bytes.
The renamed neutral comparison saves twelve code bytes and two spill bytes;
both source-mutation controls retain their 68-byte bodies. No library or NIR
contract changes are involved.

## Slice 3 Qualify behavior and publish measured results

1. Execute the actual checked API with all existing success and fault controls.
   Keep fault reasons, cursor/target writes, zero-size and exact-fit behavior,
   released-byte scratch, return values and stack unchanged.
2. Exercise neutral word values around low/high-byte boundaries, both source
   and destination mutations, aliases, live flags/registers and required home
   observations. Compare memory, required exit state and preserved ordered
   indirect/volatile accesses against the frozen baseline.
3. Cover raw/optimized NIR, both runtimes, distinct origins and real LF/CRLF
   input files. Concentrate exhaustive value boundaries in one configuration;
   batch representative cases in the remaining configurations.
4. Record complete XEX and per-routine deltas, frames, spill/reservation counts,
   zero-page usage, cycles, maximum retained facts and generation wall time.
   Count actual accepted rewrites separately from estimates or blocked sites.
5. Publish new bindings under `docs/benchmarks/6502-home-value-invalidation/`
   with a matching generator/check command. Keep the checked-arena and other
   historical scorecards frozen; their pinned compiler/source hashes describe
   earlier configurations. Do not refresh them against the new compiler.
6. Update the relevant SSA-lite/home-value contract in
   [MIR6502_PSEUDO_MACHINE_CONTRACT.md](MIR6502_PSEUDO_MACHINE_CONTRACT.md)
   with dependency invalidation, unknown fallback and remap guarantees.

Acceptance: the real TN gain survives final placement and VM qualification;
all negative cases preserve behavior. Existing SSA-lite behavior remains
enabled under its current configuration, with the qualified precision in use.

Suggested commit: `Qualify home value reuse and record TN Alloc savings`.

## Required checks

During development use focused helper tests and the affected SSA-lite tests:

```sh
cargo test --lib home_value_invalidation
cargo test --lib ssa_lite
cargo test --lib mir6502::analysis::machine_values
cargo test --lib mir6502::rewrite
cargo test --test mir6502_home_value_invalidation
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test home_value_invalidation
```

Run affected consumer regressions, selecting targets from any changed transfer
or overlap helper. These existing controls are required for final qualification:

```sh
cargo test --test mir6502_argument_staging --test mir6502_register_copy_forwarding --test mir6502_pointer_retention --test mir6502_checked_pointer_ranges --test tn_checked_arena_controls
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test argument_staging --test register_copy_forwarding --test pointer_retention --test absolute_readback --test checked_pointer_ranges --test tn_checked_arena --test tn_dispatch
```

Final qualification includes the full MIR6502 library and integration targets,
the `actionc-mir6502-sweep --emit-load` corpus, relevant ABI/runtime tests and
MADS reassembly/relocation of the changed output with LF and CRLF listings.
Run the new measurement generator's check command. Reuse passing checks when
no later change affects them; broaden coverage for actual failures or consumers.

Normalize host text before newline-sensitive instrumentation and exercise
actual LF/CRLF source parsing. Preserve exact binary/ATASCII bytes.

This is MIR6502-only work. Shared NIR snapshots, the NIR sweep and full
`cargo test` become mandatory if implementation changes shared frontend/NIR
contracts. A documentation-only plan edit requires content/link checks only.

Slice 3 qualifies the result with 1,261 MIR6502 unit tests, 112 integration
tests, the 169-fixture emission sweep, neutral and real allocator VM oracles,
consumer/runtime checks, and LF/CRLF MADS reconstruction and relocation. The
[measured results](benchmarks/6502-home-value-invalidation/RESULTS.md) bind the
net ten-byte image saving, sixteen-cycle allocator saving, collateral changes,
storage, rewrite counts and bounded-fact observations. Historical scorecards
remain frozen.

## Completion boundary

Complete when existing home facts survive proved-disjoint stores, current
consumers eliminate the real `Alloc` copies, complete TN images shrink and
alias/barrier/runtime controls pass. Do not add allocator guards, private TN
variants, unrelated arithmetic selectors or a broader memory analysis to force
the result.
