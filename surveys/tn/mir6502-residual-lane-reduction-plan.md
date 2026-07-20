# MIR6502 Residual-Lane Reduction Plan

Status: Slices 1-2 implemented; Slice 5A whole-word destination propagation is
next.

Snapshot date: 2026-07-20.

Baseline commit: `0e0ac1b` (`nir: exploit promoted routine value flow`).

Scope: `samples/tn/modern/TN.ACT`, `--profile modern --backend mir6502`.

## Problem

MIR6502 creates typed virtual temps while lowering computations. This is useful
for verification, explicit evaluation order, target selection, and later
optimization. A temp becomes costly only when one or more byte lanes survive
target expansion and propagation to the boundary immediately before
`materialize_temp_ops`.

This note calls each surviving byte component a **residual lane**. A byte temp
has one lane; a word temp normally has low and high lanes. Residual lanes are
compiler intermediates, not Action! variables, even when they carry a value
loaded from a source variable.

Home elision has shown that trying to recover register residency at this late
boundary has limited leverage. Applying all locally safe A-residency decisions
grew TN because later structural combines and spill cleanup already handled
many of them better. The accepted narrow rule saves 13 bytes. Subsequent NIR
promotion and home elision reduce source-home traffic, but the current MIR
boundary still contains 497 residual lanes and 175 final logical temp homes.

The next phase should reduce the number of costly lanes that reach home
planning. It must optimize producer/consumer structure earlier without making
the raw lane count an end in itself. Removing a lane is not a win if it disables
a stronger combine, increases the load file, or merely duplicates cleanup that
already removes the final home.

## Current TN baseline

The accepted post-NIR-Phase-8 home census reports:

| Metric | Count |
| --- | ---: |
| Residual temp lanes | 497 |
| Definitions | 497 |
| Uses | 563 |
| Same-block lanes | 483 |
| Single-use lanes | 434 |
| Cross-block lanes | 14 |
| Natural A producers | 251 |
| Natural X/Y producers | 0 / 0 |
| Final logical temp homes | 175 |
| Final virtual-ZP homes | 126 |
| Final ordinary spill homes | 49 |

The mutually exclusive home-plan decisions partition the 497 lanes:

| Primary decision | Lanes |
| --- | ---: |
| Coupled word/carry lanes | 196 |
| Terminator uses | 109 |
| Safe but not profitable for late A residency | 62 |
| Multiple uses | 57 |
| Unsupported consumer | 27 |
| Accumulator clobber | 21 |
| Live across call | 12 |
| Unused | 4 |
| Non-single definition | 2 |
| Cross-block | 1 |
| Elided in A | 6 |

Join, backedge, call, and machine-block exposure counters overlap these primary
reasons. They are constraints, not additional lanes.

The overlapping exposure census now includes 245 coupled lanes, 110 lanes live
across calls, 93 lanes live at joins, 82 lanes live across backedges, and 53
lanes live across machine blocks. These counts are constraints and are not
additive.

Three conclusions control this plan:

- block locality alone is not the principal limit: 483 of 497 lanes are
  already block-local;
- word/coupled values and terminator values are the largest gross classes, but
  their final cost is not yet attributed lane by lane;
- the 62 profitability-retained lanes and the growth seen with unrestricted
  NIR GVN show why residual count must be correlated with final code, live-range
  extension, and home fate.

The accepted TN artifact is 13,258 bytes, with 5,451 listing instructions,
12,243 measured code bytes, 435 measured data bytes, 1,632 `LDA`, and 1,296
`STA` instructions.

The optimized NIR feeding this artifact contains 1,942 operations, 717 loads,
346 stores, 379 source storage homes, one block parameter, two edge arguments,
and 13 cross-block temporary uses. The complete current NIR optimizer removes
45 loads, five stores, and five source homes relative to lowered NIR. Phase 8
deliberately restricts GVN to reuse that does not extend the canonical temp's
live range: unrestricted GVN grew TN by 228 bytes even though it removed more
NIR operations.

