# NIR Quality Improvement Implementation Plan

Snapshot date: 2026-07-19.

This note defines the implementation sequence for improving optimized NIR
quality, with the largest opportunities observed in TN addressed first. It
should be read together with
[`NIR_TN_OPTIMIZATION_NOTE.md`](NIR_TN_OPTIMIZATION_NOTE.md) and
[`NIR_TARGET_SHAPE.md`](NIR_TARGET_SHAPE.md).

The first major outcome is routine-wide promotion of private scalar storage to
typed NIR values, including values carried through joins and loops. Block-local
load forwarding is an enabling correctness slice, not the destination.

## Motivation And Priority

The TN census found:

| Item | Count |
| --- | ---: |
| Loads | 762 |
| Stores | 351 |
| Direct local accesses | 243 |
| Direct parameter accesses | 149 |
| Direct global accesses | 524 |
| Load/modify/store sequences | 91 |
| CFG joins | 122 |
| Calls | 342 |
| Temporary definitions | 1,324 |
| Temporaries used across blocks | 0 |

NIR already normalizes block-local expressions into typed temporaries. The
largest remaining quality problem is that source values cross block boundaries
through variable homes rather than NIR values. This prevents the existing
routine-wide constant, alias, sparse-edge, and liveness analyses from seeing
the values that matter.

The priority order is therefore:

1. make optimized NIR observable and measurable;
2. establish exact storage identity and safety facts;
3. forward known storage values through simple CFG shapes;
4. preserve effect-region identities across calls;
5. add typed block arguments through NIR and MIR6502;
6. promote private scalar homes through arbitrary routine CFGs;
7. eliminate dead stores and unnecessary source homes;
8. apply SCCP, GVN, range, and loop optimizations to the improved value flow.

Small cleanups such as duplicate `AddrOf` elimination and discarded call-result
temps must not displace the scalar-promotion work.

## Architectural Invariants

Every slice must preserve these boundaries:

- SemIR owns Action! storage, initialization, parameter, persistence, callable,
  and source-control-flow meaning.
- NIR owns typed values, stable storage identity, CFG structure, value merges,
  and target-independent memory effects.
- MIR6502 owns A/X/Y/flags, physical homes, spills, zero-page placement, ABI
  moves, parallel-copy realization, addressing selection, and 6502 peepholes.
- MIR6502 must not consult SemIR to recover a storage, type, or effect fact that
  should have survived into NIR.
- Unknown effects, absolute memory, hardware-visible storage, pointer writes,
  and opaque machine blocks remain conservative.
- Optimizer passes run only on verifier-clean NIR and must produce verifier-clean
  NIR.

The optimized representation must remain suitable for a future Z80 MIR. No
6502 register set, condition-code, addressing-mode, or zero-page concept belongs
in NIR.

## Prerequisite

Finish and land the sparse executable-edge data-flow work as a separate change.
The new storage propagation will reuse that solver behavior, but the two changes
must not be mixed in one patch.

## Phase 0: Observable Optimized NIR

Status: implemented. `--emit-optimized-nir` prints the post-optimizer program,
and `--emit-nir-stats` prints deterministic lowered and optimized censuses plus
the aggregate `optimize_program` delta. The current optimizer is exposed as one
pipeline total; named per-pass attribution can extend the census when the pass
driver exposes stable pass boundaries.

### Scope

Add a CLI mode that prints exactly the NIR passed to MIR6502. Acceptable command
designs include:

```text
--emit-optimized-nir
```

or:

```text
--emit-nir-stage lowered
--emit-nir-stage optimized
```

Keep the current lowered/verifier-clean output available because it is useful
for migration diagnostics. Do not change optimization behavior in this phase.

Add deterministic counters for at least:

- blocks;
- operations by kind;
- loads and stores by place class;
- temporary definitions and cross-block uses;
- block parameters and edge arguments;
- eliminated loads and stores per optimizer pass.

The counters may initially be an analysis helper rather than a stable
user-facing format.

### Acceptance Criteria

- The optimized listing is byte-for-byte the NIR consumed by MIR6502.
- Repeated output is deterministic.
- TN baseline measurements can be reproduced without temporary compiler edits.
- No generated code or existing NIR fixture changes.

