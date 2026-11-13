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
pruning, coloring and ZP reassignment remain excluded. Do not invoke the whole
core cleanup group or materialize the program again.

## Producer-deletion extension

Status: contract accepted; implementation and qualification in progress.

The store-only TN image is 9,982 bytes. Its final assembly contains two unused
loads of `global_s`/`global_s+1` in `FormatDirectoryRow` and another pair in
`SetWin`. These account for 12 candidate instruction bytes. Final image savings
must be measured after proof validation and emission.

The first producer slice admits only byte `LoadImm`, register-to-register
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
behavior remains disabled; optimized configuration enables it after qualification.
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
