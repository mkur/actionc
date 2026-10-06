# Native 65816 record and value placement implementation plan

Status: stages 0–6 complete. Logical analysis and resource/placement foundations
are qualified. Mixed consumers meet the first-tranche gates on the frozen
workload; branch, loop and invocation-backed call placement add measured
benefits. Indexed and aggregate operations share the checked resource and value
plan. Native execution preserves external accesses and resource limits,
and paired compiler costs meet the incremental gates. Unchecked hosted release
and standalone fixture qualification gaps remain open. Stage 7 has not started.

The [stage-0 scorecard](benchmarks/65816-record-placement-stage0/README.md)
contains frozen inputs, measurements, numerical gates and recorded failures.

This plan addresses directions 1 and 2 of the
[Exec816 compiler roadmap](MIR65816_EXEC_COMPILER_ROADMAP.md) together: record
and memory code quality, and value placement across complete routines. Invest
in the compiler facts and planning needed to make ordinary record-intensive
routines efficient through mixed operations, branches, loops and calls.

The intended result is one coordinated, verified placement strategy for pointer
and scalar values. Efficient record operations should follow from that strategy
and their typed addressing contracts. Exec816 supplies the main workload;
independent programs establish that the implementation applies generally.

## Scope and success criteria

Cover record-field loads and stores, captured record pointers, address formation,
embedded arrays, scalar computations consuming field values, and aggregate
operations. Plan value locations across the routine while preserving the native
ABI, task and IRQ domain contracts, and existing bank-zero reservations.

Code size is the primary objective. Measure execution cycles, private memory
traffic, frame and stack requirements, and compiler build cost alongside it.
Use the measured numerical gates in the stage-0 scorecard. Completing
this plan requires measured benefits across several Exec816 subsystems, including
branching and looping routines and routines containing calls. A leaf-only result
does not satisfy its scope. The 256 KiB release objective remains an application
goal; this plan does not assign unmeasured savings toward it.

The frozen compiler baseline is `d7d536c9`, and the frozen Exec816 commands
source is `57df0d7` (256 source files, 1,156 routines). The scorecard records
complete source and external build-input hashes. Existing
measurements of Exec `154bcf5a` describe a different workload and retain their
original scope. The older `COOKEDLINE.Recall` disassembly is a motivation for
the work; rebuild it before using its bytes as an acceptance target.

## Existing foundations and remaining gaps

The backend already has typed MIR, CFG-aware temporary interference and stack
reuse, bounded pointer and scalar DP allocation, storage-demand planning,
pointer forwarding, machine-state tracking, physical home and definition
analyses, typed replay, and checked instruction rewrites.

These facilities operate at different stages. Logical MIR liveness supports
allocation; selected-code analyses describe actual instructions and physical
homes after allocation. Pointer residency, scalar residency, accumulator
lifetimes and borrowed homes still have separate admission and consumption
paths. Useful value facts usually stop at control-flow boundaries, with narrow
exceptions. The work should connect these existing facilities through explicit
contracts rather than create another competing tracker or allocator.

Relevant starting points are
[allocation](../src/mir65816/emit/allocation.rs),
[MIR liveness](../src/mir65816/emit/liveness.rs),
[storage demand](../src/mir65816/emit/home_demand.rs),
[pointer tracking](../src/mir65816/emit/pointer_state.rs), and
[selected-code analysis](../src/mir65816/emit/analysis/mod.rs).

## Ownership and compiler pipeline

SemIR retains ownership of record meaning, typing, lvalue legality and layout.
NIR retains normalized computation, storage facts and source access order.
MIR65816 owns resource constraints, location selection, transfers and target
addressing. Emission verifies and writes the chosen machine implementation.

Use the following planning order:

1. Verify NIR and apply the admitted target promotion policy.
2. Lower to typed MIR and prepare arithmetic helpers and their call contracts.
3. Analyze logical values, storage and CFG relationships.
4. Choose legal operation forms and a coordinated value placement plan.
5. Allocate the remaining stack homes and required transfer staging.
6. Verify the plan, emit through the tracked facade, and check final selected
   effects, replay, layout and artifact metadata.

Logical analysis must precede physical placement. Selected-code analysis remains
the authority for what the emitted instructions actually read and change. If
MIR lacks a necessary frontend fact, project it explicitly from verified NIR and
tighten the relevant verifier. MIR must not consult SemIR or infer facts from
source names, record types or printed syntax.