Suggested commit:

```text
cli: expose optimized NIR output
```

## Phase 1: Direct Storage Identity And Promotability

Status: implemented. NIR now retains structured scalar/array/record/type
storage shape for locals and parameters, and analyses use stable
`NirStorageId` identities rather than parsing printable declaration text. The
read-only routine analysis reports backing, access blocks, address escape,
definite-assignment, exit persistence, call effects, machine visibility, and a
deterministic set of promotion blockers. It does not rewrite NIR or affect code
generation.

`--emit-nir-stats` includes aggregate home, promotable-home, home-kind, and
blocker counts for both lowered and optimized NIR. On the Phase 1 TN baseline,
the lowered analysis reports 384 referenced/declared homes and 77 narrow
promotion candidates (34 locals and 43 parameters). The regression coverage
checks `Sort::gap` and the ordinary `Copy` scalars explicitly.

### Storage Identity

Introduce a target-independent identity used by NIR analyses:

```rust
pub enum NirStorageId {
    Local(LocalId),
    Param(ParamId),
    Global(SymbolId),
}
```

Absolute memory, dereferences, and indexed places should retain their existing
place forms and must not be disguised as unique scalar storage.

Add helpers that map an exact direct `NirPlace` to `NirStorageId`. Field places
may become exact subregions later, but the first slice should exclude them.

### Promotability Analysis

Add a routine analysis, separate from target allocation, that records for every
direct scalar home:

- type and width;
- ordinary, absolute, or alias backing;
- address-taken or otherwise escaped status;
- load and store blocks;
- possible read before a routine-local definition;
- possible value needed at a routine exit;
- machine-block references or opaque visibility;
- whether calls can read or write the home;
- a reason when the home is not promotable.

The initial promotable set is intentionally narrow:

- ordinary scalar locals;
- ordinary scalar parameters where ABI semantics are preserved;
- no aliases or absolute backing;
- no `AddrOf` use;
- no array, record, indexed, field, or dereferenced access;
- no unresolved initialization or persistence requirement;
- no opaque machine visibility.

Do not silently assume that an uninitialized Action! local is a fresh automatic
variable. Persistent local state and omitted parameter values are observable
language behavior.

### Acceptance Criteria

- Focused fixtures classify ordinary, aliased, absolute, address-taken, and
  initialized locals correctly.
- The analysis reports deterministic exclusion reasons.
- TN reports the expected high-value candidates, including `Sort::gap` and the
  ordinary `Copy` scalars.
- No NIR rewriting or code-generation change in this phase.

Suggested commit:

```text
nir: classify promotable scalar storage
```

## Phase 2: Exact Storage-Value Propagation

Status: implemented. The optimizer now carries a separate
`NirStorageId -> NirValue` fact map, rewrites direct loads from known compatible
values, intersects equal facts at CFG joins, and runs ordinary value folding
again after storage propagation. Stores remain explicit.

The pass tracks ordinary scalar homes and exact pointer cells used by
pointer-backed arrays. Absolute, aliased, address-taken, and machine-visible
storage remains excluded. Indirect/absolute writes and opaque effects clear the
fact map. Until Phase 3 provides named call-effect regions, direct calls clear
global facts; unknown, OS, recursive, and indirect calls are full barriers.

Temp-backed facts are pressure guarded: the pass does not create a new
cross-block temp live range, and it retains a transient temp across a competing
definition only when that temp was already live there. Constants remain freely
propagatable. This avoids replacing source-home reloads with MIR spills before
typed block arguments and full promotion exist.

On TN, optimized NIR changes from 1,993 operations / 762 loads after Phase 1 to
1,963 operations / 732 loads. The hotspot load counts change as follows:

| Routine | Phase 1 | Phase 2 |
| --- | ---: | ---: |
| `SetWin` | 92 | 83 |
| `Copy` | 56 | 55 |
| `Sort` | 32 | 31 |

Against commit `0e7e7eb`, the TN load image shrinks from 13,335 to 13,282 bytes
and materialized-MIR spill references decrease from 115 to 113. These are
implementation measurements, not stable output-size guarantees.

