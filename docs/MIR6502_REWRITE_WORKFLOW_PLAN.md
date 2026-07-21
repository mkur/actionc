# MIR6502 Routine-Aware Rewrite Workflow Plan

Snapshot date: 2026-07-20.

Status: in progress. Slice 6 (all five pre-home migration sub-slices) is
complete. Unused-`LeaAddr` elimination is also complete. The hybrid
parameter-home family is split out for the later location/machine-state
workflow. Slice 7's home, machine-state, parameter/register availability, and
typed post-home proof infrastructure is complete; the compatibility helper is
still scheduled and therefore emitted code is intentionally unchanged by this
infrastructure slice. Migrated shape recognizers no longer make
suffix-liveness decisions: exact definition
identity, reaching definitions, and routine-wide lane deadness come from the
shared snapshot. Later local, terminator, exact-lane successor, and full-temp
successor uses have negative coverage. The common producer matcher subsumes
the old one- and two-load byte compare consumers, while binary-result and
loaded-call-argument selection declare their explicit A-register projections.
TN retains 112 compare-producer folds, three narrowing folds, 71 simple
call-argument producer plans, one return-slot argument forward, and all 21
call-expression selections. Call-result analysis applies 25 direct result-store
plans plus nine loaded-argument plans; the lowering stage retains all 34 legacy
result-store materializations. TN has no removable unused-`LeaAddr` candidate
at this point. Materialized MIR and XEX remained byte-identical to Slice 5
through Slice 6.2 (`bb90d361...` and `f9f26cb3...`). Slice 6.3 intentionally
changes both artifacts because shared deadness rejects five previously unsafe
TN folds. Slices 6.4 and 6.5 are byte-identical to Slice 6.3; the detailed
results are recorded under Slice 6.

This note defines the implementation plan for integrating MIR6502 peepholes
into a routine-aware compiler workflow. Local pattern matching remains useful,
but legality, availability, liveness, alias, and machine-state decisions must
come from shared analyses owned by the pass pipeline.

It should be read together with:

- [`MIR6502_PSEUDO_MACHINE_CONTRACT.md`](MIR6502_PSEUDO_MACHINE_CONTRACT.md);
- [`PROOF_ARCHITECTURE.md`](PROOF_ARCHITECTURE.md);
- the
  [MIR6502 peephole liveness audit](../surveys/tn/mir6502-peephole-liveness-audit.md).

## Goal

Replace independently wired peephole safety checks with a workflow in which:

- an immutable routine snapshot has shared CFG, use/def, reaching-definition,
  liveness, memory-effect, and machine-state facts;
- typed pre-home and post-home contexts expose legal queries;
- local matchers produce transactional rewrite plans;
- the pass driver applies non-overlapping plans and owns analysis invalidation;
- representation-changing phase boundaries are verified;
- optimization groups run to a deterministic, terminating fixed point;
- blocked and applied rewrites remain observable on TN and focused fixtures.

The intended relationship is:

```text
routine MIR
    -> verify phase contract
    -> build shared routine analyses
    -> inspect local pattern
    -> prove legality through shared queries
    -> decide profitability
    -> return rewrite plan
    -> apply plans as a batch
    -> invalidate/rebuild affected analyses
    -> verify result
```

Peepholes remain local rewrites. They become consumers of routine-wide facts,
not independent mini-optimizers.

## Architectural Boundaries

- SemIR and NIR continue to own target-independent meaning and normalized value
  flow.
- MIR6502 owns A/X/Y, individual 6502 flags, physical homes, spills, zero page,
  fixed pointer pairs, addressing strategies, ABI placement, and target
  peepholes.
- MIR6502 analyses must not consult SemIR or reconstruct facts from printed NIR.
- No 6502 register, flag, zero-page, or addressing information may move into
  NIR.
- A future Z80 backend may reuse the graph/data-flow engine and pass-driver
  concepts, but must define its own locations, effects, and rewrite contexts.
- Calls, machine blocks, hardware-visible storage, absolute memory, and unknown
  indirect effects remain conservative unless structured MIR effects prove a
  narrower behavior.

## Non-Goals

- Do not convert MIR6502 to global SSA as part of this work.
- Do not build an interprocedural optimizer. Routine call-effect summaries are
  sufficient for the initial workflow.
- Do not implement incremental data-flow maintenance initially.
- Do not move every file under `materialize/` merely to match the new
  architecture.
- Do not migrate classic-backend peepholes.
- Do not mix new optimizations into the infrastructure slices.
- Do not introduce a target-neutral union containing 6502 machine locations.

## Decisions To Freeze Before Migration

These decisions are intended to prevent another foundational refactor during
peephole migration.

### 1. Use typed phase contexts

Do not create one context containing optional temp, home, register, and flag
analyses. The valid facts change when temps are materialized into homes.

Use two principal context types:

```rust
pub(super) struct PreHomeRewriteContext<'a> {
    pub routine: &'a MirRoutine,
    pub cfg: &'a MirCfg,
    pub use_def: &'a MirUseDef,
    pub reaching_defs: &'a MirReachingDefs,
    pub liveness: &'a MirTempLiveness,
    pub effects: &'a MirEffectAnalysis,
}

pub(super) struct PostHomeRewriteContext<'a> {
    pub routine: &'a MirRoutine,
    pub cfg: &'a MirCfg,
    pub home_liveness: &'a MirHomeLiveness,
    pub machine_liveness: &'a MirMachineLiveness,
    pub effects: &'a MirEffectAnalysis,
}
```

The concrete fields may be wrapped in phase snapshots, but phase-invalid
queries must be impossible through the public API.

### 2. Analyze immutable snapshots and mutate only between batches

The first implementation must not update analysis structures incrementally.
For each rewrite round:

1. freeze the routine;
2. build or retrieve one analysis snapshot;
3. discover non-overlapping plans without mutation;
4. apply the plans;
5. drop the snapshot;
6. verify and rebuild before the next round.

This makes stale facts structurally difficult to use. Full recomputation is
acceptable until measurements show it is a compilation-time problem.

### 3. Keep CFG rewrites separate from operation rewrites