## Value and storage facts

Build an immutable routine analysis using stable routine, block, temporary,
parameter and storage identities. Reuse the exhaustive MIR operand census and
the shared dataflow framework. Include edge arguments, block parameters,
address bases and indices, indirect callees, return values and unreachable uses
where an admission rule requires a complete census.

The analysis should provide definitions and uses, dominance, edge substitution,
fixed-point liveness, definite initialization, and value identities with exact
width and representation. It must distinguish three relationships:

- The immutable value captured by a source read.
- The current contents of the storage holding a value.
- The memory reached through a pointer value.

A retained pointer does not prove that a record field still contains an earlier
value. Different source reads remain different captures unless an explicit
memory proof permits equivalence. Pointer identity does not imply object
disjointness, extent or alignment. Private-home versions must account for partial
writes and every potentially overlapping access.

Reuse existing storage ownership and escape facts to distinguish immutable
inputs, unexposed invocation storage, addressable objects, and external memory.
Keep unresolved pointee accesses, absolute memory, volatile accesses, assembly
and calls conservative. The first consumers reuse captured values and prepared
addresses while preserving ordinary external reads and writes in their original
order and extent. Stronger pointee-memory reuse requires a separate proof and
contract before admission.

Each analysis belongs to an immutable input generation. Unknown, unavailable or
unreachable facts must not become a successful proof. Edits invalidate dependent
facts; caching requires explicit ownership and invalidation.

## Operation resources and placement contracts

Describe the resource requirements of admitted operation forms before allocation:
A/X/Y lanes and widths, flag inputs and clobbers, pointer/index requirements,
temporary scratch extents, stack effects and required entry/exit state. Derive
these descriptions from a common definition checked against the actual typed
instruction effects. Instruction effects remain authoritative.

Address preparation and the field access form one complete resource window.
All inputs must survive until their final machine use. Preserve the existing
closed-operation interference rule until a specific alternative proves a
narrower lifetime. In particular, complete pointer capture must finish before
its source base can be overwritten; external transfers must touch their exact
declared extent.

Introduce a routine placement plan that distinguishes authoritative memory
homes, checked borrowed inputs, register/DP residence, and transfers between
locations. Keep these decisions separate from the public artifact's materialized
temporary-home map. A value with no home must have a verified producer, every
consumer, and every applicable boundary obligation. A plan must represent
location changes over a lifetime; one global temp-to-slot map is insufficient.
Keep canonical allocated homes truthful in the current image maps; cached
register/DP copies belong to the internal plan. Any future serialized location
ranges require an explicit artifact-format design before adoption.

The verifier recomputes value-flow, resource interference and boundary
obligations from verified MIR and the final allocation. Each emitted read must
obtain the required value/version, and every transfer must establish its claimed
location. Replay checks that the selected actions and their effects are emitted
faithfully; it complements the placement proof. Cross-block planning uses these
contracts without widening the existing local instruction-rewrite permissions.

Initially use a stack-backed strategy with bounded residence between resource
barriers. Extend to register-only and DP-only intervals after their producer and
consumer paths are qualified. Use only existing compiler-owned scratch. Calls
clobber A/X/Y, flags and all compiler scratch according to the current ABI;
values needed afterward require valid invocation-owned homes. Reentrancy and
preemption must not depend on scratch surviving a call.

Block entries and edges carry explicit value-location and machine-state
requirements. Check every predecessor, including late backedges and multiple
logical edges to the same block. Edge transfers implement simultaneous assignment
with complete widths and overlap-safe scheduling. Loop contracts require a
fixed point; lexical emission order cannot establish a retained value.

Use a deterministic size-first cost model that accounts for all setup,
transfers, spills, mode changes and frame consequences. Include execution-cost
and pressure limits. Unsupported or unprofitable regions use the established
conservative strategy. Invalid or inconsistent plans are validation failures,
not ordinary profitability refusals. Accepted plans publish only after complete
verification and successful emission.

## Delivery stages

Each stage is split into small, separately reviewable commits. Foundation
commits first expose and verify facts with unchanged output; consumer commits
then demonstrate a bounded code-quality benefit. Document deterministic
refusal reasons and preserve existing profitable leaf and scalar paths while
their ownership migrates to the common plan.

### Stage 0 Establish the Exec816 baseline

