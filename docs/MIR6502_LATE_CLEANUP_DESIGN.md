# MIR6502 late store and producer cleanup

Status: implemented, 2026-11-13. The original evidence was the pipeline and
qualification results through `278076a2`. This note defines the implemented
cleanup boundary and acceptance contract. See the
[implementation plan](MIR6502_LATE_CLEANUP_IMPLEMENTATION_PLAN.md) and
[qualification](benchmarks/6502-late-cleanup/README.md).

## Decision and evidence

The executable pipeline runs one shared, frame-preserving private-store cleanup
transaction after all late MIR6502 transformations and before final pre-emission
verification. It reuses the existing analyzed store-deletion proof and accepts
the result only when its
final placement preserves the assumptions used by earlier transformations and
its complete emitted image is smaller. Otherwise return the unchanged input.

The [pipeline audit](MIR6502_OPTIMIZATION_PIPELINE_AUDIT.md) identifies the
scheduling gap: core cleanup runs before late transformations that remove
memory consumers. The [phase-tracing findings](benchmarks/6502-phase-tracing/README.md)
demonstrate it in two independent cases:

| Case | Transformation removing the last reads | Remaining footprint |
| --- | --- | --- |
| TN: four stores in `FormatDirectoryRow`, two in `SetWin` | T3 pointer retention | Six stores emitting 18 instruction bytes; six allocated bytes |
| Argument-staging control | T5 argument staging | Two written virtual-ZP homes without remaining explicit reads |

Earlier cleanup correctly retained these stores while their readers existed.
The missing contract is that a later consumer change makes that cleanup result
out of date. An explicit-read count identifies an opportunity; it does not prove
that deleting a store is safe. A MIR load that emits zero bytes remains a
semantic consumer and is a different case.

The implementation targets instruction bytes. TN's six allocated bytes remain
reserved. The original trace established an opportunity; the separate
qualification measures accepted instruction and image savings after checking
legality, emission state and layout.

## Ownership and position

MIR6502 owns candidate discovery, home-definition proofs, machine-state
preservation and selection. Emission owns authoritative addresses, branch
relaxation, relocations and final byte counts. SemIR and NIR contracts do not
change; cleanup consumes MIR facts without recovering source semantics.

The boundary belongs in
[`prepare_program_for_emission`](../src/mir6502/mod.rs), after checked-range
pointer retention (T10), under the existing peephole master gate:

```text
core materialization and cleanup
  -> selected late transactions T1–T9
  -> checked-range pointer retention T10
  -> shared late private-store cleanup transaction
  -> optional address-stable spill-declaration pruning transaction
  -> PreEmission verification and exact pre-emission snapshot
  -> final emission
```

Putting cleanup at the end covers every late family, including T10. Adding a
private cleanup call to pointer retention alone would leave the independent
argument-staging case and future consumer changes dependent on pass ordering.

The historical materialized-MIR dump ends before T10. Preserve that boundary
and document that the exact pre-emission snapshot includes this new transaction.
All executable output modes must select the same final MIR.

## Store-deletion scope

Eligible candidates are the existing direct byte stores from a machine
register into `MirMem::Spill` or virtual `MirMem::ZeroPage` storage, as recognized
by [`store_direct_private_scratch_byte`](../src/mir6502/materialize/peepholes.rs).
Fixed zero-page ABI slots, source variables, globals, statics, absolute
addresses, indirect stores and hardware accesses are outside this scope.

With producer cleanup disabled, the transaction deletes only eligible stores
and preserves:

- Every frame declaration, storage identity, allocation and virtual-ZP mapping,
  including newly unused homes.
- Register producers, loads, arithmetic, calls, machine blocks, terminators,
  block order, routine membership and ABI placement.
- Data declarations, helper selection and inlining decisions.

The producer extension below adds a narrowly checked deletion class. Frame
pruning, coloring and ZP reassignment remain excluded from the operation-cleanup
transaction. The optional allocation experiment has its own declaration-only
acceptance and rollback boundary, described below. Do not invoke the whole core
cleanup group or materialize the program again.