Ordinary peepholes may replace a contiguous range in one block but may not add,
remove, or retarget blocks. Block splitting, compare-branch expansion, empty
jump collapse, and edge rewriting use a separate CFG-transform driver. Every
CFG mutation is followed by CFG verification and full analysis invalidation.

### 4. Use stable IDs for identity and indices only inside a snapshot

`MirBlockId`, temp IDs, spill IDs, and storage IDs remain semantic identities.
An operation index is valid only within the immutable routine generation in
which it was discovered.

```rust
pub(super) struct MirProgramPoint {
    pub block: MirBlockId,
    pub op_index: usize,
    pub generation: MirRoutineGeneration,
}
```

Plans from an old generation must not be applicable to a mutated routine.

### 5. Separate legality from profitability

A matcher first proves that the replacement preserves all observable state.
Only then does a cost model decide whether to apply it. A size or cycle benefit
must never substitute for a liveness or effects proof.

### 6. Model pointer pairs through their bytes

An address consumer is not a special exemption from liveness. A fixed or
virtual pointer pair reads two zero-page home bytes. Home liveness therefore
protects later `LoadIndirect`, `StoreIndirect`, `AdvanceAddress`,
`MaterializeIndexedAddress`, and `IndirectByteCompound` consumers without a
parallel pointer-liveness implementation.

Pointer-value availability may still be a separate forward fact, but pointer
pair deadness is a home-byte query.

## Target Workflow

The materialization pipeline should converge on these explicit stages:

```text
lowered MIR6502
    |
    | verify PreHome
    v
block-argument lowering
    |
    | analyze + pre-branch canonicalization
    v
compare/branch CFG expansion and CFG cleanup
    |
    | verify PreHomeCfgNormalized
    | build PreHomeAnalysisSnapshot
    v
pre-home rewrite fixed point
    |
    | verify PreHomeOptimized
    v
home planning and temp materialization
    |
    | verify PostHome
    | build PostHomeAnalysisSnapshot
    v
post-home structural rewrite fixed point
    |
    | verify PostHomeOptimized
    v
final layout lowering
    |
    | invalidate layout/effect-dependent facts
    | rebuild PostHomeAnalysisSnapshot
    v
final post-home cleanup fixed point
    |
    | verify PreEmission
    v
emission
```

The pre-branch canonicalization step needs its own pre-home snapshot because it
currently runs before a CFG-changing compare expansion. The snapshot is
discarded immediately after CFG expansion.

## Shared Infrastructure

### Module ownership

Create new infrastructure in stable homes and leave matcher implementations in
their current feature-oriented files during migration:

```text
src/analysis/
    mod.rs
    graph.rs
    dataflow.rs
    dominance.rs

src/mir6502/analysis/
    mod.rs
    cfg.rs
    effects.rs
    sites.rs
    use_def.rs
    reaching_defs.rs
    temp_liveness.rs
    home_liveness.rs
    machine_liveness.rs
    manager.rs

src/mir6502/rewrite/
    mod.rs
    context.rs
    queries.rs
    plan.rs
    driver.rs
```

The generic `src/analysis/` layer owns algorithms over stable graph nodes. It
must not import `nir` or `mir6502`.

`src/mir6502/analysis/` owns 6502 MIR facts and operation effects.
`src/mir6502/rewrite/` owns snapshot-safe query, planning, batching, and
scheduling contracts. Existing matchers in `materialize/calls.rs`,
`store_consumers.rs`, `indexes.rs`, `peepholes.rs`, and related files migrate to
those APIs without being moved. This avoids combining an architectural
migration with a broad file reorganization.

During extraction, existing NIR analysis modules and MIR materialization
helpers remain thin compatibility adapters. Remove an adapter only after all
of its consumers have migrated.

### Target-neutral graph and data-flow kernel

NIR already has a deterministic worklist solver in
`nir::analysis::dataflow`, plus CFG and dominance implementations. Extract the
representation-neutral pieces rather than copy them into MIR6502.

The shared layer should contain no NIR or MIR types:

```rust
pub(crate) trait DataflowGraph {
    type Node: Copy + Ord;

    fn nodes(&self) -> &[Self::Node];
    fn predecessors(&self, node: Self::Node) -> &[Self::Node];
    fn successors(&self, node: Self::Node) -> &[Self::Node];
    fn postorder(&self) -> &[Self::Node];
    fn reverse_postorder(&self) -> &[Self::Node];
    fn is_reachable(&self, node: Self::Node) -> bool;
}

pub(crate) trait DataflowProblem<G: DataflowGraph> {
    type State: Clone + Eq;

    fn direction(&self) -> DataflowDirection;
    fn bottom(&self) -> Self::State;
    fn boundary(&self, node: G::Node) -> Option<Self::State>;
    fn join(&self, into: &mut Self::State, other: &Self::State);
    fn transfer(&self, node: G::Node, state: &Self::State) -> Self::State;
    fn forward_edge_is_executable(
        &self,
        from: G::Node,
        to: G::Node,
        from_out: &Self::State,
    ) -> bool;
}
```

Requirements:

- deterministic ordering and results;
- forward and backward problems;
- loops, joins, multiple exits, and unreachable blocks;
- sparse executable-edge filtering for forward analyses;
- evaluation counters for observability;
- no knowledge of temps, storage, registers, or flags.

Keep `nir::analysis::dataflow` as a thin compatibility wrapper during
extraction. Adapt existing NIR clients in a mechanical slice and keep their
behavior byte-identical. MIR6502 then consumes the same core through a MIR CFG
adapter.

Extract generic dominance over the shared graph interface in the same
foundational slice and keep `NirDominance` as a compatibility wrapper. MIR
value-availability and reaching-definition queries will need dominance, so
deferring this extraction would create a second analysis-interface migration.
Do not make MIR6502 depend on `NirDominance`.

### MIR CFG snapshot

Add an immutable `MirCfg` keyed by `MirBlockId` with:

- entry block;
- stable block-ID to vector-index mapping;
- deterministic predecessor and successor sets;
- reachable blocks;
- postorder and reverse postorder;
- exits;
- optional edge classification for backedges after dominance is available.

All analysis APIs accept block IDs. Conversion to vector indices stays inside
the CFG adapter.