Freeze compiler, Exec816, generated source, layout, assembly, runtime and tool
inputs. Build actual optimized release, optimized guarded and raw profiles.
Capture code and data sizes, module/routine contributions, frames, private
traffic, stack/domain reservations, relevant runtime measurements, and host
compile time and memory.

Select a compact representative set spanning list manipulation, task/port
management, DOS streams or filesystem state, and cooked-line editing/history.
Include mixed pointer/scalar work, diamonds, joins, loops, and direct/indirect
calls. Use `Recall` as one representative, alongside unrelated layouts and
independent probes. Establish supported hosted test workloads and record any
baseline failures explicitly.

Exit gate: reproducible artifacts, a scorecard, and numerical acceptance targets
derived from measured recurring costs. Store bulky artifacts under `target/`
and compact evidence under `docs/benchmarks/`.

Stage-0 measurements are now recorded in the linked scorecard. Optimized release
compiler code is 444,948 bytes; guarded optimized and raw compiler code are
594,828 and 661,861 bytes. The guarded demo packages and OF816 autoboot pass their
focused checks. Unchecked hosted o65 providers reject an unguarded kernel; hosted
list/DOS fixture generation and raw port restoration also have recorded failures.
These gaps remain qualification obligations, with no claimed unchecked hosted
release result. Foundation work can use the frozen compiler inputs and passing
workloads while the separate hosted gaps are resolved.

### Stage 1 Establish routine value and storage analysis

Add the immutable logical analysis and checked query API. Reuse existing MIR
def/use and CFG logic; add dominance, value identity, storage-version and edge
mapping facts where missing. Project any required ownership facts through the
typed IR boundary. Cover malformed definitions, loops, partial writes, escaping
objects and uncertainty with independently authored graphs.

Exit gate: correct facts on the representative routines, stale/foreign query
rejection, unchanged emitted artifacts, and measured analysis overhead.

Stage 1 is implemented in the [logical analysis module](../src/mir65816/analysis/mod.rs)
and specified by the [logical analysis contract](MIR65816_LOGICAL_ANALYSIS.md).
It reuses the allocator's exhaustive operand census and ordered successors, and
shared dominance/dataflow solvers. Existing typed frame plans supply ownership
facts; no new shared frontend/NIR contract is required. Generation-scoped queries
reject stale/foreign handles, unavailable values and unknown storage versions.
Cyclic writes have unknown dynamic versions, including irreducible cycles.

The [stage-1 scorecard](benchmarks/65816-record-placement-stage1/README.md)
records exact image/inventory equality for all three profiles, facts for all
1,147 analyzable bodies and eleven representatives, unchanged results/costs for
882 native executions, and passing backend unit, integration and VM checks.
Paired warm CLI measurements meet the 5% wall-time and 10% peak-RSS limits in
every profile. Target selection and public frame maps remain unchanged.

### Stage 2 Establish resource descriptions and placement verification

Describe the existing operation forms and their complete scratch/register
requirements. Build the placement-plan representation and verifier, initially
encoding current decisions. Reconcile allocation, storage demand, pointer and
scalar plans so each operation and value has one owner. Validate candidate
resource descriptions against final typed effects and replay.
Cover the record/scalar forms needed by the first consumers initially; other
forms are explicit resource barriers until their contracts are qualified.

Exit gate: output and runtime behavior remain unchanged; forged locations,
missing homes, partial-width mistakes, scratch conflicts and inconsistent
boundary requirements are rejected. Existing optimized leaf cases retain their
qualified output.

Stage 2 is implemented by the common
[placement plan](../src/mir65816/emit/placement.rs) and
[operation resources](../src/mir65816/emit/resources.rs), specified in the
[placement contract](MIR65816_PLACEMENT_CONTRACT.md). Selection consumes its
owned current admissions; verification reconstructs them from immutable MIR,
logical facts and the final allocation. Complete dense window rows and shared
resource descriptions retain every operation's obligations. Sealed contracts
check typed effects, complete widths, scratch interference, access extents,
stack allowances and CFG requirements after selection, replay and rewrites.
Malformed plans and stale/foreign allocations fail validation.

The [stage-2 scorecard](benchmarks/65816-record-placement-stage2/README.md)
records exact image/inventory equality in all three profiles, common plans for
1,075 ordinary bodies and all eleven representatives, unchanged results/costs
for 882 native executions, and passing backend unit, integration and full VM
qualification. Existing helper and terminal-forwarding contracts remain opaque.
Five-round paired CLI measurements meet the 5% wall-time and 10% peak-RSS limits
in every profile. Target strategy and public artifact maps remain unchanged.