### Fact Domain

Extend routine value facts with:

```text
NirStorageId -> NirValue
```

The transfer rules are:

- a direct load with no known fact defines the fact as its result temp;
- a direct load with a known, width-compatible fact is replaced by that value;
- a direct store records its rewritten source value;
- a store to the same home replaces the old fact;
- an aliasing or unknown write kills every fact it may affect;
- a call or machine block applies its structured write effects;
- an opaque effect kills all possibly observable storage facts.

Keep storage replacements distinct from temp aliases so kill rules remain
auditable.

### Delivery Slices

First implement same-block forwarding. Then reuse the existing forward
data-flow solver to cross:

- single-predecessor blocks;
- dominance-safe straight-line regions;
- joins where every executable predecessor supplies the same value.

Do not invent a merge value when predecessors disagree. Phases 4 and 5 add the
required representation; Phase 6 performs the merge-producing promotion.

When a fact contains a temp, require that the definition dominates the rewritten
use. The verifier remains the final backstop, but the optimizer should reject an
invalid candidate before rewriting.

### Acceptance Criteria

- Load/store/load and load/load fixtures remove the redundant load.
- Different stores at a join do not propagate through the join.
- Equal constants and a common dominating temp do propagate through a join.
- Loop back edges cannot preserve stale entry facts.
- Calls, pointer writes, absolute writes, and machine blocks obey conservative
  kill rules.
- `SetWin`, `Copy`, and `Sort` show reduced optimized-NIR load counts.

Suggested commit:

```text
nir: propagate direct storage values
```

This phase is expected to produce useful but incomplete final-code gains. Do
not treat marginal byte savings here as evidence against full promotion; most
TN loop variables require a merge representation.

## Phase 3: Structured Memory-Effect Regions

### NIR Effect Model

Replace the current region-count shape:

```text
Known { regions: usize }
```

with optimizer-grade identities, for example:

```rust
pub enum NirMemoryRegionKind {
    Storage(NirStorageId),
    Static(SymbolId),
    AbsoluteRange,
    ZeroPage,
    Unknown,
}

pub struct NirMemoryRegion {
    pub kind: NirMemoryRegionKind,
    pub offset: u16,
    pub size: u16,
}
```

Use a region collection in `NirMemoryAccess`. Preserve stable identities from
SemIR instead of lowering the collection to its length. Add a parameter region
to MIR6502 if the corresponding effect cannot otherwise be represented.

### Call And Machine Rules

- A call that cannot write a tracked home preserves its fact.
- A call that may read a home whose promoted value is newer than memory requires
  synchronization before the call.
- A call that may write a home invalidates its promoted value after the call.
- Unknown and opaque effects conservatively read and write all observable
  memory.
- Opaque inline machine blocks remain full memory and ordering barriers.
- OS/runtime calls remain conservative unless their documented structured
  effects prove otherwise.

Effect identities are target independent. MIR6502 adds register and flag
clobbers separately.

### Acceptance Criteria

- NIR printing and verification expose malformed or missing region identities.
- A non-writing call preserves a private scalar fact.
- A writing call kills only overlapping facts.
- Offset and size overlap are checked correctly.
- Unknown effects retain current conservative behavior.
- MIR6502 effects contain the same memory-region identities received from NIR.

Suggested commit:

```text
nir: preserve structured memory effect regions
```

## Phase 4: Typed Block Arguments In NIR

### Target Shape

Use block arguments rather than target registers or stringly phi summaries:

```rust
pub struct NirBlockParam {
    pub dest: TempId,
    pub ty: NirType,
}

pub struct NirEdge {
    pub target: BlockId,
    pub args: Vec<NirValue>,
}

pub struct NirBlock {
    pub id: BlockId,
    pub label: String, // display metadata
    pub params: Vec<NirBlockParam>,
    pub ops: Vec<NirOp>,
    pub terminator: NirTerminator,
}
```

`Goto` owns one `NirEdge`; `Branch` owns its then and else edges. Move executable
CFG identity to `BlockId` while retaining labels only for readable output.