### MIR program points and sites

Define shared sites instead of allowing every pass to invent tuple formats:

```rust
pub(super) enum MirSite {
    Op {
        block: MirBlockId,
        op_index: usize,
    },
    Terminator {
        block: MirBlockId,
    },
}

pub(super) struct MirDefSite {
    pub site: MirSite,
    pub lane: MirTempLane,
}

pub(super) struct MirUseSite {
    pub site: MirSite,
    pub lane: MirTempLane,
    pub kind: MirUseKind,
}
```

`MirUseKind` must distinguish ordinary operands, addresses, call targets,
arguments, branch conditions, and edge arguments while they still exist.

### Pre-home value domain

Logical value facts are byte-lane aware:

```rust
pub(super) enum MirTempRequirement {
    Exact(MirTempLane),
    Full(MirTempId),
}

pub(super) struct MirTempLane {
    pub temp: MirTempId,
    pub byte: u8,
}
```

A full-temp use blocks removal of either surviving definition. Defining one
lane of a word narrows a later full requirement to the missing lane, matching
the existing `MirTempLiveSet` behavior.

### Post-home location domain

Track only compiler-managed byte locations eligible for rewriting:

```rust
pub(super) enum MirHomeByte {
    Spill { id: MirSpillId, offset: u16 },
    VirtualZeroPage(MirZpSlot),
    FixedZeroPage(MirFixedZpSlot),
}

pub(super) enum MirMachineLocation {
    Register(MirReg),
    Flag(MirFlag),
}
```

Public globals, statics, params, locals with ABI visibility, absolute memory,
hardware memory, and indirect application memory remain memory effects rather
than removable private-home locations. A separate proof may establish that a
particular internal local is private, but post-home liveness must not infer
privacy from an address alone.

Use individual `C`, `Z`, `N`, and `V` flags. Do not represent all flags as one
boolean, because existing transformations preserve different subsets.

### Central MIR operation effects

Add one exhaustive effect classifier for every `MirOp` and terminator. It must
report, as appropriate:

- temp lanes read and defined;
- private home bytes read and definitely written;
- registers read, written, or clobbered;
- individual flags read, written, or clobbered;
- address-consumer home bytes read or written;
- direct memory byte ranges read or written;
- indirect or unknown memory reads/writes;
- call, runtime-helper, barrier, and machine-block effects;
- whether an operation is removable when its result is dead.

The existing logic in `temp_uses.rs`, `memory.rs`, `regs.rs`, `flags.rs`, call
effects, and spill collectors should migrate behind this classifier in small
slices. Until migration completes, old helpers may delegate to the classifier;
there must not be two independently evolving definitions of operation effects.

Every new `MirOp` variant must require an explicit effects decision through an
exhaustive match.

## Analyses

### Use/def index

Build routine-wide definitions and uses for exact lanes and full-temp
requirements. Expose:

- definitions of a temp or lane;
- uses of a temp or lane;
- unique definition and unique use when they genuinely exist;
- uses inside a specific window;
- terminator and successor uses;
- whether a definition has any use outside a proposed rewrite window.

This is an index, not a reaching-definition proof.

### Reaching definitions

Block-argument lowering can create multiple predecessor definitions of the same
target temp. Add a forward analysis whose facts map a temp lane to the set of
definitions that may reach a program point.

Required queries:

- `definition_reaches_use(def, use_site)`;
- `unique_reaching_definition(use_site, lane)`;
- `definition_dominates_use(def, use_site)` where dominance is applicable;
- `value_available_at(def, point)`;
- `definition_has_uses_outside(def, window)`.

Do not use temp ID equality as a substitute for reaching-definition identity.

### Temp liveness

Move the existing MIR temp liveness onto the shared data-flow kernel without
changing its lane semantics. Expose block live-in/live-out and a standard
program-point query:

```rust
ctx.temp_definition_dead_after(def_site, window_end)
```

The query must combine:

- suffix uses in the current block;
- terminator uses;
- full-temp and exact-lane live-out;
- definition identity when multiple definitions share a temp ID.

### Home-byte liveness

Implement backward may-liveness for private home bytes after temp
materialization. Reads make a byte live; definite writes kill it; unknown
effects conservatively read or preserve relevant bytes according to their
structured effects.

Required query:

```rust
ctx.home_definition_dead_after(home, store_site, window_end)
```

It must answer whether the value written by that store can be read before being
overwritten on any path, including loops and successors. It replaces the
current rule that rejects every successor block when no local overwrite is
visible.

Address-consumer operations read both bytes of their pointer pair through the
central effects model.

### Machine-state liveness

Implement backward liveness for A, X, Y, SP where represented, and C/Z/N/V.
The transfer function uses centralized operation and terminator effects.

Required queries:

- `register_dead_after(reg, point)`;
- `flags_dead_after(flag_set, point)`;
- `exit_state_change_is_unobservable(original, replacement, window_end)`.

Routine ABI return state and flag-test terminators are boundary uses. Calls and
opaque machine operations use their structured clobber/preserve sets and remain
conservative when incomplete.

### Memory stability and alias queries

Centralize existing memory queries behind `MirEffectAnalysis`:

- direct byte-range overlap;
- `memory_stable_between(location, start, end)`;
- read-before-write and overwrite-before-read;
- call or machine-block interference;
- whether a location permits idempotent store removal;
- whether deferred reads from a home or public location are legal;
- whether a replacement may reorder observable memory.

The first implementation does not need general points-to analysis. Unknown
indirect memory aliases all externally visible memory and any address-taken
private storage not proven disjoint.

### Lattice and executable-edge policy

Every analysis must document whether its fact is a may or must fact and what
`bottom`, boundary, and join mean. The initial analyses use:

| Analysis | Kind | Join | Conservative interpretation |
| --- | --- | --- | --- |
| Temp liveness | May | Union | Live if any path may use the value before redefinition |
| Home liveness | May | Union | Live if any path may read the stored byte before overwrite |
| Register/flag liveness | May | Union | Live if any path may observe the machine location |
| Reaching definitions | May | Union | Every definition that may reach the point is retained |
| Available values | Must | Intersection/equality | A value is available only when all executable predecessors agree |
| Definite overwrite | Must | Intersection | A kill is valid only when every relevant path overwrites first |