The rise from 169 to 175 final logical temp homes is therefore not by itself a
regression. NIR promotion has exchanged some persistent source-home traffic for
explicit value flow, while the final load file has fallen by 24 bytes from the
immediately pre-promotion 13,282-byte artifact. The next analysis must connect
each new value lane to its final code and storage cost before choosing another
rewrite family.

## Goals

- Attribute every residual lane to its final eliminated, ZP, or RAM fate and
  its actual materialized traffic.
- Eliminate temps whose values can reach branches, stores, ABI homes, returns,
  or address consumers directly.
- Run propagation and dead-temp cleanup to a bounded fixed point so one rewrite
  can expose another.
- Remove boolean condition homes by representing flag-producing branches
  before generic temp materialization.
- Reduce coupled word lanes through whole-value destination propagation before
  attempting general word register allocation.
- Rematerialize cheap multi-use values when duplication is smaller than a
  surviving home.
- Preserve NIR/MIR ownership boundaries and verifier-clean typed MIR.
- Attribute every reduction to a producer/consumer class and measure its final
  code, memory-traffic, and storage effect.

## Revised execution order

The implementation order after NIR phases 6-8 is:

1. use the completed Slice 1 final-fate attribution as the current 497-lane
   baseline;
2. implement Slice 2 as a bounded, monotonic pre-home fixed point over existing
   safe rewrites;
3. regenerate the census and choose the first behavior-changing family by
   actual surviving homes and accesses, not by the gross lane count;
4. proceed to Slice 5A whole-word destination propagation: attribution confirms
   that 154 primary coupled lanes survive in final homes; do not implement
   Slice 3 for the current compare-to-terminator population because all 109 of
   those lanes are already eliminated later;
5. add routine-wide spill interference coloring after avoidable homes have been
   reduced;
6. only then widen NIR scalar promotion into more call-heavy `Copy` and `Handle`
   shapes.

This separates two problems that must remain distinct:

- Slices 2-6 avoid creating unnecessary homes and their load/store traffic.
- Routine-wide coloring and later pool work find better physical locations for
  unavoidable homes.

Range inference and loop-invariant motion remain deferred. Both can lengthen
live ranges, and the unrestricted-GVN result shows that operation-count wins at
NIR are not sufficient evidence of a 6502 win.

## Non-goals

- Avoiding all temp creation during NIR-to-MIR lowering. Canonical temps remain
  useful optimization and verification identities.
- Treating source variables, address-taken locals, absolute storage, hardware
  registers, or ABI-observable storage as compiler temps.
- General register allocation across arbitrary joins and loops.
- Phi construction or value-location merging at CFG joins.
- Physical RAM/ZP pool allocation. That is a separate decision applied to the
  unavoidable homes left by this work.
- Special cases for TN routines or source syntax.

## Placement and invariants

Residual-lane reduction belongs in MIR6502 after target-specific expansion has
made byte/word operations, address consumers, ABI homes, and carry requirements
explicit, but before home planning and `materialize_temp_ops`.

The intended boundary becomes:

```text
verified NIR
  -> MIR6502 lowering and verification
  -> target-specific MIR expansion
  -> early branch/destination canonicalization
  -> bounded propagation + dead-temp fixed point
  -> MIR6502 verification
  -> residual-lane census and home planning
  -> materialize unavoidable temps
  -> spill cleanup, coloring, and RAM/ZP placement
```

Every rewrite must preserve:

- left-to-right Action! evaluation order;
- structured call, machine-block, memory, and hardware barriers;
- explicit low/high ordering and carry/borrow chains;
- pointer scratch and alias ordering;
- existing profitable indexed, word-store, call-result, and branch combines;
- readable MIR printing and stable ID-based executable semantics.

MIR6502 must not consult SemIR to reconstruct facts that should already be in
NIR or MIR. No new stringly executable form is permitted.

## Slice 1: Final-fate residual-lane attribution