### Verifier And Analysis Work

The verifier must require:

- one unique definition for each block-parameter temp;
- edge argument arity equal to target parameter arity;
- exact type and width agreement;
- edge values available at the predecessor terminator;
- no missing or duplicate predecessor contribution;
- valid block IDs;
- no reintroduction of string-only executable edge identity.

Update CFG, use-def, dominance, and liveness analyses so edge arguments are uses
on the outgoing edge and block parameters are definitions at block entry.
Update the printer and fixtures with readable syntax.

Land manually constructed block-argument fixtures before enabling automatic
insertion. With no block arguments generated by lowering or optimization,
existing program output should remain byte-identical.

Suggested commit:

```text
nir: add typed block arguments
```

## Phase 5: Carry Block Arguments Through MIR6502

Add the corresponding target-level value merge to pre-materialization MIR6502.
Do not prematurely turn a NIR block argument back into a permanent memory home.

Required work:

- MIR block parameters and typed edge arguments;
- pre-materialization verifier rules;
- use-def and liveness support;
- critical-edge splitting where edge-specific moves are required;
- parallel-copy resolution after physical destinations are known;
- safe cycle breaking with a target-managed scratch or spill;
- readable MIR printing and focused fixtures.

Register, zero-page, stack, and spill choices remain MIR6502 decisions. NIR only
expresses the value merge.

### Acceptance Criteria

- Constant, byte, word, and pointer block arguments lower correctly.
- Conditional critical edges receive edge-specific copies.
- Parallel swaps cannot overwrite an incoming value prematurely.
- No block argument is represented by a permanent source-variable home solely
  for lowering convenience.
- Programs with no block arguments remain byte-identical.

Suggested commit:

```text
mir6502: lower typed block arguments
```

## Phase 6: Pruned Private-Scalar Promotion

### Algorithm

Implement pruned mem2reg for each promotable home:

1. collect definition blocks from direct stores;
2. compute live-in blocks for the home;
3. compute the iterated dominance frontier;
4. insert block parameters only where the home is live-in;
5. rename the current value along the dominator tree;
6. replace direct loads with the current value;
7. remove promoted intermediate stores;
8. add the current value to outgoing edges that target a parameterized block.

Use the existing CFG, dominance, liveness, and data-flow infrastructure. Add a
dominance-frontier helper if it is not already available; do not reproduce CFG
logic inside the optimizer.

### Entry And Exit Semantics

Promotion must preserve storage behavior:

- A local possibly read before its first routine-local store starts with one
  entry load.
- A parameter initially starts with one explicit typed load from its ABI home.
- A home whose final value can be observed by a later invocation receives a
  store at each observable exit.
- A home definitely assigned before every read can omit an entry load.
- A home whose persistent value is never observable can omit exit stores and may
  later lose its backing storage.
- A barrier that may read the home forces synchronization before the barrier.
- A barrier that may write the home forces a reload or unknown state afterward.

The first implementation may exclude initialized locals and questionable
parameter cases rather than guessing about persistence.

### TN Acceptance Sequence

Use increasingly difficult routines:

1. `Sort::gap`: conventional loop and only one call;
2. `Copy::j`, `mem`, `len`, `k`, `files`, and `flag`;
3. `InputLine::curpos`;
4. `Handle::ch`, after effect behavior across its calls is established.

`Sort::gap` is the first hard gate. Its loop must carry the value through a block
argument instead of repeatedly loading and storing the source home.

### Acceptance Criteria

- Loads and stores disappear for promoted straight-line and loop-carried uses.
- Block arguments are pruned rather than inserted in every join.
- Nested loops, multiple back edges, early returns, and unreachable blocks are
  covered by fixtures.
- Address-taken, aliased, absolute, indexed, and opaque-visible homes remain
  unmodified.
- At least two high-pressure TN routines show reduced MIR memory traffic.
- TN final output does not regress in size. If removed source homes are replaced
  one-for-one with permanent spills, improve MIR6502 handling before expanding
  the candidate set.

Suggested commit:

```text
nir: promote private scalar storage
```

## Phase 7: Dead Stores And Source-Home Elision