## Producer-deletion extension

Status: implemented in `97b50d4d`, enabled in optimized builds by `075aa821`.
See the [producer qualification](benchmarks/6502-late-producers/README.md).

The frozen store-only TN image is 9,982 bytes. Its assembly contained two unused
loads of `global_s`/`global_s+1` in `FormatDirectoryRow` and another pair in
`SetWin`. The qualified extension removes these four loads and saves twelve
final image bytes, producing a 9,970-byte cart image. Frame declarations remain
identical.

The first producer slice admits only byte `LoadImm`, native A↔X/A↔Y
`Move`, and direct byte `Load` operations into A/X/Y. It excludes arithmetic,
word operations, indirect loads, pointer-valued moves, raw absolute reads,
fixed-ZP reads, calls and machine blocks. This closed shape check is separate
from the proof that a particular definition is dead.

Reuse `discover_dead_register_writes` and its existing structural-plan/liveness
proof through a caller-supplied purity predicate. Core cleanup keeps its current
purity rules. Late cleanup additionally requires an emitted physical mapping
that marks a named memory source ordinary and separates it from executable
memory. Missing placement, hardware/OS memory, externally backed storage without
ordinary-memory evidence and unknown reads retain the producer. Repeat these
queries against final candidate placement; `MaterializeLayout` is insufficient
for the late read-elision decision.

Every deleted producer's destination register and N/Z outputs must be dead on
all paths, including successor blocks and backedges. Calls and machine blocks
remain barriers unless the existing structured analysis proves the affected
state unobserved. The admitted operations preserve C/V; arithmetic that changes
these flags requires a later extension. A load that emits zero bytes is still
a semantic read and requires the same liveness and purity proofs.

Store and producer plans share one monotonically decreasing, routine-local
rewrite loop. Rebuild analyses after each batch. Producer removal can make a
private store dead, and store removal can make its producer dead. Bound rounds
by the input operation count; do not repeat late strategy selection or run
whole-program materialization. Whole-program baseline callee summaries remain
conservative inputs during discovery. After rewriting, rebuild summaries and
preserve every consumed entry/exit/register/fixed-pair contract and each prior
memory-effect exclusion. Narrower may-read/may-write sets are allowed. A callee
contract failure rejects the complete transaction.

The complete-delta guard permits only these producer deletions plus the existing
private-store deletions and checked compare-index repair. Frames, data,
allocations, ABI, CFG, helper choices and remaining operations stay identical.
The earlier selected-decision sidecar remains authoritative: replay all affected
proofs from immutable witnesses using final placement. Deleting pure dead
register definitions adds no allocation dependency; all new register/flag and
read-effect obligations belong to this terminal transaction.

`enable_late_dead_register_cleanup` controls the extension inside the existing
`enable_late_private_store_cleanup` and master peephole gates. Default-config
behavior remains disabled; optimized configuration enables it.
The historical trace stage name remains stable. Report `removed_stores` and
`removed_producers` separately. Keep the same two-cost-emission budget, strict
packaged-image saving, no per-routine instruction growth and exact rollback.

Qualification compares store-only versus producer-enabled output, separately
from the earlier no-terminal-cleanup baseline. Cover TN, independent ordinary
loads and moves, live registers/flags, successor/backedge use, observable memory,
callee contracts, changed placement and rollback. Compare actual execution,
public writes and return state, with both runtimes, shifted origins and real
LF/CRLF source/listing paths. Record code/image deltas, preserved frames, trace
on/off equality and compiler work before enabling the optimized default.

## Cleanup invalidation and fact lifetimes

An accepted late transformation that changes operations must make its affected
routines eligible for shared cleanup. Requests describe the actually selected
result, including partial acceptance; rejected speculative candidates do not
contribute requests. This is compiler state, independent of tracing.

Initially, conservatively seed the request set with all routines and use the
cheap candidate-shape filter before building analyses. Complete changed-routine
reporting can later narrow that set. Missing or unfamiliar change information
must cause a broader scan, never a silent assumption that cleanup is current.
There is one terminal transaction, rather than cleanup after every request.

