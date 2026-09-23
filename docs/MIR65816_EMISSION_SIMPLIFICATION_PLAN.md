# Native 65816 emission simplification implementation plan

Status: slice 0 complete; slices 1–5 pending. See the
[implementation record](MIR65816_EMISSION_SIMPLIFICATION.md). Based on
main `9640df19`, whose qualified compiler source is `e4fd88b5`. This follows the
[completed analysis foundation](MIR65816_ANALYSIS_REWRITE_QUALIFICATION.md).
Implement and commit the slices separately, preserving existing local changes.

## Objective and scope

Remove duplicated orchestration from the existing emission/rewrite path while
preserving its proof obligations and generated output. Keep typed effects,
physical-home and stored-definition analyses, register/flag liveness, generation
checks and atomic publication. The success criterion is a smaller, clearer
production path with measured reductions in unnecessary work.

The previous 2.46× host-time increase is 0.147 to 0.361 seconds across 28 small
builds, measured against compiler `1ce9624`. It establishes neither an urgent
latency problem nor large-program scaling. Use `e4fd88b5` compiler behavior as
this plan's before state; retain `1ce9624` as the historical code-quality oracle.
Do not optimize toward an arbitrary speedup or add infrastructure merely to
improve a timing ratio.

No new forwarding eligibility, dead-store elimination, register residency,
allocation changes, NIR changes or public compiler options belong in this work.
ABI v1, image v3, o65 profile v1, stack guards, helper/call clobbers, alias rules,
IRQ/NMI reserves and Exec816's compiler pin remain unchanged.

## Findings and the boundary that must remain explicit

The current path is:

```text
selection with proved omission projections + retained actual-load candidates
  -> repeat candidate proofs on the projected routine
  -> restore each omitted load through a separate edit/CFG rebuild
  -> replay/layout the original stream
  -> for each removal: analyze, replay a prefix, edit, replay/layout, analyze
  -> publish the final routine
```

Concrete duplication:

| Location | Current work | Planned simplification |
| --- | --- | --- |
| [AnalysisSnapshot::new](../src/mir65816/emit/analysis/mod.rs) | Computes every dataflow analysis for every context. | Construct each analysis only when its query is used. |
| [pilot::apply](../src/mir65816/emit/rewrite/pilot.rs) | Repeats local proofs before the authoritative driver; restores loads one at a time. | Structural candidate validation, one expansion of the original stream, then driver authorization. |
| [replay::walk](../src/mir65816/emit/replay.rs) | Builds a CFG even though its immutable selected input already owns a verified CFG. | Use the constructor-verified input; continue validating replayed output. |
| Rewrite/reference plumbing | Retains an unused shadow runner and test mechanisms in the ordinary path. | Delete obsolete code and restrict reference controls to qualification builds. |

Selection currently observes the projected post-state, including the exact
instruction/label cursor, single-use captures and X fallback priority. Emitting
all loads during selection can change later choices even when machine values
are equal. Therefore this plan retains one explicit projection-to-original
bridge. It removes repeated reconstruction and duplicate authorization work;
it does not claim to eliminate that semantic dependency.

The retained candidate owns the actual LDA and logical temporary/home identity.
Projection observations help selection and diagnostics; they cannot authorize
final load deletion. The driver remains the sole publication authority.

## Intended ownership

| Component | Responsibility after simplification |
| --- | --- |
| Selector / tracked facade | Preflight operands, retain actual loads, prove the equivalent projection, consume witnesses on success and failure, preserve selection order. |
| Existing candidate adapter | Validate candidate/consume correspondence; reconstruct the original instruction stream once; supply diagnostics and rediscover candidates. |
| SelectedRoutine | Own immutable actions and their verified CFG; edits create a new selection generation. |
| AnalysisSnapshot / Context | Validate sites and answer requested analyses for one immutable generation. |
| Checked driver | Recompute effects, establish equivalence, replay/layout scratch output, check postconditions and publish atomically. |
| Qualification support | Run reference comparisons, mutation controls and separate work/host measurements. |

Use the existing modules. Do not introduce another IR, pass manager, generic
analysis registry, rewrite-session wrapper or cross-generation cache. New small
helpers must replace existing duplicated work, with the old path removed in the
same completed slice.

## Slice 0 — Freeze the baseline and expose work counts

Completed: [baseline/work qualification](benchmarks/65816-emission-simplification/slice0.json).

Authenticate the [final foundation record](abi/action65816-analysis-rewrite-qualification.json)
against current compiler/fixture hashes. Retain the existing 658 native artifacts,
264 comparison records and 224 comparison files. Save new evidence separately
under `docs/benchmarks/65816-emission-simplification/`; never refresh the INX or
foundation reports.

Build and retain a before CLI from the qualified compiler with the same Rust
toolchain, release settings and features used for later after measurements.
Record source/tool/binary hashes and commands. Reuse
[measure_host.py](../tools/compare65816/measure_host.py) for the 28-build corpus.