Run backward storage liveness after promotion. Remove a direct store only when
no read, call effect, machine block, exit persistence rule, or external observer
can see it before the next store.

Remove a local or parameter backing home only when:

- no load, store, address, alias, initializer, machine reference, or effect
  region still requires it;
- ABI placement does not require it;
- persistent Action! behavior cannot observe it;
- MIR6502 can consume the promoted value flow without reconstructing a permanent
  source home.

A MIR6502 spill created because a promoted live range exceeds available target
resources is legitimate. It is a transient target allocation, not the original
source-variable home.

### Acceptance Criteria

- Overwritten-before-read stores are removed in focused fixtures.
- Stores needed at calls or routine exits remain.
- Eliminated homes disappear from both NIR declarations and MIR frame storage.
- MIR spill census distinguishes transient spills from surviving source homes.
- TN and all fixtures preserve behavior.

Suggested commit:

```text
nir: eliminate dead scalar homes
```

## Phase 8: Optimizations Unlocked By Better Value Flow

After promotion is stable, prioritize:

1. rerunning sparse conditional constant propagation on promoted conditions;
2. dominance-safe GVN for pure arithmetic, casts, compares, and `AddrOf`;
3. edge-derived equality and range facts from constant comparisons;
4. loop-invariant motion for pure operations and effect-safe loads;
5. routine-local reuse of global loads across calls proven not to write them.

Do not promote mutable globals into routine SSA across arbitrary calls. Treat
this as caching between precisely modeled writes and synchronization points.
Global dead-store elimination remains deferred until whole-program observability,
pointer aliasing, absolute access, and machine effects are sufficiently strong.

Lower-priority cleanup remains:

- nine observed duplicate same-block `AddrOf` operations;
- five unused call-result temps whose results are already dropped by materialized
  MIR6502;
- NIR compare/condition forms that MIR6502 already fuses into flags correctly.

## Measurement Gates

After each transforming phase, record for TN:

| Layer | Measurements |
| --- | --- |
| Optimized NIR | loads, stores, block params, edge args, cross-block temps |
| Pre-materialized MIR6502 | loads, stores, moves, virtual temps, source homes |
| Materialized MIR6502 | spills, edge copies, reloads, call synchronizations |
| Final output | static bytes and optimization remarks |
| Runtime | startup, display, navigation, sorting, copying, and representative I/O |

The phase is not successful merely because an NIR operation disappeared. Trace
the result through MIR6502 and final bytes. Conversely, a small initial byte gain
from exact forwarding is not a reason to abandon block arguments; TN's principal
values are loop-carried and cannot be represented by the earlier slice.

The full scalar-promotion milestone requires:

- intentional cross-block value flow in optimized TN NIR;
- no repeated source-home traffic for `Sort::gap`;
- reduced home traffic in at least two of `Copy`, `Handle`, and `InputLine`;
- no TN static-size regression;
- no verifier, fixture, or runtime regression.

## Required Validation

After every NIR, verifier, printer, semantic-lowering, or MIR boundary change,
run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

For block-argument and promotion slices, also generate and compare:

```sh
cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 \
  --emit-optimized-nir samples/tn/modern/TN.ACT

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 \
  --emit-materialized-mir6502 samples/tn/modern/TN.ACT

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 \
  --emit-listing samples/tn/modern/TN.ACT
```

Use the final option spelling implemented in Phase 0 if it differs from the
examples above.

Fixture changes must be classified as one of:

- an intentional NIR contract change;
- an intentional optimized-NIR change;
- a printer-only change;
- a bug fix.

## Commit Sequence

Keep the work in vertical, independently verifiable commits:

```text
cli: expose optimized NIR output
nir: classify promotable scalar storage
nir: propagate direct storage values
nir: preserve structured memory effect regions
nir: add typed block arguments
mir6502: lower typed block arguments
nir: promote private scalar storage
nir: eliminate dead scalar homes
nir: exploit promoted routine value flow
```

Do not combine the IR contract, MIR lowering, automatic promotion, and home
elision into a single change. Each representation slice should be verifier-clean
and byte-identical until the transformation that uses it is enabled.