Backward safety analyses follow the structurally reachable CFG. They must not
discard an edge merely because a forward analysis temporarily considers it
infeasible. A constant-edge optimization first commits the branch rewrite and
removes the dead edge; the analysis manager then rebuilds liveness on the new
CFG. Sparse executable-edge filtering remains available to forward propagation
analyses whose lattice explicitly models unexecuted states.

## Rewrite API

### Query facade

Matchers must use context queries rather than reaching into analysis maps.
Representative pre-home queries:

```rust
ctx.unique_reaching_definition(use_site, lane)
ctx.definition_dead_after(def_site, window_end)
ctx.value_available_at(def_site, point)
ctx.memory_stable_between(mem, start, end)
ctx.call_is_barrier(call_site, required_effects)
```

Representative post-home queries:

```rust
ctx.home_store_dead_after(store_site, home, window_end)
ctx.register_dead_after(reg, window_end)
ctx.flags_dead_after(flags, window_end)
ctx.pointer_pair_dead_after(consumer, window_end)
ctx.exit_state_change_is_unobservable(effect_delta, window_end)
```

Queries return a proof result with a stable blocker category, not only `bool`,
so diagnostics and counters use the same decision that controls the rewrite.

### Transactional plans

A successful local matcher returns a plan and does not mutate MIR:

```rust
pub(super) struct MirRewritePlan {
    pub generation: MirRoutineGeneration,
    pub block: MirBlockId,
    pub range: Range<usize>,
    pub replacement: Vec<MirOp>,
    pub removed_defs: Vec<MirRemovedDefinition>,
    pub exit_effect_delta: MirEffectDelta,
    pub change_set: MirChangeSet,
    pub stat: &'static str,
}
```

`MirRemovedDefinition` identifies logical definitions or home stores removed by
the plan. `MirEffectDelta` records observable machine state present at the
window exit in only one sequence. `MirChangeSet` describes invalidated fact
classes.

Pre-home selection may additionally declare `SelectedResultRegister(reg)` when
it makes an abstract operation's eventual result register explicit and routes
an in-window consumer through that register. Validation permits differences
only in reads/writes of the named register and still requires all memory, home,
other-register, and flag effects to match. This is a target-selection
projection, not a general register-liveness exemption; post-home rewrites must
use machine liveness for register changes that can escape their window.

Call-argument expression selection uses the separate
`MaterializedCallArguments` delta because it expands abstract values into ABI
staging, and ordinary operation-effect equality is not meaningful across that
boundary. Its validator requires the same ordered calls, direct target
identity, indirect-target width, ABI clobber/preserve sets, and structured call
effects. Only the selected argument-staging effects may differ.

Store-consumer selection uses `MaterializedStoreConsumer`. The delta preserves
semantic direct and indirect memory effects while allowing an abstract
producer/store sequence to expose its A/X/Y/flag strategy and the private
`$AC..$AF` address-staging pair. Storage-address byte operands do not count as
reads of the addressed storage. The address-store machine-block compatibility
reload may read a byte just written by the same selected window. Removed
logical definitions are still declared and proved separately.

Pointer-consumer selection uses the parallel `MaterializedPointerConsumer`
delta. It permits an abstract pointer load/dereference pair to expose the
selected address-consumer home and private staging strategy while preserving
the indirect data access. The pointer producer's memory read is matched as an
address-carrier input rather than by its pre-layout symbolic home. Absolute
pointer loads are not dropped: the shape selector must carry them into the
replacement address materialization. Removed pointer-temp lanes remain subject
to exact reaching-definition and routine-deadness proofs.

In debug and test builds, validate declarations against the original and
replacement operations. A matcher must not silently remove an undeclared
definition or change an undeclared machine location.

### Batch application

The driver scans an immutable routine in stable block and operation order,
selects non-overlapping plans, and applies plans from highest operation index to
lowest within each block. It then advances the routine generation and drops all
snapshots invalidated by the combined change set.

Overlapping candidates use deterministic priority:

1. legality;
2. explicit pass-family priority;
3. larger byte saving;
4. larger cycle saving;
5. longer canonical window;
6. earliest source position as a final deterministic tie-breaker.

The cost model may initially contain static estimates. Existing pass ordering
is preserved until a migration explicitly changes it.

## Analysis Ownership And Invalidation

Use a routine analysis manager that builds immutable phase snapshots. It may
cache within one unchanged routine generation, but mutation always creates a
new generation.

Fact classes:

```text
Cfg
Reachability
Dominance
TempUseDef
ReachingDefs
TempLiveness
HomeLiveness
MachineLiveness
MemoryEffects
LayoutFacts
```

Minimum invalidation rules:

| Change | Invalidates |
| --- | --- |
| Replace an op in place without changing identity, reads, writes, or effects | Cost and printed form only |
| Change operation count or order | All site-indexed facts, use/def, reaching definitions, and cost positions |
| Change temp definitions or uses | Use/def, reaching definitions, temp liveness |
| Change home reads or writes | Home liveness, memory availability |
| Change register or flag effects | Machine liveness and availability |
| Change calls, barriers, or machine effects | Memory, home, machine, and value availability |
| Change terminator condition | Temp/machine liveness and sparse-edge facts |
| Change CFG edges or blocks | All routine analyses |
| Change final layout or home mapping | Home, alias, address, cost, and layout facts |

During the initial implementation, rebuild every phase snapshot after any
applied batch. Preserve/invalidate declarations are still required so later
selective caching does not require changing the rewrite API.

## Pass Scheduling And Termination

Use explicit pass groups rather than one unordered collection of matchers.

### CFG normalization group

- block-argument lowering;
- compare/branch expansion;
- edge splitting;
- empty-jump collapse;
- unreachable cleanup where already supported.

CFG transforms run individually and invalidate all facts.

### Pre-home canonicalization group

- constant and copy propagation;
- producer sinking/rematerialization;
- compare and branch canonicalization that does not change CFG;
- address and index value canonicalization;
- dead temp definitions.

