# MIR6502 phase tracing implementation plan

Prepared 2026-11-13. Status: implementation in progress; Slices 1–3 complete.
Source baseline: `c00f37a84b8c0943c81565723420ace2c0d4f8d2`.

Implement in six vertical slices. Commit each slice after its checks pass.
This plan implements the diagnostic follow-up to the
[optimization pipeline audit](MIR6502_OPTIMIZATION_PIPELINE_AUDIT.md).

The deliverable is an opt-in trace that explains where a value acquired a
memory home, how that home changed, why cleanup retained its stores, and
whether a later transformation removed its consumers. Generated code,
optimization decisions, pass order and ordinary compiler output must remain
unchanged. The first investigation covers TN's six apparently unread spill
stores and a small corpus of unrelated compiler behaviors.

This is enough scope for an implementation plan. It adds diagnostics around
existing representations and proofs; it does not introduce a new IR boundary
or optimization algorithm requiring a separate design note.

## Decisions to keep the work bounded

1. Reuse existing MIR printing, home planning/remap tracking, effect
   classification, rewrite results and emission maps. Add structured diagnostic
   adapters where needed; do not parse their display strings back into facts.
2. Keep trace state separate from `Mir6502Config` and executable IR. Thread one
   compilation-local diagnostic context explicitly through the MIR6502 path.
   Existing public entry points delegate with tracing disabled. No process
   global recorder, environment-selected trace state or thread-local session.
3. Observe the actual compilation. Do not run an extra materialization,
   optimizer pass, emission probe or proof search merely to produce a trace.
   Existing trial/replay work remains unchanged and is identified in the trace.
4. Limit detailed snapshots and home events to selected routines when requested.
   Filtering never changes optimization, callee analysis or cost evaluation.
5. Use generation-scoped operation coordinates and explicit remap records.
   Do not add persistent operation IDs throughout MIR, a general provenance
   graph, a proof dependency registry, a UI or another analysis framework.
6. Leave the historical materialized-MIR dump at its existing boundary. Add a
   distinct `pre-emission` snapshot after checked-range retention, taken from
   the exact MIR passed to final emission.
7. The first CLI integration is a side artifact of the developer
   `actionc-emit` full code-generation path. Public compiler-option redesign
   and a new `actionc` compile-mode option are outside this slice of work.

Sources to reuse:
[home census/fates](../src/mir6502/materialize/home_census.rs),
[rewrite results](../src/mir6502/rewrite/driver.rs),
[proof blockers](../src/mir6502/rewrite/context.rs),
[peephole reports](../src/mir6502/materialize/stats.rs),
[MIR printer](../src/mir6502/printer.rs),
[emission summary](../src/mir6502/emit.rs).
The scope also follows the repository's
[observability direction](OBSERVABILITY_NORTH_STAR.md).

## Trace contract

### Identity and selected history

Add a small `mir6502::trace` module with versioned diagnostic records and a
disabled context. Concrete Rust names may follow repository conventions;
the following distinctions are required:

| Identity or record | Required meaning |
| --- | --- |
| Compilation | One backend invocation, its NIR input identity, effective configuration, origin and runtime. Record source/compiler hashes in the CLI/harness manifest where available. |
| Attempt | A materialization, candidate application, repair or replay, with a compilation-local ID, parent attempt, stage and purpose. |
| Selection | Which attempt supplied the returned program or each retained routine. Distinguish accepted, rejected, partially accepted, superseded, skipped and failed work. |
| Snapshot | Unique ID, attempt, stage and routine/program scope. A routine dump during a per-routine loop must not claim that every routine has reached that stage. |
| Operation point | Snapshot/routine/block/op index, or driver-invocation ID plus its existing generation/routine/block/op index. Generation alone is insufficient because separate drivers restart their counters. |
| Home | Typed routine-owned param/local/spill/virtual-ZP identity and byte offset, or explicitly fixed storage. Names are display metadata. A reused spill ID is not an original value ID. |
| Layout | Emission invocation and finalized snapshot identity, origin/runtime and resolved physical storage. Planning addresses are labeled provisional or omitted. |

Do not equate `report == true` with acceptance. Existing gates can retain a
trial built with reporting disabled, rerun the selected source for reporting,
or restore only some routines. The inliner also rematerializes its selected
input and can move helpers. Selection records must identify what actually
reaches the next stage. A rejected parent makes its nested successes
speculative; they must not appear as accepted savings.

Use an append-only attempt/event journal and explicit selection records.
Readers can follow the selected history without pretending all trials form
one linear pass sequence. Whole-program snapshots at transaction boundaries
are the authority for composed results after partial rollback. Preserve the
actual rematerialization schedule instead of adding a special trace replay.

