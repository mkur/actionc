# MIR6502 storage observation: implementation plan

Prepared 2026-11-15. Status: slices 0–5 complete; optimized default enabled.
Design commit: `9abe9094`. Compiler baseline: `67e9343a`.

Implements the [storage ownership and observation design](MIR6502_STORAGE_OBSERVATION_DESIGN.md).
Execute slices 0–5 in order and commit after each completed slice. Keep unrelated
working-tree changes outside those commits.

## Objective and scope

Preserve object identity, owning routine and verified lifetime information when
source declarations and reads become physical ranges. Let terminal cleanup
delete an otherwise-dead virtual-ZP temporary store when its only apparent
observers are certified resident objects in other routines.

Refine both the declaration-overlap and physical-read vetoes through one shared
proof context. Keep the existing reaching-definition proof, callee contracts,
fixed frames, final-placement replay, strict image-saving rule and exact rollback.

The first exception covers direct byte stores from a register into declared,
unexposed virtual-ZP temporaries. Overlapping foreign objects must be complete,
verified ZP-bound BYTE/WORD parameters or definitely initialized locals. Their
reads must depend on their own invocation's definitions. Different routine IDs
alone do not establish that property.

Retain the existing behavior for spills and fixed pointer scratch. Source-store
DSE, capture deletion, same-routine source overlap, ABI/result/entry-home overlap,
allocation pruning/movement, new placement choices and return-home coalescing
remain outside the exception. Preserve all explicit source/physical observations.
No SemIR/NIR change, second lifetime solver or program-specific rule is planned.

## Fast iteration and local testing

The user's explicit direction is **minimal testing to iterate quickly**. This
plan replaces the design's original broad local qualification step with focused
checks. Existing full CI coverage remains; broad backend and configuration
matrices are deferred to CI or a separately requested qualification run.

- Run one relevant filtered unit group per implementation slice. Add meaningful
  cases for the changed contract, batching closely related boundaries.
- Reuse existing integration and VM targets. Run the focused new cases when the
  behavior is connected end to end; do not execute entire targets unnecessarily.
- Reuse the recorded TN baseline while building the machinery. Run one final
  optimized ActionCart `$3000` comparison with only the new gate toggled.
- Do not run full `cargo test`, the complete MIR6502 unit/integration suite,
  the NIR sweep, the 39-target VM matrix or a full MADS matrix locally for this
  backend-only task. Broaden only when a failure indicates an additional affected
  contract, and start with that consumer's focused cases.
- Do not repeat a passing check without a relevant code change. Documentation-only
  edits need content/link checks, not another code run. Report checks not run.

LF/CRLF correctness still belongs in changed fixture handling: normalize host
text before newline-sensitive operations and exercise both through the actual
loader in one small batched control. Preserve exact binary and memory oracles.
Do not turn this into a source-format/runtime/origin Cartesian product.

Reuse `build/checked-arena/shared-target` and the established debug-0, opt-level-1,
incremental-off, two-job configuration. Keep large artifacts under ignored
`build/storage-observation/`; save compact receipts in
`docs/benchmarks/6502-storage-observation/`. Avoid duplicate build trees.

## Code boundaries

| Responsibility | Code to extend |
| --- | --- |
| Complete declarations and authoritative placement | [emit.rs](../src/mir6502/emit.rs), especially `source_storage_ranges` |
| Physical inventory and conservative queries | [analysis/record_bases.rs](../src/mir6502/analysis/record_bases.rs), `MirPhysicalMemory` |
| Temporary ownership and contextual eligibility | [analysis/storage_ownership.rs](../src/mir6502/analysis/storage_ownership.rs) |
| Exposure, bindings, initialization and residence | [parameter_homes.rs](../src/mir6502/parameter_homes.rs), [locals.rs](../src/mir6502/parameter_homes/locals.rs), [lifetimes.rs](../src/mir6502/parameter_homes/lifetimes.rs), [calls.rs](../src/mir6502/parameter_homes/calls.rs) |
| Shared CFG proof and generation-scoped sites | [home_liveness.rs](../src/mir6502/analysis/home_liveness.rs), [home_definitions.rs](../src/mir6502/analysis/home_definitions.rs), [sites.rs](../src/mir6502/analysis/sites.rs) |
| Both vetoes, delta validation and final replay | [late_cleanup/mod.rs](../src/mir6502/late_cleanup/mod.rs) |
| Existing preservation boundary | [late_cleanup/preservation.rs](../src/mir6502/late_cleanup/preservation.rs) |
| Gate and diagnostics | [passes.rs](../src/mir6502/passes.rs), [trace/homes.rs](../src/mir6502/trace/homes.rs) |