Every successful rewrite must reduce a lexicographic metric such as executable
ops, temp definitions, noncanonical operands, or estimated target cost.

### Pre-home selection group

- call-argument expression selection;
- store consumers;
- compare consumers;
- pointer/index consumers;
- result-to-store selection.

These transforms may replace several abstract operations with more explicit
MIR, so termination uses a phase-rank metric: selected forms must never be
reintroduced as unselected forms.

### Post-home structural group

- spill/home forwarding;
- staged word and byte rewrites;
- indirect compounds and stores;
- indexed base staging;
- direct inc/dec selection;
- redundant reload/store elimination;
- register/flag cleanup.

No post-home rewrite may create virtual temps or return to a pre-home form.

### Cleanup group

- SSA-lite availability forwarding;
- CFG-aware dead private-home stores;
- spill coloring and zero-page placement where scheduled;
- final unused-home pruning.

Run each group to a fixed point with:

- deterministic matcher order;
- a structural termination metric;
- a generous debug assertion limit;
- release-mode diagnostic fallback if the limit is reached;
- per-round applied and blocked counters.

Do not alternate home planning/materialization with pre-home rewrites.

## Verification

Extend MIR verification with explicit stage entry points:

```text
PreHome
PreHomeCfgNormalized
PreHomeOptimized
PostHome
PostHomeOptimized
PreEmission
```

Stage checks include:

- block IDs and CFG targets are valid and unique;
- block arguments and edge arguments match until their lowering boundary;
- every use has a legal reaching definition;
- temp widths and exact-lane uses agree;
- no definition removed by a plan has surviving uses;
- no virtual temp survives post-home materialization;
- no pre-home-only pseudo returns after materialization;
- address consumers refer to valid two-byte pairs;
- call ABI homes and public shadow stores satisfy ABI contracts;
- calls, barriers, machine blocks, absolute memory, and hardware effects remain
  ordered conservatively;
- pre-emission MIR contains only emitter-supported forms.

The verifier validates structure and phase invariants. It does not rerun the
optimizer's profitability decisions.

## Observability

Extend the existing MIR6502 peephole report with:

- analysis build counts and solver evaluations;
- rewrite rounds by pass group;
- candidates, applied plans, and overlap rejections;
- blocked reasons from shared proof queries;
- analysis invalidation classes;
- verification failures tagged with phase;
- before/after operation and home counts;
- static byte/cycle estimates per rewrite family.

Stable blocker categories include:

```text
blocked-later-use
blocked-terminator-live
blocked-successor-live
blocked-nonunique-reaching-def
blocked-home-live
blocked-register-live
blocked-flags-live
blocked-pointer-pair-live
blocked-memory-alias
blocked-call-effect
blocked-machine-effect
blocked-unprofitable
```

Do not put full debug windows into aggregate keys. Site detail remains in the
existing per-site report.

## Implementation Slices

Each slice is intended to be independently reviewable and committed
separately. Infrastructure slices must not change TN output unless their
acceptance criteria explicitly permit it.

### Slice 0: freeze baseline and migration inventory

Status: complete. The frozen measurements and reproduction commands are in the
[TN rewrite workflow baseline](../surveys/tn/mir6502-rewrite-workflow-baseline.md),
with its machine-readable migration checklist in
[`mir6502-rewrite-migration-inventory.tsv`](../surveys/tn/mir6502-rewrite-migration-inventory.tsv).

- Generate current TN MIR, materialized MIR, final listing, XEX, and peephole
  report.
- Record size, hashes, aggregate transformation counts, and emulator smoke
  behavior.
- Inventory every transform that removes a temp definition, home store,
  register write, flag write, or address-pair definition.
- Assign each transform to a phase and migration batch.
- Add a temporary CI/test assertion that the inventory covers every existing
  producer-removing entry point named in the liveness audit.

Acceptance:

- no compiler behavior change;
- reproducible baseline artifacts;
- complete migration checklist.

Suggested commit: `docs: freeze MIR6502 rewrite workflow baseline`.

### Slice 1: extract the shared graph/data-flow/dominance kernel

Status: complete. `src/analysis/` now owns the target-neutral stable-node graph,
deterministic forward/backward solver, sparse forward-edge filtering, evaluation
counters, and generic dominance/frontier implementation. NIR retains thin
compatibility adapters and its previous unreachable-block policy. The TN
optimized NIR, materialized MIR, and XEX remained byte-identical.

- Add the target-neutral graph and monotone worklist interfaces.
- Preserve deterministic sparse forward-edge behavior.
- Generalize dominance over the same stable-node graph interface.
- Convert `nir::analysis::dataflow` to a compatibility wrapper.
- Convert `NirDominance` to a compatibility wrapper without changing its
  unreachable-block policy.
- Run all NIR data-flow, dominance-dependent, optimizer, and fixture tests.
- Do not add MIR6502 analysis in this slice.

Acceptance:

- optimized NIR and TN output are byte-identical;
- NIR solver evaluation behavior remains deterministic;
- dominance/frontier results remain unchanged;
- the shared modules contain no NIR or MIR types.

Suggested commit: `analysis: share deterministic dataflow solver`.

### Slice 2: add MIR CFG, sites, and generation tracking

Status: complete. MIR6502 now has an immutable `MirCfg` implementing the shared
graph interface, generation-scoped operation/terminator sites, and snapshot
validation that rejects stale program points. Materialization verifies the CFG
after compare/branch expansion and empty-jump collapse. The TN optimized NIR,
materialized MIR, and XEX remained byte-identical.

- Implement immutable `MirCfg` and its shared-graph adapter.
- Add stable sites/program points and routine generation IDs.
- Test diamonds, loops, multiple exits, unreachable blocks, and block reordering.
- Add CFG verification after current CFG-changing materialization steps.
- Do not change peephole legality yet.

Acceptance:

- current MIR and listing output are unchanged;
- CFG facts are keyed by `MirBlockId`, never display labels;
- stale program points are rejected in tests.

Suggested commit: `mir6502: add immutable routine CFG facts`.

### Slice 3: centralize MIR effects

Status: complete (`mir6502: centralize operation effects`).