Add bounded qualification-only work counts at actual execution points: home
access construction, each dataflow solver, prefix replay and visited actions,
whole replay, CFG construction, original-stream expansion and finalized layout.
Keep counters out of image metadata and ordinary release builds; scope them to
one compilation so parallel tests cannot mix observations. Count actual calls,
not estimates derived from candidate totals. Timing runs use uninstrumented CLIs.

Add a small generated size ladder based on the existing long-chain regression,
for example 16/32/64/128/160 operations, plus a branch/loop family. Exercise raw
and optimized modes, require verifier-clean programs within current frame/code
limits, and verify retained work rather than assuming optimization leaves each
source operation intact. Compare exact before/after images and execute boundary
inputs. Record size, candidate counts, wall time and per-process peak RSS; a
timing observation is not a flaky CI threshold.

**Exit:** frozen current-output baseline and enough work counts to judge removal
of duplicated work. No compiler policy change or permanent profiling subsystem.

## Slice 1 — Remove obsolete migration scaffolding

Audit call sites, then remove the unreferenced `pilot::shadow` runner. The saved
shadow qualification remains immutable evidence. Keep the active independent
reference tests in `checked_rewrites.rs` and `replay.rs` qualification coverage.

Restrict synthetic identity rules and reference-selection controls to `cfg(test)`
or `native65816-state-proof` as their actual consumers require. Ordinary builds
must have one production route into checked emission. Avoid duplicating the
whole selector to isolate a small reference choice. Retain the historical
predicate where unchanged frame/incoming mechanisms still use it.

Narrow blanket dead-code allowances only where the audit supports doing so.
Keep the tested liveness/proof APIs intended for later rules; they are deliberate
capabilities, not obsolete migration code. Do not fold unrelated module cleanup
or renames into this slice.

**Checks:** native library tests with and without the proof feature; active
reference/replay tests; ordinary release CLI build and frozen corpus equality.
**Exit:** removed shadow orchestration and no ordinary runtime reference switch.

## Slice 2 — Compute analyses on demand within each immutable snapshot

Keep the borrowed immutable `SelectedRoutine` and current fallible home-access
construction. Replace eagerly constructed home-liveness, stored-definition and
machine-liveness fields with private per-snapshot lazy results, using the
standard library's cell types rather than a new caching framework. Existing
query methods initialize their required result and return the same facts.

Validate routine/allocation/selection identity, bounds and reachability before
answering a query. An uncomputed result is never interpreted as empty, dead or
safe. Each analysis runs at most once per snapshot. A new generation starts with
new empty result cells; no result or success token transfers across an edit.

Preserve the driver's removed-definition and post-replay undefined-read checks.
Stored definitions are still needed for those checks. This slice avoids the
unused home/machine liveness solvers in adjacent-load transactions; it does not
remove safety checks under an assumption that this rule probably needs less.
Full home/machine query users still compute all requested facts.

This explicitly refines the foundation's eager "rebuild all analyses" policy:
all old facts remain invalidated after mutation, and every needed fact is freshly
computed against the new generation before it can authorize publication.
Update the analysis and checked-rewrite contracts in this commit.

**Checks:** compare eager test-oracle results with demand-driven results on the
existing CFG fixtures and every exposed query; include unreachable/foreign/stale
sites, partial writes, alias may-writes, helper clobbers and ABI entry values.
Work counts must show unused solvers absent and repeated queries computed once.
Run checked-rewrite rollback controls, affected root integration and scoped
native analysis/replay/forwarding tests.
**Exit:** unchanged query results and transaction safety with no unnecessary
home/machine liveness solves for the sole production rule.

## Slice 3 — One original-stream reconstruction and one final authorizer

Keep the current projected selection semantics. Simplify `pilot::apply` into
three explicit operations using existing typed records:

1. Validate the planned candidates against the original projected selection:
   exact ConsumeWord inputs, actual addressing form, complete request, unique
   increasing candidate order, and Code reconciliation. Missing or malformed
   records remain hard errors.
2. Expand the complete original stream in one traversal, inserting every
   projected-away actual LDA. Construct one old-to-new ordinal map, reindex all
   request/end/parent references once, then build one scratch selected routine
   and replay/layout it. Remove the reverse per-candidate `insert_load` loop and
   its repeated whole-stream cloning/CFG construction.
3. Rediscover each actual load against the current generation and let the
   existing sealed rule/driver authorize removal. A blocked final proof keeps
   the actual LDA and verified continuation. Other candidates still get fresh
   sites after any accepted edit.

Remove the preliminary whole-routine Context and repeated `adjacent_load` proofs
from the adapter. The selector already establishes its local projection; the
driver proves the actual removal. Preserve rejection observations by retaining
the planning blocker's diagnostic, or classifying it once at that boundary.
Such diagnostics never grant permission. Validate exact reason/count equality
against the existing 102-request inventory, including the two non-temporary
requests; do not create a second semantic matcher just to reconstruct messages.