Status: implemented; instrumentation only and byte-identical.

Extend the home census so the 497-lane total can be ranked by actual shape and
downstream fate. For every residual lane, record:

- producer operation and width;
- unique consumer operation, terminator use, or multi-use classification;
- exact byte lane and whether it is coupled to another lane;
- primary retention reason and overlapping barrier exposures;
- whether the derived spill receives stores and reloads after materialization;
- whether the logical home is removed, colored, promoted to ZP, or remains in
  ordinary storage.

Add aggregate and per-routine matrices such as:

```text
residual-lane-producer-<kind>
residual-lane-consumer-<kind>
residual-lane-<producer>-to-<consumer>
residual-lane-final-no-home
residual-lane-final-zp-home
residual-lane-final-ram-home
residual-lane-final-store-count
residual-lane-final-reload-count
```

Site output should remain opt-in. Aggregate reporting must be deterministic and
bounded.

Acceptance criteria:

- decisions still partition every residual lane exactly once;
- final-fate accounting reconciles with spill accounting;
- TN listing and load file remain byte-identical to the baseline;
- the note is updated with the ranked producer/consumer matrix before choosing
  later sub-slices.

The report must also explain the current 497-to-175 funnel: 497 residual lanes,
491 mandatory-materialization decisions, and 175 final logical homes. Counts at
these stages are not expected to match one-for-one because dead-store removal,
consumer folding, coloring, and ZP allocation happen after the home plan.

### Slice 1 results

The tracker preserves each lane's provenance through both basic-block spill
coloring rounds and through conversion of RAM spills to virtual ZP. It reports
producer, consumer, width, home-plan decision, final fate, and whether the final
home has reads or writes. Dynamic producer/consumer matrices are available in
aggregate and per-routine modes, while lane sites remain opt-in.

The final lane-fate partition is:

| Final lane fate | Lanes |
| --- | ---: |
| Eliminated by later combines and cleanup | 235 |
| Elided by the accepted register plan | 6 |
| Associated with a final virtual-ZP home | 191 |
| Associated with a final RAM home | 65 |
| **Total reconciled** | **497** |

Coloring allows multiple non-overlapping lane lifetimes to share one final
home. The 256 surviving lane associations therefore occupy only 175 homes:

| Final homes | Homes | Stores | Reloads |
| --- | ---: | ---: | ---: |
| Virtual ZP | 126 | 185 | 197 |
| RAM | 49 | 47 | 85 |
| **Total** | **175** | **232** | **282** |

The highest producer/consumer classes are:

| Producer to consumer | Lanes | Interpretation |
| --- | ---: | --- |
| Compare to terminator | 109 | All are eliminated later; not a current target |
| Load to indexed-address materialization | 51 | Whole-address propagation candidate |
| Load to address materialization | 48 | Whole-address propagation candidate |
| Load to multiple consumers | 32 | Retention/rematerialization candidate |
| Load to binary | 27 | Destination-propagation candidate |
| Indirect load to move | 27 | ABI/store destination candidate |
| Load to indirect store | 21 | Alias-sensitive destination candidate |
| Indirect load to compare | 19 | Existing late combines remove part of the cost |

The home-plan reason-to-fate matrix resolves several false ceilings:

- all 109 terminator lanes are eliminated before final home allocation;
- 56 of 62 profitability-retained accumulator candidates are also eliminated,
  leaving only three RAM and three ZP associations;
- 42 of 196 primary coupled lanes are eliminated, while 128 reach ZP and 26
  reach RAM;
- all four unused lanes and the one primary cross-block lane are eliminated;
- all 12 primary call-live lanes remain RAM-resident.

This selects whole-word/address destination propagation over early terminator
forwarding as the first new rewrite family after Slice 2.

Commit this instrumentation separately.

## Slice 2: Bounded propagation and cleanup fixed point

Status: implemented; infrastructure-only and TN-byte-identical.

