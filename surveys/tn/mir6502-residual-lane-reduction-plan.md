# MIR6502 Residual-Lane Reduction Plan

Status: planned.

Snapshot date: 2026-07-19.

Baseline commit: `21a37f0` (`Elide profitable MIR6502 accumulator homes`).

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
many of them better. The accepted narrow rule saves 13 bytes, but leaves 501
residual lanes and 169 final logical temp homes.

The next phase should reduce the number of costly lanes that reach home
planning. It must optimize producer/consumer structure earlier without making
the raw lane count an end in itself. Removing a lane is not a win if it disables
a stronger combine, increases the load file, or merely duplicates cleanup that
already removes the final home.

## Current TN baseline

The accepted post-Slice-3 home census reports:

| Metric | Count |
| --- | ---: |
| Residual temp lanes | 501 |
| Definitions | 501 |
| Uses | 529 |
| Same-block lanes | 490 |
| Single-use lanes | 465 |
| Cross-block lanes | 11 |
| Natural A producers | 253 |
| Natural X/Y producers | 0 / 0 |
| Final logical temp homes | 169 |
| Final virtual-ZP homes | 119 |
| Final ordinary spill homes | 50 |

The mutually exclusive home-plan decisions partition the 501 lanes:

| Primary decision | Lanes |
| --- | ---: |
| Coupled word/carry lanes | 228 |
| Terminator uses | 109 |
| Safe but not profitable for late A residency | 61 |
| Multiple uses | 32 |
| Unsupported consumer | 27 |
| Accumulator clobber | 21 |
| Live across call | 12 |
| Unused | 4 |
| Cross-block | 1 |
| Elided in A | 6 |

Join, backedge, call, and machine-block exposure counters overlap these primary
reasons. They are constraints, not additional lanes.

Two conclusions control this plan:

- block locality alone is not the principal limit: 490 of 501 lanes are
  already block-local;
- word/coupled values and terminator values are the largest early-reduction
  targets, while the 61 profitability-retained lanes show why residual count
  must be correlated with final code and home fate.

The accepted TN artifact is 13,335 bytes, with 5,468 listing instructions,
12,311 measured code bytes, 444 measured data bytes, 1,633 `LDA`, and 1,298
`STA` instructions.

## Goals

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

## Slice 1: Residual-lane attribution

Status: planned; instrumentation only.

Extend the home census so the 501-lane total can be ranked by actual shape and
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

Commit this instrumentation separately.

## Slice 2: Bounded propagation and cleanup fixed point

Status: planned.

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

## Slice 3: Early terminator and flag forwarding

Status: planned; first behavior-changing priority.

The 109 terminator lanes are the largest byte-oriented class. Move compatible
condition lowering before home planning so a boolean result does not require a
temp home merely to select a branch.

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

Status: planned; largest ceiling and highest risk.

The 228 primary coupled lanes dominate the residual population. Do not begin
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

## Slice 7: Re-census and next boundary

After each behavior-changing slice, regenerate the attribution census rather
than carrying the original 501-lane ranking forward. When Slices 2-6 are
complete or cease to win:

- measure peak simultaneously live byte lanes and word pairs per routine;
- separate caller-clobbered from call-surviving homes;
- measure the interference-coloring lower bound for RAM and ZP pools;
- compare remaining code traffic with modern/classic TN directionally.

Only then decide between:

- physical home pooling for unavoidable lanes;
- multi-use block-local value-location planning;
- straight-line CFG propagation;
- broader call/ABI coalescing.

Cross-block propagation is not an initial priority because only 11 current
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