### Snapshot boundaries

Use the audit's C/T labels in documentation; use descriptive stage names in
the trace format.

| Boundary | Capture |
| --- | --- |
| Backend entry | Initially lowered MIR; effective configuration and identity of the verified NIR input. CLI input is optimized NIR; direct API input must not be mislabeled optimized. |
| Resolved interfaces | Program after runtime interface resolution; selected source/pre-home input for each materialization attempt. |
| Before home assignment | Each routine after pre-home cleanup, before home census/plan application. |
| After temp materialization | Each routine after residual temp/Boolean/address homes are explicit, before the first subsequent home promotion/cleanup. |
| Core cleanup boundaries | Each routine after the first and second post-home cleanup groups, after ZP remapping, and at core `PostHome` after final cleanup. Record ABI-home elision/remap events between these snapshots. |
| Helper resolution | Complete materialized program after runtime helper binding. |
| Late transactions | Outcome and returned program after each enabled T1–T9 transaction; a disabled/ineligible stage has an explicit skipped/unchanged record. |
| Checked-range retention | Outcome after T10, including unavailable projection or parent-feature gating. |
| Pre-emission | Complete verified program immediately before final emission, including T10. |
| Final emission | Final instruction ranges, storage symbols/addresses, image identity and output-size measures from that successful emission. |

An unchanged snapshot may refer to a prior snapshot instead of repeating its
payload, provided the stage/outcome is still recorded. Routine filtering must
retain program-level attempt, selection and completion information.

The first implementation does not add a second NIR dump or alter NIR printers.
Existing NIR inspection remains available separately; range projection events
may name the typed NIR receipts/call coordinates they already consume.

### Home history and reasons

Record information already produced by the compilation:

- A home-plan decision with temp/lane, current producer/use coordinates and
  the existing reason, such as cross-block use, call liveness or unsupported
  consumer. Identify pre-existing source/ABI storage separately from new spills.
- Initial temp-to-spill assignment; all coloring and ZP remaps, including
  many-to-one reuse; home elision/pruning and parameter ABI-only decisions.
- Per-snapshot explicit byte reads, writes, address references and terminator
  uses, derived from the shared effect classifier. Report opaque/indirect
  observations separately: zero explicit reads does not prove a store dead.
- Accepted rewrite family and affected range/removed definitions from the
  actual applied plans. Candidate and overlap-rejected plans are distinct.
- Existing typed blocker/reason and point when a cleanup proof refuses a
  deletion. Do not create a new alias/liveness analysis to improve diagnostics.

For the compatibility dead-spill scan, expose its existing stopping reason and
point: explicit read, unknown spill reader, terminator read, or no read found
through its current traversal. Preserve traversal order, short-circuiting and
deletion decisions. For analyzed private-store deletion, retain the actual
proof blocker before the reporting layer reduces it to a string.

If a matcher was not run, did not recognize the shape, or exposes no more
specific reason, say so. Do not relabel those cases as a successful deadness
proof or a physical alias. A store's initial need and its final retention
reason are separate observations and may differ.

Use explicit rewrite/remap events for exact correspondence. For transformations
without an operation mapping, report that a consumer count changed between
two named snapshots; do not invent persistent identity by matching similar
printed instructions. Newly generated and inlined temps need their own IDs;
unavailable original-NIR correspondence remains unavailable.

### Emission and work accounting

Associate selected routines' final MIR operations and terminators with byte
ranges during the final `emit_routine` traversal. Include tail-call replacement
and zero-length ranges. A zero-length range means no bytes were emitted for
that operation; it is not by itself a general dead-code or redundancy reason.
This exposes an explicit final MIR load that disappears in emission without
adding optimization to the emitter.

Obtain physical storage and instruction ranges from the final emission's
existing layout/summary. Do not infer final addresses from `MaterializeLayout`
or run another emission to recover them. Speculative emission maps have their
own invocation IDs and must never be presented as the final layout. Mark the
final artifact complete only after relocations and output construction succeed.

Keep work counters separate:

| Counter | Definition |
| --- | --- |
| Materializations | Actual entries into core materialization, including existing reporting replays. |
| Candidate attempts | Strategy/transaction attempts, grouped by family and outcome. |
| Emission invocations | Top-level cost, physical-map, image, inliner and final emission calls, identified by purpose. |
| Inner emission probes | Code-size/branch-relaxation probe traversals inside one emission invocation. |
| Analysis builds | Existing rewrite-driver build counts, scoped to its invocation. Do not call these all compiler analyses; uninstrumented analyses are explicitly outside the counter. |