Several existing passes run once even though removing one temp can expose a new
copy, dead definition, constant use, or consumer combine. Establish a bounded
pre-home fixed point over the existing safe transformations:

1. recompute temp liveness;
2. propagate eligible constants and copies;
3. remove dead temp and dead byte-lane definitions;
4. clean redundant moves and unused address materializations;
5. rerun consumer canonicalization unlocked by the changes;
6. stop when no structural change occurs.

The loop must be monotonic. Passes in the loop may remove operations, replace a
temp use with an existing value, or simplify a terminator, but must not create
fresh temp IDs. Use a small hard iteration bound and record rounds and changes;
tests should assert convergence below the bound.

Calls, runtime helpers, machine blocks, unknown memory effects, pointer writes,
and flag-sensitive carry chains remain barriers unless existing structured
effects prove the rewrite safe.

Focused tests should cover:

- a propagation that exposes a second dead temp only on the next round;
- lane-specific word cleanup without deleting the live sibling lane;
- retention across calls, machine blocks, pointer writes, and flag consumers;
- no oscillation between canonical forms.

Acceptance criteria:

- the four currently unused TN lanes are removed when their definitions are
  side-effect-free;
- any broader reduction is reported by producer/consumer class;
- no protected combine counter regresses;
- TN does not grow, and byte-neutral changes must remove final homes or memory
  traffic to be retained.

Commit the fixed-point driver independently from new rewrite rules.

### Slice 2 results

The pre-home cleanup now recomputes routine CFG temp liveness on every round,
runs the existing copy/constant propagation and dead full/byte-temp cleanup,
then reruns the existing temp replacement and producer-sinking canonicalizer.
The latter is now explicitly live-out-aware when used after block-argument
lowering. Every round is monotonic in operation count, preserves the exact temp
ID table, and is capped at eight rounds.

On TN, all 105 routines converged in one or two total rounds. The aggregate
report records 147 rounds: 42 routines changed in round one and therefore
needed a second no-change round, while 63 routines were already stable. The
first round changed 84 blocks and removed 182 pre-home MIR operations. No TN
routine required a second change round, so this slice exposes no additional TN
optimization beyond the previously single-shot pass. A focused regression
does require two change rounds, proving that the fixed point handles a dead
byte-lane consumer that exposes a dead direct-load producer.

The TN census remains 497 residual lanes, 175 final logical homes (126 ZP and
49 RAM), 232 stores, and 282 reloads. The load file remains byte-identical at
13,258 bytes with SHA-256
`799de79c99ede76fd99ff38ec7a11e274b968bdbed58c22bf64f5f6f53b02b94`.
This is retained as bounded infrastructure for subsequent behavior-changing
word/address rewrites, which will automatically receive iterative cleanup.
Outside TN, an existing accumulator-chain regression improves intentionally:
after the original value's last store, canonical producer sinking keeps the
dependent add in A and removes one virtual-ZP store/reload pair. The test now
asserts the preserved store/add/store order and the absence of that transient
home.

## Slice 3: Early terminator and flag forwarding

Status: planned; conditional behavior-changing priority after final-fate
attribution.

The 109 terminator lanes are the largest byte-oriented class. Later compare and
branch fusion already removes some of their cost, however, so the gross count
is not an achievable saving estimate. Enable this slice only for attributed
shapes that still allocate a home, emit a store/reload, or block another
profitable combine.

For those surviving shapes, move compatible condition lowering before home
planning so a boolean result does not require a temp home merely to select a
branch.

Start with a compare temp that:

- has one definition and is used only by a branch terminator;
- is in the same block as the terminator;
- has no intervening flag clobber;
- maps through the existing `compare_branch_plan`/flag-test contract;
- does not require a source-language semantic decision.

Rewrite the compare destination to flags and the terminator to the existing
structured fused/flag condition. Then rerun the cleanup fixed point to delete
the dead boolean temp.

Deliver this in sub-slices:

1. unsigned byte compares;
2. signed byte compares already supported by the flag contract;
3. byte nonzero tests whose immediately preceding producer supplies the exact
   required flags;
4. expanded word compares only after byte behavior is stable.

Do not infer flag survival across calls, machine blocks, labels, or unrelated
operations. Do not reuse arithmetic flags when carry/overflow requirements do
not exactly match the branch.

Acceptance criteria:

- fewer terminator residual lanes and final homes;
- fewer compare-result stores/reloads and no extra branch veneers;
- focused negative tests for every intervening flag clobber;
- unchanged branch meaning for signed, unsigned, equality, and nonzero cases.

Commit each word-width or signedness expansion separately if it changes a new
flag contract.

## Slice 4: Unique-use destination propagation

Status: planned.

Propagate final byte destinations backward through single-use producers when
that avoids creating a materialized temp. This is destination coalescing, not
general register allocation.

Prioritize consumers with explicit MIR homes:

- direct stores;
- direct ABI argument homes;
- return/result homes;
- existing address/index consumers;
- compare and branch consumers not handled by Slice 3.

Where an existing structural combine requires adjacency, safely sink a
side-effect-free producer toward its unique consumer and let the established
combine perform the final lowering. Extend sinking only across operations that
cannot alter the producer's inputs, required flags, memory value, or observable
evaluation order.

Use structured memory and call effects. Pointer reads cannot cross possibly
aliasing writes; absolute/hardware reads remain conservative; calls and machine
blocks remain barriers. A destination must not overwrite an input that remains
live.

Acceptance criteria:

- reductions come from the 27 unsupported-consumer and 21 clobber-retained
  classes or from an attribution-ranked shape;
- every enabled producer/consumer pair wins independently on TN or across a
  representative fixture group;
- direct destination propagation does not duplicate a later byte-neutral
  combine;
- stale or unsupported shapes fall back to unchanged MIR.

Commit producer/consumer families separately.

## Slice 5: Coupled word-lane reduction

Status: planned; default first behavior-changing family if Slice 1 confirms
that coupled lanes survive final materialization.

The 196 primary retained coupled lanes dominate the current residual
population. Do not begin
with a general word register allocator. First propagate complete word values
into consumers that already define a safe physical or logical destination.

### Slice 5A: Non-arithmetic word values

Start with values whose low and high lanes are independent:

- constants and static/routine addresses;
- word moves and copies;
- zero-extension with a known high byte;
- direct word stores;
- ABI word argument and return homes.

Forward both lanes as one decision. Never eliminate only one lane when the
producer or consumer observes a coupled word identity.

### Slice 5B: Loads, pointer pairs, and address consumers

Forward word loads and computed addresses directly into:

- final word storage;
- fixed or virtual pointer pairs;
- indexed-address preparation;
- indirect call targets.

Preserve alias ordering when the source or destination overlaps pointer
scratch. Calls and unknown writes invalidate memory-backed forwarding.

### Slice 5C: Arithmetic and carry chains

Only after non-arithmetic word propagation is stable, forward complete
low/high arithmetic chains into a final store or ABI home. Treat the carry or
borrow edge as part of the value:

- low lane executes before high lane;
- nothing may clobber carry between lanes;
- source/destination overlap must be explicitly legal;
- compact direct word-update and word-store combines take precedence.

Acceptance criteria for every word sub-slice:

- coupled residual lanes and final homes both fall;
- no extra `PHP`/`PLP`, pointer restaging, or address recomputation appears;
- protected counters such as `word-array-store-value-staging`,
  `binary-word-store-producer-forward`, direct byte/word updates, and scaled
  indexed addressing do not regress;
- low/high alias, carry, call, and machine-block tests remain explicit.

Commit 5A, 5B, and 5C separately.

## Slice 6: Cheap multi-use rematerialization

Status: planned; reassess after Slices 2-5.

The current 32 multi-use lanes are not candidates for unique-use forwarding.
Eliminate a home only when reproducing the value at every use is cheaper than
the surviving store/reload traffic.