Prefer small helper modules under these existing homes if needed. Do not rename
the analysis modules or introduce a parallel storage authority. The conservative
inventory projection must continue to serve pruning, movement and older callers.

## Slice 0 — freeze evidence and isolate both vetoes

Use the [cross-call receipts](benchmarks/6502-cross-call-homes/README.md) as the
starting measurements. Check revision/source hashes before reusing them; do not
recompile TN merely to reproduce an unchanged baseline.

| Baseline subject | Recorded result |
| --- | ---: |
| TN packaged cart / standalone | 9,847 / 9,954 bytes |
| Ord code + ordinary homes | 80 + 5 bytes |
| Items code + ordinary homes | 56 + 3 bytes |
| Next / Strcpy / Fnamecmp code | 30 / 20 / 105 bytes |
| MoveMenuBar / Draw / Tag / TagAll code + ordinary homes | 115 + 4 / 145 + 1 / 88 + 2 / 87 + 0 bytes |
| Rejected cross-call trial | Net 48-byte saving; each of the four other routines grows two bytes |

Add a minimal manual-MIR characterization in the existing cleanup ownership
tests: one routine has a declared virtual-ZP temporary; another has a resident
parameter at the same byte. Distinguish an unused resident declaration from a
resident with a typed/fixed/pointer-pair read. Show that the declaration inventory
blocks eligibility, and inspect the read inventory independently. Do not disable
either production guard to construct the test.

Reuse existing fixed-global/source-ZP negative controls. Keep this slice's
behavior assertions at the current retained-store result; later slices update
only the explicitly enabled expectation. Use typed IDs and unrelated routine
names so the test is independent of TN.

**Minimal check:** the new characterization cases only. Commit the controls and
a short baseline receipt. No compiler output changes.

## Slice 1 — retain typed ownership in the physical inventory

Replace the inventory's sole address-range representation with complete typed
declaration records and a conservative range projection. Preserve owner, object
kind/ID, byte offset, declared extent, binding and final placement. Keep aliases
and descriptor backing explicit; do not deduplicate records only by address.

Emission populates physical placement using its existing authoritative layout.
MIR analysis owns protection classification. At this stage every declaration
continues to receive the old protection; no exception is active.

Keep `source_storage_disjoint` and the old read projection behavior unchanged.
Missing inventory or unresolved placement grants no separation proof. The same
typed byte ID in different routines must remain distinct. Parameter/local ID
spaces must not collide. A valid resident record must not conceal an absolute
alias occupying the same byte.

**Minimal check:** one grouped inventory test covering overlapping owners,
an unused absolute declaration/alias, mixed parameter lanes, missing placement
and equality of the conservative projection with the old inventory. Reuse an
existing small emitted fixture to compare image/map/relocations. Commit the
representation change with unchanged decisions.

## Slice 2 — certify invocation homes and their read sites

Extract reusable queries from existing binding/exposure, local initialization
and residence verification. Build certificates against verified expanded MIR,
not the selector's proposal or an earlier representation.

A certificate identifies a complete object and its bytes, actual capture or
logical initialization, current binding, body/CFG dependencies, callee facts and
exposure restrictions. It must establish no dependency on a previous invocation's
value. Physical scratch writes and call may-writes never count as initialization.
Do not accept an invalid baseline allocation on the assumption that subsequent
cleanup will repair its clobber.

Reuse the existing CFG transfers/solver for parameter and local definitions.
Build the derived site/lane table for typed reads and expanded fixed-ZP or
pointer-pair reads. The latter require a unique binding and reaching logical
definitions in the same routine; address equality alone is insufficient.
Preserve raw absolute, machine and ABI observations as distinct roles. Keep
general callee read/write summaries intact.

Collect structured observations with owner, generation-scoped site, access kind,
object/lane evidence where certified, and physical range. Preserve a conservative
projection for existing consumers. Cache shared facts once per current program
snapshot, with routine-local derived analyses refreshed as needed. Do not rebuild
whole-program summaries for every byte or candidate store.

**Minimal check:** one focused certificate group covering a fresh parameter
capture, an initialized local, a conditional/partial or entry-dependent local,
a physical write masquerading as initialization, and a stale body/binding or
callee fact. Include one ambiguous/raw read that cannot gain resident provenance.
Keep selection disabled. Commit the shared facts and tests.

## Slice 3 — refine both terminal vetoes behind one gate

Add `enable_scoped_storage_observation`, initially false in both default and
optimized configurations. It is subordinate to `enable_peepholes` and
`enable_late_private_store_cleanup`. The presence of a valid binding/certificate,
not a placement flag alone, authorizes its use.