No timers or counters may influence profitability. Measure wall time outside
the compiler in the qualification harness; keep deterministic trace content
free of timestamps, absolute workspace paths and timings.

### Artifact and resource policy

The backend emits diagnostic records to a caller-owned collector/sink; it
does not choose paths or write files itself. The CLI writes a versioned
`manifest.json`, `events.jsonl`, existing-format `.mir` snapshots and a concise
home summary under the requested directory. The manifest distinguishes
complete, truncated, compilation-failed and trace-write-failed runs and lists
the selected final snapshot and image identity when available.

Default to a 32 MiB detailed-trace budget per compilation, including snapshot
payloads. Reserve bounded space for completion/truncation metadata. Stop
collecting detail when the budget is reached and report the first omitted
stage; do not silently present a partial trace as complete. Use routine
filtering for TN and keep large generated bundles under ignored `build/`.
No unbounded in-memory MIR clones or buffering of every trial program.

Trace-disabled compilation must perform no trace formatting, snapshot cloning,
extra analysis or file I/O. Trace write failures stop collection and surface
through diagnostic status, without changing backend selection or fallback.
The CLI reports a requested artifact write failure as an error; it must not
claim the trace was saved. Ordinary compiler diagnostics retain their meaning.

## Implementation status

| Slice | Status | Validation |
| --- | --- | --- |
| 1 | Complete | Trace unit test; two integration tests across cart/standalone, default/optimized settings and LF/CRLF; 48 pre-change artifacts and four API outputs (including relocations) unchanged. |
| 2 | Complete | Scoped trial/return/selection records, measured work and actual replay boundaries; independent trace-on/off work observer; nested-inlining integration check; 48 frozen artifacts unchanged. Remaining corpus outcome coverage is qualified in Slice 6. |
| 3 | Complete | Four trace integration tests, existing MIR fixture snapshots, independent work-count test, and all 48 frozen artifacts unchanged. A focused indexed-store fixture covers routine coloring and distinct definitions sharing storage. |
| 4–6 | Pending | |

The pre-change compiler and artifacts are retained under ignored
`build/mir6502-phase-trace/baseline/` for differential qualification.

## Slice 1 — Establish the final boundary and trace session

Suggested commit: `mir6502: expose the final emission boundary for diagnostics`

- Freeze pre-change outputs for the small corpus described below: compiler
  revision, input hashes, runtime/origin/config and generated artifacts.
- Add the trace module, disabled context, in-memory test collector, IDs and
  versioned event envelope. Existing backend APIs keep their signatures.
- Add an additive traced generation entry point. Factor the current
  `generate_output_with_config_and_runtime` preparation sequence into one
  internal path used by both entry points: lowering, materialization, optional
  range projection/retention, `PreEmission` verification and final emission.
  Preserve its order, calls and feature gates exactly.
- Capture lowered and final pre-emission MIR and completion status. Keep
  `materialize_program_with_origin_and_runtime` and the old dump semantics.

Files: `src/mir6502/mod.rs`, new `src/mir6502/trace.rs`, focused tests.

Acceptance: existing and traced entry points return equal `CodegenOutput`,
load-file bytes and diagnostics for ordinary and checked-range fixtures.
The final snapshot contains the checked-retention result when it applies,
and the baseline when it is disabled or its proof fails. Verify nested and
back-to-back compilations cannot leak trace state.

## Slice 2 — Identify trials, selection and measured work

Suggested commit: `mir6502: distinguish selected and speculative trace attempts`

- Thread the diagnostic context through the actual inlining/nested cost gates,
  core materialization and late transactions, without changing their config,
  report switches, cost probes or fallback branches.
- Add attempt start/end and selection records at the existing accept/rollback
  decisions. Include per-routine selection for partial batches and a snapshot
  of the resulting composite program.
- Count actual materialization/emission/probe invocations and reuse existing
  rewrite-driver analysis-build counts. Record purpose at callers; avoid
  counting one delegated emission wrapper twice.
- Capture resolved-interface, selected-input, helper-resolution and T1–T10
  boundaries. A disabled or unchanged stage remains visible as metadata.

Files: `src/mir6502/mod.rs`, affected cost gates under `materialize/`,
`emit.rs`, `rewrite/driver.rs` and the trace module. Keep changes to gates
mechanical; do not consolidate their orchestration in this slice.

Acceptance: exercise accepted, rejected, partial-rollback and nested trials,
plus the inliner's existing final rematerialization. Selected history resolves
to the actual returned program. Rejected work is counted but never reported
as accepted savings. Trace on/off produces equal artifacts and equal amounts
of compiler work at the instrumented boundaries, using test-only observers
independent of the trace sink where necessary.