Build routine analyses against the selected input. Reuse the analyzed
[`structural_plan`](../src/mir6502/rewrite/posthome.rs) path and its exact
home-definition deadness proof, covering control-flow successors, backedges,
overwrites and allocated private aliases. Do not introduce a second deletion
rule based on counts or on the absence of assembly loads.

Preserve existing conservative effect rules. In particular, the analyzed home
model distinguishes compiler-private homes from public fixed-ZP storage;
unknown effects do not imply identical treatment of both. Cleanup must retain
that established distinction without relaxing call or machine-block effects
to obtain a desired saving. Where private identity alone cannot discharge a
physical alias obligation, require the corresponding physical proof or retain
the store.

Physical zero page needs an additional read proof: a numeric source pointer can
observe a virtual home's backing byte. Follow each candidate definition across
CFG edges until a definite overwrite; calls' may-write summaries do not kill it.
Reject indirect or unknown readers, including readers inside called routines
and in caller continuations after return. Context merging conservatively visits
all direct callers; only a definite overwrite ends the definition lifetime.
Ordinary private frames retain the existing non-escape contract and explicit
physical-alias checks.

A recognized scalar `SArgs` entry has a narrower ABI read footprint under the
default runtime binding: argument bytes at `$A0` onward, runtime workspace at
`$11`/`$82`–`$85`, stack and inline descriptor data. Its three-byte descriptor
and argument count must agree with the typed scalar signature. Binding
provenance is retained before helper resolution; an explicit `SET` override
cannot use this contract. The proof does not depend on display names or source
locations. Other machine blocks and unrecognized entries remain conservative.

An applied batch invalidates operation-indexed facts and routine generations.
Rebuild the required analyses before further queries. Trace coordinates are
diagnostic locations, not persistent optimizer witnesses. Routine-local
invalidation does not refresh externally supplied callee summaries or physical
maps; the outer transaction owns those dependencies.

## Layout and earlier proofs

**Preserving frames does not preserve placement.** Removing instructions can
move deferred arrays, change program-end values and execution ranges, and alter
branch encodings. `MaterializeLayout` is a planning artifact. Only converged
emission of the exact candidate at its actual origin establishes its layout.

Consequently, replaying the store-deletion proof and passing `PreEmission`
verification are necessary but insufficient. An earlier transformation may
already have removed operations using a physical disjointness, ordinary-RAM,
bounded-write or callee-exit assumption. A new physical map cannot reconstruct
that lost justification by inspecting the final operations alone.

The contract is a bounded set of **preservation obligations** retained
for selected decisions whose semantic proofs depend on layout or callee facts.
Each obligation identifies the decision's owner, stable storage/routine
identities and byte ranges, the required predicate, and any immutable witness
needed to check it. These are compilation-local proof inputs, separate from the
optional trace journal. Speculative obligations follow the same acceptance and
rollback as their decisions.

| Dependency | Required candidate check |
| --- | --- |
| A physical address used as a constant in a proof | Address equality, or reconstruction by the owning proof validator |
| Disjointness, ownership, ordinary RAM or bounded call writes | Recheck the relevant ranges against the candidate's final physical map |
| Separation from executable memory | Recheck against final execution ranges |
| Program-end or other layout-derived values | Recompute and validate the dependent predicate |
| Callee entry, exit or preservation facts consumed by a caller | Preserve the relied-on facts using fresh whole-program summaries |
| Operation-indexed checked-range evidence | Validate from its unchanged source witness or a checked deletion mapping; never reuse stale indices |

Validation covers affected assumptions from the entire selected pipeline,
including decisions before the late sequence and callers outside the cleanup
request set. Pure profitability decisions need no replay: final cost is measured
again. Semantic assumptions require either a demonstrated invariant or a
validator owned by the transformation that used them. Existing family-local
placement replays can supply those validators; they do not automatically
certify other families.

