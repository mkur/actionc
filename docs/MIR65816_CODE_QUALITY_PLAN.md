# Native 65816 code-quality improvement plan

Status: refreshed after the
[emission simplification](MIR65816_EMISSION_SIMPLIFICATION.md),
preserving bounded X loop increments `1ce9624` and their output. Native word selection, copy
scheduling/coalescing, compact staging, local/frame/parameter forwarding,
scalar DP, one checked X loop mirror and native INX updates are complete.
Historical inventories remain evidence for their own revisions;
closed-operation interference remains unchanged.

The [analysis and checked-rewrite foundation](MIR65816_ANALYSIS_REWRITE_FOUNDATION_PLAN.md)
is complete, adapting MIR6502's home/definition liveness, register/flag liveness
and checked rewrite workflow. The
[emission simplification plan](MIR65816_EMISSION_SIMPLIFICATION_PLAN.md) is complete:
unused shadow/reference paths are removed or test-scoped, analyses are computed
on demand, original loads are reconstructed once and replay uses verified input
CFGs. The checked driver remains the sole removal authority. The subsequent
Dijkstra comparison establishes emitted code size as the next priority; the
[ordered code-size backlog](BACKLOG.md#native-65816-code-size-reduction) is
deferred at user request except for the completed selected slices below. Broader
allocation and further replay machinery remain deferred.

The user-selected [BYTE and pointer comparison plan](MIR65816_BYTE_POINTER_COMPARISONS_PLAN.md)
is complete. Native BYTE predicates and three-byte Eq/Ne/null comparisons use
compact Boolean materialization and adjacent branch consumption. On frozen
Exec `8e1ff57`, optimized shell executable bytes fall 574,884→502,845 (12.5%),
with all 547 routine contracts and 2,279 guards intact. Optimized Dijkstra falls
6,197→5,779 bytes; the small corpus's 28 Action images are byte-identical.
See [measurements and qualification](benchmarks/65816-byte-pointer-comparisons/README.md).

The [native signed word comparison and branch-fusion plan](MIR65816_SIGNED_WORD_COMPARISONS_PLAN.md)
is implemented. A16 subtraction and overflow correction replace eight Dijkstra
branch sites: raw code falls 6,365→5,878 bytes and optimized code 5,779→5,290;
optimized `Find` falls 2,132→1,887. All homes and guards remain unchanged.
The small corpus and frozen Exec shell are byte-identical. See the
[measured results](benchmarks/65816-signed-word-comparisons/README.md).
Allocation and the other backlog items remain separate work; local transfer
relaxation is completed below.

The [current-source Exec size audit](benchmarks/65816-exec-size-detail/README.md)
adds a guarded raw/optimized baseline for Exec `c3500c8`. It identifies repeated
call payload initialization, compact guard encoding, four-byte equality and BYTE
returns as focused selection/layout opportunities. The
[padding-only call initialization slice](benchmarks/65816-call-padding/README.md)
is complete: 13,846 raw / 13,756 optimized executable bytes saved, with ABI v1,
all guards, stack peaks and bank-zero reservations unchanged. Full native
debug/release qualification, relocation, IRQ/NMI and CRLF checks pass.
Captured BYTE returns are completed below. No new allocation machinery or public
ABI change is proposed.

The user-selected [short guard-branch plan](MIR65816_GUARD_BRANCHES_PLAN.md)
is complete. Reusing typed dispatch/layout for four conditionals shrinks each
guard from 45 to 29 bytes while preserving every check and both JMLs. Frozen Exec
saves 37,248 raw / 37,120 optimized executable bytes, and Dijkstra saves 352 bytes
in each mode. Native debug/release, fault, IRQ/NMI, relocation and CRLF checks
pass. See the [results](benchmarks/65816-guard-branches/README.md). Other guard
and branch opportunities remain deferred.

Direct BYTE constant returns now use A16 `LDA #$00xx`, followed by the shared
frame teardown and RTL. This removes 19 bytes per eligible return: 8,626 raw /
9,424 optimized bytes on frozen Exec, with unchanged allocation and guards.
See the [results and qualification](benchmarks/65816-byte-returns/README.md).

Native LONGCARD/LONGINT Eq/Ne, including zero on either side, now compares
captured low/high words in A16 and reuses canonical Boolean materialization or
adjacent branch fusion. Frozen Exec saves 22,502 raw / 23,364 optimized code
bytes from the BYTE-return baseline, with unchanged homes, guards and ABI.
Long ordering keeps its existing path. See the
[plan](MIR65816_LONG_EQUALITY_PLAN.md) and
[results and qualification](benchmarks/65816-long-equality/README.md).

Native-width call payload copies and A/X result capture are complete. A bounded
A8/A16 choice accounts for mode-switch costs within each argument sequence,
keeping bytewise paths when widening would not save code. Frozen Exec saves
10,867 raw / 10,281 optimized executable bytes from the long-equality baseline;
Dijkstra saves 100 / 99 bytes. Layouts, frames, scratch reservations and guards
are unchanged. See the [plan](MIR65816_NATIVE_CALL_COPIES_PLAN.md) and
[measurements](benchmarks/65816-native-calls/README.md).

Remaining local jump and branch relaxation is complete. All typed internal
conditionals use the checked short-branch layout; local jumps select BRA or BRL
when in range. Frozen Exec saves another 17,902 raw / 17,257 optimized bytes,
reaching 399,816 optimized executable bytes. Dijkstra saves 125 / 126 bytes.
Frames, guards, ABI and bank-zero budgets are unchanged. See the
[plan](MIR65816_LOCAL_RELAXATION_PLAN.md) and
[measurements](benchmarks/65816-local-relaxation/README.md).

Final index scaling and numeric constant shifts are now compact. Indexed
addresses omit the unused last scratch shift; constant counts use byte moves,
zero fill and bounded residual A8/A16 chains. Frozen Exec saves 3,481 raw /
4,776 optimized executable bytes, reaching 395,040 optimized bytes. Dijkstra
saves 91 / 90 bytes. Frames, guards and ABI contracts remain unchanged. See the
[contract](MIR65816_CONSTANT_SHIFTS.md) and
[qualification](benchmarks/65816-constant-shifts/README.md).

Captured BYTE stack returns now load one byte and clear hidden B directly,
removing result scratch preparation. Frozen Exec saves 2,392 raw / 2,083
optimized executable bytes with unchanged frames, guards and ABI homes.
The small corpus and Dijkstra executable artifacts are unchanged. See the
[contract](MIR65816_CAPTURED_BYTE_RETURNS.md) and
[qualification](benchmarks/65816-captured-byte-returns/README.md).

The [completed implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
records module changes, commit boundaries and qualification gates.
Its baseline gate and typed physical-effects slices are
[complete](MIR65816_ANALYSIS_EFFECTS.md), with unchanged raw and optimized output.
The [selected-action and CFG slice](MIR65816_SELECTED_ACTIONS.md) is also complete.
[Canonical home bytes and backward liveness](MIR65816_HOME_ANALYSIS.md) are complete.
[Stored definitions and read attribution](MIR65816_HOME_DEFINITIONS.md) are complete.
[Register-lane and independent flag liveness](MIR65816_MACHINE_LIVENESS.md) is complete.
[Typed replay](MIR65816_TYPED_REPLAY.md) is authoritative with exact encoding
equality. [Checked plans and atomic application](MIR65816_CHECKED_REWRITES.md)
now own [adjacent temporary A16 forwarding](MIR65816_ADJACENT_CHECKED_FORWARDING.md)
with unchanged eligibility and output. Full native debug/release/CRLF qualification
passes. The historical foundation measurement was 0.147→0.361 seconds for 28
builds and 5.70→6.92 MiB median per-process peak RSS relative to `1ce9624`.
The simplification's new paired measurement is 0.395→0.275 seconds and
7.52→7.13 MiB against its frozen foundation compiler. These are separate host
runs; target-code bytes and execution measurements remain unchanged. See the
[full corpus and size-ladder measurements](benchmarks/65816-emission-simplification/measurements.md)
for scaling and remaining per-edit replay/definition costs.

## Objective and current baseline

Reduce emitted code size while preserving the public ABI and stack guards;
track execution cycles as a secondary constraint. The
[Dijkstra baseline](benchmarks/65816-dijkstra/README.md) complements the small
corpus below and motivates the deferred code-size priorities. Current main
already has CFG-aware stack-slot reuse and
restricted pointer-leaf and scalar DP allocators. General lifetime-based stack
reuse is an existing capability. See the
[temporary-allocation contract](MIR65816_TEMPORARY_ALLOCATION.md).

Use the qualified [INX snapshot](benchmarks/65816-loop-inx/after/tables.md)
as the working baseline for new forecasts. Its
[exact delta](benchmarks/65816-loop-inx/delta.json) checks all 28 complete Action
images and 264 records against the X-mirror baseline. Only optimized rotation
changes: 129→126 bytes, 759→735 cycles and 218→210 instructions. Its frame stays
eight bytes, and stack/DP traffic, copies, public ABI and guards remain. Eight
X-forwarded input loads become eight separately counted INX updates. Other
forwarding counts and the remaining 27 Action builds are unchanged. Keep
historical snapshots immutable.

Both compilers implement a general unsigned 16-bit sum loop; input 13 is supplied
at runtime, and both return 91. Measurements run from function entry through RTL,
including Action's stack guards and excluding caller setup:

| Mode / compiler | Code bytes | VM cycles | Additional stack bytes |
| --- | ---: | ---: | ---: |
| Optimized actionc | 120 | 1,092 | 6 |
| Optimized vbcc | 22 | 344 | 0 |
| Raw actionc | 146 | 1,587 | 14 |
| Raw vbcc | 32 | 533 | 4 |

The current optimized [Action listing](benchmarks/65816-loop-inx/after/sum_loop.optimized.actionc.lst)
uses native word arithmetic, direct single-word edge copies and adjacent A16
forwarding with checked width omission, fallthrough and short dispatch, while
retaining temporary stores in DP and the mutable counter on the stack. The
[vbcc listing](benchmarks/65816-loop-inx/after/sum_loop.optimized.vbcc.lst)
retains the counter in X and the sum in DP. This remains evidence for future
residency work after the shared analysis foundation. Public argument placement
and stack guards remain outside this work.
Use the full corpus, including calls, pointer traffic and wider values, to choose
and qualify general improvements.

## Completed milestones and historical measurements

| Milestone | Completed scope | Evidence |
| --- | --- | --- |
| Empty-edge cleanup | Compact transfers without unnecessary edge-copy setup. | [Results](MIR65816_EMPTY_EDGES.md) |
| Direct single-word edge copies | Bypass staging for one checked word assignment; retain multi-value paths, reservations and guards. | [Results](MIR65816_SINGLE_WORD_EDGE_COPIES.md) |
| Local accumulator forwarding | Adjacent eligible word operations reuse A16 with matching private-home and N/Z facts; retain stores and homes. | [Results](MIR65816_LOCAL_ACCUMULATOR_FORWARDING.md) |
| State-tracker foundation | One typed emission boundary owns instruction effects, execution modes, stack equations and the existing forwarding witness. No additional optimization. | [Results](MIR65816_STATE_TRACKER.md), [qualification](abi/action65816-state-tracker-qualification.json) |
| Control-flow slices 3a–3c | Checked MIR-entry REP omission, terminal fallthrough after copies and bounded short dispatch with final offset/relocation checks. | [Results](MIR65816_CONTROL_FLOW.md), [qualification](abi/action65816-control-flow-3c-qualification.json) |
| Direct acyclic word copies | Topological copy scheduling preserves sources and final A/N/Z; cycles retain staging and all frame reservations remain. | [Results](MIR65816_ACYCLIC_EDGES.md), [qualification](abi/action65816-acyclic-edges-qualification.json) |
| Compact staging reservations | Reserve only staged edges and actual per-index widths; recheck incoming offsets, frame accounting and guards. | [Results](MIR65816_STAGING_RESERVATIONS.md), [qualification](abi/action65816-staging-reservations-qualification.json) |
| Selective cyclic staging | Capture only sources endangered by earlier assignments, preserve original destination order, and reserve compact capture slots. | [Results](MIR65816_SELECTIVE_STAGING.md), [qualification](abi/action65816-selective-staging-qualification.json) |
| Direct frame store/load forwarding | Omit only an immediately redundant word reload from a non-addressable frame object; preserve both stores, homes and A/N/Z. | [Results](MIR65816_FRAME_FORWARDING.md), [qualification](abi/action65816-frame-forwarding-qualification.json) |
| Incoming-parameter word forwarding | Omit a repeated immutable argument load immediately or through one checked Store; retain captures and independent temp/frame witnesses. | [Results](MIR65816_PARAMETER_FORWARDING.md), [qualification](abi/action65816-parameter-forwarding-qualification.json) |
| Compatible edge-home coalescing | Transactional source-home affinities with fixed block-parameter anchors; omit direct self-copies and preserve final A/N/Z. | [Results](MIR65816_EDGE_COALESCING.md), [qualification](abi/action65816-edge-coalescing-qualification.json) |
| Bounded scalar DP allocation | Promote up to 16 existing private word-home classes in verified call-free routines; preserve forwarding and shrink real stack frames. | [Results](MIR65816_SCALAR_DP.md), [qualification](abi/action65816-scalar-dp-qualification.json) |
| Bounded X loop mirror | Keep one private unsigned loop parameter in X; use TXA/CPX and final edge TAX while retaining homes and stores. | [Results](MIR65816_LOOP_X_RESIDENCY.md), [qualification](abi/action65816-loop-x-qualification.json) |
| Bounded native INX updates | Advance the reserved loop parameter with checked INX/TXA; invalidate its mirror relation until the retained final TAX. | [Results](MIR65816_LOOP_INX.md), [qualification](abi/action65816-loop-inx-qualification.json) |
| Analysis foundation slices 0–1 | Authenticate the unchanged-output baseline and centralize typed physical effects, including verified native call/return summaries. | [Results](MIR65816_ANALYSIS_EFFECTS.md), [equality](benchmarks/65816-analysis-rewrite/slice1-equality.json) |
| Analysis foundation slice 2 | Record typed instructions and compiler requests, with scoped sites, selected CFG and unchanged emission. | [Results](MIR65816_SELECTED_ACTIONS.md), [equality](benchmarks/65816-analysis-rewrite/slice2-equality.json) |
| Analysis foundation slice 3 | Canonicalize physical home bytes with verified ownership and compute ordered backward may-liveness. | [Contract and results](MIR65816_HOME_ANALYSIS.md), [equality](benchmarks/65816-analysis-rewrite/slice3-equality.json) |
| Analysis foundation slice 4 | Attribute reads to individual physical stored definitions, preserving may-write uncertainty and possibly undefined paths. | [Contract and results](MIR65816_HOME_DEFINITIONS.md), [equality](benchmarks/65816-analysis-rewrite/slice4-equality.json) |
| Analysis foundation slice 5 | Compute physical register-lane and independent flag liveness, preserving protected environment obligations. | [Contract and results](MIR65816_MACHINE_LIVENESS.md) |
| Analysis foundation slice 6 | Replay typed actions through a fresh tracked emitter, recomputing permissions and regenerating all output metadata. | [Contract and results](MIR65816_TYPED_REPLAY.md), [equality](benchmarks/65816-analysis-rewrite/slice6-equality.json) |
| Analysis foundation slice 7 | Validate sealed plans against immutable facts and publish only after scratch replay, layout and rebuilt analyses. | [Contract and qualification](MIR65816_CHECKED_REWRITES.md) |
| Analysis foundation slice 8 | Retain original load candidates and migrate adjacent temporary forwarding through the checked driver with identical decisions. | [Contract and qualification](MIR65816_ADJACENT_CHECKED_FORWARDING.md), [equality](benchmarks/65816-analysis-rewrite/slice8-equality.json) |
| Analysis foundation slice 9 | Qualify full native debug/release and CRLF runs, frozen corpus equality, mutation controls and host compilation costs. | [Results and next priorities](MIR65816_ANALYSIS_REWRITE_QUALIFICATION.md) |
| Emission simplification | Demand-driven analyses, one original-load reconstruction, immutable input CFG reuse and test-scoped migration machinery; unchanged generated code and checked publication. | [Results](MIR65816_EMISSION_SIMPLIFICATION.md), [qualification](abi/action65816-emission-simplification-qualification.json) |

The original roadmap used the
[empty-edge snapshot](benchmarks/65816-empty-edges/after/tables.md). The measured
progress for `sum_loop(13)` is:

| Qualified stage | Raw bytes / cycles | Optimized bytes / cycles |
| --- | ---: | ---: |
| Empty-edge cleanup | 174 / 2,032 | 154 / 1,735 |
| Direct single-word edge copies | 174 / 2,032 | 146 / 1,595 |
| Adjacent accumulator forwarding | 164 / 1,767 | 140 / 1,395 |
| State-tracker foundation | 164 / 1,767 | 140 / 1,395 |
| Checked MIR-entry width omission (3a) | 158 / 1,683 | 132 / 1,308 |
| Terminal fallthrough (3b) | 150 / 1,627 | 124 / 1,252 |
| Short conditional dispatch (3c) | 146 / 1,587 | 120 / 1,212 |
| Direct acyclic word copies | 146 / 1,587 | 120 / 1,212 |
| Compact staging reservations | 146 / 1,587 | 120 / 1,212 |
| Selective cyclic staging | 146 / 1,587 | 120 / 1,212 |
| Direct frame forwarding | 146 / 1,587 | 120 / 1,212 |
| Incoming-parameter forwarding | 146 / 1,587 | 120 / 1,212 |
| Compatible edge-home coalescing | 146 / 1,587 | 120 / 1,212 |
| Scalar DP | 146 / 1,587 | 120 / 1,092 |
| Bounded X mirror | 146 / 1,587 | 120 / 1,092 |
| Bounded native INX (current baseline; sum-loop unchanged) | 146 / 1,587 | 120 / 1,092 |

The observed stack peak remains 14 bytes raw. It stayed 16 bytes optimized
through acyclic scheduling, then fell to 12 with compact staging reservations
and six with scalar DP. The original direct-copy forecasts and selection details
remain in its
[detailed plan](MIR65816_SINGLE_WORD_EDGE_COPIES_PLAN.md); the
[measured results](MIR65816_SINGLE_WORD_EDGE_COPIES.md) confirmed those forecasts.
They are completed work, not forecasts for the next slice.

## Ordered remaining slices

Typed selection/effects, home and machine analyses, authoritative replay,
checked transactions and the adjacent temporary forwarding migration are
complete and qualified. The
[BYTE and pointer comparison plan](MIR65816_BYTE_POINTER_COMPARISONS_PLAN.md),
prepared from the Exec816 measurements, and the
[signed-word implementation plan](MIR65816_SIGNED_WORD_COMPARISONS_PLAN.md)
are also implemented. The remaining
[code-size backlog](BACKLOG.md#native-65816-code-size-reduction) orders broader
local branch relaxation, compact address construction and compact guard encoding
ahead of further allocation work. Re-inventory the current output before planning
branch relaxation; the earlier candidate counts describe older comparison
sequences. The address-generation-first throughput recommendation remains
superseded. Other work remains backlogged pending a focused scope.

Removable-store inventory, mutable-counter promotion, broader residency and
selective DP extensions remain later candidates. Use the existing stored-definition,
home and register/flag queries to measure them before selecting a bounded rule.

The [completed simplification](MIR65816_EMISSION_SIMPLIFICATION.md) retains
projection semantics, proof obligations and generation invalidation. Per-edit
full replay and definition postconditions remain; cross-generation caching and
edit batching are deferred. Its measurements do not justify another abstraction.

Efficiency refactors retain the strict byte/full-record equality gate. Each new
optimization needs its own measured delta and must use the checked proof API.

The table below retains the original roadmap's completed scopes and deferred
extensions, including the former combined control-flow step split into 3a–3c:

| Order | Improvement | Initial scope |
| --- | --- | --- |
| 3a (complete) | Checked MIR-entry width omission | Omit redundant REP only with checked complete predecessor obligations; retain value/flag barriers. |
| 3b (complete) | Jumps to adjacent blocks | Checked terminal fallthrough follows every edge assignment; earlier arms retain their jumps. |
| 3c (complete) | Short-branch selection | Checked routine finalization and bank placement preserve fixups, PER, traces and o65 relocation, with a long-transfer fallback. |
| 4 (bounded slice complete) | Parallel-copy scheduling and coalescing | Both compatible initialization pairs now share anchored destinations; retain closed-operation interference and frame accounting. |
| Frame forwarding (complete) | Direct frame store/load forwarding | Typed object/displacement/home witness, exact A/N/Z and one omitted load; both stores and all frame contracts remain. |
| Parameter forwarding (complete) | Repeated incoming-parameter loads | Separate checked read witness, immediate reload or one admitted Store; both measured raw-code reloads removed. |
| 5 (bounded slice complete) | Scalar DP allocation | Up to 16 word-home classes; call-free whitelist, explicit scratch partition, mixed-location verification and exact stack accounting. |
| 6 (bounded X slice complete) | X/Y residency across loops | One private unsigned loop parameter has checked X16 tests and increments. Mutable-counter promotion, broader loops and Y allocation require separate proofs. |

The [movement inventory](MIR65816_MOVEMENT_INVENTORY.md) found no additional
opportunities within the existing temp-producer/consumer classes. Three reached
reloads required new frame/parameter load consumers. The direct frame case now
saves two bytes and 40 cycles in optimized `loop_rotation(13)`. Repeated
incoming-parameter forwarding now saves two bytes in each raw build and 5/65
cycles in rotation/recursion at input 13. Coalescing subsequently saves eight
bytes and 20 cycles in optimized rotation. Each slice was measured separately.

These are native MIR65816 strategy and emission changes. Consume verified typed
facts; do not recover semantics from source strings or SemIR. If a later slice
needs stronger NIR facts, introduce and verify those in a separate boundary
change before relying on them.

## Completed control-flow scope and inventories

The [3a–3c implementation plan](MIR65816_CONTROL_FLOW_IMPLEMENTATION_PLAN.md)
is qualified. The tracker grants width-omission permission only at checked MIR
entries, retaining value/flag/home barriers. Terminal fallthrough executes all
edge copies first; short dispatch preserves arm order and long fallbacks. The
[emission contract](MIR65816_EMISSION_CONTRACT.md) records the invariants.

The historical [slice 4 inventory](MIR65816_COPY_INVENTORY.md) found two staged
word edges in optimized `loop_rotation`. Its initialization chain now copies
directly: measured code falls from 160 to 148 bytes and cycles from 1,146 to 1,116,
with the 26-byte peak retained at that stage. Compact staging now reduces it to
20 bytes. Selective staging then reduces it to 16 bytes, with only the endangered
backedge source saved. General
acyclic scheduling also handles reordered copies and restores final A/N/Z when
needed; all four existing single-word corpus edges retain their behavior.

Compact staging removes the wholly unused four-byte slots in `sum_loop` and
`byte_sum`, plus the unused upper halves in `loop_rotation`. Measured frames
fall 16→12, 22→18 and 26→20 bytes respectively, with independently checked
operand changes, alignment, incoming offsets, frame bounds and guards. Selective
staging within the cyclic backedge is now [qualified](MIR65816_SELECTIVE_STAGING.md):
eight fewer code bytes and 160 fewer cycles per rotation call, with staging
reduced 6→2 bytes and frame/peak reduced 20→16. Existing single-word and acyclic
schedules are preserved. The frozen plan's forecasts all matched.

Freeze a fresh baseline for every later optimization. Use the qualified native-INX
snapshot for new forecasts. The movement inventory's edge counts
originally had zero physical self-copies, two compatible initialization pairs
and five interfering pairs. Both compatible pairs now share homes; the five
interfering pairs retain their copies. All three reload candidates are complete.

## Completed candidates and next allocation strategy

The [movement inventory](MIR65816_MOVEMENT_INVENTORY.md) covered 172 word stack-load
sites and 10 nonempty edge assignments across 28 builds before frame forwarding.
Frame forwarding removed one static load executed 48 times per incoming I state,
saving 240 cycles and 96 stack-byte reads across the six rotation vectors.
Incoming-parameter forwarding subsequently removed two static loads in raw
rotation/recursion: 28 fewer instructions, 140 cycles and 56 stack-byte reads
across vectors per incoming I state. These isolated deltas matched their frozen
forecasts, and all unaffected artifacts and metrics remained equal.
Existing adjacent-temp forwarding counts retain their meaning; frame and
parameter proof families report their own reached executions.

The optimized rotation's two compatible initialization pairs are now coalesced.
Moving both source homes together passes full frame verification while retaining
all block-parameter anchors. Exactly two producer operands change and four edge
instructions disappear: eight bytes and 20 cycles saved per call, with four
fewer stack-byte reads and writes. That slice left the frame at 16 bytes. The five other
pairs interfere, including the sum-loop result and its input during their shared
arithmetic operation; their copies remain.

The [scalar DP inventory and plan](MIR65816_SCALAR_DP_PLAN.md) are implemented.
At input 13 the sum loop now saves 120 cycles and six stack bytes; rotation
saves 103 cycles and eight stack bytes. The slice preserves the existing logical
copies and forwarding. Rejected raw casts, wider values and calling routines
retain their previous strategy.

The historical [post-DP register inventory](MIR65816_REGISTER_INVENTORY.md)
measured all 28 builds and reconciled memory traffic for all 132 Action records.
It identified rotation's private counter as the first bounded X candidate.
The [implemented plan](MIR65816_LOOP_X_RESIDENCY_PLAN.md) preserves the DP home,
stores and input/update interference while selecting TXA and CPX immediate,
with a final TAX on each incoming edge. Qualification matches its frozen
130→129-byte and 793→759-cycle forecast exactly.

The [post-X inventory](MIR65816_POST_X_INVENTORY.md) classifies 2,148 instruction
sites and 176 word memory-load sites across the same 28 builds. It measures
retained stores, backedge loads, staging and mutable-frame traffic. The resulting
[native INX slice](MIR65816_LOOP_INX.md) matches its 129→126-byte and
759→735-cycle forecast, keeping all memory traffic unchanged. Rotation retains
16 staging-byte reads/writes per call; sum-loop retains 80 mutable-parameter
byte reads and 28 writes at input 13. These are remaining costs, not forecasts
of removable work. Keep these measurements as candidate evidence and use the
completed analysis/rewrite foundation to establish removal safety before
selecting a store-elimination or residency rule.

Mutable counter promotion (including sum-loop's frame parameter), broader scalar
admission, partial DP allocation and cross-call residency need separate
alias/effect and profitability proofs; they are not implied by this inventory
or the current allocator.

## Proof obligations for later slices

**Shared analyses and checked rewrites.** The forward state tracker does not
replace backward liveness. Base physical home and register/flag proofs on the
actual typed selected sequence, including internal branches and helper/copy
effects. Separate a particular stored definition from the home it occupies;
may-write aliases cannot kill a live definition. Bind plans to immutable
routine/allocation generations, verify replacement effects, and rebuild facts
after mutation. Adapt MIR6502's existing contracts and tests while preserving
native widths, stack equations and preemption guarantees.

**Control flow.** Remove jumps only when control reaches the intended successor
directly, including any required edge assignments. Short branches need range and
bank checks after layout, compatible relocation handling, and long-transfer
fallbacks. Keep guard behavior and fault paths intact. Width omission, fallthrough
and branch relaxation have separate acceptance gates.

**Copy scheduling and coalescing.** Prove physical byte-range compatibility and
preserve swaps, cycles, repeated sources, mutable parameter homes and successor
live-ins. Account for A/N/Z effects when eliminating edge loads, as well as their
memory traffic. Retain a checked staging fallback. Do not globally weaken the
current closed-operation interference rule. Further reservation removals require
proof that selection no longer writes those bytes; then recompute frame extent,
incoming displacements and stack-guard accounting.

**DP and register allocation.** Use the tracker's explicit instruction effects,
then define the allocator's lifetime and scratch reservations. Observing a value
in a register does not reserve that register against later selector use. Allocate
only approved DP ranges; apparently unused scratch bytes alone are not a contract.
Begin with call-free scalar routines and a conservative operation whitelist.
Keep values spanning ordinary or helper calls on their invocation's stack and
retain fallback emission for unsupported operations. Mutable parameters and
addressable locals need separate storage and alias proofs before promotion.
Add X/Y residency only when selection can honor addressing, width and clobber
constraints across every edge.

**Preemption.** Preserve the existing domain-owned DP and complete CPU-state
restoration contract. Live scratch may survive asynchronous suspension only
under that contract. Qualify interruptions while resident values and arithmetic
flags are live. Keep IRQ/NMI isolation and interrupt reserves unchanged.

## Validation and completion criteria

For each slice:

1. Preserve the previous snapshot and declare expected instruction and traffic
   changes before implementation. Retain the 14-pair / 66-vector corpus as the
   comparison core and add general regression cases, never sample-specific rules.
2. Check exact selection, rejection and fallback cases. Include boundary words,
   frame displacements, both branch arms, backedges, cyclic copies and malformed
   plans as relevant. Keep independent ca65 encoding checks for new sequences.
   Use immutable state traces as claims to verify against executing code, never
   as the VM's input or the expected-result oracle.
3. Execute serialized raw and optimized machine code through the
   [qualification runner](../tools/native65816-runtime-tests/README.md) in debug
   and release hosts, with both incoming interrupt-mask states. Cover clobbering
   direct/indirect/helper calls, aliasing and volatile traffic, IRQ/NMI suspension,
   relocated o65 code, recursion and stack-boundary failures as affected.
4. Record code bytes, cycles, stack depth and stack/DP traffic. Permit only the
   explicitly predicted changes; require unchanged unaffected artifacts and
   behavior. Keep byte-identical refactors on the
   [strict equality gate](../tools/compare65816/check_state_tracker.py). Give
   optimizations their own independently checked deltas. Recheck LF/CRLF corpus
   builds and retain the known external vbcc optimized unlink failure rather
   than exempting it.
5. Run the compiler checks required by the affected contracts. NIR changes also
   require the contributor-mandated snapshots, sweep and full compiler suite.
   Update the relevant contract, save qualification evidence and commit the
   completed slice while preserving unrelated local changes.

Preserve [physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md), image v3, the
[o65 profile](MIR65816_O65_PROFILE.md), public argument/result placement and stack
guards. Frame sizes may shrink only with verified accounting. No slice reduces
the platform's interrupt headroom or changes Exec816's compiler pin; adopting
new compiler output in Exec816 remains a separate integration qualification.

The width/control-flow and copy slices target stack-resident code. DP and X/Y
allocation address the larger remaining gap to vbcc. Set further numerical
targets from each new baseline rather than promising cumulative gains before
implementation.