Build one contextual proof view for the terminal transaction. For an eligible
virtual-ZP byte store, evaluate every declaration overlap and every relevant
physical observation. Permit only certified foreign invocation-home overlaps;
retain protected aliases, same-routine source overlaps, ABI/result/entry roles,
unattributed reads and unknown evidence. The role of ABI bytes comes from typed
contracts, not a numeric whitelist.

Feed qualifying stores to the existing structural dead-store proof. A resident
certificate removes an irrelevant global veto; it does not prove the store dead.
Real temporary reads, argument consumption, successors, backedges and calls keep
their existing interpretation. Do not skip callee reads merely because the callee
also owns resident storage.

Thread the same contextual eligibility through all transaction paths:

- Logical preflight: candidate discovery only; no permission from missing layout.
- Physical discovery: typed inventory and observations resolved with the measured
  baseline map.
- `only_cleanup_deletions`: validate each deleted store with the same baseline
  certificate, rather than rejecting it again through the old blanket query.
- Final-placement deletion replay: baseline operations and their logical proof
  sites, resolved using the new final map. Candidate operation indices cannot
  substitute for baseline proof sites.
- Candidate verification and preservation: rebuild/validate candidate facts and
  retain existing callee-contract and earlier-decision replay.

Rebuild or invalidate affected site facts after rewrites. If an external routine,
callee or binding dependency changes, a routine-local generation increment alone
is insufficient. Historical programs supply exposure/preservation constraints;
do not count their old allocations as additional current source objects.

Retain exact fallback, frames, bindings and virtual allocations. Keep conservative
queries in pruning/movement and leave producer purity and register/flag liveness
unchanged. Existing cleanup cascades may remove a newly dead producer only under
their current rules. Add no materialization, outer search or extra cleanup pass.

Add bounded diagnostics for early declaration rejection as well as physical
read rejection. Include owner/object/site and the failed proof category, not
only an address. Diagnostics are not executable evidence.

**Minimal check:** the slice-0 cases with gate off/on; one real reader and one
protected alias retain the store; stale/final-map replay rejects a deletion;
feature-off artifacts remain identical. Use existing rollback tests where they
already cover unchanged machinery. Commit the gated consumer; leave defaults off.

## Slice 4 — exercise the complete pipeline with a small VM control

Extend the existing `mir6502_cross_call_homes` integration and VM
`cross_call_homes` targets. Reuse their source controls where they expose the
interaction; add a small independent fixture only if needed. It must contain
both a resident parameter/local owner and a routine whose dead argument-staging
temporary shares the chosen byte. Ensure a real call remains in the isolated
case and retain one normal optimized configuration case.

Check an actual deleted store and final byte saving, unchanged source results,
canonical arguments, stack balance and repeated invocations. Batch an initialized
local and a source-owned ZP negative control into the same small test group.
Use sentinels for source memory; do not assert the final value of deleted private
scratch. Include actual LF/CRLF loading through the existing helper.

Exercise feature off/on for this small fixture. Use ActionCart at `$3000` for
the main comparison and one standalone execution smoke case, rather than a full
runtime/origin/NIR matrix. Retain a focused raw-NIR case only if the changed path
depends on optimization-created structure.

Extend the existing parameter-home measurement tool with an explicit toggle for
the new gate and the independent control. Keep leaf/handoff/cross-call selection
enabled in both comparison arms. Record normalized source, compiler/library and
configuration hashes, image/map/relocation equality or deltas, and proof/work
counts. A hand-bound unit fixture alone does not establish end-to-end savings.

**Minimal check:** only the new integration cases and small VM group. Broad MADS,
origin and backend matrices remain deferred. Commit the controls and focused
receipts; do not rerun TN during fixture development without a specific reason.

## Slice 5 — measure TN and choose the optimized default

Run one final TN A/B comparison at ActionCart `$3000` with optimized defaults,
changing only `enable_scoped_storage_observation`. Preserve TN source. Record:

- Selected and trial image sizes; Ord/Items code, inline and ZP bytes.
- Code/store deltas for MoveMenuBar, Draw, Tag and TagAll, plus unchanged leaf
  controls Next, Strcpy and Fnamecmp.
- Whether both global vetoes were discharged, or the exact remaining blocker.
- Core materializations, emission/replay work, inventory/certificate builds,
  compile time and peak memory. One timing is an observation, not a benchmark.

The previous net 48-byte saving includes the four extra stores. Removing them
also changes trial size and potentially final layout; do not hard-code 48 bytes
as the new expected gain. Accept only measured savings under the existing
no-routine-growth policy. Do not relax it to force a TN result.

Do not add a core materialization or upstream reselection. Legitimate existing
candidate emission/replay counts may change when cleanup succeeds; distinguish
that work from new analysis overhead. Investigate accidental per-store
whole-program rebuilds rather than treating them as an inherent cost.