### Stage 3 Deliver mixed record flows within blocks

Use the common plan for pointer and scalar residence in ordinary mixed blocks.
Integrate record-field accesses, address formation, and consumers of captured
field values. Start with stack-backed residence, then omit homes for proved
intervals. Recompute frames and stack obligations from actual demand. Resource
pressure and unsupported operations end an interval with a checked transfer.

Extend the 65816 profitability policy for legally promotable private storage
only when qualified placement consumers can benefit. Keep promotion legality
in the existing shared NIR storage analysis. Such policy changes are separate
commits with shared-contract validation.

Exit gate: measurable size and private-traffic improvements in mixed routines
outside the pointer-only leaf profile, unchanged external access traces, and
correct fallback under pressure and alias barriers.

Stage 3 is implemented by the common plan's
[mixed residence admissions](../src/mir65816/emit/mixed.rs) and checked field
consumers. Complete DP-only intervals omit stack homes; profitable bounded
prefixes retain their invocation home and establish one verified private copy.
Every source field access retains its original order and extent. Calls,
unsupported resource windows and block boundaries end residence. Dedicated
scalar, pointer-leaf and fused top-bit selectors retain their ownership.
Actual demand and edge staging determine frames; choices that increase frame
extent or local peak are refused. Pressure retains complete stack captures.

The [stage-3 scorecard](benchmarks/65816-record-placement-stage3/README.md)
records release code of 434,502 bytes (10,446 bytes saved), a 3.04% reduction
in the fixed representatives, benefits in three Exec subsystems and independent
probe improvements. All three profiles meet the first-tranche gates with no
routine frame/peak or bank-zero growth. External access traces match in 882
candidate executions; full native, backend library and integration qualification
passes. Paired CLI timings remain close to baseline; the scorecard retains
RSS variability and the raw-only repeat explicitly.

This tranche needs no broader shared NIR promotion policy: existing typed
captures establish the required benefit. Further private-storage admission
requires qualified consumer benefit and a separate shared-contract change.

### Stage 4 Extend placement through branches and joins

Add verified block-entry location contracts and edge transfer plans for mixed
scalar/pointer values. Retain a fact only when all incoming obligations establish
it. Account for block-parameter substitution, different edge arguments, identity
transfers, overlap, and cycles. Allocate staging from actual transfer demand.

Exit gate: benefits in representative branching record routines, correct
simultaneous transfers and truthful frame maps. Diamonds, multiple returns,
same-target edges with different arguments and deliberately conflicting
predecessors must execute correctly.

Stage 4 is implemented by complete acyclic capture regions, explicit block-entry
home tables and typed simultaneous edge schedules in the common placement plan.
Every incoming edge establishes its parameter bindings or preserves the same
inherited complete home. Closed CFG interference includes full pointer/scalar
extents. Mixed schedules allocate only actual whole-source captures, including
overlap and cycles; allocation, selected requests and fresh replay verify the
same obligations. Calls, unsupported windows and the conservative cyclic core
retain their earlier storage choices. Trial allocation refuses frame/peak growth,
and edge-only loop preheaders retain existing stack affinities.

The [stage-4 scorecard](benchmarks/65816-record-placement-stage4/README.md)
records release code of 434,444 bytes, 58 fewer than stage 3, and a new
branching-representative benefit: `COOKEDLINE.Recall` is 899 bytes, 18 fewer.
All three profiles retain the first-tranche gates and avoid routine frame/peak
growth against both stages 0 and 3. Canonical mixed transfers add a small
measured cycle/traffic cost in the independent Flow probe, within the per-vector
limits. Native tests independently execute diamonds, distinct parallel bindings,
separate returns, conflicting predecessor fallback and reentrant task/IRQ/NMI
flows. Source private-storage admission is unchanged. Serial compiler costs,
including the initial guarded RSS variation and its focused repeat, are retained.

### Stage 5 Extend placement through loops and call boundaries

Qualify loop-carried value and pointer locations with fixed-point entry,
backedge and exit obligations. Handle zero-trip loops, nested loops, changed
bases and interrupted execution. Split residence around calls; preserve live
captures in authoritative invocation storage and reestablish needed locations
afterward. Extend private-storage promotion across control flow only with the
existing legality proof and supported placement consumers.