Begin with:

- byte and word constants;
- static, global, and routine addresses;
- known constant byte lanes;
- immutable values whose safety does not depend on alias analysis.

Use an explicit cost model based on final addressing modes rather than the
gross six-byte store/reload estimate. Ordinary memory loads, pointer reads, and
hardware values are not rematerializable without stronger alias and volatility
facts.

Acceptance criteria:

- each rematerialized lane loses its final home;
- duplicated instructions are smaller or faster according to the declared
  policy;
- no evaluation-order or memory-observation change occurs;
- byte-neutral transformations are rejected unless they measurably reduce
  storage pressure needed by a later allocator.

Commit constant/address classes separately from any memory-derived class.

## Slice 7: Re-census, routine-wide coloring, and next boundary

After each behavior-changing slice, regenerate the attribution census rather
than carrying the current 497-lane ranking forward. When Slices 2-6 are
complete or cease to win:

- measure peak simultaneously live byte lanes and word pairs per routine;
- separate caller-clobbered from call-surviving homes;
- measure the interference-coloring lower bound for RAM and ZP pools;
- compare remaining code traffic with modern/classic TN directionally.

The current allocator already reuses the `$E0-$EF` virtual-ZP scratch range
between routines. Do not introduce a second common ZP pool merely to rename
that mechanism. Improve eligibility or allocation only when the census shows
pressure within a routine.

Ordinary spill coloring is currently limited to values wholly contained in one
basic block. The next placement slice should use routine CFG liveness to color
non-interfering surviving homes across blocks. It must preserve contiguous word
and pointer pairs, keep call-live values distinct from clobbered pools, and
fall back conservatively for terminator, machine-block, or unknown-effect
exposure.

Cross-routine RAM pooling is later work. It requires call-graph, recursion,
indirect-call, and caller-live reasoning; its maximum direct storage benefit is
currently bounded by the 49 ordinary RAM temp homes and it normally removes no
load/store instructions.

Only then decide between:

- physical home pooling for unavoidable lanes;
- multi-use block-local value-location planning;
- straight-line CFG propagation;
- broader call/ABI coalescing.

Cross-block propagation is not an initial priority because only 14 current
lanes have cross-block uses and only one reaches that primary rejection reason.
If earlier reductions materially change that population, create a separate CFG
plan with dominance, edge-state, and join rules.

## Validation matrix

Every behavior-changing slice must include focused positive and negative tests
for its exact contract, plus:

```sh
cargo test
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
```

For TN, regenerate:

```sh
ACTIONC_MIR6502_PEEPHOLES=sites \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend mir6502 --emit-listing \
    samples/tn/modern/TN.ACT

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-load \
  samples/tn/modern/TN.ACT
```

Record after every slice:

- residual lanes, definitions, uses, and primary reasons;
- final logical, virtual-ZP, and RAM homes;
- spill stores, reloads, accesses, and adjacent store/reload pairs;
- load-file bytes, listing instructions, code/data bytes, and `LDA`/`STA`;
- protected combine counters and affected routine deltas.

Fixture output changes must be classified as an intentional MIR contract
change, printer-only change, or code-generation fix.

## Stop rules and commit discipline

- Do not keep a rewrite solely because it reduces residual-lane count.
- Reject a TN-growing slice unless a broader representative fixture set proves
  a justified win and the tradeoff is documented.
- Reject byte-neutral rewrites that remove neither final homes nor memory
  traffic.
- Do not special-case a TN routine, symbol, block, or source expression.
- Preserve the existing conservative materialization path for rejected shapes.
- Keep instrumentation, fixed-point infrastructure, byte terminators, word
  propagation families, and rematerialization in separate commits.
- Update this note with measured results after every accepted major slice.

The primary success criterion is fewer costly residual lanes together with
smaller code or fewer final homes and memory accesses. The raw lane count is a
diagnostic, not the optimization objective.