## Slice 3 — Capture core phases and home transitions

Suggested commit: `mir6502: trace home assignment and storage remapping`

- Add the core snapshot boundaries from the table, with routine scope inside
  per-routine loops and program scope at actual whole-program boundaries.
- Expose structured diagnostic views of existing home-plan decisions and
  `HomeFateTracker` updates; record temp-to-spill creation, coloring, ZP
  remapping, ABI-home elision and pruning at their existing call sites.
- Add explicit home access counts through shared effect classification.
  Include frame declarations in structured records: the current MIR text
  printer alone does not show every uninitialized home declaration.
- Reuse MIR formatting. If filtering needs a routine-format helper, extract
  it without changing existing printed output or NIR printing.

Files: `materialize.rs`, `home_census.rs`, `temps.rs`, `spills.rs`, `abi.rs`,
`printer.rs` and trace adapters, limited to observations at existing decisions.

Acceptance: cover a register-elided byte, a cross-block/call-live spill,
two values colored into one home, two definitions of the same home, a ZP
remap and ABI-only parameter metadata. Every reported remap joins the correct
snapshot identities; untouched aliases/source homes stay distinct from
compiler-private spills. Tracing does not change existing MIR text.

## Slice 4 — Explain retained stores and connect final bytes

Suggested commit: `mir6502: report cleanup blockers and emitted operation ranges`

- Attach actual rewrite application/blocker events to scoped points in the
  shared driver, without broadening proof queries or matcher eligibility.
- Expose retention outcomes from the compatibility spill scan and existing
  analyzed private-store proof. Keep diagnostic observation separate from
  the Boolean/result that controls deletion.
- Record final MIR operation/terminator byte ranges and physical home
  placements from the successful final emission. Keep speculative probe
  ranges separate; associate empty ranges explicitly with the final MIR op.
- Build the home summary from typed records, showing initial demand, remaps,
  last observed consumers, cleanup outcome and final allocation. Report a
  late consumer disappearance as an observation, not permission to delete.

Files: `rewrite/driver.rs`, relevant diagnostic adapter in `rewrite/context.rs`,
`materialize/dead_spills.rs`, `emit.rs` and trace formatting.
Avoid modifying shared `TrackedEmitter` optimization behavior; operation-range
observation belongs in the MIR6502 emitter wrapper for this implementation.

Acceptance: explicit successor/loop read, covering overwrite, unknown helper
or machine-block reader, address/alias blocker and a retained frame after
consumer removal. Include a fixture where a final MIR load emits no bytes.
Trace observations must agree with the existing cleanup outcomes; unknown
effects must not be displayed as zero reads or proven deadness.

## Slice 5 — Add the developer CLI artifact

Suggested commit: `cli: write opt-in MIR6502 phase traces`

- Add `actionc-emit --trace-mir6502 <directory>` as a side artifact for the
  existing full MIR6502 code-generation modes. It uses the traced backend
  entry point from Slice 1 and compiles only once.
- Add repeatable `--trace-routine <name-or-id>` and optional
  `--trace-max-bytes <bytes>`. Resolve names to MIR routine IDs for filtering;
  ambiguous/unknown names are diagnostic errors, never optimizer inputs.
- Reject combinations with other backends or early-exit IR dump modes;
  explain that full code generation is required for the final trace.
- Write the bundle through the CLI-owned sink with bounded storage, explicit
  completion status and no extra stdout/stderr noise on successful ordinary
  output. Use a fresh destination and do not mix old/new trace artifacts.
- Do not enable `ACTIONC_MIR6502_PEEPHOLES` or force its `Sites` mode to obtain
  trace data: existing report mode can cause additional materialization.

Files: `src/cli.rs`, trace writer/serialization module and a new focused
`tests/mir6502_phase_trace.rs` integration target. No new dependency is needed
for JSON; `serde`/`serde_json` already exist.

Acceptance: output equality with and without tracing; deterministic bundles;
routine filtering without compilation changes; unsupported combinations;
budget truncation; invalid destination/write failure; failed compilation;
and compatibility with existing peephole-report settings. Test real LF and
CRLF source paths, normalizing host text only where line endings are irrelevant.

## Slice 6 — Qualify and diagnose the fixed corpus

Suggested commit: `mir6502: qualify phase tracing and record pipeline findings`

- Add a small reproducible harness, proposed as
  `tools/compare6502/phase_trace.py`, that records input/config hashes, compares
  pre-change, trace-disabled and trace-enabled artifacts, validates trace
  completeness, and produces a concise diagnosis table. Provide `--check`.