Implemented one exhaustive classifier for every MIR6502 operation and
terminator. It reports logical temp accesses, private-home bytes, direct and
unknown memory behavior, registers, individual flags, address-consumer pairs,
projected spill accesses, and dead-result removability. The existing temp,
memory, register, flag, home, and spill queries now delegate to the classifier;
small documented compatibility views preserve their prior local semantics for
runtime ABI homes, edge arguments, compare/call projected definitions, and
opaque call memory effects.

Table-driven tests exercise all 23 operation families, all terminator shapes,
typed effect domains, and conservative call/machine behavior. The full test
suite passed (1,611 library tests plus integration/doc tests). TN optimized
NIR, materialized MIR, and XEX remained byte-identical to Slice 2:
`1715e20b...`, `bb90d361...`, and `f9f26cb3...`, respectively.

- Introduce logical, home, memory, register, flag, and address-consumer effect
  summaries.
- Cover every `MirOp` and terminator exhaustively.
- Make existing temp, memory, register, flag, and spill helper APIs delegate to
  the classifier where behavior matches.
- Preserve conservative call and machine-block behavior.

Acceptance:

- no behavior change;
- table-driven tests cover every operation family;
- adding a new operation requires an explicit effect decision.

Suggested commit: `mir6502: centralize operation effects`.

### Slice 4: build the pre-home analysis snapshot

Status: complete (`mir6502: expose typed pre-home rewrite facts`).

- Complete: lane-aware routine use/definition index with stable definition and
  use sites, typed operand/address/call/branch/edge uses, block-entry parameter
  definitions, and window/terminator/successor queries.
- Complete: forward may-reaching definitions on the shared solver, including
  exact-lane kills, multi-definition joins, definition-identity queries, and
  explicit unreachable-block results.
- Complete: backward may-liveness moved behind the shared solver with the old
  materializer API retained as an adapter; MIR dominance combines block facts
  with intra-block ordering; a generation-scoped snapshot and typed proof
  facade expose reaching, availability, dominance, live-out, and dead-after-
  window queries with stable blocker categories.

The full suite passed with 1,619 library tests plus integration/doc tests. TN
optimized NIR, materialized MIR, and XEX remained byte-identical to Slice 3:
`1715e20b...`, `bb90d361...`, and `f9f26cb3...`, respectively.

- Add lane-aware use/def sites.
- Move MIR temp liveness to the shared solver.
- Add reaching definitions for multiple predecessor definitions.
- Wrap shared dominance with MIR block IDs for value-availability queries.
- Implement the typed `PreHomeRewriteContext` and read-only query facade.
- Compare new liveness results with the existing implementation on all tests and
  TN before switching consumers.

Acceptance:

- existing liveness tests pass unchanged;
- new tests cover multi-definition joins, loops, exact lanes, full-temp uses,
  terminators, and successor-only uses;
- analysis-only TN output is byte-identical.

Suggested commits:

1. `mir6502: index routine temp definitions and uses`;
2. `mir6502: analyze reaching temp definitions`;
3. `mir6502: expose typed pre-home rewrite facts`.

### Slice 5: add the transactional pre-home driver

Status: complete (`mir6502: drive pre-home rewrites from shared facts`).

Implemented immutable plans with routine generations, removed-definition and
effect-delta declarations, fact invalidation sets, deterministic overlap/cost
ordering, reverse-index batch application, fixed-point snapshot rebuilding,
and convergence accounting. Validation rejects stale plans, malformed ranges,
missing site-indexed invalidations, mismatched removed definitions, and
undeclared non-logical effect changes.

The analyzed pilots cover unused `LeaAddr` definitions and a literal compare
producer. Driver tests compare the ordinary local folds while proving that
terminator and successor uses block LEA removal; they also cover idempotence,
overlap priority, stale generations, and declaration failures. Production
matcher scheduling remains unchanged until the coherent migrations in Slice 6.
The full suite passed with 1,625 library tests plus integration/doc tests, and
TN optimized NIR, materialized MIR, and XEX remain byte-identical to Slice 4.

- Add immutable candidate discovery, rewrite plans, overlap resolution,
  generations, batch application, and snapshot rebuilding.
- Add change-set declarations even though the first version rebuilds all facts.
- Add debug validation for removed definitions and effect deltas.
- Pilot the driver with `is_unused_lea_addr` and one compare producer fold.
- Preserve the old implementations behind test-only comparison helpers until
  the pilot is stable.

Acceptance:

- successor and terminator uses block the pilot rewrites;
- ordinary local cases still fold;
- applying the same driver again reaches a fixed point;
- plans cannot be applied to a later routine generation.

Suggested commit: `mir6502: drive pre-home rewrites from shared facts`.

### Slice 6: migrate pre-home rewrites

Status: complete. Sub-slice 1 migrated compare producers, narrowing, and
compare consumers to the routine-aware driver. In sub-slice 2, simple
call-argument producers and return-slot result-to-argument forwarding are
complete. Expression selection, call-result store consumers, and unused
`LeaAddr` elimination are also complete. Parameter-home forwarding is no longer
part of this sub-slice because its current helper combines pre-home temp
rewrites with physical-register availability and post-home flag behavior.
All five sub-slices are complete.

Migrate in behaviorally coherent sub-slices:

1. compare producers, narrowing, and compare consumers;
2. call-argument, return-slot, and call-result consumers;
3. direct copy, cast, store-expression, byte-store, and word-store consumers;
4. pointer rematerialization and pointer consumers;
5. delayed index, indexed copy, and dynamic index preparation.

Sub-slice 3 routes address, cast, byte-multiply, word, direct-copy, byte, and
store-expression consumers through one routine-aware selection driver. Shape
recognizers can still run with local suffix checks in direct unit tests, but
the production selection path disables those checks and uses exact reaching
definitions plus routine-wide lane deadness. Delayed byte-index producers
consumed by a byte-store selection are included in the same transactional
window; unrelated intervening operations are preserved.