Final ownership/equivalence failures must still be detected by the driver. The
test that withholds allocation identity after planning must still emit the
original LDA. Projection success is not final permission, and projection failure
must still consume the witness and preserve the existing X fallback behavior.
Remove the migration-only runtime assertion comparing a second proof against a
failed projection; reference equality tests retain coverage of missed cases.

Keep original projected ordinals only for historical observations. They are not
current proof sites. Centralize expansion/reindexing and candidate rediscovery;
validate request/load correspondence rather than trusting encoded PCs or stale
site arithmetic. Reuse the driver's existing symbolic reindexing logic where
possible. Any helper left solely for synthetic tests must be test-scoped.

**Checks:** zero/one/many candidates; mixed accepted/blocked results; multiple
requests in one source span; branches, nested requests, labels, return/PER fixups;
wrong operands, duplicates, missing ends, corrupted bytes and withheld ownership.
Compare the one-pass expansion with the old reconstruction in tests before
deleting the old production implementation. Preserve failed-consumption, X,
frame/incoming behavior, the 75 accepted / 27 blocked decisions and all existing
output/trace/metadata fields. Re-run the full native debug/release suites and
frozen corpus gate for this change to production orchestration.
**Exit:** no adapter-level duplicate equivalence pass, no per-load restoration
edits, and one authoritative removal path. The one necessary projection bridge
remains documented rather than hidden behind another representation.

## Slice 4 — Use the immutable input's verified CFG during replay

Audit every way a `SelectedRoutine` can be constructed or edited. Its private
records and CFG must only be published together after successful validation.
Then remove the redundant `SelectedCfg::build` at the start of `replay::walk`;
both full and prefix walks consume the existing immutable verified input.

Keep original Code reconciliation at transaction entry because Code bytes and
position metadata can be altered independently. Keep site and reachability
validation, fresh replay of compiler decisions, exact action/child comparisons,
construction/validation of newly edited and replayed output, and final layout
reconciliation. An existing input CFG never licenses an edited output.

Do not introduce a caller-provided "already validated" flag or persistent cache.
If the construction audit finds a mutable escape, close it before removing the
duplicate validation. Do not extend this slice into skipping all reconciliation
or reusing old computed state.

**Checks:** constructor/edited-stream rejection controls for malformed control
flow and request structure, stale sites, altered Code, deterministic replay and
failed replay rollback. Work counts show no CFG construction merely to begin a
prefix/full walk; edited/final-output CFG validation remains. Run affected
root/native replay, control-flow, o65 and preemption coverage.
**Exit:** validation belongs to immutable construction and publication boundaries,
with no duplicate input-CFG build on each replay.

## Slice 5 — Final qualification and architecture report

Qualify the final compiler in native debug/release and an isolated CRLF rebuild.
Require all existing 658 artifacts and full historical corpus records to remain
equal, including exactly the known optimized vbcc `unlink` vector-0 failure.
The comparison checker may retain its existing listing source-path normalization;
no additional normalization or golden refresh may hide a difference.

Re-run host measurements against the frozen slice-0 compiler with identical
toolchain/features and no concurrent build/qualification. Report absolute times,
ratios, per-child peak RSS and the size ladder. Save work-count deltas separately
from historical artifact schemas. Do not require a particular speedup; explain
any regression and verify that the promised duplicated operations were removed.

Update the emission contract, state-tracker design, analysis/checked-rewrite
documents and quality plan around the final ownership. Add a qualification
record with exact source/tool/artifact hashes. Retain all mutation controls;
new controls cover lazy-query demand, original-stream expansion and constructor
validation. Document remaining whole-routine replay/postcondition costs without
automatically proposing another abstraction to reduce them.

**Exit:** the planned dead/duplicate paths are deleted, one checked production
route remains, current-output and safety gates pass, and actual host costs are
reported. Remove only owned scratch build checkouts after retaining evidence.

## Validation schedule and limits

Use the established commands in the
[foundation implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md#validation-schedule).
Run native library and directly affected tests at each slice. Broaden at the
production-orchestration change and final qualification; avoid repeating full
passing matrices after documentation-only edits. Extend only the affected
measurement-tool tests when its inputs or aggregation change.

The final gate includes the affected root integration targets, all native
debug/release tests, isolated CRLF source rebuilding, the 28 Action raw/optimized
builds, complete comparison debug/release records, source-stability mutations,
and the new focused controls. Source/fixture changes during a native run must
still cause qualification to reject its manifest. New generated text fixtures
must pass LF and CRLF through their real compilation path.

This remains an emission-only task. Crossing NIR/semantic/verifier boundaries
requires a separate change with the contributor-mandated snapshots, sweep and
full root tests.

Full per-edit replay/layout, the strict decreasing-byte metric and definition
postchecks remain. Reusing the just-built post-edit analyses, batching multiple
edits, replacing prefix replay with transferred snapshots, or removing the
projection bridge would require additional lifetime/selection-dependency proofs.
They are deferred. The concrete simplifications above do not need those designs.