The executable preparation boundary retains a sidecar with a closed family tag,
immutable input and selected MIR snapshots, and the selected routine IDs.
Each family's validator reruns its proof/application logic against the candidate
map and requires the same selected result. Partial selections replay only their
accepted routines. Checked-range evidence uses its original immutable snapshot.
These validators perform no emission, materialization or profitability search.
Snapshots also supply address references removed by earlier transformations;
those references never allocate storage or become executable operations.

The sidecar starts at completed core materialization and covers every selected
T1–T10 mutation. An unrecorded mutation breaks its snapshot chain and declines
cleanup before costing. Core allocation proofs retain their inputs because the
transaction preserves declarations, fixed backing, mappings and origin. Pure
dead producers are deleted only under the extension's register/flag and read-effect
proofs; this does not alter core allocation inputs. The [plan's dependency inventory](MIR6502_LATE_CLEANUP_IMPLEMENTATION_PLAN.md#dependency-inventory-and-implementation-choice)
records that boundary. An unfamiliar affected dependency requires a new
family-owned validator before cleanup can accept it.

Requiring all relevant addresses to remain identical is a valid conservative
bootstrap, but may reject the TN case because code shrink moves deferred data.
It is not sufficient qualification for the intended TN improvement. Relational
proofs should allow movement when the required disjointness and bounds still
hold; they must not assume that movement is harmless.

## Transaction and acceptance

Keep an immutable baseline at the new boundary. Discover and apply proved
deletions in deterministic routine/block order using the existing rewrite
driver. Rebuild facts as required by its generation rules. Deletion-only progress
strictly reduces the operation count; do not run a fixed point over unrelated
optimization families or search subsets for a more profitable result.

Accept the candidate only when all of these checks pass:

1. Only permitted store/producer deletions occurred, frames and allocations are identical,
   and the result passes `PreEmission` verification.
2. Each store deletion has a valid home-definition proof, and each producer
   deletion has valid register/flag liveness and read-purity proofs under the
   candidate's applicable memory/effect facts. Candidate placement validates the deletion
   decisions made from the baseline.
3. Fresh whole-program callee analysis preserves all consumed contracts,
   including observable accumulator results, required flags and entry inputs,
   register/fixed-pair preservation, and memory-effect assumptions. Equal
   frames alone do not establish this.
4. All affected preservation obligations from earlier selected decisions pass
   under final candidate placement.
5. Completed emission, including relocation finalization and runtime/output
   packaging, yields a strictly smaller image at the same origin and settings.
   No routine's emitted instruction footprint grows. Estimated MIR costs only
   rank candidates; they cannot accept the transaction.

Candidate verification, layout, proof or cost failure restores the whole
baseline. Keep a typed rejection reason. Failure to emit the baseline remains
a normal compilation error; it cannot be hidden as an optimization rejection.
An input with no proved deletions returns directly without cost emission.

Budget at most two additional top-level cost emissions, baseline and candidate,
and zero additional full materializations. Reuse an existing measurement only
if its exact program, origin, runtime and emission settings match. Proof
validators operate on supplied layouts and witnesses; they must not hide
unbounded emission or rematerialization loops. Count inner layout/branch probes
separately. Final artifact emission remains the ordinary final step.

## Observability and qualification

Give the transaction its own trace stage with input, proposed and returned
snapshots. Report requests, proved deletions, proof blockers, invalidated facts,
acceptance or rollback, exact instruction/image deltas and resource counters.
Selected history must distinguish attempted deletions from retained savings.
With tracing disabled, selection and generated artifacts must be identical.

Qualification should establish the following distinct behaviors:

- TN's six stores and the argument-staging homes are either safely removed or
  have explicit proof blockers. A remaining blocker means the motivating
  optimization is unfinished, even if conservative fallback is correct. Measure
  final code and image savings separately; retain the allocated homes.
- Reads on successor paths or backedges, physical aliases, unknown observable
  effects and required callee exit facts prevent unsafe deletion. An unrelated
  zero-byte emitted load remains a MIR use.
- Code shrink that moves deferred data passes when all predicates remain true;
  a changed alias, bound, execution range or unsupported dependency rolls back.
  Include T10 and selected partial-rollback histories.
- Positive and rejection fixtures agree in execution and observable memory,
  cover cart and standalone runtimes and shifted origins, and exercise complete
  fixture APIs where CLI reachability would prune the relevant routines.
- Final MIR6502 backend qualification includes its affected unit, integration,
  VM/runtime and MADS listing checks, with LF/CRLF fixture paths. Record trace
  on/off equivalence, materialization/emission counts and compile-time/memory
  cost against the frozen tracing corpus.

This is a MIR6502/emission-boundary change. Qualification follows the
repository's backend-scoped checks; shared compiler contracts are unchanged.
No NIR migration or new source-language rule is part of this design.

## Deferred decisions

Frame compaction needs a separate design extension: it changes home addresses
and can invalidate allocation, alias and call proofs beyond instruction shrink.
Further producer classes, including arithmetic and indirect loads, require
separate register/flag and memory-read contracts. They are outside the admitted
byte-load/move extension.

The shared transaction restores cleanup after consumer changes, with one owner
for acceptance and rollback. Compact typed witnesses may eventually replace the
coarse retained snapshots, provided their preservation checks remain equivalent.

## Allocation-pruning experiment

Status: implemented as an opt-in experiment, disabled in both default and
optimized configurations. TN retains all six candidate spill bytes and has no
additional image saving. The [qualification report](benchmarks/6502-late-spills/README.md)
records the address-stable control, conservative blockers and remaining gaps.

The first allocation slice removes only completely unreferenced one-byte
`frame.spills` declarations. It retains live storage IDs, order, parameters,
locals, data declarations, fixed backing, ABI, virtual-ZP allocations and every
operation. Reuse the core spill-pruning candidate scan, intersected with the
structured effect/reference inventory. Address references, including references
in immutable earlier-decision witnesses, retain a spill. Structured reads and
writes, terminators, call results and frame-slot backing also count as uses;
a missing explicit load is insufficient. Data relocations cannot name spills.
Unknown or indirect readers/writers retain storage unless existing callee
summaries provide exact footprints. Compiler-private identity alone does not
justify discarding zero-initialized bytes that unknown code might observe.

### Core allocation preservation

The existing sidecar starts after core materialization. It cannot certify a
change to assumptions made by core alias checks, captured comparisons, scalar
captures, constructor storage or zero-page selection. The initial experiment
therefore requires exact equality of all surviving `MaterializeLayout` storage
addresses and attributes, and unchanged zero-page allocations. Removing only
unreferenced ordinary declarations cannot change source typing, descriptor
facts, logical alias identities or storage widths. The address-equality guard
preserves the remaining allocation-dependent core inputs. Removed spills must
be private, unexposed and absent from executable references; they supply no
surviving value or address.

This is deliberately an address-stable subset of compaction. It usually admits
unused storage at the end of ordinary allocation. It does not authorize moving
live cells to fill interior holes. TN's six holes precede live storage, so the
experiment must report a core-allocation blocker if equality fails. General
interior compaction needs retained, replayable core allocation predicates; a
new final map alone cannot reconstruct assumptions erased by earlier rewrites.

### Placement and rollback

Run the optional pruning transaction inside executable terminal preparation,
after the existing accepted store/producer cleanup. Its immutable baseline is
that already-cleaned program. A pruning rejection returns it exactly, retaining
previous savings. Keep the original operation-cleanup transaction unchanged.
No recursive strategy selection, coloring, zero-page reassignment, subset
search or full rematerialization is permitted.

Before costing, reject changes to surviving core allocation inputs. Baseline
and candidate converged emissions then supply final placement, protected code
ranges, relocation results and packaged-image cost. Check surviving physical
storage addresses and ordinary-memory facts, and replay the existing selected
late decisions with the candidate map. Missing addresses in historical proofs
cause rejection; never invent ghost addresses for removed storage. Declarations
referenced by proof witnesses do not allocate memory during replay.

Require a strictly smaller packaged image and no per-routine instruction growth.
Verify the exact permitted declaration-only delta and `PreEmission` invariants,
preserve whole-program callee contracts, and restore the complete pruning
baseline on verification, layout, preservation or cost failure. A no-op or a core
allocation blocker causes no cost emission. A nontrivial pruning attempt is
bounded by two cost emissions and zero full materializations. Track these
separately from the preceding operation-cleanup transaction.

The experiment is explicitly configured by `enable_late_spill_pruning`, under
the existing terminal/master gates, and remains disabled in both default and
optimized configurations. Trace candidate counts, retained reasons, final
removed bytes, actual cost and rollback independently of operation deletion.
Trace-off builds must make identical decisions.

### Qualification

Compare pruning off/on with store and producer cleanup held enabled. Cover TN
and independent controls at multiple origins in both runtimes. Report TN's
actual result even if zero; report six candidate bytes separately from accepted
savings. Positive controls must exercise generated code as well as declaration
removal, and execute against a byte oracle. Negative controls include interior
holes, surviving spill/address references, relocation-driven movement, unknown
placement, observable fixed aliases, live zero-page mappings and cost rollback.
Reconstruct and relocate listings through real LF/CRLF paths. Record work counts
and preserve the producer-only qualification receipt. Leave the experiment
opt-in if the narrow address-stability contract yields insufficient general
benefit; broader compaction is a separate design and proof extension.

## Core allocation dependency census

The first follow-up is diagnostic only. `TraceOptions::core_layout_census`
(`--trace-core-layout` with `--trace-mir6502`) observes the initial, post-home
and post-ZP `MaterializeLayout` instances in core materialization. It does not
change any query result, rewrite, allocation guard, preservation witness or
cost decision. Diagnostic state is excluded from layout equality.

Address observations carry structured storage IDs, offsets, optional planning
addresses, layout epoch and source location. Raw address queries have no claimed
access width. Annotated predicates additionally record equality, range
membership, disjointness, or the above-ZP/no-wrap condition, with their original
boolean result and operand ranges. `planning_address` distinguishes ordinary
planning queries from fixed-address/ZP identities; it is not an escape or
movability proof. Unknown addresses remain unknown.
Predicate names group existing checks; their exact checked/wrapping arithmetic
still belongs to the recorded source site. They are not a replay instruction set.

The shared rewrite drivers bound collection to an analysis/discovery round and
record the actual applied sites, including the routine, generation, block and
range. This is a **conservative round envelope**: queries may support shared
analysis or rejected candidates within a successful round. They must not be
presented as necessary predicates of every applied rewrite. Rounds with no
applied rewrite remain distinguishable. Parent pipeline attempts may still
reject a locally applied result; the existing attempt/selection provenance
remains authoritative for interpreting diagnostics.

Queries outside those drivers are explicitly unattributed. Source-level
constructor/capture transactions that build separate planning layouts remain
outside this initial dynamic census. Raw query sites also identify consumers
whose relational predicate has not yet been annotated. These coverage gaps
prevent any claim that the census is a complete preservation certificate.

Collection is compilation-local, opt-in and bounded independently of the normal
trace byte budget. Repeated observations are counted; exhausted bounds are
reported as truncation. Consumers must check both trace and census status.
No diagnostic record authorizes compaction or replaces an optimizer fact.
Findings and qualification are recorded in the
[census report](benchmarks/6502-core-layout-census/README.md).


## Ordinary word-result destination proof

Known-callee word-result placement may store the high lane first because the
callee leaves that byte in A. Its destination checks can use a typed declaration
proof instead of five physical-byte comparisons. This proof belongs to MIR6502
materialization and consumes only MIR declarations, not SemIR or source names.

The admitted range has a directly declared ordinary global, local or parameter
scalar root, a known extent, and two adjacent bytes fully within that extent.
Global aliases, local alias views, descriptors, array/deferred storage, ABI-only
parameters, spills, virtual/fixed ZP and absolute backing do not qualify. Store
reordering permission remains a separate mandatory check.

The origin must be at least $0100. A conservative whole-program storage budget
must fit between the origin and $10000: sum declaration sizes plus initializer
payloads for allocating roots, statics and spill bytes. This intentionally
counts some bytes twice and includes deferred storage. Qualifying scalar roots
are allocated inline before code; their addresses therefore stay inside this
budget independently of code size. Bounds arithmetic is wide/saturating toward
rejection, never a truncated or wrapped address calculation. This extra budget
is necessary because current layout arithmetic saturates; declaration bounds
alone do not prove two lanes distinct near $FFFF.

These facts establish distinct destination lanes and separation from all fixed
ZP bytes, including the $A0/$A1 return slots, without consulting an individual
planning address. They remain valid under movements that preserve the root,
its extent/storage class, inline allocation, origin bound and total budget.
The fact table is scoped to the materialization layout; a consumer must rebuild
it after allocation changes. It is not a persistent preservation witness.

Other destinations retain the existing explicit address checks. Both destination
addresses must resolve before that fallback can succeed; unknown is not a
separation proof. In particular, aliases gain no new movement permission.
The diagnostic `ordinary-range-outside-zero-page` observation records the
structural result and range with no numeric address. It must not be counted as
an unresolved numeric query.

### Supporting-analysis audit

`run_analyzed_known_callee_word_result_placements` uses the shared driver's
`run_fixed_point_with_known_callees`, which supplies an empty physical memory
map. Its accumulator fact is checked immediately after the call against the
fixed high return slot. MIR callee exit summaries also run machine-value
analysis with an empty map; machine-block summaries use declared/fixed contracts.
Register/flag liveness and structural-plan validation use CFG, effects and typed
home identities. None of these entry points queries `MaterializeLayout`.

The accumulator and observable-exit checks, A/N/Z liveness, home-definition
checks and callee contracts are unchanged. Earlier transformations of callee
bodies can still depend on layout, and later allocation changes must preserve
those bodies' contracts. Eliminating this family's direct numeric queries does
not certify the whole pipeline, authorize compaction, or cover other layout
owners. Existing terminal acceptance, rollback and exact surviving-address
guards remain in force; spill pruning stays disabled by default.


## Pointer setup and candidate attribution

Pointer-pair selection may choose the existing scratch pair without an individual
planning-address query when a complete ordinary two-byte source range satisfies
the ordinary-storage bounds proof. The source lanes must still be adjacent.
Fixed/absolute sources, aliases, descriptors and unknown bounds keep the existing
explicit placement path. Only addresses through $FE can start a complete fixed
ZP pair; $FF cannot.

Plain-address source separation may use the same proof for each one-byte source
cell when the entire scratch pair is inside zero page. Pair starts at $FF do not
qualify: the existing check's high address is $0100. Unsupported/unknown sources
keep the conservative numeric check. Source-read observability, memory invalidation,
register/flag liveness, callee effects and the address-reuse rules are unchanged.
This does not broaden ordinary storage eligibility or permit allocation movement.

The census now tags the local observations of explicitly scoped pointer candidates
with family, block and starting operation; routine, generation and attempt come
from the containing round. A recorded shape also has its exclusive end. Driver
candidate/overlap-rejected/applied events update only an exactly matching range
in that round. No plan or an unselected shape is not an accepted transformation.
Locally applied still means speculative until every enclosing attempt selects it.

Scopes cover direct-pointer rematerialization, pointer-temp dereference selection,
and local gates for redundant plain/indexed address preparation. SSA prefix
observation and fact learning occur outside candidate scopes: their queries remain
shared analysis and can support a later candidate. Candidate observations therefore
are local-gate records, not complete necessary proof obligations. Queries outside
drivers and other layout owners retain their prior coverage limitations. No scope,
missing record or empty local observation set certifies layout independence.

Observation and candidate counts are bounded. Omitted candidate records mark the
census truncated, just like omitted observations; the trace byte limit remains
independent. Disabled/filtered diagnostics must not change decisions or work.
Candidate tags are diagnostics only and never enter optimizer plans or proofs.