TN applies 11 address, 75 byte, 42 direct-copy, and 28 word store plans. The
one former store-expression site in `Draw` is selected by the earlier-priority
byte-store family instead. Compared with the legacy scan, shared deadness
blocks five unsafe selections: direct-copy sites in `Free` and `UpdDis`, plus
byte-store sites in `InputLine`, `IsProtected`, and `IsDirectory`. In the
clearest `InputLine` case, the old fold changed a load/sub/store result still
used by the following call into `dec.b param p2+0` and then loaded an unwritten
spill. The analyzed path preserves the computed value in A for that call.

That correctness fix grows the TN XEX from 12,118 to 12,138 bytes. The Slice
6.3 materialized-MIR SHA-256 is
`5d930dabb43d77acb3c3d4d5b628a4f4e1b28a0c88d1649410b96b31ebd00432`;
the XEX SHA-256 is
`8127b508d0d3ca74937edff304e2bb8783ba7cee88a1398223c8cf174d4a0b28`.
The extra per-routine driver raises TN analysis builds from 935 to 1,082; Slice
9 still owns pass-group consolidation.

Sub-slice 4 routes direct pointer-temp rematerialization and the adjacent
pointer-temp dereference selector through one analyzed batch. Production shape
recognizers no longer inspect the block suffix for temp deadness. The shared
plan proves the exact pointer definition reaches the dereference and that both
lanes are dead after the complete window; pointer-source memory clobbers remain
shape-level barriers. Redefinition, later local use, terminator use, exact-lane
successor use, and full-temp successor use have negative coverage.

TN applies 34 direct pointer rematerializations. The adjacent materializing
selector discovers 22 overlapping candidates, all of which are intentionally
subsumed by the earlier direct-rematerialization priority, matching the legacy
pass order. Materialized MIR, XEX size, and hashes remain byte-identical to
Slice 6.3. The additional routine batch raises TN analysis builds from 1,082
to 1,201; Slice 9 owns consolidation of these currently separate batches.

Sub-slice 5 routes delayed byte indexes, indexed byte/word copies, and dynamic
byte/word index preparation through one analyzed batch. Delayed expression
matching retains structural single-owner, prior-carry, and source-memory
stability barriers, but terminator and successor liveness now comes from the
shared routine snapshot. Producer definitions are part of the same transaction
as their indexed consumer. Indexed-word-copy rematerializations also declare
the external base/index producers they absorb. A common invariant rejects any
candidate whose replacement still reads a definition it declares removed;
this caught a multi-use index hazard while migrating the family.

TN applies 39 delayed-index consumer plans, observes 51 absorbed delayed
producer operations, and applies one indexed-byte and one indexed-word copy.
The dynamic preparation families have no TN site at this revision. The shared
fixed-point driver selects seven delayed-index transactions beyond the legacy
single scan after overlapping windows are rewritten; later cleanup already
canonicalized those sites to the same operations, so materialized MIR and XEX
remain byte-identical to Slice 6.3. Their hashes remain
`5d930dabb43d77acb3c3d4d5b628a4f4e1b28a0c88d1649410b96b31ebd00432`
and `8127b508d0d3ca74937edff304e2bb8783ba7cee88a1398223c8cf174d4a0b28`,
respectively, with a 12,138-byte XEX. TN analysis builds rise from 1,201 to
1,324, with 589 candidates, 473 applied plans, and 116 overlap rejections;
Slice 9 owns consolidation. Local later uses, terminator uses, exact-lane and
full-temp successor uses, delayed-index successor uses, and source-memory
clobbers have negative coverage.

For every migrated family:

- identify exact removed definition sites;
- use reaching definitions rather than temp ID equality;
- prove deadness through the shared context;
- add later-use, terminator-use, exact-lane successor, and full-temp successor
  negative tests;
- record shared blocker reasons;
- remove or make private the legacy raw suffix helper used by that family.

Acceptance:

- no migrated matcher makes an independent liveness decision;
- all known positive shape tests remain green unless a previously unsafe fold
  is intentionally blocked;
- every TN count reduction is explained by a blocker category;
- emulator smoke behavior remains correct.

Suggested commits: one commit for each numbered family.

#### Deferred parameter-home split

Do not migrate `forward_param_register_homes` as one matcher. It currently
combines operations that belong to different proof domains:

- `try_forward_param_word_store_consumer` removes logical temp definitions and
  `forward_param_call_target` changes pre-home values. They must remain before
  home assignment, but use shared forward parameter/register availability plus
  reaching-definition and temp-deadness proofs;
- `forward_param_reload` rewrites or deletes physical register loads. In
  particular, deleting a same-register reload changes N/Z production, so that
  decision belongs to the post-home machine-liveness group.

Until those facts exist, the current block-local helper remains scheduled as a
legacy compatibility pass. This is an explicit deferral, not a completed
migration. Slice 7 supplies the location and machine facts; Slice 8 splits and
migrates the consumers without moving temp-based matching after temp
materialization.

### Slice 7: build post-home location and machine liveness

Status: complete. Private home-byte and per-location machine liveness,
routine-wide parameter/register availability, ABI boundary seeding, and the
typed post-home proof facade are implemented. The legacy hybrid parameter-home
helper remains scheduled until Slice 8 migrates its consumers independently.

- Implement private home-byte identities and backward home liveness.
- Treat address consumers as reads of their pair bytes.
- Implement per-register and per-flag machine liveness.
- Implement forward parameter-home/register availability with call, memory,
  and register invalidation.
- Seed ABI and flag-test terminator boundary uses.
- Add typed `PostHomeRewriteContext` queries.
- Retain the existing conservative helper until differential tests agree.

Acceptance:

- loops and successors preserve stores read before overwrite;
- local and successor overwrites kill old home values precisely;
- pointer-pair reuse is visible as a two-byte home read;
- N/Z, C, and V tests distinguish individual flag liveness;
- calls and opaque machine blocks remain conservative.

Suggested commits:

1. `mir6502: analyze private home byte liveness`;
2. `mir6502: analyze register and flag liveness`;
3. `mir6502: expose typed post-home rewrite facts`.

### Slice 8: migrate post-home structural rewrites

Status: complete. All listed structural families use the routine-level
transactional post-home driver. Parameter-home consumers are split across the
pre-home and post-home proof domains; the former hybrid helper is test-only.

Migrate in conservative sub-slices:

- [x] staged byte/word and next-style word forwarding;
- [x] word-array value staging and indexed base-pointer staging;
- [x] indirect compound and direct/constant store families;
- [x] staged RHS, adjacent reload, spill forwarding, and dead scratch stores;
- [x] inc/dec, dead register writes, and reload forwarding; split parameter-home
   forwarding into pre-home availability consumers and post-home reload
   consumers.

Each matcher declares removed home stores and its machine-state exit delta. The
shared post-home context proves home, pointer-pair, register, and flag deadness.

Acceptance:

- every liveness-audit P0/P1 family is migrated or explicitly disabled;
- same-register parameter reload deletion is blocked while N/Z is live;
- old pointer pairs remain valid when reused later or across successors;
- the narrow staged-word regression remains covered;
- conservative blocked sites can be recovered by precise home liveness.

Suggested commits: one commit for each numbered family.

### Slice 9: install explicit pass groups and fixed points

Status: planned.

- Replace the current hand-written sequence with named CFG, pre-home
  canonicalization, pre-home selection, post-home structural, and cleanup
  groups.
- Rebuild snapshots between applied batches.
- Add termination metrics and debug iteration limits.
- Preserve current relative matcher priority unless a test documents a change.
- Run the final-layout post-home group with a new snapshot.

Acceptance:

- repeated optimization is idempotent;
- no phase recreates an earlier-phase form;
- fixed-point counters are deterministic;
- no silent iteration-limit exit.

Suggested commit: `mir6502: schedule routine-aware rewrite fixed points`.

### Slice 10: strengthen phase verification and enforcement

Status: planned.

- Add explicit MIR verification stages.
- Validate rewrite declarations in debug/test builds.
- Make definition-eliding matcher APIs require a typed context.
- Prohibit new raw `&[MirOp]` deletion helpers through review/test conventions.
- Remove compatibility paths and duplicate effects logic after all migrations.

Acceptance:

- old unsafe matcher signatures are gone;
- phase-invalid MIR receives a focused diagnostic;
- the liveness audit migration checklist is empty;
- no duplicate operation-effects implementation remains.

Suggested commit: `mir6502: enforce analyzed rewrite contracts`.

### Slice 11: recover precision and measure profitability

Status: planned.

- Compare applied and blocked TN sites with the baseline.
- Improve alias or effect precision only for measured blockers.
- Add byte/cycle profitability estimates to competing candidates.
- Consider selective analysis caching only after measuring compile time.
- Document which shared infrastructure a future MIRZ80 backend can reuse.

Acceptance:

- TN behavior is smoke-tested and listing changes are explained;
- correctness blockers are not weakened for size recovery;
- compile-time cost is measured before caching work;
- no target-specific fact leaks into NIR or the shared solver.

Suggested commit: `mir6502: recover analyzed peephole opportunities`.

## Migration Checklist

The following existing components are reference implementations, not immediate
deletion targets:

- `temp_liveness.rs`: lane semantics and successor tests;
- `word_values.rs`: combined suffix, terminator, live-out, use-count, and
  stability checks;
- pre-home fixed-point cleanup: routine liveness recomputation discipline;
- `dead_spills.rs`: CFG read-before-write behavior;
- SSA-lite: call/barrier invalidation and flag-sensitive reload handling;
- `memory.rs`, `regs.rs`, and `flags.rs`: source material for centralized
  effects;
- peephole statistics: applied/blocked observability.

They should be migrated behind the shared contracts before duplicate logic is
removed.

## Required Tests

### Shared infrastructure

- deterministic forward and backward diamonds;
- loops and nested loops;
- multiple exits;
- unreachable blocks;
- sparse executable edges;
- generation mismatch and overlapping plans;
- fixed-point termination and idempotence.

### Pre-home analyses

- exact-lane and full-temp liveness;
- multiple reaching definitions at joins;
- unique reaching definition on a single-predecessor edge;
- block-argument lowering definitions used in successors;
- terminator and call-argument uses;
- source-memory invalidation across stores and calls.

### Post-home analyses

- per-byte word-home liveness;
- overwrite-before-read on one and all successor paths;
- loop-carried home reads;
- pointer-pair reuse;
- individual register and C/Z/N/V liveness;
- call, runtime-helper, barrier, machine-block, and unknown-memory effects;
- public and absolute memory never treated as private scratch.

### Rewrite families

Every definition-removing family gets:

1. an applied local case;
2. a later-use blocker;
3. a terminator-use blocker;
4. an exact-lane successor blocker;
5. a full-temp successor blocker where applicable;
6. an overwrite-before-read case that remains applicable;
7. a call/machine-effect blocker where memory or state can escape;
8. an idempotence assertion.

## Validation Gates

Run after infrastructure-only slices:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test mir6502::materialize::tests --lib
cargo test
```

Run after every behavioral MIR6502 migration slice:

```sh
cargo test mir6502::materialize::tests --lib
cargo test mir6502
cargo test
```

Also regenerate and retain for comparison:

- optimized MIR6502;
- materialized MIR6502;
- final TN listing;
- peephole report;
- XEX size and hash;
- automated emulator smoke result where available.

A listing or hash change is not automatically a failure, but it must map to an
applied rewrite, a newly blocked unsafe rewrite, layout movement, or an explicit
contract change.

## Completion Criteria

The workflow is complete when:

- peepholes receive typed phase contexts rather than manually threaded
  liveness fragments;
- all temp-definition removals use shared reaching-definition and liveness
  queries;
- all private-home removals use CFG home-byte liveness;
- register, flag, and pointer-pair changes use shared machine/home facts;
- calls and machine blocks obtain their behavior from centralized effects;
- rewrite plans are immutable, transactional, generation-checked, and
  declarative about removed definitions and changed state;
- the pass driver owns analysis rebuilding, overlap resolution, fixed points,
  and observability;
- MIR verification runs at every representation boundary;
- no liveness-audit family remains on a legacy unchecked path;
- TN and focused regression programs pass behavioral validation;
- shared graph/data-flow infrastructure contains no 6502-specific information.

At that point, adding a new peephole should normally require only a local
matcher, shared legality queries, a cost decision, and focused tests. It should
not require inventing another liveness scan or data-flow pass.