- Reuse one build target/cache and existing fixtures/oracles. Keep trace
  bundles out of version control; commit compact receipts and findings under
  proposed `docs/benchmarks/6502-phase-tracing/`.
- Trace TN's four `FormatDirectoryRow` and two `SetWin` spill-store candidates.
  Locate them from the frozen assembly/map and typed final MIR, then follow
  scoped home/store identities. Never put TN names, spill numbers or numeric
  listing addresses into compiler tracing decisions.
- Explain each candidate using evidence: original home demand, last explicit
  consumer, cleanup blocker at that stage, later consumer changes and final
  allocation/emission. Separate several applicable causes. If correspondence
  or a reason is missing, name the missing diagnostic and complete that narrow
  observation before claiming the investigation finished.
- Compare unrelated cases and recommend one evidence-backed consolidation
  slice. This commit does not implement that optimization or claim byte savings.

Start with these maintained inputs; add only small focused cases missing from
their behavior coverage:

| Input | Purpose |
| --- | --- |
| [TN](../samples/tn/modern/TN.ACT) | Six reviewed stores; real late-stage interactions and existing UI/runtime oracle. |
| [Home value invalidation](../fixtures/runtime/home_value_invalidation.act) | Copies among homes, alias-sensitive invalidation and calls. |
| [Argument staging](../fixtures/runtime/argument_staging.act) | ABI setup, capture lifetime and late removal of traffic. |
| [Word load sharing](../fixtures/runtime/word_load_sharing.act) | Multi-destination word values and a frame-preserving late transaction. |
| [Pointer invariant loop](../fixtures/runtime/pointer_invariant_loop.act) | CFG edges, pointer setup and retained state. |
| [Checked pointer ranges](../fixtures/runtime/checked_pointer_ranges.act) | Final-MIR endpoint after T10, successful and rejected range proofs. |
| [Captured call arguments](../surveys/tn/reproducers/captured-call-arguments.act) | A pre-home capture/cost gate whose trial history must be distinguished. |

Cover cart and standalone. Use the baseline's normal origin and one displaced
origin on the small placement-sensitive fixtures. Use both default and
optimized configurations on focused fixtures; avoid a full cross-product of
all feature flags. Include an inlining/cost-rejection fixture to exercise
nested trial history even if TN's current shape does not do so.

## Validation policy and completion criteria

Each implementation slice runs its focused unit/integration checks and checks
the frozen differential artifacts affected by that slice. At completion:

1. **Artifact invariance:** pre-change, new trace-disabled and new trace-enabled
   builds have identical load-file bytes, code/data maps, relocations and
   listings for fixed inputs/settings. A trace-on/off comparison alone could
   miss a refactor that changed both paths, so retain the pre-change reference.
2. **Trace integrity:** selected history, scoped points, home remaps, final
   pre-emission snapshot and finalized layout agree. Only complete traces
   qualify the diagnosis. Reports make speculative/rejected work explicit.
3. **No hidden compiler work:** instrumented materialization/emission/probe
   counts are equal with tracing on and off. Disabled mode adds no snapshot
   allocation or formatting. Report trace size, peak collector use and harness
   wall time separately; do not turn noisy timings into correctness thresholds.
4. **Correctness checks:** run focused MIR6502 library tests, the new trace
   integration target, existing CLI emit-mode and rewrite-inventory tests, and
   the affected home/staging/pointer/checked-range/word-sharing integration
   targets. Run matching MIR6502 VM targets for the frozen corpus and the
   maintained TN runtime/UI oracle. Reuse the successful bytes rather than
   creating another implementation of their behavior checks.
5. **Cross-platform artifacts:** test LF and CRLF through the actual source and
   trace parsing/writing path. Preserve binary/ATASCII bytes. Include MADS
   roundtrip checks on the traced listing samples where the maintained harness
   already supports them.
6. **Actionable diagnosis:** all six TN candidates and the unrelated controls
   have supported retention/removal explanations. The report identifies the
   common mechanism, if any, for the next implementation plan; it does not
   assume a final cleanup pass is the answer.

Backend-only instrumentation uses MIR6502 test scope. If implementation
changes shared compiler contracts or a shared emitter instead of merely
observing it, broaden checks to the actual consumers under `AGENTS.md`.
NIR/semantic/verifier/printer contract changes are outside this plan and
would require the shared-contract fixture, sweep and full test checks.

Update this plan's slice status, the pipeline audit's diagnostic limitations
and the documentation index as the work lands. Keep the resulting contract
focused on trace identity, boundaries and completeness; store measurements
and investigation evidence in the benchmark report.