Enable the optimized default only after the focused positive, negative, replay
and VM checks pass and independent final-image savings are established. Keep the
conservative default off. If proof or behavior remains unresolved, leave the
gate off and record the blocker; an unchanged TN image is not itself failure
when independent cases demonstrate the qualified scope.

After changing the default, run one public-default integration case against the
explicit-on result and a supported branch-free parent-gate control. Do not rerun
all prior passing checks for a configuration-only change. Publish exact local
test filters, artifact hashes, accepted/rejected cases and deferred CI coverage.
Update the design/plan status and commit the final default decision and receipts.

## Suggested focused commands and completion criteria

Group new unit cases under a `storage_observation` name so development can use:

```sh
cargo test --lib storage_observation
cargo test --test mir6502_cross_call_homes storage_observation
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes storage_observation
```

These filters are introduced by the implementation. Confirm a nonzero test count;
a successful command selecting zero tests is not evidence. Choose the relevant
subset per slice. Use existing named tests when they already establish a required
negative; do not duplicate an entire old test matrix under the new prefix.

Completion means both global vetoes consume shared typed evidence, existing
deadness and final replay remain authoritative, independent controls pass and
save final bytes, TN has a measured outcome, and local validation limits are
reported honestly. Each completed slice has a commit. Broad qualification remains
in CI under the user's fast-iteration direction.

## Slice 0 completion

The grouped ownership characterization passes for unused, typed-read,
fixed-ZP-read and pointer-pair-read residents. Declaration and read vetoes are
independently established. Existing source hashes match the recorded baseline;
[baseline receipt](benchmarks/6502-storage-observation/baseline.json) reuses those
measurements without recompiling TN. One filtered unit test passed.

## Slice 1 completion

The emitted declaration inventory now retains typed identity, routine owner,
binding, lane offset and extent. The conservative overlap query is unchanged.
Two focused tests pass, including mixed lanes, overlapping routine owners,
absolute aliases and missing inventory. An initial test attempted an invalid
resident/global overlap; the existing verifier rejected it as required. The
corrected inventory test keeps those distinct cases valid.

## Slice 2 completion

Invocation-home certificates now reuse the resident lifetime solver to exclude
entry-dependent bytes. Read observations retain routine and operation-site
identity; raw absolute and machine observations remain physical. The immutable
program borrow ties certificates to current bodies, bindings and callees.
Four focused tests pass, including logical versus physical initialization,
parameter/local homes, stale bindings and typed/fixed/pointer-pair read sites.
The local test initially used a program-entry routine, which correctly failed
residence verification; its corrected Action routine passes. Cleanup still uses
the conservative projection. [Check receipt](benchmarks/6502-storage-observation/slice2-checks.json).

## Slice 3 completion

Both terminal vetoes, deletion-delta validation and final-placement replay now
share the same immutable invocation-home proof. Historical programs restrict
exposure without contributing old allocations. The new gate is off in both
configurations. Existing structural deadness, frames, cost and preservation
checks remain unchanged. Five focused tests pass, including a full transaction,
live/raw reads, historical exposure, stale snapshots and a conflicting final
inventory. [Check receipt](benchmarks/6502-storage-observation/slice3-checks.json).

## Slice 4 completion

The independent fixture saves 34 final bytes (261 to 227) with no routine growth.
The old 229-byte trial is rejected for a two-byte staging-store regression; the
new proof removes that store and permits the existing selection to succeed.
Materializations, emissions and preservation replays remain unchanged. The new
integration and VM groups pass, including the initialized counter, a protected
source window, repeated calls, ABI arguments, stack balance, sentinels, real
LF/CRLF loads and one standalone execution. The VM fixture's initial whole-output
newline comparison was corrected to exclude debug source spans. No compiler
change was needed. [Measurements and scope](benchmarks/6502-storage-observation/README.md).

## Slice 5 completion

TN ActionCart `$3000` saves 56 bytes, 9,847 to 9,791, with unchanged Next/Strcpy/
Fnamecmp and no growth in the four previously regressing routines. Off artifacts
match the recorded baseline exactly. Four more staging stores disappear in the
cross-call trial. Core materializations remain six; emissions, physical
inventories and preservation replays are unchanged. Three immutable invocation
snapshots are built across existing transactions. The optimized default is on,
the conservative default off, and the public API/parent-gate check passes.
[Final receipts, timings, test corrections and deferred coverage](benchmarks/6502-storage-observation/README.md#tn-and-optimized-default)
record the focused qualification. All six slices have been implemented; no
broader backend or TN runtime matrix was run.