Exit gate: measured benefits in traversal, scanning and state-management
routines containing loops and calls. Direct, indirect, recursive, helper and
assembly calls must obey the same clobber and storage rules. Wider loop shapes
may retain fallback until their contracts are represented and checked.

The implementation uses complete call-free loop homes and bounded invocation-
backed pointer segments. Existing Native65816 private-storage promotion supplies
legal loop values without a shared policy change. The
[stage-5 scorecard](benchmarks/65816-record-placement-stage5/README.md) records
incremental loop/call benefits, independent native execution and compiler costs.

### Stage 6 Complete indexed and aggregate operation integration

Integrate embedded-array indexing and aggregate operations into the same value
and resource plan. Preserve complete address arithmetic, field offsets, element
strides and copy extents. Aggregate transfer choices must honor the existing
overlap-safe copy semantics; uncertain aliasing retains a safe strategy.
Aggregate internal scratch must participate in placement interference.

Exit gate: efficient representative indexed record workflows and qualified
aggregate handling, including bank crossings, overlapping copies, self-copy,
large offsets and pressure fallback. Any proposed change to an external access
protocol needs its own explicit contract and validation.

The integrated contract qualifies indexed record workflows on the frozen Exec
profiles and aggregate transfers through independent native execution. Exact
extents, overlap policy and scratch ownership remain checked through replay;
uncertain or unsupported placement retains complete invocation storage. The
[stage-6 scorecard](benchmarks/65816-record-placement-stage6/README.md) records
incremental code and native-cost benefits, preserved resource bounds and paired
compiler costs. The frozen Exec corpus has no MIR aggregate-copy operations, so
aggregate qualification is reported separately from Exec benefits.

### Stage 7 Qualify the integrated compiler and Exec816 artifact

Run the full affected backend qualification, rebuild the frozen comparison
profiles, and execute the agreed hosted Exec816 workloads using the measured
compiler and runtime. Review routine growth, cycle tradeoffs, stack peaks,
reserved memory and compile-time scaling. Update the lowering, emission,
allocation and tracking contracts to describe the final invariants.

Remove superseded planner ownership only after equivalent coverage has moved
to the common model. Keep useful selectors and their independent execution
oracles. Reassess the next roadmap direction from the resulting application
scorecard.

Exit gate: net code-quality benefits across the agreed subsystems, no unexplained
correctness or resource regressions, and artifact-bound qualification. Backend
VM evidence, hosted Exec816 evidence and hardware evidence retain their
respective scopes.

## Validation strategy

Use independently specified graph facts, arithmetic and memory results, actual
decoded instruction effects, and native execution. Compare ordinary external
access traces while separately measuring private traffic. Include exact
three-byte pointers, dirty hidden accumulator bytes, mode and flag boundaries,
canaries, aligned and odd addresses, bank carry/wrap, nested calls and relocated
images. Exercise IRQ/NMI and task suspension while resident values and transfers
are live. Check truthful homes, absent homes, frame extent, local peaks and
final stack balance.

During development run the affected MIR65816 unit, root integration and native
targets. Existing target families include `home_demand`, `address_consumers`,
`memory`, `pointer_allocation`, `resident_pointers`, `mixed_edges`,
`edge_coalescing`, `stack_allocation`, `loop_x`, `contexts`, `preemption` and
`pointer_preemption`; select from the actual consumers of each change. Run
full native qualification at major emission/allocation milestones and final
integration, covering required modes, checked/unchecked builds and supported
relocation profiles.

Changes to NIR, semantic lowering, the verifier, printer or shared contracts
also require the repository's shared checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

Normalize host fixture text where newline conventions are irrelevant and verify
changed parsing or instrumentation paths with actual LF and CRLF inputs. Keep
binary, ATASCII and line-ending-specific fixtures exact.

At each consumer milestone report application and representative-routine
deltas, eligibility and refusal coverage, runtime cost, private storage, and
compiler scaling. Set a host build-cost budget in stage 0; investigate expensive
repeated analysis before expanding eligibility further. Foundation-only
milestones report output equality and analysis cost.

## First implementation tranche

Start with stages 0 through 3: establish the baseline, land verified analysis and
resource contracts, and deliver the first benefits in mixed record operations
within blocks. This tranche proves the shared model before extending machine
facts across CFG boundaries. The subsequent stages complete the routine-wide
scope; completing the first tranche alone does not complete this plan.
